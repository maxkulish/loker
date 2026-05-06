# Design: CLO-319 - First-write-wins per-phase advisory lock

## 1. Problem

Per the discovery report (`docs/discovery/clo-319.md`), operators running `loker resume` on workflows with HITL-blocked phases (T-048 `HumanVerifier`) can race two concurrent processes into the same paused phase. Today `RunLock` (`src/resume/lock.rs`) takes an exclusive advisory lock on `run_dir/.lock` for the entire run directory — too coarse to discriminate between phases, and offering no path to enumerating who currently holds work-in-progress for `loker ls --blocked` (T-044). Once `RunLock` is released after planning, two resume processes can simultaneously enter `PhaseRunner::run` for the same phase, create overlapping attempt directories, and overwrite markers. T-050 is the prerequisite locking layer before T-051 (per-gate fallback server) and T-053 (UI sessions list) can safely execute against paused phases.

## 2. Goals / Non-goals

### Goals

- Add a `PhaseLock` primitive at `src/run_state/phase_lock.rs` keyed by `(run_id, phase_name)`.
- Use OS-level advisory locking (`fs2::FileExt::try_lock_exclusive`) to enforce first-write-wins.
- Persist a JSON body (`PhaseLockBody`) into the lock file so non-holders can inspect state without acquiring.
- Detect and reclaim stale locks via PID liveness (Unix) and TTL comparison (Unix + Windows).
- Integrate into `ResumeRunner::run_phase` so the lock is acquired before `PhaseRunner::run` and released on drop.
- Surface `PhaseLockError::LockInUse` as a typed `ResumeError` variant with holder PID, host, phase, and timestamp.
- Default TTL of 60 s, overridable per call (shorter than the 300 s run-level heartbeat).

### Non-goals

- Distributed or cross-host locking.
- Database-backed lock store or external coordinator.
- Lock fairness, queueing, or wait-with-timeout semantics.
- Per-phase auto-fail timeouts (deferred to T-049).
- Rewriting `RunLock` or marker schemas; both stay as-is.
- Implementing `loker ls --blocked` itself (T-044 consumes the JSON exposed here).

## 3. Architecture

### Module layout

```
src/run_state/
  mod.rs              re-export PhaseLock, PhaseLockBody, PhaseLockError
  phase_lock.rs       NEW

src/resume.rs         ResumeRunner::run_phase acquires PhaseLock
src/resume/lock.rs    unchanged (RunLock stays as coarse outer guard)
```

### Run-directory layout (additive)

```
runs/<id>/
├── .lock                       existing RunLock (run-scoped)
├── manifest.json
├── attempts/
├── trace.jsonl
└── locks/                      NEW
    └── <phase>.lock            advisory lock fd + JSON body
```

### Data flow

```
loker resume <id>
        │
        ▼
ResumeRunner::execute
        │  RunLock::acquire(run_dir/.lock)        ← coarse, run-scoped
        │  plan = ResumePlanner::plan(...)
        │  RunLock dropped
        ▼
for each phase in plan:
   ResumeRunner::run_phase(phase)
        │  PhaseLock::acquire(run_dir, phase, run_id, ttl=60s)
        │      ├─ stat run_dir/locks/<phase>.lock
        │      ├─ if present, read body → check pid_alive(body.pid) && now-acquired_at<ttl
        │      │     ├─ live  → Err(LockInUse{...})
        │      │     └─ stale → fall through to try_lock_exclusive (OS is the real guard)
        │      ├─ open(create=true, truncate=false) + try_lock_exclusive
        │      └─ atomic_write(body) inside locks/
        │
        │  PhaseRunner::run(phase)                 ← runs strategy + HumanVerifier
        │  PhaseLock dropped → fd close → flock released
        ▼
report PhaseStatus
```

### Stale detection

- **Unix:** `libc::kill(pid as i32, 0) == 0`. `ESRCH` → process dead → stale.
- **Windows:** No PID-liveness probe (cross-platform `windows-sys` integration is out of scope for v0). Compare `now - acquired_at` against `ttl_seconds`; older → stale. Discovery debt notes this is acceptable while the project is single-machine-scoped.
- The OS advisory `try_lock_exclusive` remains the authoritative guard. Stale-body inspection only chooses whether to fall through to the lock attempt or short-circuit with `LockInUse`. If two processes both observe a stale body and race, only one wins `try_lock_exclusive`; the other gets `LockInUse` from the OS.

### Atomic-write reuse

The body is written via the same `atomic_write` helper used by `MarkerWriter` (D3 protocol: tmp file + rename in the same directory). The tmp file lives inside `locks/` to keep rename atomic on the same filesystem.

### Symlink hardening

Before opening `run_dir/locks/<phase>.lock`, validate that `run_dir` is a real directory (not a symlink), matching the pattern used by `sweep_stale_tmp`. The `<phase>` segment is sanitized to reject path separators, using the same normalization logic as `AttemptDir` (`src/run_state/attempt_dir.rs`) to ensure consistent filesystem behaviour across all run-directory artifacts.

## 4. Public API surface

### `src/run_state/phase_lock.rs`

```rust
use std::fs::File;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

/// Default TTL applied when [`PhaseLock::acquire`] is called with `ttl_seconds = None`.
/// Shorter than the run-level heartbeat TTL (300 s) so crashed phase workers
/// release faster than crashed run-level writers.
pub const DEFAULT_PHASE_LOCK_TTL_SECONDS: u64 = 60;

/// Advisory exclusive lock scoped to one phase within a run.
///
/// Drop releases the OS advisory lock by closing the fd and truncates the lock
/// file to 0 bytes so that [`PhaseLock::inspect`] returns `Ok(None)` for unlocked
/// phases. Use [`PhaseLock::release`] to release explicitly without relying on
/// drop order.
#[derive(Debug)]
pub struct PhaseLock {
    file: File,
    path: PathBuf,
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
    ) -> Result<Self, PhaseLockError>;

    /// Release the lock by truncating the lock file to 0 bytes and closing the fd.
    /// After release, `inspect` returns `Ok(None)` because the file is empty.
    /// Equivalent to `drop(self)` but explicit.
    pub fn release(self);

    /// Path to the lock file under `run_dir/locks/`.
    pub fn path(&self) -> &Path;

    /// Read lock state without holding the lock.
    /// Returns `Ok(None)` when no lock file exists.
    pub fn inspect(run_dir: &Path, phase: &str) -> Result<Option<PhaseLockBody>, PhaseLockError>;
}
```

### `src/run_state/mod.rs` (re-exports added)

```rust
pub use phase_lock::{
    PhaseLock,
    PhaseLockBody,
    PhaseLockError,
    DEFAULT_PHASE_LOCK_TTL_SECONDS,
};
```

### `src/resume.rs` (new error variant + integration)

```rust
#[derive(Debug, thiserror::Error)]
pub enum ResumeError {
    // ... existing variants ...

    #[error("phase '{phase}' is already being resumed by pid {pid} on {host} (since {since})")]
    PhaseLocked {
        phase: String,
        pid: u32,
        host: String,
        since: DateTime<Utc>,
    },

    #[error("phase lock error: {0}")]
    PhaseLock(#[from] PhaseLockError),
}

impl ResumeRunner {
    fn run_phase(&self, /* existing args */) -> Result<PhaseStatus, ResumeError> {
        let _phase_lock = PhaseLock::acquire(
            run_dir,
            &cfg.phase,
            &run_id.to_string(),
            None,
        )
        .map_err(|e| match e {
            PhaseLockError::LockInUse { phase, pid, host, since } => {
                ResumeError::PhaseLocked { phase, pid, host, since }
            }
            other => ResumeError::from(other),
        })?;

        // existing PhaseRunner::run call
    }
}
```

`src/main.rs` resume handler is unchanged; `RunLock` continues to wrap planning, and `PhaseLock` is acquired internally per phase by `ResumeRunner`.

## 5. Test plan

### Unit tests — `src/run_state/phase_lock.rs`

- `acquire_creates_lock_file_and_writes_body` — first acquire succeeds, file exists at `run_dir/locks/<phase>.lock`, body matches inputs (phase, run_id, current pid, current hostname, ttl).
- `concurrent_acquire_returns_lock_in_use` — first acquire held; second acquire on the same phase in the same process returns `LockInUse` carrying the first holder's pid/host/since.
- `acquire_different_phases_does_not_conflict` — two `PhaseLock::acquire` calls with different phase names both succeed.
- `stale_lock_with_dead_pid_is_reclaimable` (Unix-gated) — write a body with a synthetic dead pid (e.g. pid 1 in a test sandbox is not portable; instead spawn a child, capture its pid, wait for exit, then write that pid into a synthesized body), acquire succeeds, body is overwritten.
- `stale_lock_by_ttl_is_reclaimable` — write a body with `acquired_at = now - 2 * ttl` and current pid; acquire still succeeds because TTL says stale (cross-platform, exercises the Windows path too).
- `inspect_reads_without_holding` — acquire, then `PhaseLock::inspect` returns `Some(body)`; release, then inspect still returns the on-disk body until next acquire (lock file is not deleted on release).
- `inspect_returns_none_when_no_lock_file` — fresh run dir, inspect returns `Ok(None)`.
- `release_allows_reacquire` — explicit `release()`, second `acquire` succeeds and updates body.
- `invalid_phase_name_rejected` — phase containing `/` or `..` returns `PhaseLockError::InvalidPhaseName`.
- `corrupt_body_does_not_panic` — write garbage JSON into the lock file, acquire either reclaims (treating as stale) or returns a structured error; never panics.

### Integration tests — `tests/phase_lock.rs`

- `phase_lock_blocks_concurrent_resume` — set up a run with a `started` marker for phase `design`, spawn two threads both invoking `ResumeRunner::run_phase` for that phase against a stub `PhaseRunner`; assert exactly one returns success and the other returns `ResumeError::PhaseLocked`.
- `stale_lock_recovery_after_crash` — pre-populate `locks/design.lock` with a body whose pid is dead and timestamp older than TTL; assert `ResumeRunner::run_phase` reclaims and proceeds.
- `lock_body_exposes_required_fields_for_ls_blocked` — acquire a lock, parse the on-disk JSON, assert it contains `phase`, `run_id`, `writer_pid`, `writer_host`, `acquired_at`, `ttl_seconds` (the schema T-044 will consume).
- `concurrent_resume_on_different_phases_succeeds` — two threads, each acquiring for a different phase name on the same run; both succeed.

### Manual verification

1. Start a run that pauses at a HITL gate (HumanVerifier wired up per T-048 stub).
2. In terminal A: `loker resume <id>` — observe it block on the gate.
3. In terminal B: `loker resume <id>` — observe a clean error citing the holder pid + host + acquired-at timestamp, with no spawned backend work.
4. Kill terminal A, then run `loker resume <id>` again — observe the lock is reclaimed and resume proceeds.
5. Inspect `runs/<id>/locks/<phase>.lock` while terminal A is paused; confirm JSON body is human-readable.

### Regression

- All existing `cargo test` resume suites pass unmodified.
- `make check` (fmt + clippy + test) green.

## 6. Migration / rollout

- **Backward compatibility:** Purely additive. The `locks/` directory is created on first acquire; older run directories without it are unaffected. `RunLock` (`run_dir/.lock`) stays in place as the coarse outer guard during planning.
- **Marker schema:** Unchanged. No migration of `started`, `completed`, `failed`, or `manifest.json`.
- **Feature flag:** None. The lock is acquired unconditionally inside `ResumeRunner::run_phase`. Rationale: there is no existing per-phase lock to coexist with, and the failure mode (clean `PhaseLocked` error) is strictly safer than the current race.
- **Rollout order:**
  1. Land `phase_lock.rs` + unit tests (mergeable in isolation, no integration).
  2. Wire `ResumeRunner::run_phase` to acquire/release; add `ResumeError::PhaseLocked`; integration tests.
  3. Downstream: T-044 (`loker ls --blocked`) consumes `PhaseLock::inspect` / on-disk JSON.

## 7. Open questions

- **TTL default of 60 s.** Discovery debt flags 60 s as a working default (shorter than the 300 s run heartbeat). It is plausible that long-running phases with synchronous backend calls could legitimately exceed 60 s without the holder being dead. Tradeoff: shorter TTL recovers faster from crashes but risks false reclaim of healthy long phases; longer TTL is safer but delays recovery. Should `PhaseLock` participate in the existing heartbeat refresh loop (rewriting `acquired_at` periodically) instead of relying on a fixed TTL? Resolution depends on whether T-049 (per-phase auto-fail timeouts) wants to share the heartbeat path.
- **Windows PID-liveness check.** v0 falls back to TTL-only on Windows. If single-machine non-Unix users materialize, we may need `windows-sys::Win32::System::Threading::OpenProcess` to gate stale reclaim by liveness. Tradeoff: extra dependency surface vs. correctness of stale detection on Windows. Discovery scoped this out.
- **Lock-file lifecycle on release.** ❌ RESOLVED: On graceful release (`release()` / `drop`), the lock file is truncated to 0 bytes via `set_len(0)` on the held fd, then the fd is closed (releasing the OS advisory lock). `inspect` returns `Ok(None)` for empty files, which consumers interpret as "not held". This avoids having consumers (T-044) interpret staleness themselves. The race with concurrent `inspect` callers is benign: a truncated-but-not-empty file (race during write) causes a parse error that `inspect` reports as `Err`, which consumers can treat as "not held" (lenient reading).
- **Phase-name normalization.** If phase names ever contain characters that are valid in workflow YAML but problematic on Windows filesystems (e.g. `:`), the `<phase>.lock` filename could collide or fail to open. Should `PhaseLock` apply the same normalization as `AttemptDir`, or assume upstream validation? Tradeoff: defense in depth vs. divergent name spaces between attempt dirs and lock files.
