use std::fs;
use std::path::Path;

use chrono::{Duration, Utc};
use loker::manifest::{Kind, Manifest, Producer};
use loker::run_state::{LoadError, PhaseStatus, RunState};

fn marker_completed(run_dir: &Path, phase: &str, sha: &str) {
    let markers_dir = run_dir.join("markers");
    fs::create_dir_all(&markers_dir).unwrap();
    let payload = serde_json::json!({
        "phase": phase,
        "attempt": 1,
        "completed_at": Utc::now().to_rfc3339(),
        "manifest_entry_sha256": sha,
        "artefact_paths": [format!("{phase}/out")],
    });
    fs::write(
        markers_dir.join(format!("{phase}.completed")),
        payload.to_string(),
    )
    .unwrap();
}

fn marker_started(run_dir: &Path, phase: &str) {
    let markers_dir = run_dir.join("markers");
    fs::create_dir_all(&markers_dir).unwrap();
    let payload = serde_json::json!({
        "phase": phase,
        "attempt": 1,
        "started_at": Utc::now().to_rfc3339(),
        "writer_pid": 123,
        "writer_host": "localhost",
        "heartbeat_ttl_seconds": 300,
    });
    fs::write(
        markers_dir.join(format!("{phase}.started")),
        payload.to_string(),
    )
    .unwrap();
}

fn build_entry_payload(name: &str, payload: &[u8]) -> (loker::manifest::ManifestEntry, Vec<u8>) {
    (
        loker::manifest::ManifestEntry::from_payload(
            name.to_string(),
            Kind::DesignMd,
            1,
            Producer::Single,
            Some("design".to_string()),
            Some(1),
            payload,
        ),
        payload.to_vec(),
    )
}

fn write_manifest_with_run_state(
    tmp: &std::path::Path,
    entries: Vec<(loker::manifest::ManifestEntry, Vec<u8>)>,
    run_id: &str,
) -> Manifest {
    let mut manifest = Manifest::new(run_id);
    for (entry, bytes) in entries {
        let relpath = Path::new(&entry.name);
        let abs = tmp.join(relpath);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(abs, &bytes).unwrap();
        manifest.entries.push(entry);
    }
    let manifest_path = tmp.join("manifest.json");
    fs::write(manifest_path, manifest.to_json().unwrap()).unwrap();
    manifest
}

#[test]
fn happy_path_load_returns_surviving_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let run_id = "run-001";

    let (entry1, payload1) = build_entry_payload("design/design.md", b"hello design");
    let (entry2, payload2) = build_entry_payload("review/review.md", b"hello review");

    let manifest = write_manifest_with_run_state(
        tmp.path(),
        vec![(entry1.clone(), payload1), (entry2.clone(), payload2)],
        run_id,
    );

    marker_completed(tmp.path(), "design", &entry1.sha256);
    marker_completed(tmp.path(), "review", &entry2.sha256);

    let run_state = RunState::load(tmp.path(), 300).unwrap();
    assert_eq!(run_state.run_id, run_id);
    assert_eq!(run_state.entries.len(), 2);
    assert_eq!(run_state.dropped_orphans.len(), 0);
    assert!(run_state.entries.iter().any(|e| e.name == entry1.name));
    assert!(run_state.entries.iter().any(|e| e.name == entry2.name));
    assert_eq!(manifest.run_id, run_id);
}

#[test]
fn schema_mismatch_returns_artefact_schema_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let text = r#"{"loker.run_id":"run-002","schema_version":2,"entries":[]}"#;
    fs::write(tmp.path().join("manifest.json"), text).unwrap();

    let err = RunState::load(tmp.path(), 300).unwrap_err();
    match err {
        LoadError::ArtefactSchemaMismatch {
            expected, found, ..
        } => {
            assert_eq!(expected, 1);
            assert_eq!(found, 2);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn corrupt_entry_returns_artefact_corrupt() {
    let tmp = tempfile::tempdir().unwrap();
    let (entry, bytes) = build_entry_payload("design/design.md", b"good bytes");
    write_manifest_with_run_state(tmp.path(), vec![(entry.clone(), bytes)], "run-003");
    marker_completed(tmp.path(), "design", &entry.sha256);

    fs::write(tmp.path().join("design/design.md"), b"bad bytes").unwrap();

    let err = RunState::load(tmp.path(), 300).unwrap_err();
    match err {
        LoadError::ArtefactCorrupt {
            path,
            expected,
            found,
        } => {
            assert!(path.ends_with("design/design.md"));
            assert_eq!(expected, entry.sha256);
            assert_ne!(found, expected);
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn missing_entry_returns_artefact_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let (entry, bytes) = build_entry_payload("design/design.md", b"exists first");
    write_manifest_with_run_state(tmp.path(), vec![(entry.clone(), bytes)], "run-004");
    marker_completed(tmp.path(), "design", &entry.sha256);

    fs::remove_file(tmp.path().join("design/design.md")).unwrap();

    let err = RunState::load(tmp.path(), 300).unwrap_err();
    match err {
        LoadError::ArtefactMissing { path } => assert!(path.ends_with("design/design.md")),
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn orphan_sweep_drops_non_completed_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let run_id = "run-005";
    let (entry1, payload1) = build_entry_payload("design/design.md", b"keep this");
    let (entry2, payload2) = build_entry_payload("review/review.md", b"drop this");
    write_manifest_with_run_state(
        tmp.path(),
        vec![(entry1.clone(), payload1), (entry2.clone(), payload2)],
        run_id,
    );

    marker_completed(tmp.path(), "design", &entry1.sha256);

    let run_state = RunState::load(tmp.path(), 300).unwrap();
    assert_eq!(run_state.entries.len(), 1);
    assert_eq!(run_state.entries[0].name, entry1.name);
    assert_eq!(run_state.dropped_orphans.len(), 1);
    assert_eq!(run_state.dropped_orphans[0].name, entry2.name);
}

#[test]
fn stale_heartbeat_is_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let (entry, payload) = build_entry_payload("design/design.md", b"heartbeat test");
    write_manifest_with_run_state(tmp.path(), vec![(entry.clone(), payload)], "run-006");

    let stale = Utc::now() - Duration::seconds(120);
    let heartbeat = serde_json::json!({
        "writer_pid": 42,
        "writer_host": "test-host",
        "tick_at": stale.to_rfc3339(),
    });
    fs::write(tmp.path().join("heartbeat.json"), heartbeat.to_string()).unwrap();

    let run_state = RunState::load(tmp.path(), 60).unwrap();
    match run_state.heartbeat {
        Some(loker::run_state::HeartbeatStatus::Stale {
            ttl_seconds,
            last_tick,
        }) => {
            assert_eq!(ttl_seconds, 60);
            assert!(last_tick <= stale);
        }
        other => panic!("unexpected heartbeat: {other:?}"),
    }
}

#[test]
fn live_heartbeat_is_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let (entry, payload) = build_entry_payload("design/design.md", b"live heartbeat");
    write_manifest_with_run_state(tmp.path(), vec![(entry.clone(), payload)], "run-007");

    let heartbeat = serde_json::json!({
        "writer_pid": 77,
        "writer_host": "host-x",
        "tick_at": Utc::now().to_rfc3339(),
    });
    fs::write(tmp.path().join("heartbeat.json"), heartbeat.to_string()).unwrap();

    let run_state = RunState::load(tmp.path(), 300).unwrap();
    match run_state.heartbeat {
        Some(loker::run_state::HeartbeatStatus::Live(hb)) => {
            assert_eq!(hb.writer_pid, 77);
            assert_eq!(hb.writer_host, "host-x");
        }
        other => panic!("unexpected heartbeat: {other:?}"),
    }
}

#[test]
fn empty_manifest_loads_empty_runstate() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest = Manifest::new("run-008");
    fs::write(
        tmp.path().join("manifest.json"),
        manifest.to_json().unwrap(),
    )
    .unwrap();

    let run_state = RunState::load(tmp.path(), 300).unwrap();
    assert_eq!(run_state.entries.len(), 0);
    assert_eq!(run_state.dropped_orphans.len(), 0);
    assert_eq!(run_state.phase_status.len(), 0);
}

#[test]
fn missing_markers_directory_keeps_all_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let (entry1, payload1) = build_entry_payload("design/design.md", b"entry one");
    let (entry2, payload2) = build_entry_payload("review/review.md", b"entry two");
    write_manifest_with_run_state(
        tmp.path(),
        vec![(entry1.clone(), payload1), (entry2.clone(), payload2)],
        "run-009",
    );

    let run_state = RunState::load(tmp.path(), 300).unwrap();
    assert_eq!(run_state.entries.len(), 2);
    assert_eq!(run_state.dropped_orphans.len(), 0);
}

#[test]
fn markers_without_completed_hashes_keeps_all_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let (entry1, payload1) = build_entry_payload("design/design.md", b"entry one");
    let (entry2, payload2) = build_entry_payload("review/review.md", b"entry two");
    write_manifest_with_run_state(
        tmp.path(),
        vec![(entry1.clone(), payload1), (entry2.clone(), payload2)],
        "run-009b",
    );

    marker_started(tmp.path(), "design");
    fs::write(
        tmp.path().join("markers/design.failed"),
        serde_json::json!({
            "phase": "design",
            "attempts_made": 1,
            "failed_at": Utc::now().to_rfc3339(),
        })
        .to_string(),
    )
    .unwrap();

    let run_state = RunState::load(tmp.path(), 300).unwrap();
    assert_eq!(run_state.entries.len(), 2);
    assert_eq!(run_state.dropped_orphans.len(), 0);
}

#[test]
fn phase_status_is_derived_from_markers() {
    let tmp = tempfile::tempdir().unwrap();
    let (design_entry, design_payload) = build_entry_payload("design/design.md", b"done now");
    write_manifest_with_run_state(
        tmp.path(),
        vec![(design_entry.clone(), design_payload)],
        "run-010",
    );

    marker_completed(tmp.path(), "design", &design_entry.sha256);
    marker_started(tmp.path(), "review");

    let run_state = RunState::load(tmp.path(), 300).unwrap();
    assert_eq!(
        run_state.phase_status.get("design"),
        Some(&PhaseStatus::Completed)
    );
    assert_eq!(
        run_state.phase_status.get("review"),
        Some(&PhaseStatus::Started)
    );
}

#[test]
fn changes_dir_entry_is_verified_with_digest() {
    let tmp = tempfile::tempdir().unwrap();

    let dir_entry_digest = {
        let digest_root = tmp.path().join("changes");
        fs::create_dir_all(digest_root.join("sub")).unwrap();
        fs::write(digest_root.join("a.txt"), b"alpha").unwrap();
        fs::write(digest_root.join("sub/b.txt"), b"beta").unwrap();
        loker::manifest::dir_digest(&digest_root).unwrap()
    };

    let manifest_entry = loker::manifest::ManifestEntry {
        name: "changes/".to_string(),
        kind: Kind::ChangesDir,
        schema_version: 1,
        sha256: dir_entry_digest,
        producer: Producer::Single,
        phase: Some("design".to_string()),
        attempt: Some(1),
        created_at: None,
    };

    let manifest = Manifest {
        run_id: "run-010".to_string(),
        schema_version: 1,
        entries: vec![manifest_entry.clone()],
    };
    fs::write(
        tmp.path().join("manifest.json"),
        manifest.to_json().unwrap(),
    )
    .unwrap();
    marker_completed(tmp.path(), "design", &manifest_entry.sha256);

    let run_state = RunState::load(tmp.path(), 300).unwrap();
    assert_eq!(run_state.entries[0].kind, Kind::ChangesDir);
}
