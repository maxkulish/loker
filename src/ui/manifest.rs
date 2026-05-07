//! Shared manifest.json parsing for the loker UI daemon.
//!
//! Provides `read_manifest_entries` and `build_phase_timeline`, used by
//! both `discovery.rs` (run summaries) and `routes.rs` (detail view).

use std::fs;
use std::path::Path;

use crate::ui::templates::{ManifestEntry, PhaseStep};

/// Read manifest entries from a manifest.json file.
///
/// Returns an empty Vec on any error (missing file, invalid JSON, wrong
/// shape) — the caller should handle the empty case gracefully.
pub fn read_manifest_entries(manifest_path: &Path) -> Vec<ManifestEntry> {
    let text = match fs::read_to_string(manifest_path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let json: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let entries = match json.get("entries").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    entries
        .iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.to_string();
            let kind = entry.get("kind")?.as_str()?.to_string();
            let schema_version = entry.get("schema_version")?.as_u64()? as u32;
            let sha256 = entry
                .get("sha256")
                .and_then(|v| v.as_str())
                .map(String::from);
            Some(ManifestEntry {
                name,
                kind,
                schema_version,
                sha256,
            })
        })
        .collect()
}

/// Build phase timeline from markers/ directory.
///
/// Scans `<run_dir>/markers/` for status marker files and returns a
/// timeline sorted by phase name.
pub fn build_phase_timeline(run_dir: &Path) -> Vec<PhaseStep> {
    let markers_dir = run_dir.join("markers");
    let entries = match fs::read_dir(&markers_dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    use std::collections::BTreeMap;
    let mut phases: BTreeMap<String, (u8, String)> = BTreeMap::new();

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };
        if let Some((phase, status_val, rank)) = classify_marker(&name) {
            let current = phases.entry(phase).or_insert((0, String::new()));
            if rank > current.0 {
                *current = (rank, status_val.to_string());
            }
        }
    }

    phases
        .into_iter()
        .map(|(name, (_, status))| PhaseStep {
            name,
            status,
            started_at: None,
            completed_at: None,
        })
        .collect()
}

/// Classify a marker filename into (phase, status, rank).
/// Higher rank = more significant (completed > failed > started).
fn classify_marker(name: &str) -> Option<(String, &'static str, u8)> {
    if let Some(phase) = name.strip_suffix(".completed") {
        return Some((phase.to_string(), "completed", 3));
    }
    if let Some(phase) = name.strip_suffix(".failed") {
        return Some((phase.to_string(), "failed", 2));
    }
    if let Some(phase) = name.strip_suffix(".started") {
        return Some((phase.to_string(), "started", 1));
    }
    if let Some(idx) = name.find(".started.") {
        let phase = &name[..idx];
        if !phase.is_empty() {
            return Some((phase.to_string(), "started", 1));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_read_manifest_entries_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "workflow_name": "test-wf",
            "loker.run_id": "run-test-wf",
            "entries": [
                {"name": "design.md", "kind": "text/markdown", "schema_version": 1, "sha256": "abc123"}
            ]
        });
        let path = tmp.path().join("manifest.json");
        fs::write(&path, manifest.to_string().as_bytes()).unwrap();

        let entries = read_manifest_entries(&path);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "design.md");
        assert_eq!(entries[0].kind, "text/markdown");
        assert_eq!(entries[0].schema_version, 1);
        assert_eq!(entries[0].sha256.as_deref(), Some("abc123"));
    }

    #[test]
    fn test_read_manifest_entries_corrupt() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("manifest.json");
        fs::write(&path, b"not valid json").unwrap();
        let entries = read_manifest_entries(&path);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_read_manifest_entries_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("manifest.json");
        let entries = read_manifest_entries(&path);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_build_phase_timeline_states() {
        let tmp = tempfile::tempdir().unwrap();
        let markers_dir = tmp.path().join("markers");
        fs::create_dir_all(&markers_dir).unwrap();
        fs::write(markers_dir.join("design.completed"), b"").unwrap();
        fs::write(markers_dir.join("review.started"), b"").unwrap();
        fs::write(markers_dir.join("plan.failed"), b"").unwrap();

        let timeline = build_phase_timeline(tmp.path());
        assert_eq!(timeline.len(), 3);
        let get_status = |name: &str| {
            timeline
                .iter()
                .find(|s| s.name == name)
                .map(|s| s.status.as_str())
        };
        assert_eq!(get_status("design"), Some("completed"));
        assert_eq!(get_status("review"), Some("started"));
        assert_eq!(get_status("plan"), Some("failed"));
    }

    #[test]
    fn test_build_phase_timeline_empty_markers() {
        let tmp = tempfile::tempdir().unwrap();
        let timeline = build_phase_timeline(tmp.path());
        assert!(timeline.is_empty());
    }
}
