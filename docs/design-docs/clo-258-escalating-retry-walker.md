# CLO-258 - EscalatingRetry Walker

## Summary

Implement the M2 `Strategy::EscalatingRetry` walker as a new strategy boundary.
It runs an ordered backend ladder, applies a verify hook to successful backend
outputs, stops at the first verify pass, and returns structured attempt records
when the ladder is exhausted.

## Context

CLO-258 maps roadmap T-013 and PRD FR-7. CLO-249 has shipped the
`BackendError::is_retryable()` classification that the walker needs. The local
repo does not yet contain the T-020 `VerifyHook` trait / `VerifyResult` enum,
only the older `apply_verify` structs. To make this walker compile cleanly
without implementing concrete hooks, this task introduces the small trait/result
surface the strategy needs and leaves RunCommand, LLMVerifier, TestRunner, and
HumanVerifier to their roadmap tasks.

Existing backend contracts live in `src/backend/mod.rs`:

- `Backend::query(prompt, cwd, model)` returns `QueryOutput` or `BackendError`.
- `BackendError::is_retryable()` returns true for transient failures.
- `QueryOutput` carries backend/model/duration/usage metadata.

## Non-goals

- No concrete verify hook implementations.
- No phase runner or manifest writes.
- No prompt mutation for failure context; the field is scaffolded only.
- No TOML parser integration until strategy config tasks wire it into runtime
  workflows.

## Affected Files

- `src/lib.rs`: export new modules.
- `src/verify.rs`: minimal verify trait/result API.
- `src/strategy.rs`: strategy config, escalating walker, structured results,
  and unit tests with mock backends/hooks.
- `docs/status/clo-258-workflow.yaml`: lifecycle state.
- `docs/design-docs/clo-258-escalating-retry-walker.md`: this design.
- `docs/reviews/clo-258-codex-design-review.md`: design review.
- `docs/plans/clo-258-escalating-retry-walker.md`: implementation plan.

## Proposed Approach

Add a `verify` module with:

- `VerifyResult::{Pass, Fail, Repair, Score}` per FR-18, with helpers for
  `is_pass()` and phase-result status strings.
- `VerifyHook` async trait with `name()` and `verify(&QueryOutput)`.

Add a `strategy` module with:

- `BackendId = String`.
- `Strategy::EscalatingRetry(EscalatingRetry)`.
- `EscalatingRetry { backends, verify, pass_failure_context }`, where
  `pass_failure_context` defaults to false.
- `StrategyRunner` owning a map of backend IDs to `Arc<dyn Backend>`.
- `PhaseError::Exhausted { attempts }` and configuration/backend lookup errors.
- `EscalatingAttempt` records for backend outputs, backend errors, and verify
  results in execution order.
- `PhaseResult::Escalating { attempts, final_status }` with JSON conversion
  matching the existing `phase_result_escalating.schema.json` shape.

Execution semantics:

1. For each configured backend ID, call that backend with the original prompt.
2. If the backend returns `QueryOutput`, call the verify hook.
3. If verify passes, return a succeeded result immediately.
4. If verify fails/repairs/scores, record the attempt and continue.
5. If the backend returns `BackendError`, record the attempt and continue.
   Retryable/non-retryable is preserved in the attempt so phase runner policy
   can inspect it later; each failed backend still consumes exactly its slot.
6. If no attempt passes, return `PhaseError::Exhausted` with all attempts.

The task wording says retryability flags decide whether a transport-level
failure burns the current backend slot or moves to the next. With no nested
per-backend retry policy in scope, the walker records `retryable` and moves to
the next backend after the failed slot. `RetryExecutor` remains responsible for
within-backend retries where configured.

## Data/API/Config Changes

This adds public Rust APIs only. It does not add TOML fields yet.

`pass_failure_context` is present on `EscalatingRetry`, defaulting to false.
It is intentionally not used to append failed output to subsequent prompts in
this task; CLO-260 owns that behavior.

## Error Handling

- Empty backend ladder: `PhaseError::InvalidStrategy`.
- Unknown backend ID: `PhaseError::BackendNotFound`.
- Exhausted ladder: `PhaseError::Exhausted { attempts }`.
- Verify hook errors: represented as failed attempts with `VerifyResult::Fail`
  carrying the error string, so later backends can still run.

## Security And Compatibility

The walker does not shell out or access the network directly. It calls existing
backend and hook traits. It stores output paths/placeholders in result JSON but
does not write model output files, avoiding accidental secret persistence in
this slice.

Compatibility is additive: new modules are exported from `src/lib.rs`; existing
backend and workflow modules are untouched.

## Acceptance Criteria

- First-pass success returns immediately and does not call later backends.
- Mid-list pass returns the winner and keeps earlier failed attempts.
- Full exhaustion returns `PhaseError::Exhausted` with N attempts.
- Non-retryable backend errors are recorded and do not prevent later backends.
- `pass_failure_context` defaults false.
- Serialized escalating result shape matches the existing schema fixture style.

## Validation Plan

- `cargo test -q strategy::`
- `cargo test -q run_artefact_schemas_validate_their_fixtures`
- `make check`
