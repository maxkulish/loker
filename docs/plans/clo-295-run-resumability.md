# Plan: CLO-295 Run Resumability via Status Markers

## Context

- **Design:** `docs/designs/clo-295-run-resumability.md`
- **Discovery:** `docs/discovery/clo-295.md`
- **PRD:** `docs/prds/clo-295-run-resumability.md`
- **Linear:** https://linear.app/cloud-ai/issue/CLO-295/t-031-implement-run-resumability-via-status-markers
- **Branch:** `feat/clo-295-run-resumability`

## Sub-tasks

### ST1 Extend heartbeat schema with `ttl_seconds`
**Files:** `src/run_state/heartbeat.rs`, `Cargo.toml`, `src/run_state/run_dir.rs`
**What:** Add optional `ttl_seconds: u64` field to `HeartbeatBody` (default 300) so `resume` can recover the exact TTL used by the original run. Update `HeartbeatWriter` to persist it. Update `HeartbeatConfig` construction in `run_dir.rs`.
**Acceptance:** `cargo test heartbeat` passes; existing heartbeat tests still green.
**Estimate:** S

### ST2 Add `fs2` dependency
**Files:** `Cargo.toml`
**What:** Add `fs2 = "0.4"` to `[dependencies]` for cross-platform advisory file locks.
**Acceptance:** `cargo check` passes.
**Estimate:** XS

### ST3 Implement advisory lock (`src/resume/lock.rs`)
**Files:** `src/resume/lock.rs`, `src/resume.rs` (module declaration)
**What:** `RunLock::acquire(run_dir)` creates `run_dir/.lock`, acquires non-blocking exclusive lock via `fs2::FileExt::try_lock_exclusive()`. Implement `Drop` for auto-release. Return `ResumeError::LockInUse` if lock is held.
**Acceptance:** `cargo test resume_lock` passes (unit tests for success, contention, drop-release).
**Estimate:** S

### ST4 Implement stale tmp sweep (`src/resume/sweep.rs`)
**Files:** `src/resume/sweep.rs`, `src/resume.rs` (module declaration)
**What:** Walk `run_dir` recursively, find `*.tmp` files with `mtime > ttl_seconds`, move to `run_dir/attempts/_orphan_tmp/<timestamp>/`. Returns swept paths. Hard error on IO failure (disk-full, permission denied).
**Acceptance:** `cargo test resume_sweep` passes (unit tests for stale-sweep, no-match, io-error).
**Estimate:** S

### ST5 Implement archive operation
**Files:** `src/resume.rs` (archive helper), `src/run_state/attempt_dir.rs` (if adding to existing module)
**What:** `archive_current_attempt(run_dir, phase, attempt)` moves `run_dir/<phase>/` → `run_dir/attempts/<phase>/<attempt>/` via `std::fs::rename` (atomic on same fs) + parent fsync. Target must not exist. If cross-mount, fallback to copy+delete.
**Acceptance:** `cargo test archive_attempt` passes (unit tests for happy path, target-exists error, cross-filesystem).
**Estimate:** S

### ST6 Implement ResumePlanner + types
**Files:** `src/resume.rs`
**What:** Define `ResumeError`, `PhaseAction`, `ResumePlan`, `ResumePlanner`. Implement `ResumePlanner::plan()` that consumes `RunState::load()` output and produces per-phase actions (Skip / Resume / RunFresh). Include 5 planner unit tests covering all edge cases in the test plan.
**Acceptance:** `cargo test planner_` passes (all 5 unit tests from design §5.1).
**Estimate:** M

### ST7 Implement ResumeRunner and wire CLI subcommand
**Files:** `src/resume.rs`, `src/main.rs`
**What:** Implement `ResumeRunner::execute()` that iterates over `ResumePlan.actions`, calls `PhaseRunner::run()` for non-Skip phases, handles manifest entry chaining, and archives prior attempts for Resume actions. Wire `resume` subcommand in `src/main.rs` with `--ttl` optional flag. Use `heartbeat_ttl()` helper to read persisted TTL if present.
**Acceptance:** `cargo test runner_` passes; `cargo run -- resume --help` displays correct usage.
**Estimate:** M

### ST8 Write integration tests (`tests/resume.rs`)
**Files:** `tests/resume.rs`
**What:** Implement all 5 TDD scenarios from PRD / design §5.2:
1. Kill mid-phase-2
2. Already complete
3. Corrupt manifest entry
4. Live writer
5. Stale writer
Uses `tempfile`, `tokio::test`, and the in-memory `FakeClock`/`FakeWriter` patterns from existing tests.
**Acceptance:** `cargo test --test resume` passes.
**Estimate:** M

### ST9 Final pre-merge cleanup
**Files:** All modified files
**What:** Run `make check` (fmt + clippy + test). Fix any warnings. Update README with `resume` usage example if needed. Ensure no `unwrap()` calls in production code paths.
**Acceptance:** `make check` green.
**Estimate:** S

## Pre-merge gate

- `make check` (fmt + clippy + test)

## Risks

| Risk | Mitigation |
|------|------------|
| `fs2` does not compile on some CI target | fs2 is mature and cross-platform; if issues arise, switch to `nix::fcntl::flock` for Unix + manual lockfile for Windows |
| `PhaseRunner::run()` signature needs upstream manifest entries | Verify existing signature before ST7; if missing, add a `Vec<ManifestEntry>` parameter in a small preliminary refactor |
| `Workflow` → `PhaseConfig` adapter undefined | ST7 accepts `Vec<PhaseConfig>` from CLI layer; `Workflow` adapter deferred to follow-up issue |
| Integration tests are flaky due to timing | Use `FakeClock` from `heartbeat.rs` tests; do not depend on real wall-clock time |
