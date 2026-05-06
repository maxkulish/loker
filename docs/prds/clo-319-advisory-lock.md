# PRD: CLO-319 — First-write-wins per-phase advisory lock

## 1. Summary

Introduce a per-phase advisory lock primitive (`PhaseLock`) that prevents concurrent resolution of the same paused phase. The lock is filesystem-based, keyed by `(run_id, phase_name)`, and uses OS-level advisory locking (`flock` on Unix, `LockFileEx` on Windows) for first-write-wins semantics. Stale locks are reclaimable when the owning process is gone or the lock TTL has expired. Lock state is human-readable JSON on disk so that `loker ls --blocked` (T-044) can enumerate blocked phases.

## 2. Acceptance Criteria

- Two simultaneous resume attempts on the same paused phase result in exactly one winner; the loser sees a clear, actionable error.
- Stale locks (process gone or TTL exceeded) are reclaimable on the next acquire attempt.
- Lock state is exposed as a JSON file in the run directory, readable without holding the lock.
- Powers `loker ls --blocked` (T-044) by exposing lock state.

## 3. Non-goals

- Distributed locking across hosts.
- Database-backed lock store.
- Per-phase lock timeouts that auto-fail the phase (handled in T-049).
- Lock fairness or queueing (first-write-wins is sufficient).

## 4. Architecture

### Module layout

```
src/run_state/
  mod.rs              (re-export PhaseLock, PhaseLockError)
  phase_lock.rs       (NEW)

src/main.rs          (acquire PhaseLock in resume handler per phase)
src/resume.rs        (update run_phase to acquire + release PhaseLock)
```

### Lock location

```
runs/<id>/
├── locks/
│   └── <phase>.lock          # advisory lock file + JSON body
```

### Data types

```rust
use std::path::{Path, PathBuf};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Body written into the lock file so non-holders can inspect state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PhaseLockBody {
    pub phase: String,
    pub run_id: String,
    pub writer_pid: u32,
    pub writer_host: String,
    pub acquired_at: DateTime<Utc>,
    pub ttl_seconds: u64,
}

/// Errors emitted by PhaseLock operations.
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

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Advisory lock scoped to one phase within a run.
///
/// # Lifecycle
/// 1. `PhaseLock::acquire(run_dir, phase_name, ttl_seconds)` checks for stale lock,
///    opens/creates `locks/<phase>.lock`, and attempts `try_lock_exclusive`.
/// 2. On success, the lock body is atomically written into the file.
/// 3. On failure, the existing body is read and returned as `PhaseLockError::LockInUse`.
/// 4. Dropping `PhaseLock` closes the fd, releasing the OS advisory lock.
#[derive(Debug)]
pub struct PhaseLock {
    file: std::fs::File,
    path: PathBuf,
}
```

### Acquire algorithm

```
acquire(run_dir, phase, ttl_seconds):
    create locks/ directory if absent
    let path = run_dir.join("locks").join(format!("{phase}.lock"))

    // Stale detection: if file exists, read body
    if path.exists():
        let body = read_json(path)
        if body is valid:
            if body.writer_pid is alive AND now - body.acquired_at < ttl:
                // Lock is genuinely held
                return Err(LockInUse { ... })
            else:
                // Stale — owner is dead or TTL exceeded
                // We still need to try_lock_exclusive; if another process
                // already reclaimed it, we will get LockInUse from the OS.
                // If we win, overwrite body and proceed.
                pass

    // Open (create if absent) and attempt exclusive advisory lock
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)
    file.try_lock_exclusive() ?

    // Write fresh body atomically (tmp+rename inside locks/)
    let body = PhaseLockBody { phase, run_id, writer_pid: current_pid, writer_host, acquired_at: now, ttl_seconds }
    atomic_write(path, serde_json::to_string_pretty(body))

    return PhaseLock { file, path }
```

### Stale detection details

- **PID alive check (Unix):** `unsafe { libc::kill(pid as i32, 0) == 0 }`. If the call returns `ESRCH`, the process is dead.
- **PID alive check (Windows):** Fall back to timestamp-only check (no PID liveness). If `now - acquired_at > ttl_seconds`, treat as stale. This is acceptable because Windows is single-user-desktop in the target environment; PID reuse within TTL is extremely unlikely.
- **TTL:** Default 60 seconds, overridable by caller. Shorter than the run-level heartbeat TTL (300s) so that crashed phase workers release faster than crashed run-level writers.

## 5. Public API

### `src/run_state/phase_lock.rs`

```rust
impl PhaseLock {
    /// Acquire an exclusive advisory lock for `phase` under `run_dir`.
    ///
    /// `run_id` is embedded in the lock body for observability.
    /// `ttl_seconds` defaults to 60 if `None`.
    pub fn acquire(
        run_dir: &Path,
        phase: &str,
        run_id: &str,
        ttl_seconds: Option<u64>,
    ) -> Result<Self, PhaseLockError>;

    /// Release the lock early (close fd). Optional; drop does the same.
    pub fn release(self);

    /// Path to the lock file.
    pub fn path(&self) -> &Path;

    /// Read lock state without holding the lock.
    /// Returns `None` if no lock file exists.
    pub fn inspect(run_dir: &Path, phase: &str) -> Result<Option<PhaseLockBody>, PhaseLockError>;
}
```

## 6. Integration points

### `src/resume.rs` — `ResumeRunner::run_phase`

Before invoking `PhaseRunner::run`, acquire `PhaseLock` for the phase. On `PhaseLockError::LockInUse`, surface it as a `ResumeError` variant so the CLI prints the holder PID and host.

```rust
// Inside ResumeRunner::run_phase
let _phase_lock = PhaseLock::acquire(run_dir, &cfg.phase, &run_id.to_string(), None)
    .map_err(|e| match e {
        PhaseLockError::LockInUse { phase, pid, host, since } =>
            ResumeError::PhaseLocked { phase, pid, host, since },
        other => ResumeError::from(other),
    })?;
```

### `src/main.rs` — resume handler

No direct change needed if `ResumeRunner` acquires the lock internally. The existing `RunLock` (run-level) stays in place as a coarse guard.

### `src/strategy/verify/human_verifier.rs` — verifier callback

No direct change. The advisory lock is acquired by the resume runner before `PhaseRunner::run` is called, so it naturally covers the verifier callback as well.

## 7. Test plan

### Unit tests (`src/run_state/phase_lock.rs`)

- `acquire_creates_lock_file_and_lock_body` — acquire succeeds, body matches phase / pid / run_id.
- `concurrent_acquire_returns_lock_in_use` — two `PhaseLock::acquire` calls in the same process; second returns `LockInUse`.
- `stale_lock_is_reclaimable` — write an old body with a dead PID, then acquire succeeds and overwrites.
- `stale_lock_by_ttl_is_reclaimable` — write a body where `acquired_at` is older than TTL, acquire succeeds.
- `inspect_reads_without_holding` — acquire, then `PhaseLock::inspect` returns the body; release, then inspect returns `None`.
- `release_allows_reacquire` — explicit release, then second acquire succeeds.

### Integration tests (`tests/phase_lock.rs`)

- `phase_lock_blocks_concurrent_resume` — set up a run with a started marker, spawn two threads both trying to resume the same phase; assert exactly one succeeds.
- `stale_lock_recovery_after_crash` — write a lock body with a non-existent PID + old timestamp; assert resume succeeds and reclaims it.
- `lock_body_exposes_phase_run_id` — assert the JSON file contains expected fields for `loker ls --blocked` consumption.

### Regression

- `cargo test` all existing resume tests still pass (run-level lock untouched).
- `make check` (fmt + clippy + test) passes.

## 8. Migration / rollout

- Additive only: new `locks/` subdirectory; no changes to marker schema or manifest.
- `RunLock` (run-level, `run_dir/.lock`) remains unchanged.
- No feature flag required; the lock is acquired unconditionally by `ResumeRunner::run_phase`.

## 9. Security / threat model

- Symlink attack: `locks/` may be a symlink to an attacker-controlled path. Mitigation: validate that `run_dir` is a directory (not a symlink) before descending, matching the pattern in `sweep_stale_tmp`.
- Stale lock takeover: a malicious process could write a stale body with a dead PID to steal a lock. Mitigation: the OS advisory lock (`try_lock_exclusive`) is the real guard; the stale check is only to reclaim locks from crashed processes, not to authorize acquisition.
- PID reuse: on Unix, `kill(pid, 0)` returning success does not guarantee the same binary is running — only that a process with that PID exists. Given the 60s TTL and single-machine scope, this is acceptable (risk window is small and impact is bounded: the new process would incorrectly see the lock as held).
