use loker::run_state::{
    next_attempt, CompletedMarker, FailedMarker, MarkerWriter, StartedMarker,
};

// ---------------------------------------------------------------------------
// Helper: create a temp directory to use as a run directory
// ---------------------------------------------------------------------------

fn temp_run_dir() -> (tempfile::TempDir, MarkerWriter) {
    let tmp = tempfile::tempdir().unwrap();
    let writer = MarkerWriter::new(tmp.path());
    (tmp, writer)
}

// ---------------------------------------------------------------------------
// Marker round-trip tests
// ---------------------------------------------------------------------------

#[test]
fn marker_roundtrip_started() {
    let (_tmp, writer) = temp_run_dir();
    let phase = "design";
    let attempt = 0;

    let marker = writer.write_started(phase, attempt).unwrap();
    assert_eq!(marker.phase, phase);
    assert_eq!(marker.attempt, attempt);
    assert!(marker.writer_pid > 0);
    assert!(!marker.writer_host.is_empty());
    assert_eq!(marker.heartbeat_ttl_seconds, 300);

    // Read it back from disk
    let path = writer.markers_dir().join("design.started");
    let content = std::fs::read_to_string(path).unwrap();
    let deserialized: StartedMarker = serde_json::from_str(&content).unwrap();
    assert_eq!(deserialized, marker);
}

#[test]
fn marker_roundtrip_completed() {
    let (_tmp, writer) = temp_run_dir();
    let phase = "design";
    let attempt = 0;
    let sha = "abc123deadbeef";
    let artefact_paths = vec!["docs/designs/design.md".to_string()];

    let marker = writer
        .write_completed(phase, attempt, sha, &artefact_paths)
        .unwrap();
    assert_eq!(marker.phase, phase);
    assert_eq!(marker.attempt, attempt);
    assert_eq!(marker.manifest_entry_sha256, sha);
    assert_eq!(marker.artefact_paths, artefact_paths);

    // Read it back from disk
    let path = writer.markers_dir().join("design.completed");
    let content = std::fs::read_to_string(path).unwrap();
    let deserialized: CompletedMarker = serde_json::from_str(&content).unwrap();
    assert_eq!(deserialized, marker);
}

#[test]
fn marker_roundtrip_failed() {
    let (_tmp, writer) = temp_run_dir();
    let phase = "design";
    let attempts_made = 3;
    let error_class = "PhaseError::ArtefactSchemaMismatch";
    let last_attempt_path = "runs/r1/logs/design.attempt_2.log";

    let marker = writer
        .write_failed(phase, attempts_made, error_class, last_attempt_path)
        .unwrap();
    assert_eq!(marker.phase, phase);
    assert_eq!(marker.attempts_made, attempts_made);
    assert_eq!(marker.error_class, error_class);
    assert_eq!(marker.last_attempt_path, last_attempt_path);

    // Read it back from disk
    let path = writer.markers_dir().join("design.failed");
    let content = std::fs::read_to_string(path).unwrap();
    let deserialized: FailedMarker = serde_json::from_str(&content).unwrap();
    assert_eq!(deserialized, marker);
}

// ---------------------------------------------------------------------------
// Atomic write crash-safety tests
// ---------------------------------------------------------------------------

#[test]
fn atomic_rename_crash_between_tmp_and_rename() {
    // Simulate a crash after tmp write but before rename completes.
    // The final marker path should not exist.
    let (_tmp, writer) = temp_run_dir();
    // Write a marker normally.
    writer.write_started("design", 0).unwrap();

    // Now verify the marker path exists (it was renamed successfully).
    let path = writer.markers_dir().join("design.started");
    assert!(path.exists(), "marker file should exist after completed write");

    // Also verify no .tmp files remain.
    let has_tmp = std::fs::read_dir(writer.markers_dir())
        .unwrap()
        .any(|e| e.unwrap().file_name().to_string_lossy().contains(".tmp"));
    assert!(!has_tmp, "no tmp files should remain after completed write");
}

#[test]
fn atomic_rename_tmp_cleaned_after_success() {
    let (_tmp, writer) = temp_run_dir();
    writer.write_started("design", 0).unwrap();
    writer.write_completed("design", 0, "sha", &[]).unwrap();
    writer.write_failed("design", 1, "err", "path").unwrap();

    // No .tmp files should remain after multiple writes.
    let dir = std::fs::read_dir(writer.markers_dir()).unwrap();
    for entry in dir {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        assert!(!name.contains(".tmp"), "tmp file leftover: {}", name);
    }
}

// ---------------------------------------------------------------------------
// next_attempt tests
// ---------------------------------------------------------------------------

#[test]
fn next_attempt_zero_markers() {
    let tmp = tempfile::tempdir().unwrap();
    let markers_dir = tmp.path().join("markers");
    // Don't create the markers directory at all.
    let n = next_attempt(&markers_dir, "design").unwrap();
    assert_eq!(n, 0, "no markers dir → attempt 0");
}

#[test]
fn next_attempt_single_marker() {
    let (_tmp, writer) = temp_run_dir();
    writer.write_started("design", 0).unwrap();
    let n = next_attempt(writer.markers_dir(), "design").unwrap();
    assert_eq!(n, 1, "one marker at attempt 0 → next is 1");
}

#[test]
fn next_attempt_three_markers() {
    let (_tmp, writer) = temp_run_dir();
    writer.write_started("design", 0).unwrap();
    writer.write_started("design", 1).unwrap();
    writer.write_started("design", 2).unwrap();
    let n = next_attempt(writer.markers_dir(), "design").unwrap();
    assert_eq!(n, 3, "three markers (0,1,2) → next is 3");
}

#[test]
fn next_attempt_with_gaps() {
    let (_tmp, writer) = temp_run_dir();
    // Markers for attempt 0 and 2 (missing 1 → simulates partial cleanup).
    writer.write_started("design", 0).unwrap();
    writer.write_started("design", 2).unwrap();
    let n = next_attempt(writer.markers_dir(), "design").unwrap();
    assert_eq!(n, 3, "gaps: max attempt is 2 → next is 3");
}

// ---------------------------------------------------------------------------
// Concurrent writer test
// ---------------------------------------------------------------------------

#[test]
fn concurrent_writers_no_corruption() {
    let tmp = tempfile::tempdir().unwrap();
    let markers_dir = tmp.path().join("markers");

    // Two threads writing different markers concurrently.
    let dir_a = tmp.path().to_owned();
    let dir_b = tmp.path().to_owned();

    let t1 = std::thread::spawn(move || {
        let w = MarkerWriter::new(&dir_a);
        w.write_started("design", 0).unwrap();
        w.write_completed("design", 0, "shasha", &[]).unwrap();
    });

    let t2 = std::thread::spawn(move || {
        let w = MarkerWriter::new(&dir_b);
        w.write_started("review", 0).unwrap();
        w.write_completed("review", 0, "shasha2", &[]).unwrap();
    });

    t1.join().unwrap();
    t2.join().unwrap();

    // Both markers should be present and valid JSON.
    let design_started = std::fs::read_to_string(markers_dir.join("design.started")).unwrap();
    let design_completed = std::fs::read_to_string(markers_dir.join("design.completed")).unwrap();
    let review_started = std::fs::read_to_string(markers_dir.join("review.started")).unwrap();
    let review_completed = std::fs::read_to_string(markers_dir.join("review.completed")).unwrap();

    serde_json::from_str::<StartedMarker>(&design_started).unwrap();
    serde_json::from_str::<CompletedMarker>(&design_completed).unwrap();
    serde_json::from_str::<StartedMarker>(&review_started).unwrap();
    serde_json::from_str::<CompletedMarker>(&review_completed).unwrap();
}

// ---------------------------------------------------------------------------
// Helper tests for the MarkerWriter
// ---------------------------------------------------------------------------

#[test]
fn markers_dir_created_automatically() {
    let tmp = tempfile::tempdir().unwrap();
    let markers_dir = tmp.path().join("markers");
    assert!(!markers_dir.exists(), "markers dir should not exist yet");

    let writer = MarkerWriter::new(tmp.path());
    writer.write_started("design", 0).unwrap();

    assert!(markers_dir.exists(), "markers dir should be created on first write");
    assert!(markers_dir.join("design.started").exists());
}

#[test]
fn different_phases_do_not_interfere() {
    let (_tmp, writer) = temp_run_dir();
    writer.write_started("design", 0).unwrap();
    writer.write_started("implement", 0).unwrap();
    writer.write_started("review", 0).unwrap();

    let n_design = next_attempt(writer.markers_dir(), "design").unwrap();
    let n_implement = next_attempt(writer.markers_dir(), "implement").unwrap();
    let n_review = next_attempt(writer.markers_dir(), "review").unwrap();

    assert_eq!(n_design, 1);
    assert_eq!(n_implement, 1);
    assert_eq!(n_review, 1);
}
