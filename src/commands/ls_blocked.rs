use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::strategy::verify::human_verifier::PendingRequest;
use crate::strategy::verify::HumanSeverity;

/// One unresolved HITL gate, materialised from a `pending/<phase>.json`
/// file with no matching `responses/<phase>.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedEntry {
    pub run_id: String,
    pub phase: String,
    pub severity: HumanSeverity,
    pub opened_at: DateTime<Utc>,
    pub timeout_at: Option<DateTime<Utc>>,
    pub pending_path: PathBuf,
    pub response_path: PathBuf,
    pub response_display_path: PathBuf,
}

/// Walk `<root>/runs/*/pending/*.json`, skip entries whose
/// `responses/<phase>.json` sibling exists, and return the survivors
/// sorted by `opened_at` ascending (ties: `(run_id, phase)`).
///
/// Malformed pending files are logged to stderr and skipped; the scan
/// does not abort.
pub fn scan_blocked(root: &Path) -> Result<Vec<BlockedEntry>> {
    let runs_dir = root.join("runs");
    if !runs_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for run_entry in fs::read_dir(&runs_dir)
        .with_context(|| format!("failed to read runs directory {}", runs_dir.display()))?
    {
        let run_entry = run_entry?;
        let run_dir = run_entry.path();
        if !run_dir.is_dir() {
            continue;
        }

        let run_id = run_entry.file_name().to_string_lossy().to_string();
        let pending_dir = run_dir.join("pending");
        if !pending_dir.is_dir() {
            continue;
        }

        for pending_entry in fs::read_dir(&pending_dir).with_context(|| {
            format!("failed to read pending directory {}", pending_dir.display())
        })? {
            let pending_entry = pending_entry?;
            let pending_path = pending_entry.path();
            if !pending_path.is_file()
                || pending_path.extension().and_then(|e| e.to_str()) != Some("json")
            {
                continue;
            }

            let phase = match pending_path.file_stem().and_then(|s| s.to_str()) {
                Some(phase) if !phase.is_empty() => phase.to_string(),
                _ => continue,
            };
            let response_path = run_dir.join("responses").join(format!("{phase}.json"));
            if response_path.exists() {
                continue;
            }

            match parse_pending(&run_id, &phase, &pending_path, response_path) {
                Ok(entry) => entries.push(entry),
                Err(err) => eprintln!(
                    "warning: skipping malformed pending request for run {run_id} phase {phase}: {err:#}"
                ),
            }
        }
    }

    entries.sort_by(|a, b| {
        a.opened_at
            .cmp(&b.opened_at)
            .then_with(|| a.run_id.cmp(&b.run_id))
            .then_with(|| a.phase.cmp(&b.phase))
    });
    Ok(entries)
}

fn parse_pending(
    run_id: &str,
    phase: &str,
    pending_path: &Path,
    response_path: PathBuf,
) -> Result<BlockedEntry> {
    let text = fs::read_to_string(pending_path)
        .with_context(|| format!("failed to read {}", pending_path.display()))?;
    let pending: PendingRequest = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", pending_path.display()))?;

    if pending.schema_version != 1 {
        anyhow::bail!(
            "unsupported pending schema version {}",
            pending.schema_version
        );
    }

    let opened_at = parse_rfc3339_utc(&pending.opened_at, "opened_at")?;
    let timeout_at = pending
        .timeout_at
        .as_deref()
        .map(|value| parse_rfc3339_utc(value, "timeout_at"))
        .transpose()?;

    // The pending file's location is the source of truth for the entry key.
    // HumanVerifier writes matching JSON fields today, but using disk stems for
    // the response path prevents drifted JSON from telling operators to write a
    // response file the scanner will never check.
    let response_display_path = PathBuf::from("runs")
        .join(run_id)
        .join("responses")
        .join(format!("{phase}.json"));

    Ok(BlockedEntry {
        run_id: run_id.to_string(),
        phase: phase.to_string(),
        severity: pending.severity,
        opened_at,
        timeout_at,
        pending_path: pending_path.to_path_buf(),
        response_path,
        response_display_path,
    })
}

fn parse_rfc3339_utc(value: &str, field: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .with_context(|| format!("invalid {field} timestamp '{value}'"))
}

/// Render `entries` as a plain-text table to `writer`. `now` is injected
/// so age columns are deterministic in tests. Empty input writes
/// `no blocked runs\n` and returns Ok(()).
pub fn render_table<W: Write>(
    entries: &[BlockedEntry],
    now: DateTime<Utc>,
    writer: &mut W,
) -> Result<()> {
    if entries.is_empty() {
        writeln!(writer, "no blocked runs")?;
        return Ok(());
    }

    writeln!(
        writer,
        "{:<24} {:<16} {:<8} {:<6} RESPONSE",
        "RUN", "PHASE", "SEVERITY", "AGE"
    )?;
    for entry in entries {
        writeln!(
            writer,
            "{:<24} {:<16} {:<8} {:<6} {}",
            entry.run_id,
            entry.phase,
            entry.severity.as_str(),
            format_age(entry.opened_at, now),
            entry.response_display_path.display()
        )?;
    }
    Ok(())
}

fn format_age(opened_at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let seconds = (now - opened_at).num_seconds().max(0);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 60 * 60 {
        format!("{}m", seconds / 60)
    } else if seconds < 60 * 60 * 24 {
        format!("{}h", seconds / (60 * 60))
    } else {
        format!("{}d", seconds / (60 * 60 * 24))
    }
}

/// CLI entry point. Scans `root/runs` and renders the blocked list to
/// stdout. Exit code is 0 on success regardless of empty / non-empty.
pub fn run(root: &Path) -> Result<()> {
    let entries = scan_blocked(root)?;
    render_table(&entries, Utc::now(), &mut std::io::stdout())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    fn write_pending(root: &Path, run_id: &str, phase: &str, opened_at: &str) -> PathBuf {
        write_pending_with_payload_ids(root, run_id, phase, run_id, phase, opened_at)
    }

    fn write_pending_with_payload_ids(
        root: &Path,
        disk_run_id: &str,
        disk_phase: &str,
        payload_run_id: &str,
        payload_phase: &str,
        opened_at: &str,
    ) -> PathBuf {
        let path = root
            .join("runs")
            .join(disk_run_id)
            .join("pending")
            .join(format!("{disk_phase}.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let payload = json!({
            "schema_version": 1,
            "run_id": payload_run_id,
            "workflow": "wf",
            "phase": payload_phase,
            "severity": "high",
            "opened_at": opened_at,
            "timeout_at": null,
            "artefact": {
                "path": "review.md",
                "kind": "text/markdown",
                "preview_lines": 20
            },
            "context": {
                "preceded_by": [],
                "next_phase": null,
                "prompt_summary": "summary"
            },
            "decision_options": ["approve", "reject"]
        });
        fs::write(&path, serde_json::to_vec_pretty(&payload).unwrap()).unwrap();
        path
    }

    fn write_response(root: &Path, run_id: &str, phase: &str) {
        let path = root
            .join("runs")
            .join(run_id)
            .join("responses")
            .join(format!("{phase}.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "{}").unwrap();
    }

    #[test]
    fn scan_blocked_empty_runs_dir_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join("runs")).unwrap();
        assert!(scan_blocked(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn scan_blocked_runs_dir_missing_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(scan_blocked(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn scan_blocked_skips_resolved_phase() {
        let tmp = tempfile::tempdir().unwrap();
        write_pending(tmp.path(), "run-1", "review", "2026-05-06T10:00:00Z");
        write_response(tmp.path(), "run-1", "review");
        assert!(scan_blocked(tmp.path()).unwrap().is_empty());
    }

    #[test]
    fn scan_blocked_lists_unmatched_pending() {
        let tmp = tempfile::tempdir().unwrap();
        write_pending(tmp.path(), "run-1", "review", "2026-05-06T10:00:00Z");
        let entries = scan_blocked(tmp.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].run_id, "run-1");
        assert_eq!(entries[0].phase, "review");
        assert_eq!(entries[0].severity, HumanSeverity::High);
        assert_eq!(
            entries[0].response_path,
            tmp.path().join("runs/run-1/responses/review.json")
        );
        assert_eq!(
            entries[0].response_display_path,
            PathBuf::from("runs/run-1/responses/review.json")
        );
    }

    #[test]
    fn scan_blocked_uses_disk_stems_when_payload_ids_drift() {
        let tmp = tempfile::tempdir().unwrap();
        write_pending_with_payload_ids(
            tmp.path(),
            "disk-run",
            "review",
            "payload-run",
            "payload-phase",
            "2026-05-06T10:00:00Z",
        );
        let entries = scan_blocked(tmp.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].run_id, "disk-run");
        assert_eq!(entries[0].phase, "review");
        assert_eq!(
            entries[0].response_display_path,
            PathBuf::from("runs/disk-run/responses/review.json")
        );
    }

    #[test]
    fn scan_blocked_handles_multi_phase_run() {
        let tmp = tempfile::tempdir().unwrap();
        write_pending(tmp.path(), "run-1", "design", "2026-05-06T09:00:00Z");
        write_pending(tmp.path(), "run-1", "review", "2026-05-06T10:00:00Z");
        write_response(tmp.path(), "run-1", "design");
        let entries = scan_blocked(tmp.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].phase, "review");
    }

    #[test]
    fn scan_blocked_sorts_oldest_first() {
        let tmp = tempfile::tempdir().unwrap();
        write_pending(tmp.path(), "run-c", "review", "2026-05-06T12:00:00Z");
        write_pending(tmp.path(), "run-a", "design", "2026-05-06T10:00:00Z");
        write_pending(tmp.path(), "run-b", "review", "2026-05-06T11:00:00Z");
        let entries = scan_blocked(tmp.path()).unwrap();
        let keys: Vec<_> = entries
            .iter()
            .map(|entry| (entry.run_id.as_str(), entry.phase.as_str()))
            .collect();
        assert_eq!(
            keys,
            vec![
                ("run-a", "design"),
                ("run-b", "review"),
                ("run-c", "review")
            ]
        );
    }

    #[test]
    fn scan_blocked_logs_and_skips_malformed_pending() {
        let tmp = tempfile::tempdir().unwrap();
        let bad = tmp.path().join("runs/run-1/pending/review.json");
        fs::create_dir_all(bad.parent().unwrap()).unwrap();
        fs::write(&bad, "not json").unwrap();
        write_pending(tmp.path(), "run-2", "review", "2026-05-06T10:00:00Z");
        let entries = scan_blocked(tmp.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].run_id, "run-2");
    }

    #[test]
    fn render_table_empty_writes_no_blocked_runs() {
        let mut out = Vec::new();
        render_table(&[], Utc::now(), &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "no blocked runs\n");
    }

    #[test]
    fn render_table_renders_age_severity_paths() {
        let entry = BlockedEntry {
            run_id: "run-1".into(),
            phase: "review".into(),
            severity: HumanSeverity::High,
            opened_at: Utc.with_ymd_and_hms(2026, 5, 6, 10, 0, 0).unwrap(),
            timeout_at: None,
            pending_path: PathBuf::from("runs/run-1/pending/review.json"),
            response_path: PathBuf::from("runs/run-1/responses/review.json"),
            response_display_path: PathBuf::from("runs/run-1/responses/review.json"),
        };
        let mut out = Vec::new();
        render_table(
            &[entry],
            Utc.with_ymd_and_hms(2026, 5, 6, 11, 0, 0).unwrap(),
            &mut out,
        )
        .unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains("run-1"));
        assert!(out.contains("review"));
        assert!(out.contains("high"));
        assert!(out.contains("1h"));
        assert!(out.contains("runs/run-1/responses/review.json"));
    }

    #[test]
    fn render_table_age_units() {
        let now = Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap();
        assert_eq!(format_age(now - chrono::Duration::seconds(12), now), "12s");
        assert_eq!(format_age(now - chrono::Duration::minutes(4), now), "4m");
        assert_eq!(format_age(now - chrono::Duration::hours(3), now), "3h");
        assert_eq!(format_age(now - chrono::Duration::days(2), now), "2d");
    }
}
