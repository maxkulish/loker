//! Summary writer: emits `runs/<id>/summary.json` at run completion.
//!
//! Follows the same pattern as `trace` — a `SummarySink` trait with
//! file-backed and in-memory implementations.
//!
//! The main entry point is [`SummaryWriter::finalize`] which:
//! 1. Reads trace.jsonl to aggregate token usage per backend+model
//! 2. Collects per-phase outcomes from marker files
//! 3. Computes cost via PriceTable
//! 4. Builds the Summary struct
//! 5. Writes summary.json via atomic write
//! 6. Registers the artefact in the manifest

use std::path::Path;
use std::sync::Mutex;

use serde::Serialize;

use crate::manifest::{Kind, Manifest, ManifestEntry, Producer};
use crate::run_state::atomic_write;

pub mod prices;
pub mod reader;

pub use prices::PriceTable;
pub use reader::{BackendUsage, TraceReader, TraceReaderError};

/// Per-phase outcome status for the summary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    Completed,
    Failed,
    Skipped,
}

/// Overall run status for the summary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Success,
    Failed,
    Partial,
    Aborted,
}

/// Aggregate token/cost totals for the summary.
#[derive(Debug, Clone, Serialize)]
pub struct Totals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// Per-phase entry in the summary.
#[derive(Debug, Clone, Serialize)]
pub struct PhaseSummary {
    pub phase: String,
    pub status: PhaseStatus,
    pub attempts: u32,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
}

/// The summary.json envelope, matching `docs/schemas/summary.schema.json`.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    #[serde(rename = "loker.run_id")]
    pub run_id: String,
    pub schema_version: u32,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: u64,
    pub status: RunStatus,
    pub totals: Totals,
    pub backends: Vec<BackendUsage>,
    pub phases: Vec<PhaseSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_budget_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_warning: Option<bool>,
}

/// Error type for summary operations.
#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Trace parse error: {0}")]
    TraceParse(#[from] TraceReaderError),
    #[error("Manifest error: {0}")]
    Manifest(#[from] crate::manifest::ManifestError),
}

/// Trait for emitting run summaries.
///
/// Follows the same pattern as `TraceSink` — implementations may write to
/// disk or capture in memory for tests.
pub trait SummarySink: Send + Sync {
    fn finalize(&self, summary: &Summary, run_dir: &Path) -> Result<(), SummaryError>;
}

/// File-backed `SummarySink` that writes `runs/<id>/summary.json`
/// via atomic write (tmp + fsync + rename) and registers the artefact
/// in the manifest.
pub struct SummaryWriter {
    fsync: bool,
}

impl SummaryWriter {
    pub fn new(fsync: bool) -> Self {
        Self { fsync }
    }

    /// Build and write the summary for a completed run.
    ///
    /// Orchestrates trace reading, cost computation, phase collection,
    /// summary.json writing, and manifest registration.
    pub fn write_summary(
        &self,
        run_dir: &Path,
        manifest: &mut Manifest,
        trace_path: &Path,
        cost_budget_usd: Option<f64>,
    ) -> Result<Summary, SummaryError> {
        // 1. Read trace and aggregate token usage
        let mut backends = TraceReader::aggregate(trace_path)?;

        // 2. Compute costs from price table
        //
        // Use the built-in prices.toml (compiled into the binary) as the primary
        // source. This is always available regardless of CWD or deployment layout.
        //
        // For test overrides, probe the run_dir for a local prices.toml first:
        //   - run_dir/docs/prices.toml         (flat test dir)
        //   - run_dir/../../docs/prices.toml   (production: run is <root>/runs/<uuid>/)
        //
        // Unknown models still produce cost_usd: None.
        // This avoids the fragile CWD-relative path probing that silently broke costs
        // in deployed installs (see F3 rework).
        let price_table = [
            run_dir.join("docs/prices.toml"),
            run_dir.join("../../docs/prices.toml"),
        ]
        .into_iter()
        .find(|p| p.exists())
        .map(|p| PriceTable::load(p.as_path()))
        .unwrap_or_else(PriceTable::builtin);

        let mut total_cost_usd = 0.0_f64;
        let mut any_cost_known = false;
        for backend in &mut backends {
            match price_table.compute_cost(
                &backend.name,
                &backend.model,
                backend.input_tokens,
                backend.output_tokens,
            ) {
                Some(cost) => {
                    backend.cost_usd = Some(cost);
                    total_cost_usd += cost;
                    any_cost_known = true;
                }
                None => {
                    // cost_usd stays None — cost_unknown
                }
            }
        }

        // 3. Collect per-phase outcomes from markers
        let phases = collect_phases(run_dir);

        // 4. Compute duration and timestamps from marker data
        let now = chrono::Utc::now();
        let finished_at = compute_finished_at(run_dir).unwrap_or_else(|| now.to_rfc3339());
        let started_at = get_run_start_time(run_dir).unwrap_or_else(|| now.to_rfc3339());
        let duration_ms = compute_duration_ms(run_dir);

        // 5. Determine run status
        let status = compute_run_status(&phases);

        // 6. Build totals
        let totals_input: u64 = backends.iter().map(|b| b.input_tokens).sum();
        let totals_output: u64 = backends.iter().map(|b| b.output_tokens).sum();
        let totals_cost = if any_cost_known {
            Some(total_cost_usd)
        } else {
            None
        };

        // 7. Compute budget warning
        let budget_warning = match (cost_budget_usd, totals_cost) {
            (Some(budget), Some(cost)) if cost > budget => Some(true),
            _ => None,
        };

        // 8. Build Summary
        let run_id = manifest.run_id.clone();
        let summary = Summary {
            run_id,
            schema_version: 1,
            started_at,
            finished_at,
            duration_ms,
            status,
            totals: Totals {
                input_tokens: totals_input,
                output_tokens: totals_output,
                cost_usd: totals_cost,
            },
            backends,
            phases,
            cost_budget_usd,
            budget_warning,
        };

        // 9. Write summary.json via SummarySink
        self.finalize(&summary, run_dir)?;

        // 10. Register in manifest (idempotent — replaces on re-finalize)
        let json_bytes = serde_json::to_string_pretty(&summary)?;
        let entry = ManifestEntry::from_payload(
            "summary.json",
            Kind::SummaryJson,
            1,
            Producer::Single,
            None,
            None,
            json_bytes.as_bytes(),
        );
        manifest.replace_by_name(entry, &run_dir.join("manifest.json"))?;

        Ok(summary)
    }
}

impl SummarySink for SummaryWriter {
    fn finalize(&self, summary: &Summary, run_dir: &Path) -> Result<(), SummaryError> {
        let path = run_dir.join("summary.json");
        let json_bytes = serde_json::to_string_pretty(summary)?;
        atomic_write(&path, json_bytes.as_bytes())?;
        if self.fsync {
            // atomic_write already does fsync + rename
        }
        Ok(())
    }
}

/// In-memory test sink that captures summaries (follows `InMemorySink` pattern).
pub struct InMemorySummarySink {
    summaries: Mutex<Vec<Summary>>,
}

impl InMemorySummarySink {
    pub fn new() -> Self {
        Self {
            summaries: Mutex::new(Vec::new()),
        }
    }

    pub fn summaries(&self) -> Vec<Summary> {
        self.summaries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn clear(&self) {
        self.summaries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

impl Default for InMemorySummarySink {
    fn default() -> Self {
        Self::new()
    }
}

impl SummarySink for InMemorySummarySink {
    fn finalize(&self, summary: &Summary, _run_dir: &Path) -> Result<(), SummaryError> {
        self.summaries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(summary.clone());
        Ok(())
    }
}

// ─── Phase collector ─────────────────────────────────────────────────────

/// Collect per-phase outcomes from `run_dir/markers/` directory.
///
/// Reads `*.completed`, `*.failed`, `*.started.<N>` marker files to determine
/// each phase's status, attempt count, duration, and timestamps.
///
/// Timestamps are sourced from the marker JSON bodies:
/// - `StartedMarker.started_at` — per-attempt start
/// - `CompletedMarker.completed_at` — successful completion
/// - `FailedMarker.failed_at` — terminal failure
///
/// Phase duration is computed from first started marker to terminal marker
/// (completed or failed) of the same phase. Run-level timestamps are derived
/// from the earliest started marker across all phases and the latest terminal
/// marker across all phases.
fn collect_phases(run_dir: &Path) -> Vec<PhaseSummary> {
    let markers_dir = run_dir.join("markers");
    if !markers_dir.is_dir() {
        return Vec::new();
    }

    let mut phases: Vec<PhaseSummary> = Vec::new();

    // First pass: collect all started markers (regardless of read_dir order).
    // Use a separate read_dir iteration so terminal marker parsing never races
    // with started marker collection.
    let mut started_markers: std::collections::HashMap<String, Vec<chrono::DateTime<chrono::Utc>>> =
        std::collections::HashMap::new();

    if let Ok(started_entries) = std::fs::read_dir(&markers_dir) {
        for entry in started_entries.flatten() {
            let file_name = match entry.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };
            if let Some((phase, attempt_str)) = file_name.rsplit_once(".started.") {
                if attempt_str.parse::<u32>().is_err() {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(marker) =
                        serde_json::from_str::<crate::run_state::markers::StartedMarker>(&content)
                    {
                        started_markers
                            .entry(phase.to_string())
                            .or_default()
                            .push(marker.started_at);
                    }
                }
            }
        }
    }

    // Second pass: process terminal markers and compute phase durations.
    // duration_ms uses the earliest started marker (min) for the phase,
    // not insertion order (read_dir is nondeterministic).
    if let Ok(terminal_entries) = std::fs::read_dir(&markers_dir) {
        for entry in terminal_entries.flatten() {
            let file_name = match entry.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };

            let path = entry.path();

            // Skip started markers — handled in first pass
            if file_name.contains(".started.") {
                continue;
            }

            // Parse terminal markers: <phase>.completed or <phase>.failed
            let (phase, status, attempts, phase_end_time) = if file_name.ends_with(".completed") {
                let phase = file_name.trim_end_matches(".completed").to_string();
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(marker) =
                        serde_json::from_str::<crate::run_state::markers::CompletedMarker>(&content)
                    {
                        // attempt field from marker is 0-based; display as 1-based
                        let display_attempts = marker.attempt + 1;
                        (
                            phase,
                            PhaseStatus::Completed,
                            display_attempts,
                            Some(marker.completed_at),
                        )
                    } else {
                        // Fallback: filename-only parse
                        (phase, PhaseStatus::Completed, 1, None)
                    }
                } else {
                    (phase, PhaseStatus::Completed, 1, None)
                }
            } else if file_name.ends_with(".failed") {
                let phase = file_name.trim_end_matches(".failed").to_string();
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(marker) =
                        serde_json::from_str::<crate::run_state::markers::FailedMarker>(&content)
                    {
                        (
                            phase,
                            PhaseStatus::Failed,
                            marker.attempts_made,
                            Some(marker.failed_at),
                        )
                    } else {
                        // Fallback: filename-only parse
                        (phase, PhaseStatus::Failed, 1, None)
                    }
                } else {
                    (phase, PhaseStatus::Failed, 1, None)
                }
            } else {
                continue;
            };

            // Compute phase duration from earliest started marker to terminal time.
            // Use min() for deterministic ordering regardless of read_dir order.
            let duration_ms = phase_end_time
                .and_then(|end| {
                    started_markers.get(&phase).and_then(|starts| {
                        starts.iter().min().map(|earliest_start| {
                            let dur = end.signed_duration_since(*earliest_start);
                            std::cmp::max(0, dur.num_milliseconds()) as u64
                        })
                    })
                })
                .unwrap_or(0);

            phases.push(PhaseSummary {
                phase,
                status,
                attempts,
                duration_ms,
                strategy: None,
            });
        }
    }

    // Sort by phase name for deterministic output
    phases.sort_by(|a, b| a.phase.cmp(&b.phase));
    phases
}

/// Get the run start time from the earliest started marker across all phases.
fn get_run_start_time(run_dir: &Path) -> Option<String> {
    let markers_dir = run_dir.join("markers");
    if !markers_dir.is_dir() {
        return None;
    }

    let mut earliest: Option<chrono::DateTime<chrono::Utc>> = None;

    if let Ok(entries) = std::fs::read_dir(&markers_dir) {
        for entry in entries.flatten() {
            let file_name = match entry.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };
            if !file_name.contains(".started.") {
                continue;
            }
            let content = match std::fs::read_to_string(entry.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let marker: crate::run_state::markers::StartedMarker =
                match serde_json::from_str(&content) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
            if earliest.map_or(true, |e| marker.started_at < e) {
                earliest = Some(marker.started_at);
            }
        }
    }

    earliest.map(|dt| dt.to_rfc3339())
}

/// Get the run finish time from the latest terminal marker (completed or failed)
/// across all phases. Falls back to the latest started marker if no terminal
/// marker exists.
fn compute_finished_at(run_dir: &Path) -> Option<String> {
    let markers_dir = run_dir.join("markers");
    if !markers_dir.is_dir() {
        return None;
    }

    let mut latest: Option<chrono::DateTime<chrono::Utc>> = None;

    if let Ok(entries) = std::fs::read_dir(&markers_dir) {
        for entry in entries.flatten() {
            let file_name = match entry.file_name().to_str() {
                Some(n) => n.to_string(),
                None => continue,
            };
            let content = match std::fs::read_to_string(entry.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if file_name.ends_with(".completed") {
                if let Ok(marker) =
                    serde_json::from_str::<crate::run_state::markers::CompletedMarker>(&content)
                {
                    if latest.map_or(true, |e| marker.completed_at > e) {
                        latest = Some(marker.completed_at);
                    }
                }
            } else if file_name.ends_with(".failed") {
                if let Ok(marker) =
                    serde_json::from_str::<crate::run_state::markers::FailedMarker>(&content)
                {
                    if latest.map_or(true, |e| marker.failed_at > e) {
                        latest = Some(marker.failed_at);
                    }
                }
            } else if file_name.contains(".started.") {
                // Fallback: if no terminal markers exist yet, use latest started
                if latest.is_none() {
                    if let Ok(marker) =
                        serde_json::from_str::<crate::run_state::markers::StartedMarker>(&content)
                    {
                        if latest.map_or(true, |e| marker.started_at > e) {
                            latest = Some(marker.started_at);
                        }
                    }
                }
            }
        }
    }

    latest.map(|dt| dt.to_rfc3339())
}

/// Compute wall-clock duration of the run from the earliest started marker
/// to the latest terminal marker (completed or failed).
fn compute_duration_ms(run_dir: &Path) -> u64 {
    let markers_dir = run_dir.join("markers");
    if !markers_dir.is_dir() {
        return 0;
    }

    let mut earliest_start: Option<chrono::DateTime<chrono::Utc>> = None;
    let mut latest_end: Option<chrono::DateTime<chrono::Utc>> = None;

    let entries = match std::fs::read_dir(&markers_dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        if file_name.contains(".started.") {
            if let Ok(marker) =
                serde_json::from_str::<crate::run_state::markers::StartedMarker>(&content)
            {
                if earliest_start.map_or(true, |e| marker.started_at < e) {
                    earliest_start = Some(marker.started_at);
                }
            }
        } else if file_name.ends_with(".completed") {
            if let Ok(marker) =
                serde_json::from_str::<crate::run_state::markers::CompletedMarker>(&content)
            {
                if latest_end.map_or(true, |e| marker.completed_at > e) {
                    latest_end = Some(marker.completed_at);
                }
            }
        } else if file_name.ends_with(".failed") {
            if let Ok(marker) =
                serde_json::from_str::<crate::run_state::markers::FailedMarker>(&content)
            {
                if latest_end.map_or(true, |e| marker.failed_at > e) {
                    latest_end = Some(marker.failed_at);
                }
            }
        }
    }

    match (earliest_start, latest_end) {
        (Some(start), Some(end)) => {
            let dur = end.signed_duration_since(start);
            std::cmp::max(0, dur.num_milliseconds()) as u64
        }
        _ => 0,
    }
}

/// Determine the top-level run status from per-phase outcomes.
fn compute_run_status(phases: &[PhaseSummary]) -> RunStatus {
    if phases.is_empty() {
        return RunStatus::Failed;
    }

    let all_completed = phases
        .iter()
        .all(|p| matches!(p.status, PhaseStatus::Completed));
    let any_failed = phases
        .iter()
        .any(|p| matches!(p.status, PhaseStatus::Failed));

    if all_completed {
        RunStatus::Success
    } else if any_failed {
        RunStatus::Partial
    } else {
        RunStatus::Aborted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_markers_no_phases() {
        let tmp = tempfile::tempdir().unwrap();
        let phases = collect_phases(tmp.path());
        assert!(phases.is_empty());
    }

    #[test]
    fn single_completed_phase() {
        let tmp = tempfile::tempdir().unwrap();
        let markers = tmp.path().join("markers");
        std::fs::create_dir_all(&markers).unwrap();
        std::fs::write(markers.join("design.completed"), "{}").unwrap();

        let phases = collect_phases(tmp.path());
        assert_eq!(phases.len(), 1);
        assert!(matches!(phases[0].status, PhaseStatus::Completed));
    }

    #[test]
    fn failed_phase_collected() {
        let tmp = tempfile::tempdir().unwrap();
        let markers = tmp.path().join("markers");
        std::fs::create_dir_all(&markers).unwrap();
        std::fs::write(
            markers.join("implement.failed"),
            "{\"error\":\"verify_failed\"}",
        )
        .unwrap();

        let phases = collect_phases(tmp.path());
        assert_eq!(phases.len(), 1);
        assert!(matches!(phases[0].status, PhaseStatus::Failed));
    }

    #[test]
    fn multiple_phases_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let markers = tmp.path().join("markers");
        std::fs::create_dir_all(&markers).unwrap();
        std::fs::write(markers.join("review.completed"), "{}").unwrap();
        std::fs::write(markers.join("design.completed"), "{}").unwrap();

        let phases = collect_phases(tmp.path());
        assert_eq!(phases.len(), 2);
        assert_eq!(phases[0].phase, "design");
        assert_eq!(phases[1].phase, "review");
    }

    #[test]
    fn started_markers_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let markers = tmp.path().join("markers");
        std::fs::create_dir_all(&markers).unwrap();
        std::fs::write(markers.join("design.completed"), "{}").unwrap();
        std::fs::write(markers.join("review.started"), "{}").unwrap();

        let phases = collect_phases(tmp.path());
        assert_eq!(phases.len(), 1);
        assert_eq!(phases[0].phase, "design");
    }

    #[test]
    fn compute_run_status_all_success() {
        let phases = vec![PhaseSummary {
            phase: "design".into(),
            status: PhaseStatus::Completed,
            attempts: 1,
            duration_ms: 100,
            strategy: None,
        }];
        assert!(matches!(compute_run_status(&phases), RunStatus::Success));
    }

    #[test]
    fn compute_run_status_partial_when_failed() {
        let phases = vec![
            PhaseSummary {
                phase: "design".into(),
                status: PhaseStatus::Completed,
                attempts: 1,
                duration_ms: 100,
                strategy: None,
            },
            PhaseSummary {
                phase: "implement".into(),
                status: PhaseStatus::Failed,
                attempts: 3,
                duration_ms: 500,
                strategy: None,
            },
        ];
        assert!(matches!(compute_run_status(&phases), RunStatus::Partial));
    }

    #[test]
    fn compute_run_status_empty_is_failed() {
        assert!(matches!(compute_run_status(&[]), RunStatus::Failed));
    }

    #[test]
    fn summary_sink_in_memory() {
        let sink = InMemorySummarySink::new();
        let tmp = tempfile::tempdir().unwrap();
        let summary = Summary {
            run_id: "test-run-id".into(),
            schema_version: 1,
            started_at: "2026-01-01T00:00:00Z".into(),
            finished_at: "2026-01-01T01:00:00Z".into(),
            duration_ms: 3600000,
            status: RunStatus::Success,
            totals: Totals {
                input_tokens: 100,
                output_tokens: 200,
                cost_usd: None,
            },
            backends: vec![],
            phases: vec![],
            cost_budget_usd: None,
            budget_warning: None,
        };
        sink.finalize(&summary, tmp.path()).unwrap();
        assert_eq!(sink.summaries().len(), 1);
        assert_eq!(sink.summaries()[0].run_id, "test-run-id");
    }

    #[test]
    fn summary_serializes_to_valid_json() {
        let summary = Summary {
            run_id: "uuid-1234".into(),
            schema_version: 1,
            started_at: "2026-05-03T12:00:00Z".into(),
            finished_at: "2026-05-03T13:00:00Z".into(),
            duration_ms: 3600000,
            status: RunStatus::Success,
            totals: Totals {
                input_tokens: 1000,
                output_tokens: 500,
                cost_usd: Some(0.05),
            },
            backends: vec![BackendUsage {
                name: "mock".into(),
                model: "m1".into(),
                input_tokens: 1000,
                output_tokens: 500,
                cost_usd: Some(0.05),
            }],
            phases: vec![PhaseSummary {
                phase: "design".into(),
                status: PhaseStatus::Completed,
                attempts: 1,
                duration_ms: 100,
                strategy: Some("single".into()),
            }],
            cost_budget_usd: Some(0.10),
            budget_warning: None,
        };
        let json = serde_json::to_string_pretty(&summary).unwrap();
        // Verify it parses back
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["loker.run_id"], "uuid-1234");
        assert_eq!(parsed["schema_version"], 1);
        assert!(parsed["totals"]["cost_usd"].is_number());
    }

    #[test]
    fn summary_writer_writes_file() {
        let tmp = tempfile::tempdir().unwrap();
        let writer = SummaryWriter::new(false);
        let summary = Summary {
            run_id: "test-write".into(),
            schema_version: 1,
            started_at: "2026-01-01T00:00:00Z".into(),
            finished_at: "2026-01-01T01:00:00Z".into(),
            duration_ms: 3600000,
            status: RunStatus::Success,
            totals: Totals {
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: None,
            },
            backends: vec![],
            phases: vec![],
            cost_budget_usd: None,
            budget_warning: None,
        };
        writer.finalize(&summary, tmp.path()).unwrap();
        let summary_path = tmp.path().join("summary.json");
        assert!(summary_path.exists());
        let content = std::fs::read_to_string(&summary_path).unwrap();
        assert!(content.contains("test-write"));
    }

    #[test]
    fn write_summary_full_pipeline() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path();

        // Create markers directory with one completed phase
        let markers = run_dir.join("markers");
        std::fs::create_dir_all(&markers).unwrap();
        std::fs::write(markers.join("design.completed"), "{}").unwrap();

        // Create trace.jsonl with backend call data
        let trace_path = run_dir.join("trace.jsonl");
        std::fs::write(
            &trace_path,
            r#"{"name":"backend.mock","gen_ai.system":"mock","gen_ai.request.model":"m1","gen_ai.usage.input_tokens":100,"gen_ai.usage.output_tokens":200}
{"name":"backend.mock","gen_ai.system":"mock","gen_ai.request.model":"m1","gen_ai.usage.input_tokens":50,"gen_ai.usage.output_tokens":75}
"#,
        )
        .unwrap();

        // Create manifest
        let manifest_path = run_dir.join("manifest.json");
        let mut manifest = Manifest::new("test-run-uuid");
        manifest
            .append(
                ManifestEntry::from_payload(
                    "preexisting.md",
                    Kind::DesignMd,
                    1,
                    Producer::Single,
                    Some("design".into()),
                    Some(0),
                    b"hello",
                ),
                &manifest_path,
            )
            .unwrap();

        // Create prices.toml for cost computation
        let prices_path = tmp.path().join("docs");
        std::fs::create_dir_all(&prices_path).unwrap();
        std::fs::write(
            prices_path.join("prices.toml"),
            r#"["mock:m1"]
input_per_mtok = 10.0
output_per_mtok = 30.0
"#,
        )
        .unwrap();

        // Run the full pipeline
        let writer = SummaryWriter::new(false);
        let summary = writer
            .write_summary(run_dir, &mut manifest, &trace_path, Some(0.50))
            .expect("write_summary should succeed");

        // Verify summary.json was written
        let summary_path = run_dir.join("summary.json");
        assert!(summary_path.exists());

        // Verify summary content
        assert_eq!(summary.run_id, "test-run-uuid");
        assert_eq!(summary.status as u8, RunStatus::Success as u8);
        assert_eq!(summary.totals.input_tokens, 150);
        assert_eq!(summary.totals.output_tokens, 275);
        assert!(!summary.backends.is_empty());
        assert_eq!(summary.backends[0].name, "mock");
        assert_eq!(summary.backends[0].input_tokens, 150);
        assert_eq!(summary.backends[0].output_tokens, 275);
        // Cost: 150/1M * $10 + 275/1M * $30 = $0.0015 + $0.00825 = $0.00975
        let cost = summary.backends[0].cost_usd.unwrap();
        assert!((cost - 0.00975).abs() < 0.001);
        // Budget not exceeded (0.00975 < 0.50), so no warning
        assert!(summary.budget_warning.is_none());

        // Verify manifest entry
        let manifest_entry = manifest.sha256_for("summary.json");
        assert!(
            manifest_entry.is_some(),
            "manifest should have summary.json entry"
        );

        // Verify phases
        assert!(!summary.phases.is_empty());
        assert_eq!(summary.phases[0].phase, "design");
    }

    #[test]
    fn write_summary_budget_exceeded() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path();

        // Create markers + trace
        let markers = run_dir.join("markers");
        std::fs::create_dir_all(&markers).unwrap();
        std::fs::write(markers.join("design.completed"), "{}").unwrap();

        let trace_path = run_dir.join("trace.jsonl");
        std::fs::write(
            &trace_path,
            r#"{"name":"backend.expensive","gen_ai.system":"expensive","gen_ai.request.model":"big","gen_ai.usage.input_tokens":100000,"gen_ai.usage.output_tokens":200000}
"#,
        )
        .unwrap();

        // Create manifest
        let manifest_path = run_dir.join("manifest.json");
        let mut manifest = Manifest::new("budget-test");
        manifest
            .append(
                ManifestEntry::from_payload(
                    "existing.md",
                    Kind::DesignMd,
                    1,
                    Producer::Single,
                    Some("design".into()),
                    Some(0),
                    b"data",
                ),
                &manifest_path,
            )
            .unwrap();

        // Create prices.toml with high prices
        let prices_path = tmp.path().join("docs");
        std::fs::create_dir_all(&prices_path).unwrap();
        std::fs::write(
            prices_path.join("prices.toml"),
            r#"["expensive:big"]
input_per_mtok = 50.0
output_per_mtok = 150.0
"#,
        )
        .unwrap();

        // Budget is $1.00, but cost will be higher
        let writer = SummaryWriter::new(false);
        let summary = writer
            .write_summary(run_dir, &mut manifest, &trace_path, Some(1.00))
            .expect("write_summary should succeed");

        // Cost: 0.1M * $50 + 0.2M * $150 = $5 + $30 = $35 >> $1 budget
        assert_eq!(summary.budget_warning, Some(true));
    }

    #[test]
    fn write_summary_without_prices() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path();

        // Create markers + trace (no prices.toml)
        let markers = run_dir.join("markers");
        std::fs::create_dir_all(&markers).unwrap();
        std::fs::write(markers.join("design.completed"), "{}").unwrap();

        let trace_path = run_dir.join("trace.jsonl");
        std::fs::write(
            &trace_path,
            r#"{"name":"backend.noprice","gen_ai.system":"noprice","gen_ai.request.model":"x","gen_ai.usage.input_tokens":100,"gen_ai.usage.output_tokens":200}
"#,
        )
        .unwrap();

        let manifest_path = run_dir.join("manifest.json");
        let mut manifest = Manifest::new("noprice-test");
        manifest
            .append(
                ManifestEntry::from_payload(
                    "existing.md",
                    Kind::DesignMd,
                    1,
                    Producer::Single,
                    Some("design".into()),
                    Some(0),
                    b"data",
                ),
                &manifest_path,
            )
            .unwrap();

        // No prices.toml exists — cost should be None
        let writer = SummaryWriter::new(false);
        let summary = writer
            .write_summary(run_dir, &mut manifest, &trace_path, None)
            .expect("write_summary should succeed even without prices");

        assert!(summary.totals.cost_usd.is_none());
        assert!(summary.backends[0].cost_usd.is_none());
        assert!(summary.budget_warning.is_none());
    }
}
