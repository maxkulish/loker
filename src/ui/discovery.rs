//! Run directory discovery for the loker UI daemon.
//!
//! Scans `<project_root>/runs/` and returns summaries for each run directory.
//! IO errors on individual entries are logged to stderr and skipped — the
//! daemon must never crash due to a corrupt or partially-written run.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Summary of one run for the runs-list endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    /// Directory basename (e.g. `design-20260505-123456-abc1def2`).
    pub id: String,
    /// Absolute path to the run directory.
    pub path: PathBuf,
    /// Workflow name from `manifest.json` (if available).
    pub workflow: Option<String>,
    /// Run UUID from `manifest.json` (if available).
    pub run_id: Option<String>,
    /// ISO-8601 creation timestamp from `manifest.json` (if available).
    pub created_at: Option<String>,
    /// Phase status map: phase name -> "completed" | "started" | "failed".
    pub phase_status: BTreeMap<String, String>,
}

/// Scan `<project_root>/runs/` and return summaries for each run directory.
///
/// Errors on individual entries are logged to stderr and skipped; the function
/// itself never returns an error. Non-directory entries (e.g. `.DS_Store`)
/// are silently ignored.
pub fn discover_runs(project_root: &Path) -> Vec<RunSummary> {
    let runs_dir = project_root.join("runs");
    if !runs_dir.exists() {
        return Vec::new();
    }

    let dir_entries = match fs::read_dir(&runs_dir) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!(
                "WARN: failed to read runs directory {}: {e}",
                runs_dir.display()
            );
            return Vec::new();
        }
    };

    let mut runs: Vec<RunSummary> = Vec::new();

    for entry in dir_entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("WARN: skipping entry in runs/: {e}");
                continue;
            }
        };

        // Skip non-directory entries (e.g. .DS_Store, temp files).
        let file_type = match entry.file_type() {
            Ok(t) => t,
            Err(e) => {
                eprintln!(
                    "WARN: skipping {}: cannot read file type: {e}",
                    entry.path().display()
                );
                continue;
            }
        };

        if !file_type.is_dir() {
            continue;
        }

        let dir_path = entry.path();
        let dir_name = entry.file_name().to_string_lossy().to_string();

        match load_run_summary(&dir_path, &dir_name) {
            Ok(summary) => runs.push(summary),
            Err(e) => {
                eprintln!("WARN: skipping run {dir_name}: {e}");
            }
        }
    }

    // Sort deterministically by directory name for consistent output.
    runs.sort_by(|a, b| a.id.cmp(&b.id));

    runs
}

/// Load a single run summary from a run directory.
///
/// Reads `manifest.json` for metadata and scans `markers/` for phase status.
/// Returns an error if `manifest.json` is missing, unreadable, or contains
/// invalid JSON.
pub(crate) fn load_run_summary(dir_path: &Path, dir_name: &str) -> anyhow::Result<RunSummary> {
    let manifest_path = dir_path.join("manifest.json");
    let manifest_text = fs::read_to_string(&manifest_path)?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text)?;

    let workflow = manifest
        .get("workflow_name")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // The manifest serialises run_id as "loker.run_id" (see Manifest struct).
    let run_id = manifest
        .get("loker.run_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Derive created_at from the directory's modification time as a fallback;
    // the Manifest struct has no top-level timestamp field.
    let created_at = dir_path
        .metadata()
        .ok()
        .and_then(|m| m.created().or(m.modified()).ok())
        .map(|t| {
            let datetime: chrono::DateTime<chrono::Utc> = t.into();
            datetime.to_rfc3339()
        });

    // Scan markers directory for phase status.
    let phase_status = scan_markers(&dir_path.join("markers"));

    Ok(RunSummary {
        id: dir_name.to_string(),
        path: dir_path.to_path_buf(),
        workflow,
        run_id,
        created_at,
        phase_status,
    })
}

/// Scan a markers directory and return phase status map.
///
/// Marker file conventions:
/// - `<phase>.completed`       -> "completed"
/// - `<phase>.failed`          -> "failed"
/// - `<phase>.started`         -> "started"
/// - `<phase>.started.<n>`     -> "started"
///
/// If multiple markers exist for the same phase, the highest-severity status
/// wins: completed > failed > started > none.
fn scan_markers(markers_dir: &Path) -> BTreeMap<String, String> {
    let mut status: BTreeMap<String, (u8, String)> = BTreeMap::new();

    let entries = match fs::read_dir(markers_dir) {
        Ok(entries) => entries,
        Err(_) => return BTreeMap::new(),
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            _ => continue,
        };

        let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else {
            continue;
        };

        let (phase, marker_status, rank) = classify_marker(&name);
        if let Some(phase) = phase {
            let entry = status.entry(phase).or_insert((0, String::new()));
            if rank > entry.0 {
                *entry = (rank, marker_status.to_string());
            }
        }
    }

    status.into_iter().map(|(k, (_, v))| (k, v)).collect()
}

/// Classify a marker filename into (phase, status, rank).
///
/// Returns `None` for unrecognised filenames.
fn classify_marker(name: &str) -> (Option<String>, &'static str, u8) {
    if let Some(phase) = name.strip_suffix(".completed") {
        return (Some(phase.to_string()), "completed", 3);
    }
    if let Some(phase) = name.strip_suffix(".failed") {
        return (Some(phase.to_string()), "failed", 2);
    }
    // Match `.started` or `<phase>.started.<attempt>`
    if let Some(phase) = name.strip_suffix(".started") {
        return (Some(phase.to_string()), "started", 1);
    }
    if let Some(idx) = name.find(".started.") {
        let phase = &name[..idx];
        if !phase.is_empty() {
            return (Some(phase.to_string()), "started", 1);
        }
    }

    (None, "", 0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_run_dir(tmp: &tempfile::TempDir, name: &str) -> PathBuf {
        let dir = tmp.path().join("runs").join(name);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_manifest(dir: &Path, workflow: &str) {
        let manifest = serde_json::json!({
            "schema_version": 1,
            "workflow_name": workflow,
            "loker.run_id": format!("run-{workflow}"),
            "entries": []
        });
        fs::write(dir.join("manifest.json"), manifest.to_string().as_bytes()).unwrap();
    }

    fn create_marker(dir: &Path, phase: &str, status: &str) {
        let markers_dir = dir.join("markers");
        fs::create_dir_all(&markers_dir).unwrap();
        let file_name = format!("{phase}.{status}");
        fs::write(markers_dir.join(&file_name), b"{}").unwrap();
    }

    #[test]
    fn discover_runs_empty_runs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("runs")).unwrap();
        let runs = discover_runs(tmp.path());
        assert!(runs.is_empty());
    }

    #[test]
    fn discover_runs_missing_runs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let runs = discover_runs(tmp.path());
        assert!(runs.is_empty());
    }

    #[test]
    fn discover_runs_missing_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = create_run_dir(&tmp, "run-without-manifest-123");
        // No manifest.json
        let runs = discover_runs(tmp.path());
        // The errored entry is skipped
        assert!(runs.is_empty());
        assert!(!dir.join("manifest.json").exists());
    }

    #[test]
    fn discover_runs_corrupt_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = create_run_dir(&tmp, "corrupt-run-456");
        fs::write(dir.join("manifest.json"), b"not valid json").unwrap();
        let runs = discover_runs(tmp.path());
        assert!(runs.is_empty());
    }

    #[test]
    fn discover_runs_populated() {
        let tmp = tempfile::tempdir().unwrap();
        let dir_a = create_run_dir(&tmp, "aaa-workflow-111");
        write_manifest(&dir_a, "test-workflow-a");
        create_marker(&dir_a, "design", "completed");
        create_marker(&dir_a, "review", "started");

        let dir_b = create_run_dir(&tmp, "zzz-workflow-222");
        write_manifest(&dir_b, "test-workflow-b");
        create_marker(&dir_b, "plan", "completed");

        let runs = discover_runs(tmp.path());
        assert_eq!(runs.len(), 2);

        // Ordered by directory name.
        assert_eq!(runs[0].id, "aaa-workflow-111");
        assert_eq!(runs[1].id, "zzz-workflow-222");

        // Check metadata.
        assert_eq!(runs[0].workflow.as_deref(), Some("test-workflow-a"));
        assert_eq!(runs[0].run_id.as_deref(), Some("run-test-workflow-a"));
        assert!(runs[0].created_at.is_some());

        // Check phase status.
        let status_a = &runs[0].phase_status;
        assert_eq!(
            status_a.get("design").map(|s| s.as_str()),
            Some("completed")
        );
        assert_eq!(status_a.get("review").map(|s| s.as_str()), Some("started"));

        let status_b = &runs[1].phase_status;
        assert_eq!(status_b.get("plan").map(|s| s.as_str()), Some("completed"));
    }

    #[test]
    fn discover_runs_non_directories_in_runs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let runs_dir = tmp.path().join("runs");
        fs::create_dir_all(&runs_dir).unwrap();

        // A regular file, not a directory.
        fs::write(runs_dir.join(".DS_Store"), b"garbage").unwrap();

        let runs = discover_runs(tmp.path());
        assert!(runs.is_empty());
    }

    #[test]
    fn discover_runs_partial_phase_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = create_run_dir(&tmp, "partial-run-333");
        write_manifest(&dir, "partial-workflow");
        create_marker(&dir, "design", "completed");
        create_marker(&dir, "review", "started");
        // review has no .completed — should report "started"

        let runs = discover_runs(tmp.path());
        assert_eq!(runs.len(), 1);

        let status = &runs[0].phase_status;
        assert_eq!(status.get("design").map(|s| s.as_str()), Some("completed"));
        assert_eq!(status.get("review").map(|s| s.as_str()), Some("started"));
        // plan has no marker at all — should not be in the map
        assert!(!status.contains_key("plan"));
    }

    #[test]
    fn scan_markers_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let status = scan_markers(&tmp.path().join("markers"));
        assert!(status.is_empty());
    }

    #[test]
    fn classify_marker_formats() {
        assert_eq!(
            classify_marker("design.completed"),
            (Some("design".into()), "completed", 3)
        );
        assert_eq!(
            classify_marker("review.failed"),
            (Some("review".into()), "failed", 2)
        );
        assert_eq!(
            classify_marker("plan.started"),
            (Some("plan".into()), "started", 1)
        );
        assert_eq!(
            classify_marker("plan.started.3"),
            (Some("plan".into()), "started", 1)
        );
        assert_eq!(classify_marker("random.txt"), (None, "", 0));
        assert_eq!(classify_marker(""), (None, "", 0));
    }
}
