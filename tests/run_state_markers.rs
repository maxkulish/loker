use loker::run_state::{
    is_stale, next_attempt, CompletedMarker, FailedMarker, HeartbeatBody, HeartbeatConfig,
    HeartbeatWriter, MarkerWriter, PhaseOrderGuard, PhaseState, StartedMarker,
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
    let (tmp, writer) = temp_run_dir();
    let phase = "design";
    let attempt = 0;

    let marker = writer.write_started(phase, attempt).unwrap();
    assert_eq!(marker.phase, phase);
    assert_eq!(marker.attempt, attempt);
    assert!(marker.writer_pid > 0);
    assert!(!marker.writer_host.is_empty());
    assert_eq!(marker.heartbeat_ttl_seconds, 300);

    // Read it back from disk
    let path = writer.markers_dir().join("design.started.0");
    let content = std::fs::read_to_string(path).unwrap();
    let deserialized: StartedMarker = serde_json::from_str(&content).unwrap();
    assert_eq!(deserialized, marker);
}

#[test]
fn marker_roundtrip_completed() {
    let (tmp, writer) = temp_run_dir();
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
    let (tmp, writer) = temp_run_dir();
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
fn atomic_write_leaves_no_temporary_files_on_success() {
    // Verify that after a successful atomic write, no temporary files are left behind.
    let (tmp, writer) = temp_run_dir();
    // Write a marker normally.
    writer.write_started("design", 0).unwrap();

    // Now verify the marker path exists (it was renamed successfully).
    let path = writer.markers_dir().join("design.started.0");
    assert!(
        path.exists(),
        "marker file should exist after completed write"
    );

    // Also verify no .tmp files remain.
    let has_tmp = std::fs::read_dir(writer.markers_dir())
        .unwrap()
        .any(|e| e.unwrap().file_name().to_string_lossy().contains(".tmp"));
    assert!(!has_tmp, "no tmp files should remain after completed write");
}

#[test]
fn atomic_rename_tmp_cleaned_after_success() {
    let (tmp, writer) = temp_run_dir();
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
    // Don't create the markers directory at all.
    let n = next_attempt(tmp.path(), "design").unwrap();
    assert_eq!(n, 0, "no markers dir → attempt 0");
}

#[test]
fn next_attempt_single_marker() {
    let (tmp, writer) = temp_run_dir();
    writer.write_started("design", 0).unwrap();
    let n = next_attempt(tmp.path(), "design").unwrap();
    assert_eq!(n, 1, "one marker at attempt 0 → next is 1");
}

#[test]
fn next_attempt_three_markers() {
    let (tmp, writer) = temp_run_dir();
    writer.write_started("design", 0).unwrap();
    writer.write_started("design", 1).unwrap();
    writer.write_started("design", 2).unwrap();
    let n = next_attempt(tmp.path(), "design").unwrap();
    assert_eq!(n, 3, "three markers (0,1,2) → next is 3");
}

#[test]
fn next_attempt_with_gaps() {
    let (tmp, writer) = temp_run_dir();
    // Markers for attempt 0 and 2 (missing 1 → simulates partial cleanup).
    writer.write_started("design", 0).unwrap();
    writer.write_started("design", 2).unwrap();
    let n = next_attempt(tmp.path(), "design").unwrap();
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
    let design_started = std::fs::read_to_string(markers_dir.join("design.started.0")).unwrap();
    let design_completed = std::fs::read_to_string(markers_dir.join("design.completed")).unwrap();
    let review_started = std::fs::read_to_string(markers_dir.join("review.started.0")).unwrap();
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

    assert!(
        markers_dir.exists(),
        "markers dir should be created on first write"
    );
    assert!(markers_dir.join("design.started.0").exists());
}

#[test]
fn different_phases_do_not_interfere() {
    let (tmp, writer) = temp_run_dir();
    writer.write_started("design", 0).unwrap();
    writer.write_started("implement", 0).unwrap();
    writer.write_started("review", 0).unwrap();

    let n_design = next_attempt(tmp.path(), "design").unwrap();
    let n_implement = next_attempt(tmp.path(), "implement").unwrap();
    let n_review = next_attempt(tmp.path(), "review").unwrap();

    assert_eq!(n_design, 1);
    assert_eq!(n_implement, 1);
    assert_eq!(n_review, 1);
}

// ---------------------------------------------------------------------------
// PhaseOrderGuard tests
// ---------------------------------------------------------------------------

#[test]
fn phase_order_guard_valid_transitions() {
    let mut guard = PhaseOrderGuard::new("design".to_string(), 0);
    assert_eq!(*guard.state(), PhaseState::Idle);

    guard.mark_started();
    assert_eq!(*guard.state(), PhaseState::Started);

    guard.mark_artefact_written();
    assert_eq!(*guard.state(), PhaseState::ArtefactWritten);

    guard.mark_manifest_appended();
    assert_eq!(*guard.state(), PhaseState::ManifestAppended);

    guard.mark_completed();
    assert_eq!(*guard.state(), PhaseState::Completed);
}

#[test]
#[should_panic(expected = "invalid transition")]
fn phase_order_guard_invalid_skip() {
    // Attempt Idle → Completed directly — should panic in debug.
    let mut guard = PhaseOrderGuard::new("design".to_string(), 0);
    guard.mark_completed();
}

// ---------------------------------------------------------------------------
// Heartbeat and is_stale tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn heartbeat_file_gets_created_and_contains_valid_json() {
    let tmp = tempfile::tempdir().unwrap();

    let config = HeartbeatConfig::new(tmp.path().to_path_buf());
    let _handle = HeartbeatWriter::spawn(config);

    // Give the task time to create the directory and write one tick.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    let heartbeat_path = tmp.path().join("heartbeat.json");
    assert!(
        heartbeat_path.exists(),
        "heartbeat file should exist after a tick"
    );

    let content = std::fs::read_to_string(&heartbeat_path).unwrap();
    let body: HeartbeatBody = serde_json::from_str(&content).unwrap();
    assert!(body.writer_pid > 0);
    assert!(!body.writer_host.is_empty());
}

#[tokio::test]
async fn heartbeat_file_updated_on_subsequent_ticks() {
    let tmp = tempfile::tempdir().unwrap();

    // Use a very short interval so we get multiple ticks quickly.
    let mut config = HeartbeatConfig::new(tmp.path().to_path_buf());
    config.interval_seconds = 1; // 1 second
    let _handle = HeartbeatWriter::spawn(config);

    // Wait long enough for 2 ticks.
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;

    let heartbeat_path = tmp.path().join("heartbeat.json");
    assert!(heartbeat_path.exists());

    let content = std::fs::read_to_string(&heartbeat_path).unwrap();
    let body: HeartbeatBody = serde_json::from_str(&content).unwrap();
    // The tick_at should be recent (within the last 2 seconds).
    let now = chrono::Utc::now();
    let age = now - body.tick_at;
    assert!(
        age.num_seconds() < 5,
        "heartbeat tick should be recent, but age is {}s",
        age.num_seconds()
    );
}

#[test]
fn is_stale_returns_true_when_expired() {
    let now = chrono::Utc::now();
    let ttl_seconds = 300;

    // Heartbeat from T-1s → not stale.
    let recent = chrono::Utc::now() - chrono::Duration::seconds(ttl_seconds as i64 - 1);
    let body = HeartbeatBody {
        writer_pid: 12345,
        writer_host: "host-a".to_string(),
        tick_at: recent,
    };
    assert!(!is_stale(&body, &now, ttl_seconds), "not stale at T-1s");

    // Heartbeat from T+1s → stale.
    let old = chrono::Utc::now() - chrono::Duration::seconds(ttl_seconds as i64 + 1);
    let body = HeartbeatBody {
        writer_pid: 12345,
        writer_host: "host-a".to_string(),
        tick_at: old,
    };
    assert!(is_stale(&body, &now, ttl_seconds), "stale at T+1s");
}

#[test]
fn is_stale_boundary_exact_ttl() {
    let ttl_seconds: u64 = 300;
    let now = chrono::Utc::now();

    // At exactly ttl seconds of age → not stale yet.
    let exactly_ttl_ago = now - chrono::Duration::seconds(ttl_seconds as i64);
    let body = HeartbeatBody {
        writer_pid: 12345,
        writer_host: "host-a".to_string(),
        tick_at: exactly_ttl_ago,
    };
    assert!(
        !is_stale(&body, &now, ttl_seconds),
        "not stale at exactly TTL"
    );

    // At ttl + 1s → stale.
    let one_second_over = now - chrono::Duration::seconds(ttl_seconds as i64 + 1);
    let body = HeartbeatBody {
        writer_pid: 12345,
        writer_host: "host-a".to_string(),
        tick_at: one_second_over,
    };
    assert!(is_stale(&body, &now, ttl_seconds), "stale at TTL+1s");
}
