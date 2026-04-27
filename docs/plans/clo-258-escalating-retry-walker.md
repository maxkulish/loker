# CLO-258 - EscalatingRetry Walker Implementation Plan

## Status

- [x] Task intake and dependency check
- [x] Design document
- [x] Codex design review
- [x] Implementation
- [x] Validation
- [x] Pre-PR review
- [x] PR

## Architecture Context

The new strategy primitive should be additive. Existing backend code already
exposes `Backend`, `QueryOutput`, and retryable `BackendError` variants. The
walker can be unit-tested with mock backends and mock verify hooks without
touching production CLIs, TensorZero, TOML parsing, or workflow execution.

## Tasks

- [x] Add `src/verify.rs`.
  - `VerifyResult` enum with Pass/Fail concrete variants plus Repair/Score
    reserved variants.
  - `VerifyHook` async trait over `QueryOutput`.

- [x] Add `src/strategy.rs`.
  - `Strategy::EscalatingRetry` config.
  - `EscalatingRetry` builder/default behavior.
  - `StrategyRunner` with backend registry.
  - `PhaseError`, `PhaseResult`, `EscalatingAttempt`, and JSON shape helpers.

- [x] Add focused tests.
  - First-pass success skips later backend calls.
  - Mid-list pass preserves earlier attempts and returns winner.
  - Exhaustion returns all attempts.
  - Non-retryable error does not block later backend.
  - `pass_failure_context` defaults false.
  - Phase result JSON matches escalating schema shape.

- [x] Export modules from `src/lib.rs`.

## Validation Commands

- `cargo test -q strategy::`
- `cargo test -q run_artefact_schemas_validate_their_fixtures`
- `make check`

## Open Risks

- Concrete verify hooks are intentionally still future work.
- TOML/runtime integration is intentionally future work.
