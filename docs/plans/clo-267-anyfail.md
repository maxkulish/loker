# Plan: CLO-267 Implement Aggregator::AnyFail (first failure wins)

| Field | Value |
|-------|-------|
| Task | CLO-267 |
| Branch | `feat/clo-267-anyfail` |
| Design | `docs/designs/clo-267-anyfail.md` |
| Discovery | `docs/discovery/clo-267.md` |

## Context

Implement the `Aggregator::AnyFail` evaluation logic for `Strategy::ParallelFanOut`.
A new thin `src/aggregator/` module holds `any_fail_evaluate()` and `AnyFailReason`.
`StrategyError` gains `AnyFail` variant; `PhaseError` gains `AggregatorContract`.
Tests are mock-backend fixture driven with deterministic delays.

## Sub-tasks

### ST1 Scaffold new error variants and aggregator module
**Files:** `src/family.rs`, `src/strategy/mod.rs`, `src/aggregator/mod.rs` (new)
**Acceptance:** `cargo check` passes; module compiles with placeholder bodies.
**Estimate:** S

1. Add `PhaseError::AggregatorContract { message: String }` to `src/family.rs`.
2. Add `StrategyError::AnyFail { … }` and `AnyFailReason` enum to `src/strategy/mod.rs`.
3. Create `src/aggregator/mod.rs` with `any_fail_evaluate()` (skeleton returning `Ok(())` for now) + `strip_markdown_fences()`.
4. Wire `src/aggregator/mod.rs` into `src/lib.rs`.

### ST2 Extend MockBackend with deterministic delays
**Files:** `src/strategy/parallel_fanout.rs`
**Acceptance:** Existing `cargo test strategy::parallel_fanout` still green.
**Estimate:** S

1. Add `MockBackend::delayed_ok(name, text, delay_ms)` and `MockBackend::delayed_fail(name, err, delay_ms)` constructors.
2. Ensure `MockBackend::query` respects delay via `tokio::time::sleep`.

### ST3 Implement AnyFail walker in ParallelFanOut
**Files:** `src/strategy/parallel_fanout.rs`, `src/aggregator/mod.rs`
**Acceptance:** New `cargo test any_fail_` tests pass; existing parallel_fanout tests green.
**Estimate:** M

1. Implement `any_fail_evaluate` body in `src/aggregator/mod.rs`.
2. In `ParallelFanOut::execute`, branch on `self.aggregator == Aggregator::AnyFail` inside the `FuturesUnordered` loop.
   - Parse each `Ok(query)` with `any_fail_evaluate`.
   - First `Err(reason)` → return `StrategyError::AnyFail { backend, reason, offender, output }`.
   - Backend errors → `StrategyError::AnyFail` with `AnyFailReason::BackendError`.
   - Disable `min_responses` short-circuit under AnyFail (await all branches).
3. On success, set `verify: VerifyOutcome::passed("Aggregator::AnyFail")`.

### ST4 Add JSON-fixture-driven unit tests
**Files:** `src/strategy/parallel_fanout.rs` (tests module)
**Acceptance:** `cargo test any_fail` passes all 12 test cases.
**Estimate:** M

Test cases:
- `all_pass`
- `first_fails` (deterministic delay)
- `mid_list_fails` (deterministic delay)
- `all_fail`
- `backend_error_treated_as_fail`
- `missing_pass_field`
- `wrong_pass_type`
- `empty_query_text`
- `markdown_fenced_json`
- `markdown_fenced_fail`
- `valid_json_extra_keys`
- `non_deterministic_offender` (relaxed assertion)

### ST5 Snapshot schema validation
**Files:** `docs/schemas/verdict.schema.json` (new)
**Acceptance:** `cargo test` includes schema validation assertion on `StrategyOutput` JSON against `phase_result_parallel.schema.json`.
**Estimate:** S

1. Create `docs/schemas/verdict.schema.json`.
2. Write a test helper or inline assertion that serializes `StrategyOutput` and validates it.

### ST6 Pre-merge gate
**Acceptance:** `make check` (fmt + clippy + test) passes in ≤ 60 s.
**Estimate:** S

1. Run `make check`.
2. Fix any formatting, clippy, or test regressions.

## Pre-merge gate

- `make check` (fmt + clippy + test)

## Risks

| Risk | Mitigation |
|---|---|
| `MockBackend` delays add wall-clock time to tests | Keep delays tiny (1–10 ms); total test count is small. |
| `StrategyError` variant addition breaks downstream pattern matches | `#[non_exhaustive]` already on enum; downstream must handle `_` anyway. |
| FuturesUnordered short-circuit drops in-flight requests | Documented as v0 limitation (inherited from CLO-259). |

## References

- `docs/designs/clo-267-anyfail.md`
- `docs/prds/clo-267-anyfail.md`
- PRD FR-11
- `src/strategy/parallel_fanout.rs`
- `src/strategy/mod.rs`
