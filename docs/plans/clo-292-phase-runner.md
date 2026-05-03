# Plan: CLO-292 [T-028] Implement PhaseRunner composing Strategy + Aggregator + VerifyHook

## Context

- Design: `docs/designs/clo-292-phase-runner.md`
- Discovery: `docs/discovery/clo-292.md`
- PRD: `docs/prds/clo-292-phase-runner.md`
- Linear: https://linear.app/cloud-ai/issue/CLO-292/t-028-implement-phaserunner-composing-strategy-aggregator

CLO-292 adds the thin phase-level coordinator between existing strategy, aggregator, verify, manifest, and run-state marker primitives. The implementation must remain additive: no CLI wiring, no workflow TOML parser, and no replacement of existing legacy command paths.

## Sub-tasks

### ST1 Extend aggregator vocabulary for PhaseRunner labels

**Files:** `src/aggregator/concat.rs`, `src/aggregator/mod.rs`, `src/strategy/mod.rs`

**Work:**
- Add additive `First` and `AllPass` variants to the runtime `aggregator::Aggregator` enum.
- Add matching schema-facing `strategy::Aggregator::{First, AllPass}` labels and `as_str()` mappings.
- Implement minimal behavior needed by PhaseRunner: `First` selects the first successful branch output; `AllPass` inspects every branch/verification verdict before failing so diagnostics are complete.
- Keep existing concat, vote, LLM judge, and any-fail behavior unchanged.

**Acceptance:** `cargo test -q aggregator_first_and_all_pass_variants_map_to_schema_labels` passes

**Estimate:** S

### ST2 Add PhaseRunner public API shell and library exports

**Files:** `src/phase_runner.rs`, `src/lib.rs`

**Work:**
- Introduce `PhaseRunner`, `PhaseConfig`, `PhaseInputs`, `PhaseOutcome`, `PhaseError`, `StrategyName`, `AggregatorName`, and `VerifyHookName` as designed.
- Re-export the new API surface from `src/lib.rs`.
- Keep the initial `PhaseRunner::run` implementation minimal or stubbed until later subtasks fill dispatch and persistence.

**Acceptance:** `cargo test -q phase_runner_api_surface_compiles` passes

**Estimate:** S

### ST3 Implement name dispatch helpers

**Files:** `src/phase_runner.rs`, `src/phase_runner/dispatch.rs`, `src/strategy/single_model.rs`, `src/strategy/parallel_fanout.rs`, `src/strategy/escalating_retry.rs`, `src/strategy/verify/mod.rs`

**Work:**
- Resolve `StrategyName::{Single, Parallel, EscalatingRetry}` to the existing strategy implementations.
- Resolve `AggregatorName::{First, Concat, Vote, AnyFail, AllPass}` to existing/additive aggregator values.
- Resolve `VerifyHookName::{None, RunCommand, LlmVerifier}` by using caller-provided hook instances for executable verifiers and returning `None` for no-op verification.
- Add deterministic config validation errors through `PhaseError` rather than panics.

**Acceptance:** `cargo test -q name_dispatch_resolves_known` passes

**Estimate:** M

### ST4 Implement persistence helpers and error-class mapping

**Files:** `src/phase_runner.rs`, `src/phase_runner/persist.rs`, `src/manifest.rs`, `src/run_state/markers.rs`, `docs/run-state.md`

**Work:**
- Implement `start_attempt`, `archive_failed_attempt`, `commit_success`, and `record_terminal_failure`.
- Ensure success order is artefact atomic write → manifest append → completed marker.
- Ensure failure order is archive runner-owned debris/failure summary → failed marker.
- Map `PhaseError` variants to stable marker `error_class` strings: `strategy_failed`, `verify_failed`, `aggregator_failed`, `manifest_failed`, `marker_failed`, `io_failed`.

**Acceptance:** `cargo test -q phase_runner_persist` passes

**Estimate:** M

### ST5 Implement single-strategy success path

**Files:** `src/phase_runner.rs`, `src/phase_runner/dispatch.rs`, `src/phase_runner/persist.rs`, `tests/phase_runner_integration.rs`

**Work:**
- Wire `PhaseRunner::run` for the simplest success case: `single` strategy, `first` aggregator, `none` verify.
- Read canonical bytes from the winning `Attempt.output_path` surfaced by `StrategyOutput`.
- Persist exactly one canonical artefact, exactly one manifest entry, and `<phase>.completed`.

**Acceptance:** `cargo test -q single_first_no_verify_emits_one_artefact_and_completed_marker` passes

**Estimate:** M

### ST6 Implement parallel aggregation and verifier success paths

**Files:** `src/phase_runner.rs`, `src/phase_runner/dispatch.rs`, `src/phase_runner/persist.rs`, `tests/phase_runner_integration.rs`, `tests/fixtures/`

**Work:**
- Wire parallel strategy outputs through concat/vote/any-fail/all-pass aggregation where applicable.
- Run configured verify hooks against the canonical artefact before commit.
- Use wiremock and stub verifier hooks only; do not require live network or local external commands beyond controlled test fixtures.
- Add coverage for all-pass collecting all failures before returning a typed failure.

**Acceptance:** `cargo test -q phase_runner_parallel` passes

**Estimate:** M

### ST7 Implement escalating retry and terminal failure behavior

**Files:** `src/phase_runner.rs`, `src/phase_runner/persist.rs`, `tests/phase_runner_integration.rs`, `tests/fixtures/`

**Work:**
- Ensure escalating retry can recover after earlier failed verifier outcomes and commit only the final passing attempt.
- Archive failed attempts under `attempts/<phase>/<n>/` before subsequent attempts or terminal markers.
- On terminal verifier or strategy failure, write `<phase>.failed` with the correct `error_class` and propagate the typed `PhaseError`.

**Acceptance:** `cargo test -q phase_runner_retry_and_failure` passes

**Estimate:** M

### ST8 Run full pre-merge validation and tidy documentation

**Files:** `docs/designs/clo-292-phase-runner.md`, `docs/plans/clo-292-phase-runner.md`, implementation/test files touched above

**Work:**
- Run the full repository check.
- Fix formatting, clippy, and test failures.
- Update docs only if implementation discovered a small drift from the finalized design; otherwise leave design intact.

**Acceptance:** `make check` passes

**Estimate:** S

## Pre-merge gate

- `make check` (fmt + clippy + test)

## Risks

- Existing strategy implementations may not expose enough canonical output information for every aggregator path; keep any adapter code private to `phase_runner` and avoid expanding public strategy output unless tests prove it necessary.
- Marker file naming and attempt indexing must align with `docs/run-state.md` and existing `MarkerWriter` behavior; persistence tests should pin this before integration tests depend on it.
- `RunCommand` and `LLMVerifier` can involve external effects in production; tests must use stubs/wiremock so `make check` remains network-free.
- Extending aggregator enums is additive, but schema snapshots/tests may need updates for the new `first` and `all_pass` labels.
