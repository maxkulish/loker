//! Pending gate discovery — scans all run directories for pending HITL gates.
//!
//! Provides `discover_pending_gates` which walks `<project_root>/runs/*/pending/*.json`
//! and returns a severity-sorted list of pending gates.

use std::fs;
use std::path::{Path, PathBuf};

use crate::ui::templates::PendingGateDisplay;

/// A pending HITL gate with full context for the route handler.
///
/// The `pending_file_path` and `run_dir` are used internally to read
/// gate context and write responses — never exposed in HTML.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PendingGate {
    pub run_id: String,
    pub run_dir: PathBuf,
    pub phase: String,
    pub workflow: String,
    pub severity: String,
    pub artefact_path: String,
    pub pending_file_path: PathBuf,
}

/// Severity rank for sorting (high → medium → low → default).
fn severity_rank(severity: &str) -> u8 {
    match severity {
        "high" | "critical" => 0,
        "medium" => 1,
        "low" => 2,
        _ => 3,
    }
}

/// Scan all run directories for pending/<phase>.json files.
///
/// Returns gates sorted by severity (high → medium → low), then by
/// run_id for deterministic ordering.
pub fn discover_pending_gates(project_root: &Path) -> Vec<PendingGate> {
    let runs_dir = project_root.join("runs");
    if !runs_dir.exists() {
        return Vec::new();
    }

    let dir_entries = match fs::read_dir(&runs_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut gates: Vec<PendingGate> = Vec::new();

    for entry in dir_entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }

        let run_dir = entry.path();
        let run_id = entry.file_name().to_string_lossy().to_string();
        let pending_dir = run_dir.join("pending");
        if !pending_dir.exists() {
            continue;
        }

        let pending_entries = match fs::read_dir(&pending_dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Read manifest for workflow name (best-effort).
        let manifest_path = run_dir.join("manifest.json");
        let workflow = fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|v| {
                v.get("workflow_name")
                    .and_then(|w| w.as_str().map(String::from))
            })
            .unwrap_or_default();

        for pending_entry in pending_entries {
            let pending_entry = match pending_entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !pending_entry
                .file_type()
                .map(|t| t.is_file())
                .unwrap_or(false)
            {
                continue;
            }

            let phase = pending_entry
                .file_name()
                .to_str()
                .and_then(|n| n.strip_suffix(".json"))
                .map(|n| n.to_string())
                .unwrap_or_default();
            if phase.is_empty() {
                continue;
            }

            let pending_path = pending_entry.path();

            // Read pending file for severity and artefact path
            let (severity, artefact_path) = fs::read_to_string(&pending_path)
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                .map(|v| {
                    let sev = v
                        .get("severity")
                        .and_then(|s| s.as_str())
                        .unwrap_or("medium")
                        .to_string();
                    let art = v
                        .get("artefact")
                        .and_then(|a| a.get("path"))
                        .and_then(|p| p.as_str())
                        .unwrap_or("")
                        .to_string();
                    (sev, art)
                })
                .unwrap_or(("medium".to_string(), String::new()));

            gates.push(PendingGate {
                run_id: run_id.clone(),
                run_dir: run_dir.clone(),
                phase,
                workflow: workflow.clone(),
                severity,
                artefact_path,
                pending_file_path: pending_path,
            });
        }
    }

    // Sort: severity (high first), then run_id for determinism.
    gates.sort_by(|a, b| {
        severity_rank(&a.severity)
            .cmp(&severity_rank(&b.severity))
            .then_with(|| a.run_id.cmp(&b.run_id))
    });

    gates
}

/// Convert PendingGate list to PendingGateDisplay list for templates.
/// Strips filesystem paths — no internal paths leaked to HTML.
pub fn to_display(gates: &[PendingGate]) -> Vec<PendingGateDisplay> {
    gates
        .iter()
        .map(|g| PendingGateDisplay {
            run_id: g.run_id.clone(),
            phase: g.phase.clone(),
            workflow: g.workflow.clone(),
            severity: g.severity.clone(),
            artefact_path: g.artefact_path.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_run(tmp: &tempfile::TempDir, name: &str) -> PathBuf {
        let dir = tmp.path().join("runs").join(name);
        fs::create_dir_all(&dir).unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "workflow_name": "test-wf",
            "loker.run_id": name,
            "entries": []
        });
        fs::write(dir.join("manifest.json"), manifest.to_string().as_bytes()).unwrap();
        dir
    }

    fn create_pending_gate(run_dir: &Path, phase: &str, severity: &str) {
        let pending_dir = run_dir.join("pending");
        fs::create_dir_all(&pending_dir).unwrap();
        let pending = serde_json::json!({
            "severity": severity,
            "artefact": {"path": format!("{}.md", phase), "kind": "text/markdown"}
        });
        fs::write(
            pending_dir.join(format!("{phase}.json")),
            pending.to_string().as_bytes(),
        )
        .unwrap();
    }

    #[test]
    fn test_discover_pending_gates_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let gates = discover_pending_gates(tmp.path());
        assert!(gates.is_empty());
    }

    #[test]
    fn test_discover_pending_gates_populated() {
        let tmp = tempfile::tempdir().unwrap();
        let run_a = setup_run(&tmp, "run-aaa");
        create_pending_gate(&run_a, "review", "medium");

        let gates = discover_pending_gates(tmp.path());
        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0].run_id, "run-aaa");
        assert_eq!(gates[0].phase, "review");
        assert_eq!(gates[0].severity, "medium");
        assert_eq!(gates[0].workflow, "test-wf");
    }

    #[test]
    fn test_discover_pending_gates_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        let run_a = setup_run(&tmp, "run-aaa");
        create_pending_gate(&run_a, "approval", "high");
        let run_b = setup_run(&tmp, "run-bbb");
        create_pending_gate(&run_b, "review", "low");
        let run_c = setup_run(&tmp, "run-ccc");
        create_pending_gate(&run_c, "deploy", "medium");

        let gates = discover_pending_gates(tmp.path());
        assert_eq!(gates.len(), 3);
        // high first, then medium, then low
        assert_eq!(gates[0].severity, "high");
        assert_eq!(gates[1].severity, "medium");
        assert_eq!(gates[2].severity, "low");
    }

    #[test]
    fn test_discover_pending_gates_no_runs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        // No runs/ directory at all
        let gates = discover_pending_gates(tmp.path());
        assert!(gates.is_empty());
    }

    #[test]
    fn test_to_display_strips_paths() {
        let gates = vec![PendingGate {
            run_id: "run-1".into(),
            run_dir: PathBuf::from("/tmp/secret/run-1"),
            phase: "review".into(),
            workflow: "test-wf".into(),
            severity: "high".into(),
            artefact_path: "review.md".into(),
            pending_file_path: PathBuf::from("/tmp/secret/run-1/pending/review.json"),
        }];
        let display = to_display(&gates);
        assert_eq!(display.len(), 1);
        assert_eq!(display[0].run_id, "run-1");
        assert_eq!(display[0].phase, "review");
        assert_eq!(display[0].severity, "high");
        // Paths should NOT be in the display struct
        // (PendingGateDisplay has no path fields)
        assert!(!display[0].artefact_path.contains('/'));
    }
}
