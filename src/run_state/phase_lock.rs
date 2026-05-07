use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Body persisted into `run_dir/locks/<phase>.lock` so that non-holders can
/// inspect lock state via [`PhaseLock::inspect`] without acquiring the lock.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PhaseLockBody {
    pub phase: String,
    pub run_id: String,
    pub writer_pid: u32,
    pub writer_host: String,
    pub acquired_at: DateTime<Utc>,
    pub ttl_seconds: u64,
}

/// Errors returned by [`PhaseLock`] operations.
#[derive(Debug, thiserror::Error)]
pub enum PhaseLockError {
    #[error("phase '{phase}' is already locked by pid {pid} on {host} since {since}")]
    LockInUse {
        phase: String,
        pid: u32,
        host: String,
        since: DateTime<Utc>,
    },

    #[error("stale lock detected for phase '{phase}' but could not reclaim: {reason}")]
    StaleReclaimFailed { phase: String, reason: String },

    #[error("invalid phase name '{phase}': contains path separator or is empty")]
    InvalidPhaseName { phase: String },

    #[error("io error: {0}")]
    Io(#[from] io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Default TTL applied when [`PhaseLock::acquire`] is called with `ttl_seconds = None`.
/// Shorter than the run-level heartbeat TTL (300 s) so crashed phase workers
/// release faster than crashed run-level writers.
pub const DEFAULT_PHASE_LOCK_TTL_SECONDS: u64 = 60;

/// Advisory exclusive lock scoped to one phase within a run.
///
/// Drop releases the OS advisory lock by closing the fd and truncates the
/// lock file to 0 bytes so that non-holders can distinguish "not held"
/// (empty file or missing file) from "held" (valid JSON body).
/// Use [`PhaseLock::release`] to release explicitly without relying on
/// drop order.
#[derive(Debug)]
pub struct PhaseLock {
    file: File,
    path: PathBuf,
    lock_dir: PathBuf,
}

impl PhaseLock {
    /// Acquire an exclusive advisory lock on `run_dir/locks/<phase>.lock`.
    ///
    /// `run_id` is recorded in the lock body for observability.
    /// `ttl_seconds` defaults to [`DEFAULT_PHASE_LOCK_TTL_SECONDS`] when `None`.
    ///
    /// Returns `PhaseLockError::LockInUse` when another process holds the lock and
    /// the stored body is non-stale (live PID on Unix, or `now - acquired_at < ttl`).
    pub fn acquire(
        run_dir: &Path,
        phase: &str,
        run_id: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<Self, PhaseLockError> {
        let ttl = ttl_seconds.unwrap_or(DEFAULT_PHASE_LOCK_TTL_SECONDS);

        // Validate phase name (shared helper also used by `inspect`)
        validate_phase_name(phase)?;

        // Symlink hardening: ensure run_dir is a real directory (not a symlink)
        if !std::fs::symlink_metadata(run_dir)
            .map(|m| m.is_dir())
            .ok()
            .unwrap_or_default()
        {
            return Err(PhaseLockError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "run directory not found or is a symlink: {}",
                    run_dir.display()
                ),
            )));
        }

        let lock_dir = run_dir.join("locks");
        let lock_path = lock_dir.join(format!("{}.lock", phase));

        // Check for existing lock and staleness *before* trying the OS lock.
        // This is an optimization: if the holder is live, we short-circuit
        // without hitting the filesystem lock at all.
        // Use lenient reading: corrupt or unparseable JSON is treated as stale
        // (the OS lock is the authoritative guard).
        if lock_path.exists() {
            if let Ok(Some(body)) = read_lock_body(&lock_path) {
                if is_lock_body_live(&body) {
                    return Err(PhaseLockError::LockInUse {
                        phase: phase.to_owned(),
                        pid: body.writer_pid,
                        host: body.writer_host,
                        since: body.acquired_at,
                    });
                }
                // Stale — fall through to try the OS lock
            }
        }

        // Ensure the locks/ directory exists
        std::fs::create_dir_all(&lock_dir)?;

        // Open/create the lock file and try the OS advisory lock
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;

        file.try_lock_exclusive().map_err(|e| {
            if e.kind() == io::ErrorKind::WouldBlock {
                // The OS lock is held by another process.
                // Re-read the body (it may have been updated since our earlier check)
                // to construct a useful LockInUse error.
                match read_lock_body(&lock_path) {
                    Ok(Some(body)) => PhaseLockError::LockInUse {
                        phase: phase.to_owned(),
                        pid: body.writer_pid,
                        host: body.writer_host,
                        since: body.acquired_at,
                    },
                    _ => PhaseLockError::LockInUse {
                        phase: phase.to_owned(),
                        pid: 0,
                        host: "unknown".to_owned(),
                        since: Utc::now(),
                    },
                }
            } else {
                PhaseLockError::Io(e)
            }
        })?;

        // We hold the OS lock. Atomically write our body.
        let body = PhaseLockBody {
            phase: phase.to_owned(),
            run_id: run_id.to_owned(),
            writer_pid: std::process::id(),
            writer_host: hostname(),
            acquired_at: Utc::now(),
            ttl_seconds: ttl,
        };
        let json = serde_json::to_string_pretty(&body)?;
        // Write the body directly through our locked fd rather than via
        // atomic_write.  atomic_write creates a temp file and renames it to
        // the target path, which would produce a NEW inode — our flock is on
        // the OLD inode, so the lock would be lost.  Since we hold the
        // exclusive advisory lock, no other process can write concurrently.
        file.set_len(0)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;

        Ok(Self {
            file,
            path: lock_path,
            lock_dir,
        })
    }

    /// Release the lock by truncating the lock file and unlocking it.
    /// The body is cleared from the lock file so that `inspect` returns
    /// `Ok(None)`. The OS advisory lock is released when the fd is
    /// unlocked and closed.
    pub fn release(self) {
        // Explicitly trim and sync before dropping the file handle to ensure
        // immediate visibility to other phases.
        let _ = self.file.set_len(0);
        let _ = self.file.sync_all();

        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            // fs2 only guarantees the advisory lock is released when the fd is
            // closed, but forcing an unlock on Unix avoids a rare flakiness mode
            // where the underlying handle can be observed as still locked.
            let _ = unsafe { libc::flock(self.file.as_raw_fd(), libc::LOCK_UN) };
        }
    }

    /// Path to the lock file under `run_dir/locks/`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read lock state without holding the lock.
    ///
    /// Returns `Ok(None)` when no lock file exists or the file is empty.
    /// Returns `Err` if the file contains unparseable JSON (never panics).
    /// Validates the phase name to prevent path traversal.
    pub fn inspect(run_dir: &Path, phase: &str) -> Result<Option<PhaseLockBody>, PhaseLockError> {
        validate_phase_name(phase)?;
        let lock_path = run_dir.join("locks").join(format!("{}.lock", phase));
        if !lock_path.exists() {
            return Ok(None);
        }
        read_lock_body(&lock_path)
    }
}

impl Drop for PhaseLock {
    fn drop(&mut self) {
        // Truncate the lock file to 0 bytes so that `inspect` can distinguish
        // "not held" (empty) from "held" (valid JSON).
        let _ = self.file.set_len(0);
        let _ = self.file.sync_all();
        // fd close releases the OS advisory lock.
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Validate a phase name for use as a lock file path segment.
/// Rejects empty names, path separators, null bytes, and directory traps.
fn validate_phase_name(phase: &str) -> Result<(), PhaseLockError> {
    if phase.is_empty()
        || phase.contains('/')
        || phase.contains('\\')
        || phase.contains('\0')
        || phase.trim() == "."
        || phase.trim() == ".."
    {
        return Err(PhaseLockError::InvalidPhaseName {
            phase: phase.to_owned(),
        });
    }
    Ok(())
}

/// Read and deserialize a lock body from the given path.
/// Returns `Ok(None)` if the file exists but is empty.
fn read_lock_body(path: &Path) -> Result<Option<PhaseLockBody>, PhaseLockError> {
    let mut file = File::open(path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;

    if contents.trim().is_empty() {
        return Ok(None);
    }

    match serde_json::from_str(&contents) {
        Ok(body) => Ok(Some(body)),
        Err(e) => Err(PhaseLockError::Json(e)),
    }
}

/// Determine whether a lock body represents a live holder.
///
/// On Unix, checks PID liveness via `libc::kill(pid, 0)`.  On all platforms,
/// also checks whether the acquired-at + TTL window has expired.  A holder
/// is considered "live" only if BOTH the PID is alive (Unix) AND the TTL has
/// not expired.  If either check says "dead", the lock is stale.
fn is_lock_body_live(body: &PhaseLockBody) -> bool {
    let now = Utc::now();
    let age_secs = (now - body.acquired_at).num_seconds();
    // Use signed comparison: if acquired_at is in the future (clock skew or
    // corrupt body) age_secs is negative, which should not count as expired.
    if age_secs >= body.ttl_seconds as i64 {
        // TTL has expired — definitely stale regardless of PID.
        return false;
    }

    // Unix: check PID liveness as a second factor.
    #[cfg(unix)]
    {
        let pid = body.writer_pid as i32;
        // Safety: kill with sig=0 is a probe — it doesn't send a signal.
        // ESRCH means the process doesn't exist; any other error (EPERM)
        // means the process exists but can't be signalled (sufficient for us).
        unsafe {
            if libc::kill(pid, 0) != 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::ESRCH) {
                    return false; // process is dead
                }
                // EPERM or other: process exists but we can't probe it.
                // Treat as live to be safe.
            }
        }
    }

    // Windows fallback or TTL-within-range + PID alive: lock is live.
    true
}

/// Best-effort hostname. Falls back to "unknown" on error.
fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown".to_string())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a PhaseLockBody with a given pid and age offset.
    fn make_body(phase: &str, pid: u32, age_seconds: i64, ttl: u64) -> PhaseLockBody {
        let acquired_at = Utc::now() - chrono::Duration::seconds(age_seconds);
        PhaseLockBody {
            phase: phase.to_owned(),
            run_id: "test-run".to_owned(),
            writer_pid: pid,
            writer_host: "test-host".to_owned(),
            acquired_at,
            ttl_seconds: ttl,
        }
    }

    #[test]
    fn acquire_creates_lock_file_and_writes_body() {
        let tmp = tempfile::tempdir().unwrap();
        let phase = "design";
        let run_id = "test-run-001";

        let lock = PhaseLock::acquire(tmp.path(), phase, run_id, Some(60)).unwrap();
        let lock_path = tmp.path().join("locks").join("design.lock");

        assert!(lock_path.exists(), "lock file should exist");

        // Read the body back
        let body: PhaseLockBody = {
            let contents = std::fs::read_to_string(&lock_path).unwrap();
            serde_json::from_str(&contents).unwrap()
        };

        assert_eq!(body.phase, "design");
        assert_eq!(body.run_id, "test-run-001");
        assert_eq!(body.writer_pid, std::process::id());
        assert!(body.writer_host == "test-host" || !body.writer_host.is_empty());
        assert_eq!(body.ttl_seconds, 60);

        drop(lock);
    }

    #[test]
    fn concurrent_acquire_returns_lock_in_use() {
        let tmp = tempfile::tempdir().unwrap();
        let run_id = "test-run-002";

        let lock1 = PhaseLock::acquire(tmp.path(), "design", run_id, Some(60)).unwrap();
        let result = PhaseLock::acquire(tmp.path(), "design", run_id, Some(60));
        drop(lock1);

        assert!(
            matches!(result, Err(PhaseLockError::LockInUse { .. })),
            "expected LockInUse, got {:?}",
            result
        );
    }

    #[test]
    fn acquire_different_phases_does_not_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let run_id = "test-run-003";

        let lock1 = PhaseLock::acquire(tmp.path(), "design", run_id, Some(60)).unwrap();
        let lock2 = PhaseLock::acquire(tmp.path(), "review", run_id, Some(60)).unwrap();
        drop(lock1);
        drop(lock2);
    }

    #[test]
    fn stale_lock_by_ttl_is_reclaimable() {
        let tmp = tempfile::tempdir().unwrap();
        let run_id = "test-run-004";
        let phase = "design";
        let ttl = 60u64;

        // Pre-write a lock body that is clearly stale (age > ttl) with the
        // current PID — this mimics a lock left by a crashed process whose
        // TTL has elapsed but whose PID was reused.
        let body = make_body(phase, std::process::id(), ttl as i64 * 2, ttl);
        let lock_dir = tmp.path().join("locks");
        std::fs::create_dir_all(&lock_dir).unwrap();
        let lock_path = lock_dir.join("design.lock");
        // Create the file so the OS lock would be held... but we simulate a
        // crash by just writing the body without actually holding an advisory
        // lock. The acquire should succeed because the body is TTL-stale.
        std::fs::write(&lock_path, serde_json::to_string_pretty(&body).unwrap()).unwrap();

        // Acquire should succeed (stale lock is reclaimed)
        let lock = PhaseLock::acquire(tmp.path(), phase, run_id, Some(ttl)).unwrap();

        // The body should now be overwritten with fresh data
        let fresh_body: PhaseLockBody = {
            let contents = std::fs::read_to_string(&lock_path).unwrap();
            serde_json::from_str(&contents).unwrap()
        };
        assert_eq!(
            fresh_body.run_id, run_id,
            "body should be overwritten with new run_id"
        );
        assert_eq!(fresh_body.ttl_seconds, ttl);

        drop(lock);
    }

    #[test]
    fn inspect_reads_without_holding() {
        let tmp = tempfile::tempdir().unwrap();
        let run_id = "test-run-005";

        // No lock yet — inspect returns None
        assert!(
            PhaseLock::inspect(tmp.path(), "design").unwrap().is_none(),
            "no lock file yet"
        );

        let lock = PhaseLock::acquire(tmp.path(), "design", run_id, Some(60)).unwrap();

        // Inspect while held
        let body = PhaseLock::inspect(tmp.path(), "design")
            .unwrap()
            .expect("body should be readable");
        assert_eq!(body.run_id, "test-run-005");

        drop(lock);

        // After release, the file is removed, so inspect returns None.
        let body_after = PhaseLock::inspect(tmp.path(), "design").unwrap();
        assert!(
            body_after.is_none(),
            "lock file should be removed after release"
        );
    }

    #[test]
    fn inspect_returns_none_when_no_lock_file() {
        let tmp = tempfile::tempdir().unwrap();
        let result = PhaseLock::inspect(tmp.path(), "nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn release_allows_reacquire() {
        let tmp = tempfile::tempdir().unwrap();
        let run_id = "test-run-006";

        let lock = PhaseLock::acquire(tmp.path(), "design", run_id, Some(60)).unwrap();
        lock.release();

        // Should be able to acquire again after explicit release
        let lock2 = PhaseLock::acquire(tmp.path(), "design", run_id, Some(60)).unwrap();
        drop(lock2);
    }

    #[test]
    fn invalid_phase_name_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let run_id = "test-run-007";

        // Empty phase name
        let result = PhaseLock::acquire(tmp.path(), "", run_id, Some(60));
        assert!(matches!(
            result,
            Err(PhaseLockError::InvalidPhaseName { .. })
        ));

        // Phase with path separator
        let result = PhaseLock::acquire(tmp.path(), "design/../evil", run_id, Some(60));
        assert!(matches!(
            result,
            Err(PhaseLockError::InvalidPhaseName { .. })
        ));

        // Phase that is just ".."
        let result = PhaseLock::acquire(tmp.path(), "..", run_id, Some(60));
        assert!(matches!(
            result,
            Err(PhaseLockError::InvalidPhaseName { .. })
        ));
    }

    #[test]
    fn corrupt_body_does_not_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let run_id = "test-run-008";
        let phase = "design";
        let lock_dir = tmp.path().join("locks");
        std::fs::create_dir_all(&lock_dir).unwrap();
        let lock_path = lock_dir.join("design.lock");
        std::fs::write(&lock_path, b"this is not valid json").unwrap();

        // Acquire should either reclaim (treating corrupt body as stale) or
        // return a structured error — never panic.
        let result = PhaseLock::acquire(tmp.path(), phase, run_id, Some(60));
        match result {
            Ok(_) => {}                        // reclaimed, treating corrupt body as stale — fine
            Err(PhaseLockError::Json(_)) => {} // structured JSON error — fine
            Err(e) => panic!("unexpected error: {:?}", e),
        }
    }

    #[cfg(unix)]
    #[test]
    fn stale_lock_with_dead_pid_is_reclaimable() {
        let tmp = tempfile::tempdir().unwrap();
        let run_id = "test-run-009";
        let phase = "design";
        let ttl = 60u64;

        // Spawn a child process, capture its PID, wait for it to exit.
        let child = std::process::Command::new("true")
            .spawn()
            .expect("failed to spawn child");
        let dead_pid = child.id();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "child should exit successfully");

        // Write a lock body with the dead PID and a recent timestamp (within TTL).
        let body = make_body(phase, dead_pid, 1, ttl);
        let lock_dir = tmp.path().join("locks");
        std::fs::create_dir_all(&lock_dir).unwrap();
        let lock_path = lock_dir.join("design.lock");
        std::fs::write(&lock_path, serde_json::to_string_pretty(&body).unwrap()).unwrap();

        // Acquire should succeed because the PID is dead (our pid-liveness check
        // should detect it via libc::kill).
        let lock = PhaseLock::acquire(tmp.path(), phase, run_id, Some(ttl)).unwrap();
        drop(lock);
    }

    #[test]
    fn lock_body_has_required_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let run_id = "test-run-010";

        let lock = PhaseLock::acquire(tmp.path(), "implement", run_id, Some(120)).unwrap();
        let lock_path = tmp.path().join("locks").join("implement.lock");

        let contents = std::fs::read_to_string(&lock_path).unwrap();
        let body: serde_json::Value = serde_json::from_str(&contents).unwrap();

        assert!(body.get("phase").is_some(), "missing 'phase' field");
        assert!(body.get("run_id").is_some(), "missing 'run_id' field");
        assert!(
            body.get("writer_pid").is_some(),
            "missing 'writer_pid' field"
        );
        assert!(
            body.get("writer_host").is_some(),
            "missing 'writer_host' field"
        );
        assert!(
            body.get("acquired_at").is_some(),
            "missing 'acquired_at' field"
        );
        assert!(
            body.get("ttl_seconds").is_some(),
            "missing 'ttl_seconds' field"
        );

        assert_eq!(body["run_id"], run_id);
        assert_eq!(body["writer_pid"], std::process::id());
        assert_eq!(body["ttl_seconds"], 120);

        drop(lock);
    }
}
