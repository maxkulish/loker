//! Integration tests for per-phase advisory locking.
//!
//! These tests verify the concurrent behaviour of `PhaseLock` across threads,
//! including stale lock recovery and the schema that `loker ls --blocked` (T-044)
//! will consume.

use std::sync::{Arc, Barrier};
use std::thread;

use loker::run_state::{PhaseLock, PhaseLockBody, PhaseLockError};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a minimal run directory structure with a `locks/` subdirectory.
fn setup_run_dir(tmp: &tempfile::TempDir) {
    std::fs::create_dir_all(tmp.path().join("locks")).unwrap();
    // We don't need markers, manifest, etc. — PhaseLock only needs runs/<id>/locks/.
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Two threads racing to lock the same phase: exactly one should succeed and
/// the other should get `LockInUse`.
#[test]
fn phase_lock_blocks_concurrent_resume() {
    let tmp = tempfile::tempdir().unwrap();
    setup_run_dir(&tmp);
    let run_dir = Arc::new(tmp.path().to_path_buf());
    let barrier = Arc::new(Barrier::new(2));

    let run_dir1 = Arc::clone(&run_dir);
    let barrier1 = Arc::clone(&barrier);
    let t1 = thread::spawn(move || {
        barrier1.wait(); // sync with t2
        PhaseLock::acquire(&run_dir1, "design", "run-1", None)
    });

    let run_dir2 = Arc::clone(&run_dir);
    let barrier2 = Arc::clone(&barrier);
    let t2 = thread::spawn(move || {
        barrier2.wait(); // sync with t1
        PhaseLock::acquire(&run_dir2, "design", "run-2", None)
    });

    let r1 = t1.join().expect("thread 1 panicked");
    let r2 = t2.join().expect("thread 2 panicked");

    // Exactly one should succeed, the other gets LockInUse.
    match (&r1, &r2) {
        (Ok(_), Err(PhaseLockError::LockInUse { .. }))
        | (Err(PhaseLockError::LockInUse { .. }), Ok(_)) => {} // expected
        other => panic!("expected one Ok + one LockInUse, got {:?}", other),
    }
}

/// Pre-populate a dead-PID lock body and verify it is reclaimed.
#[cfg(unix)]
#[test]
fn stale_lock_recovery_after_crash() {
    let tmp = tempfile::tempdir().unwrap();
    setup_run_dir(&tmp);

    // Spawn a child, capture its PID, wait for it to exit.
    let child = std::process::Command::new("true")
        .spawn()
        .expect("failed to spawn child");
    let dead_pid = child.id();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "child should exit successfully");

    // Write a lock body with the dead PID and a recent timestamp (within TTL).
    let body = PhaseLockBody {
        phase: "review".to_owned(),
        run_id: "stale-run".to_owned(),
        writer_pid: dead_pid,
        writer_host: "crash-host".to_owned(),
        acquired_at: chrono::Utc::now(),
        ttl_seconds: 60,
    };
    let lock_path = tmp.path().join("locks").join("review.lock");
    std::fs::write(
        &lock_path,
        serde_json::to_string_pretty(&body).unwrap(),
    )
    .unwrap();

    // Acquire should succeed because the PID is dead.
    let lock = PhaseLock::acquire(tmp.path(), "review", "new-run", None)
        .expect("should reclaim stale lock with dead PID");
    drop(lock);
}

/// Acquire a lock, then verify all six fields are present in the on-disk JSON.
#[test]
fn lock_body_exposes_required_fields_for_ls_blocked() {
    let tmp = tempfile::tempdir().unwrap();
    setup_run_dir(&tmp);

    let lock = PhaseLock::acquire(tmp.path(), "implement", "run-ls-blocked", Some(120))
        .expect("should acquire");
    let lock_path = tmp.path().join("locks").join("implement.lock");

    let contents = std::fs::read_to_string(&lock_path).unwrap();
    let body: serde_json::Value = serde_json::from_str(&contents).unwrap();

    assert!(
        body.get("phase").and_then(|v| v.as_str()) == Some("implement"),
        "phase field missing or wrong"
    );
    assert!(
        body.get("run_id").and_then(|v| v.as_str()) == Some("run-ls-blocked"),
        "run_id field missing or wrong"
    );
    assert!(
        body.get("writer_pid").and_then(|v| v.as_u64()).is_some(),
        "writer_pid field missing"
    );
    assert!(
        body.get("writer_host").and_then(|v| v.as_str()).is_some(),
        "writer_host field missing"
    );
    assert!(
        body.get("acquired_at").and_then(|v| v.as_str()).is_some(),
        "acquired_at field missing"
    );
    assert!(
        body.get("ttl_seconds").and_then(|v| v.as_u64()) == Some(120),
        "ttl_seconds field missing or wrong"
    );

    drop(lock);
}

/// Two threads acquiring for different phases on the same run both succeed.
#[test]
fn concurrent_resume_on_different_phases_succeeds() {
    let tmp = tempfile::tempdir().unwrap();
    setup_run_dir(&tmp);
    let run_dir = Arc::new(tmp.path().to_path_buf());
    let barrier = Arc::new(Barrier::new(2));

    let run_dir1 = Arc::clone(&run_dir);
    let barrier1 = Arc::clone(&barrier);
    let t1 = thread::spawn(move || {
        barrier1.wait();
        PhaseLock::acquire(&run_dir1, "design", "run-a", None)
    });

    let run_dir2 = Arc::clone(&run_dir);
    let barrier2 = Arc::clone(&barrier);
    let t2 = thread::spawn(move || {
        barrier2.wait();
        PhaseLock::acquire(&run_dir2, "review", "run-b", None)
    });

    let r1 = t1.join().expect("thread 1 panicked");
    let r2 = t2.join().expect("thread 2 panicked");

    assert!(r1.is_ok(), "first phase should acquire successfully");
    assert!(r2.is_ok(), "second phase should acquire successfully");
}
