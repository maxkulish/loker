# Plan: CLO-325 Resume Round-Trip Integration Test

## Context

- **Design:** docs/designs/clo-325-resume-roundtrip.md
- **Discovery:** docs/discovery/clo-325.md
- **PRD:** docs/prds/clo-325-resume-roundtrip.md
- **Linear:** https://linear.app/cloud-ai/issue/CLO-325/add-runner-level-resume-round-trip-integration-test-after
- **Chosen approach:** Pure Rust in-process round-trip test (Approach A) using `MockBackend` + `PhaseRunner` + `ResumePlanner` + `ResumeRunner` in a single integration test file.
- **File:** `tests/resume_roundtrip.rs`

## Sub-tasks

### ST1 Add shared test helper fixtures

**Acceptance:** `cargo test --test resume_roundtrip test_helpers -- --nocapture` compiles and runs (may initially fail on missing tests; acceptance is that the helper functions type-check and are callable).

**Files:** `tests/resume_roundtrip.rs` (shared preamble)

**Details:** Write the reusable test infrastructure that all three scenarios depend on:

1. `make_mock_backend_pair()` — Returns two `Arc<MockBackend>` instances (one per phase) with canned text output. Reuses the existing `MockBackend` pattern from `tests/phase_runner_integration.rs`.

2. `build_roundtrip_run_dir(state: InitialState) -> Result<(TempDir, RunDir, Vec<PhaseConfig>)>` — Creates a temp directory, builds a `RunDir` with a two-phase workflow configuration, and sets the initial marker/manifest state per `InitialState`:
   - `AllComplete`: phases 1 and 2 both have `.completed` markers and manifest entries.
   - `InterruptedPhase`: phase 1 completed, phase 2 has `.started.0` marker but no terminal marker.
   - `FailedPhase`: phase 1 completed, phase 2 has `.failed` marker for attempt 0.

3. `run_phase_with_interrupt(...)` — Runs `PhaseRunner::run` for a phase and optionally interrupts mid-execution by writing only a `.started` marker then returning (no `.completed`/`.failed` marker).

4. `assert_sentinel_unchanged(...)` — Captures mtime of a phase sentinel file before resume and asserts it is unchanged after resume.

**Estimate:** M

### ST2 Add integration test `test_resume_roundtrip_all_complete`

**Acceptance:** `cargo test --test resume_roundtrip test_resume_roundtrip_all_complete -- --nocapture` passes.

**Files:** `tests/resume_roundtrip.rs`

**Details:** The simplest scenario:

1. Call `build_roundtrip_run_dir(InitialState::AllComplete)` to create a fully-completed two-phase run.
2. Capture mtime of phase 1 sentinel.
3. Load `RunState` via `RunState::load`.
4. Call `ResumePlanner::plan` — assert all actions are `PhaseAction::Skip`.
5. Call `ResumeRunner::execute` — assert `Ok(())`.
6. Assert sentinel mtime is unchanged (phase 1 was not re-run).
7. Assert manifest still marks workflow as completed.

**Estimate:** S

### ST3 Add integration test `test_resume_roundtrip_kill_phase2`

**Acceptance:** `cargo test --test resume_roundtrip test_resume_roundtrip_kill_phase2 -- --nocapture` passes.

**Files:** `tests/resume_roundtrip.rs`

**Details:**

1. Call `build_roundtrip_run_dir(InitialState::InterruptedPhase)` to set up a run where phase 1 completed and phase 2 has only `.started.0`.
2. Capture mtime of phase 1 sentinel.
3. Load `RunState` — assert phase 1 is `Completed`, phase 2 is `Started`.
4. `ResumePlanner::plan` — assert phase 1 is `Skip`, phase 2 is `Resume { next_attempt: 1 }`.
5. `ResumeRunner::execute` — assert `Ok(())`.
6. Assert phase 1 sentinel mtime unchanged.
7. Assert phase 2 has `.completed` marker and manifest entry with attempt index >= 1.
8. Assert run manifest marks workflow as completed.

**Estimate:** S

### ST4 Add integration test `test_resume_roundtrip_phase2_failed_then_retry`

**Acceptance:** `cargo test --test resume_roundtrip test_resume_roundtrip_phase2_failed_then_retry -- --nocapture` passes.

**Files:** `tests/resume_roundtrip.rs`

**Details:**

1. Call `build_roundtrip_run_dir(InitialState::FailedPhase)` where phase 2 has a `.failed` marker via mock backend error. The `MockBackend` is configured to return `Err(BackendError::Provider("simulated failure"))` for phase 2 attempt 0.
2. Capture mtime of phase 1 sentinel.
3. Load `RunState` — assert phase 1 `Completed`, phase 2 `Failed`.
4. `ResumePlanner::plan` — assert phase 1 `Skip`, phase 2 `Resume { next_attempt: 1 }`.
5. `ResumeRunner::execute` — assert `Ok(())`.
6. Assert phase 1 sentinel mtime unchanged.
7. Assert phase 2 has `.completed` marker for attempt 1 (no `.completed` for attempt 0).
8. Assert manifest correctly records the retried phase outcome.

**Estimate:** S

### ST5 Make check green

**Acceptance:** `make check` passes (fmt + clippy + test).

**Files:** (none — validation gate)

**Details:**

1. `cargo fmt --check`
2. `cargo clippy -- -D warnings`
3. `cargo test -q --test resume_roundtrip`

Fix any formatting, lint, or compilation issues before merging.

**Estimate:** S

## Pre-merge gate

```bash
make check  # fmt + clippy + cargo test
```

## Risks

1. **`RunState::load` path requirements:** If `RunState::load` requires a valid `manifest.json` with entries matching all markers, the `build_roundtrip_run_dir` helper must ensure manifest and markers are consistent. Low risk — existing tests (`tests/resume.rs`, `tests/run_state_load.rs`) already demonstrate this pattern.
2. **`ResumeRunner::execute` signature mismatch:** The design proposed a certain `execute` signature. The actual `ResumeRunner::execute(&self, plan: &ResumePlan)` takes `&ResumePlan` directly (not backends/run_dir/trace separately). If helper wiring diverges, adjust the test code — the plan is the source of truth.
3. **`PhaseConfig` construction:** Need to verify that two-phase `PhaseConfig` with `MockBackend` names works correctly with `ResumePlanner` and `ResumeRunner`. If `ResumeRunner` tries to resolve backends by name and fails, the test helper may need backend injection via `backends: Vec<Arc<dyn Backend>>`.
