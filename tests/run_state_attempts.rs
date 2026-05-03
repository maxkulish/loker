use loker::manifest::{Kind, Manifest, ManifestEntry, Producer};
use loker::run_state::{next_attempt, AttemptDir, LatestPointer, MarkerWriter};

// ---------------------------------------------------------------------------
// Helper: create a temp directory to use as a run directory
// ---------------------------------------------------------------------------

fn temp_run_dir() -> (tempfile::TempDir, MarkerWriter) {
    let tmp = tempfile::tempdir().unwrap();
    let writer = MarkerWriter::new(tmp.path());
    (tmp, writer)
}

// ---------------------------------------------------------------------------
// 1. First attempt
// ---------------------------------------------------------------------------

#[test]
fn first_attempt_creates_dir_and_next_returns_zero() {
    let (tmp, _writer) = temp_run_dir();
    let n = next_attempt(tmp.path(), "design").unwrap();
    assert_eq!(n, 0, "no prior markers or dirs → attempt 0");

    let attempt_dir = AttemptDir::new(tmp.path(), "design", n);
    attempt_dir.create().unwrap();
    assert!(attempt_dir.path().exists());

    // Producer can write into the attempt dir
    let file = attempt_dir.path().join("design.md");
    std::fs::write(&file, b"first design attempt").unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "first design attempt"
    );
}

// ---------------------------------------------------------------------------
// 2. Second attempt after failure
// ---------------------------------------------------------------------------

#[test]
fn second_attempt_after_failure() {
    let (tmp, writer) = temp_run_dir();

    // Attempt 0 fails
    writer.write_started("design", 0).unwrap();
    writer
        .write_failed("design", 1, "SchemaMismatch", "attempts/design/0/")
        .unwrap();

    // Next attempt should be 1
    let n = next_attempt(tmp.path(), "design").unwrap();
    assert_eq!(n, 1, "after failed attempt 0 → next is 1");

    // Create attempt-1 dir
    let attempt_1 = AttemptDir::new(tmp.path(), "design", n);
    attempt_1.create().unwrap();
    std::fs::write(attempt_1.path().join("design.md"), b"attempt 1").unwrap();

    // Attempt-0 dir should not exist yet (no debris), but if we retroactively
    // create it to simulate a pre-existing archive, attempt-1 should still be next.
    let attempt_0 = AttemptDir::new(tmp.path(), "design", 0);
    std::fs::create_dir_all(attempt_0.path()).unwrap();
    std::fs::write(attempt_0.path().join("design.md"), b"attempt 0 debris").unwrap();

    // Re-derive next_attempt — should still be 1 (max of markers=1, dirs=1 → max=1, but +1 applied)
    // Wait: marker says attempt 0 started, so next_attempt_from_markers returns 1.
    // dirs has 0 and 1, so next_attempt_from_dirs returns 2.
    // max(1, 2) = 2. So next_attempt returns 2 after we created attempt-1 dir.
    let n2 = next_attempt(tmp.path(), "design").unwrap();
    assert_eq!(n2, 2, "dirs show attempt 1 exists → next is 2");

    // Attempt-0 debris should be untouched
    assert_eq!(
        std::fs::read_to_string(attempt_0.path().join("design.md")).unwrap(),
        "attempt 0 debris"
    );
}

// ---------------------------------------------------------------------------
// 3. Manifest entry pins attempt
// ---------------------------------------------------------------------------

#[test]
fn manifest_entry_pins_attempt() {
    let payload = b"design content";
    let entry = ManifestEntry::from_payload(
        "design/design.md",
        Kind::DesignMd,
        1,
        Producer::Single,
        Some("design".to_string()),
        Some(1),
        payload,
    );

    assert_eq!(entry.attempt, Some(1));
    assert_eq!(entry.name, "design/design.md");
    assert_eq!(entry.phase, Some("design".to_string()));

    // Round-trip through JSON
    let json = serde_json::to_string(&entry).unwrap();
    let restored: ManifestEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.attempt, Some(1));
    assert_eq!(restored.name, "design/design.md");
}

// ---------------------------------------------------------------------------
// 4. Latest pointer
// ---------------------------------------------------------------------------

#[test]
fn latest_pointer_resolves_to_last_completed() {
    let tmp = tempfile::tempdir().unwrap();

    // Simulate attempts 0, 1, 2 with attempt 2 completing
    LatestPointer::update(tmp.path(), "design", 0).unwrap();
    LatestPointer::update(tmp.path(), "design", 1).unwrap();
    LatestPointer::update(tmp.path(), "design", 2).unwrap();

    let resolved = LatestPointer::resolve(tmp.path(), "design").unwrap();
    let s = resolved.to_string_lossy();
    assert!(
        s.contains("attempts/design/2"),
        "latest should point to attempt 2, got: {s}"
    );
}

// ---------------------------------------------------------------------------
// 5. Attempt counter survives restart
// ---------------------------------------------------------------------------

#[test]
fn attempt_counter_survives_restart() {
    let tmp = tempfile::tempdir().unwrap();
    let writer = MarkerWriter::new(tmp.path());

    // Attempt 0 started
    writer.write_started("design", 0).unwrap();

    // Also create attempt-1 dir (simulates a crash after dir creation but before marker write)
    let attempt_1 = AttemptDir::new(tmp.path(), "design", 1);
    attempt_1.create().unwrap();
    std::fs::write(attempt_1.path().join("design.md"), b"attempt 1").unwrap();

    // "Restart": derive from disk only
    let n = next_attempt(tmp.path(), "design").unwrap();
    assert_eq!(n, 2, "markers say 1, dirs say 2 → next is 2");
}

// ---------------------------------------------------------------------------
// 6. Cross-phase isolation
// ---------------------------------------------------------------------------

#[test]
fn cross_phase_isolation() {
    let (tmp, writer) = temp_run_dir();

    writer.write_started("design", 0).unwrap();
    writer.write_started("design", 1).unwrap();

    writer.write_started("review", 0).unwrap();

    let n_design = next_attempt(tmp.path(), "design").unwrap();
    let n_review = next_attempt(tmp.path(), "review").unwrap();

    assert_eq!(n_design, 2, "design has markers 0,1 → next is 2");
    assert_eq!(n_review, 1, "review has marker 0 → next is 1");
}

// ---------------------------------------------------------------------------
// 7. Promotion is atomic
// ---------------------------------------------------------------------------

#[test]
fn promotion_is_atomic() {
    let tmp = tempfile::tempdir().unwrap();
    let attempt_dir = AttemptDir::new(tmp.path(), "design", 0);
    attempt_dir.create().unwrap();

    // Write file into attempt dir
    let attempt_file = attempt_dir.path().join("design.md");
    std::fs::write(&attempt_file, b"promoted content").unwrap();

    let canonical = tmp.path().join("design");
    attempt_dir.promote_to_canonical(&canonical).unwrap();

    // Canonical should have the file
    assert!(canonical.join("design.md").exists());
    assert_eq!(
        std::fs::read_to_string(canonical.join("design.md")).unwrap(),
        "promoted content"
    );

    // Attempt dir should be gone
    assert!(!attempt_dir.path().exists());
}

// ---------------------------------------------------------------------------
// 8. Archive on failure
// ---------------------------------------------------------------------------

#[test]
fn archive_on_failure_leaves_debris_intact() {
    let tmp = tempfile::tempdir().unwrap();
    let writer = MarkerWriter::new(tmp.path());

    // Attempt 0 starts and writes something, then fails
    let attempt_dir = AttemptDir::new(tmp.path(), "design", 0);
    attempt_dir.create().unwrap();
    std::fs::write(attempt_dir.path().join("design.md"), b"failed attempt").unwrap();

    writer.write_started("design", 0).unwrap();
    writer
        .write_failed("design", 1, "Timeout", "attempts/design/0/")
        .unwrap();

    // The attempt dir should still exist with its contents
    assert!(attempt_dir.path().exists());
    assert!(attempt_dir.path().join("design.md").exists());
    assert_eq!(
        std::fs::read_to_string(attempt_dir.path().join("design.md")).unwrap(),
        "failed attempt"
    );

    // next_attempt should still see attempt 0 and return 1
    let n = next_attempt(tmp.path(), "design").unwrap();
    assert_eq!(n, 1);
}

// ---------------------------------------------------------------------------
// Bonus: next_attempt_from_dirs only (no markers)
// ---------------------------------------------------------------------------

#[test]
fn next_attempt_from_dirs_without_markers() {
    let tmp = tempfile::tempdir().unwrap();

    // Create attempt dirs without any markers
    let d0 = AttemptDir::new(tmp.path(), "design", 0);
    d0.create().unwrap();
    let d2 = AttemptDir::new(tmp.path(), "design", 2);
    d2.create().unwrap();

    let n = next_attempt(tmp.path(), "design").unwrap();
    assert_eq!(n, 3, "dirs have 0 and 2 → next is 3");
}
