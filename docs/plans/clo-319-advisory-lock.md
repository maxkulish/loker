# Plan: CLO-319 — First-write-wins per-phase advisory lock

## Context

- **Design:** `docs/designs/clo-319-advisory-lock.md`
- **Discovery:** `docs/discovery/clo-319.md`
- **PRD:** `docs/prds/clo-319-advisory-lock.md`
- **Linear:** https://linear.app/cloud-ai/issue/CLO-319/t-050-first-write-wins-per-phase-advisory-lock
- **Branch:** `feat/clo-319-advisory`

## Sub-tasks

### ST1 Create `PhaseLock` primitive with unit tests

**Files:**
- `src/run_state/phase_lock.rs` (NEW) — core lock primitive
- `src/run_state/mod.rs` — add `pub(crate) mod phase_lock;` and re-exports

**Acceptance:** `cargo test --lib run_state::phase_lock` passes all 10 unit tests

**Changes in `src/run_state/phase_lock.rs`:**

1. `PhaseLockBody` struct with `phase`, `run_id`, `writer_pid: u32`, `writer_host: String`, `acquired_at: DateTime<Utc>`, `ttl_seconds: u64`. Derive `Serialize`/`Deserialize` with `#[serde(deny_unknown_fields)]`.

2. `PhaseLockError` enum with variants:
   - `LockInUse { phase, pid, host, since }`
   - `StaleReclaimFailed { phase, reason }`
   - `InvalidPhaseName { phase }`
   - `Io(#[from] std::io::Error)`
   - `Json(#[from] serde_json::Error)`

3. `DEFAULT_PHASE_LOCK_TTL_SECONDS: u64 = 60`

4. `PhaseLock { file: File, path: PathBuf }` with:
   - **`acquire(run_dir, phase, run_id, ttl_seconds)`** — Validate phase name (reject empty, `/`, `..` using the same `path_segment_is_safe` pattern as `AttemptDir`); ensure `locks/` directory exists; check for existing lock body and determine staleness (Unix: `libc::kill(pid, 0)` PID liveness + TTL; Windows/fallback: TTL-only); `File::create_new` + `fs2::FileExt::try_lock_exclusive` for OS-level first-write-wins; write body atomically via `atomic_write`.
   - **`release(self)`** — Truncate lock file to 0 bytes and close fd (drop also does this).
   - **`path(&self) -> &Path`** — Return the lock file path.
   - **`inspect(run_dir, phase) -> Result<Option<PhaseLockBody>>`** — Read-only; returns `Ok(None)` if file missing or empty, parses body if present, returns structured error on corrupt JSON (never panics).
   - **Drop impl** — Truncate file to 0 bytes, close fd (same as `release`, called automatically).

5. `fn hostname() -> String` — Same pattern as `src/run_state/markers.rs` (`HOSTNAME` env var → `HOST` env var → `"unknown"`).

6. **Dependencies used** (already in `Cargo.toml`):
   - `fs2::FileExt::try_lock_exclusive` — OS advisory lock
   - `chrono::Utc` — timestamps
   - `serde` / `serde_json` — body serialization
   - `thiserror` — error derives
   - `std::os::unix` (conditionally) — `libc::kill` for PID liveness

**Unit tests (10):**
1. `acquire_creates_lock_file_and_writes_body`
2. `concurrent_acquire_returns_lock_in_use`
3. `acquire_different_phases_does_not_conflict`
4. `stale_lock_with_dead_pid_is_reclaimable` (Unix-gated)
5. `stale_lock_by_ttl_is_reclaimable`
6. `inspect_reads_without_holding`
7. `inspect_returns_none_when_no_lock_file`
8. `release_allows_reacquire`
9. `invalid_phase_name_rejected`
10. `corrupt_body_does_not_panic`

**Estimate:** M

---

### ST2 Wire `PhaseLock` into `ResumeRunner::run_phase`

**Files:**
- `src/resume.rs` — add `ResumeError::PhaseLocked` variant, `#[from] PhaseLockError`, acquire lock in `run_phase`

**Acceptance:** `cargo build` compiles; `cargo test --lib resume` passes existing tests unmodified

**Changes in `src/resume.rs`:**

1. Add to `ResumeError`:
   ```rust
   #[error("phase '{phase}' is already being resumed by pid {pid} on {host} (since {since})")]
   PhaseLocked { phase: String, pid: u32, host: String, since: DateTime<Utc> },

   #[error("phase lock error: {0}")]
   PhaseLock(#[from] PhaseLockError),
   ```

2. In `run_phase(…)`, before `runner.run(cfg, inputs, attempt)`:
   ```rust
   let _phase_lock = PhaseLock::acquire(
       run_dir,
       &cfg.phase,
       &run_id.to_string(),
       None,   // use default TTL (60s)
   )
   .map_err(|e| match e {
       PhaseLockError::LockInUse { phase, pid, host, since } => {
           ResumeError::PhaseLocked { phase, pid, host, since }
       }
       other => ResumeError::from(other),
   })?;
   ```

   The `_phase_lock` binding keeps the lock alive until `run_phase` returns; drop releases it.

3. Add `use crate::run_state::{..., PhaseLock, PhaseLockError};` to imports.

**Estimate:** S

---

### ST3 Integration tests for phase locking

**Files:**
- `tests/phase_lock.rs` (NEW)

**Acceptance:** `cargo test --test phase_lock` passes all 4 integration tests

**Changes in `tests/phase_lock.rs`:**

1. **`phase_lock_blocks_concurrent_resume`** — Set up a run dir with a `started` marker for a phase. Spawn two threads both calling a stub `ResumeRunner::run_phase`-like flow. Assert exactly one returns success, the other returns `PhaseLocked` error.

2. **`stale_lock_recovery_after_crash`** — Pre-populate `locks/design.lock` with a body containing a dead PID (spawn a child, capture PID, wait for exit) and timestamp older than TTL. Assert that `PhaseLock::acquire` succeeds (reclaims) and proceeds.

3. **`lock_body_exposes_required_fields_for_ls_blocked`** — Acquire a lock, read the on-disk JSON, assert all six fields present (`phase`, `run_id`, `writer_pid`, `writer_host`, `acquired_at`, `ttl_seconds`).

4. **`concurrent_resume_on_different_phases_succeeds`** — Two threads each acquiring for a different phase on the same run dir; both succeed.

**Estimate:** M

---

### ST4 Pre-merge gate

**Files:** — (no new files)

**Acceptance:** `make check` (fmt + clippy + test) green

**Checks:**
1. `cargo fmt --all --check` — no formatting issues
2. `cargo clippy --all-targets -- -D warnings` — no lint violations
3. `cargo test` — all unit tests pass
4. `cargo test --test phase_lock` — all integration tests pass
5. `cargo test --test resume` — existing resume tests unmodified

**Estimate:** S

---

## Pre-merge gate

- `make check` (fmt + clippy + test)

## Risks

1. **`libc` dependency for PID liveness** — The design uses `libc::kill(pid, 0)` on Unix. If `libc` is not already a dependency, add it to `Cargo.toml`. Fallback: use `nix` crate or raw `extern "C"` FFI. Low risk since `libc` is a common transitive dep.

2. **Windows `fs2::FileExt::try_lock_exclusive`** — `fs2` already supports Windows via `LockFileEx`, but PID-liveness requires a separate code path. The TTL-only fallback on Windows is acceptable for v0 but means stale detection is less precise. Documented in design open questions.

3. **`atomic_write` integration** — The lock body must be written atomically to prevent partial reads by concurrent `inspect` callers. Reuse the existing `atomic_write` helper from `src/run_state/atomic.rs` (tmp file + rename on same filesystem). Test that the tmp file path resolves inside `locks/` to keep rename atomic.

4. **Race: stale check → OS lock** — Two processes could both see a stale lock, race into `try_lock_exclusive`, and only one wins. This is by design — the OS lock is the real guard; the stale check is an optimisation. Ensure error handling maps OS-level `try_lock_exclusive` failures back to `LockInUse` (with the *new* holder's body, read after failure).

5. **Phase name edge cases** — Phase names with special characters (e.g. `:`, `\0` on Windows) could produce invalid lock file paths. Use the same `path_segment_is_safe` validation as `AttemptDir`. If no such function exists, implement inline: reject empty strings, strings containing `/` or `\0` or `..`.

6. **`make check` environment** — The integration tests in `tests/phase_lock.rs` use `tempfile::tempdir` and thread spawning. Verify they don't have race conditions in CI with constrained resources. Use `std::sync::Barrier` for deterministic ordering in concurrent tests.

## Rollout order (from design)

1. ✅ **ST1** → `phase_lock.rs` + unit tests (mergeable in isolation)
2. ✅ **ST2** → wire `ResumeRunner::run_phase` (depends on ST1)
3. ✅ **ST3** → integration tests (depends on ST2)
4. ✅ **ST4** → pre-merge gate (depends on ST1–ST3)
