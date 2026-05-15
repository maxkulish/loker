# Design: Runner-Level Resume Round-Trip Integration Test (CLO-325)

## 1. Problem

CLO-325 requires a concrete integration test that proves resume works end-to-end for real phase execution state. The CLI `loker resume` is already implemented, as is `PhaseRunner` marker persistence, but there is no round-trip integration test that creates real markers from execution, interrupts mid-phase, resumes, and verifies completed work is not re-run. Existing coverage has unit tests for planner decisions and guard clauses, but no test currently validates the full marker lifecycle (`.started` → missing terminal marker → resume skip/replay logic) plus sentinel mtime invariance. This is a correctness gap for users relying on recovery from interrupted runs.

## 2. Goals / Non-goals

### Goals

- Create an in-process round-trip test using production runner primitives (`PhaseRunner`, `ResumePlanner`, `ResumeRunner`) in `tests`.
- Generate real persisted phase artifacts and markers under an actual `RunDir` using the existing `run_state`/`run_dir` layout.
- Simulate phase interruption by creating a terminally incomplete phase run state that must be resumed.
- Verify `ResumePlanner` classifies earlier completed phases as `Skip` and interrupted/failed phases as resumable.
- Assert `ResumeRunner` resumes only incomplete phase(s) and does not duplicate completed work, using sentinel mtime invariance.
- Validate the final run manifest state to confirm the resumed workflow is marked as completed overall.
- Keep scope in-memory/backed by mock backend and temporary filesystem only (no subprocess or external service dependency).

### Non-goals

- Full binary-level CLI subprocess validation of `loker resume` signals and run lock contention. This is already covered by existing guard-level binary tests.
- New production runner behavior beyond test harness and fixture wiring.
- Runtime changes to marker semantics or resumability algorithm.
- Parallelization of the test matrix beyond the minimal scenarios required.

## 3. Architecture

### 3.1 Modules and data flow

The test will live in `tests/resume_roundtrip.rs` and compose existing modules:

1. `run_state::RunDir` + `run_state::RunState` to create/load run directories and marker/manifest state.
2. `phase_runner::PhaseRunner` to execute phases with persisted markers and artefacts.
3. `phase_runner::{PhaseConfig, PhaseInputs, ...}` and mock execution support already present in phase-runner integration tests.
4. `resume::{ResumePlanner, ResumeRunner}` to plan resumption and execute only needed phases.

Data flow:

```
Test setup (tempdir)
  └─> build phase definitions (PhaseConfig + mock backends)
      └─> execute phase1 normally via PhaseRunner::run(phase1, attempt=0)
          └─> execute phase2 partially (started marker written, no terminal marker)
              └─> load run_state via RunState::load(...)
                  └─> ResumePlanner::plan(run_state, phase configs)
                      └─> ResumeRunner::execute(plan, phase inputs)
                          └─> completed resume behavior + manifest update
```

### 3.2 Concrete test strategy

- **Scenario A – killed phase 2:** phase 1 completes, phase 2 is marked started without completed marker.
  - Assertion: resume skips phase 1, resumes phase 2.
- **Scenario B – failed then resume:** phase 1 completes, phase 2 gets failed attempt.
  - Assertion: resume archives failed attempt and re-runs phase 2.
  - Assertion: phase2 failure is created by mock backend returning an error so failure path is real (not synthetic marker-only injection).
- **Scenario C – all complete:** all phases already completed.
  - Assertion: resume no-ops all phases.

No subprocess for signal handling; interruption is simulated through persisted state mutation (missing terminal marker) to keep test deterministic.

("Sentinel mtime invariance" means no `sentinel` output file for fully completed prior phases changes its filesystem `mtime` across resume, proving those phases were skipped.

## 4. Public API surface

No new exported API is required; tests will use existing signatures and types. We only add helper functions in test module scope as needed.

### Runtime/test types and signatures

```rust
// From production
pub async fn PhaseRunner::run(
    &self,
    cfg: &PhaseConfig,
    inputs: PhaseInputs<'_>,
    initial_attempt: u32,
) -> Result<PhaseOutcome, PhaseError>;

pub async fn ResumePlanner::plan(
    state: &RunState,
    phase_configs: &[PhaseConfig],
    maybe_ctx: Option<&serde_json::Value>,
) -> ResumePlan;

pub async fn ResumeRunner::execute(
    &self,
    plan: ResumePlan,
    backends: &HashMap<String, Arc<dyn Backend>>,
    run_dir: &RunDir,
    trace: Option<Arc<TraceWriter>>,
    verify: Option<&Arc<dyn VerifyHook>>,
) -> Result<Vec<PhaseOutcome>, ResumeError>;
```

### Test-local helper signatures (proposed)

```rust
enum InitialState {
    AllComplete,
    InterruptedPhase,
    FailedPhase,
}

async fn build_roundtrip_run_dir(
    state: InitialState,
) -> Result<(TempDir, RunDir, Vec<PhaseConfig>), anyhow::Error>;
fn make_mock_backend_pair() -> (
    Arc<MockBackend>,
    Arc<MockBackend>,
);
async fn run_phase_with_interrupt(
    runner: &PhaseRunner,
    cfg: &PhaseConfig,
    inputs: PhaseInputs<'_>,
    run_dir: &RunDir,
    attempt: u32,
    leave_started_marker_only: bool,
) -> Result<(), TestError>;

async fn assert_sentinel_unchanged(
    run_dir: &RunDir,
    phase_name: &str,
    before: std::time::SystemTime,
) -> anyhow::Result<()>;
```

## 5. Test plan

### 5.1 Unit tests (helper-level)

- `build_roundtrip_run_dir` is parameterized by `InitialState` to reuse setup across all scenarios (`AllComplete`, `InterruptedPhase`, `FailedPhase`).
- `test_fake_marker_state_for_resume_plan`: directly build incomplete phase marker state and validate `ResumePlanner` output classes.
- `test_sentinel_helpers`: isolate sentinel mtime capture and equality checks.

### 5.2 Integration tests (`tests/resume_roundtrip.rs`)

1. **`test_resume_roundtrip_kill_phase2`**
   - Build two-phase workflow.
   - Execute phase1 successfully.
   - Execute phase2 with intentional incomplete state (`started` only).
   - Capture mtime of phase1 sentinel.
   - `ResumeRunner::execute` resumes from loaded state.
   - Assert: phase1 skipped, phase2 has completed marker and updated attempt/manifest entry; phase1 mtime unchanged; run manifest marks workflow as completed after resume.

2. **`test_resume_roundtrip_phase2_failed_then_retry`**
   - Similar setup.
   - Leave phase2 failed marker for attempt 0.
   - Resume and ensure phase2 has attempt 1 completed marker.
   - Assert phase1 not re-run.

3. **`test_resume_roundtrip_all_complete`**
   - Both phases completed.
   - Resume returns no-op outcomes and does not modify any sentinels.

### 5.3 Manual verification

- Run:
  - `cargo test -q --test resume_roundtrip`
- Confirm run completes quickly (no sleeps/wait loops) and deterministically.

## 6. Migration / rollout

No production migration needed. This is a test-only enhancement.

- Add one new integration test file and shareable helper fixture code.
- Keep test names aligned with existing `tests/resume.rs` for discoverability.
- Existing `run_state` and marker formats are reused unchanged.
- Rollout is automatic when tests land in branch and CI verifies `cargo test`.

## 7. Open questions

1. Should phase interruption be represented via helper that intentionally aborts backend execution mid-run or via direct marker injection?
   - **Decision:** helper-generated interrupted state (started marker + absent completion) keeps determinism and mirrors real-world partial writes.
2. Should scenario B assert failed attempt archival path explicitly (attempt index renaming / tmp cleanup)?
   - **Decision:** yes, assert manifest transition includes attempt archive entry or next-attempt `started` marker.
3. Should test additionally include mixed strategy types (single + parallel)?
   - **Decision:** initial coverage in these three scenarios using single-phase strategies is sufficient; extend later if `Parallel` introduces separate race-prone resume behavior.
