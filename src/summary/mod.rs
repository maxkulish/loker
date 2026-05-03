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

use std::path::{Path, PathBuf};
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
        let prices_path = run_dir.join("../../docs/prices.toml");
        // Try canonical path first, then relative to CWD
        let price_table = if prices_path.exists() {
            PriceTable::load(&prices_path)
        } else {
            PriceTable::load(Path::new("docs/prices.toml"))
        };

        let mut total_cost_usd = 0.0_f64;
        let mut any_cost_known = false;
        for backend in &mut backends {
            match price_table.compute_cost(&backend.name, &backend.model, backend.input_tokens, backend.output_tokens) {
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

        // 4. Compute duration and timestamps
        let now = chrono::Utc::now();
        let finished_at = now.to_rfc3339();
        let started_at = get_run_start_time(run_dir)
            .unwrap_or_else(|| now.to_rfc3339());
        let duration_ms = compute_duration_ms(run_dir);

        // 5. Determine run status
        let status = compute_run_status(&phases);

        // 6. Build totals
        let totals_input: u64 = backends.iter().map(|b| b.input_tokens).sum();
        let totals_output: u64 = backends.iter().map(|b| b.output_tokens).sum();
        let totals_cost = if any_cost_known { Some(total_cost_usd) } else { None };

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

        // 10. Register in manifest
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
        manifest.append(entry, &run_dir.join("manifest.json"))?;

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
        self.summaries.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn clear(&self) {
        self.summaries.lock().unwrap_or_else(|e| e.into_inner()).clear();
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
/// Reads `*.completed`, `*.failed` marker files to determine each phase's
/// status, attempt count, and duration.
fn collect_phases(run_dir: &Path) -> Vec<PhaseSummary> {
    let markers_dir = run_dir.join("markers");
    if !markers_dir.is_dir() {
        return Vec::new();
    }

    let mut phases: Vec<PhaseSummary> = Vec::new();

    let entries = match std::fs::read_dir(&markers_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    for entry in entries.flatten() {
        let file_name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };

        let (phase_name, status, attempts) = match parse_marker_name(&file_name) {
            Some(t) => t,
            None => continue,
        };

        let duration_ms = match entry.metadata() {
            Ok(meta) => match meta.modified() {
                Ok(time) => {
                    // Estimate duration from file mtime vs now
                    match std::time::SystemTime::now().duration_since(time) {
                        Ok(d) => d.as_millis() as u64,
                        Err(_) => 0,
                    }
                }
                Err(_) => 0,
            },
            Err(_) => 0,
        };

        phases.push(PhaseSummary {
            phase: phase_name,
            status,
            attempts,
            duration_ms,
            strategy: None, // Strategy info is not in marker files currently
        });
    }

    // Sort by phase name for deterministic output
    phases.sort_by(|a, b| a.phase.cmp(&b.phase));
    phases
}

/// Parse a marker file name like `design.completed`, `implement.failed`.
///
/// Returns `(phase_name, status, attempts_from_metadata)`.
fn parse_marker_name(name: &str) -> Option<(String, PhaseStatus, u32)> {
    if name.ends_with(".completed") {
        let phase = name.trim_end_matches(".completed").to_string();
        // Attempt count is encoded in the marker's JSON content, not the filename.
        // For v0 we default to 1 for completed phases.
        Some((phase, PhaseStatus::Completed, 1))
    } else if name.ends_with(".failed") {
        let phase = name.trim_end_matches(".failed").to_string();
        Some((phase, PhaseStatus::Failed, 3)) // Placeholder: actual count from marker content
    } else if name.ends_with(".started") {
        // Started markers alone don't tell us the outcome
        None
    } else {
        None
    }
}

/// Get the run start time from the earliest marker or directory ctime.
fn get_run_start_time(run_dir: &Path) -> Option<String> {
    // Try markers directory as earliest timestamp proxy
    let markers_dir = run_dir.join("markers");
    if markers_dir.is_dir() {
        if let Ok(meta) = markers_dir.metadata() {
            if let Ok(time) = meta.created().or_else(|_| meta.modified()) {
                let dur = time.duration_since(std::time::UNIX_EPOCH).ok()?;
                let secs = dur.as_secs() as i64;
                let nanos = dur.subsec_nanos();
                let datetime = chrono::DateTime::from_timestamp(secs, nanos)?;
                return Some(datetime.to_rfc3339());
            }
        }
    }
    None
}

/// Compute wall-clock duration of the run.
fn compute_duration_ms(_run_dir: &Path) -> u64 {
    // For v0, we don't have a reliable duration source independent of the trace.
    // Return 0 as a placeholder — the run-level executor will populate this.
    0
}

/// Determine the top-level run status from per-phase outcomes.
fn compute_run_status(phases: &[PhaseSummary]) -> RunStatus {
    if phases.is_empty() {
        return RunStatus::Failed;
    }

    let all_completed = phases.iter().all(|p| matches!(p.status, PhaseStatus::Completed));
    let any_failed = phases.iter().any(|p| matches!(p.status, PhaseStatus::Failed));

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
        std::fs::write(markers.join("implement.failed"), "{\"error\":\"verify_failed\"}").unwrap();

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
        let phases = vec![
            PhaseSummary {
                phase: "design".into(),
                status: PhaseStatus::Completed,
                attempts: 1,
                duration_ms: 100,
                strategy: None,
            },
        ];
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
            totals: Totals { input_tokens: 100, output_tokens: 200, cost_usd: None },
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
            totals: Totals { input_tokens: 1000, output_tokens: 500, cost_usd: Some(0.05) },
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
            totals: Totals { input_tokens: 0, output_tokens: 0, cost_usd: None },
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
}
