# Implementation Plan: CLO-259 — Strategy::ParallelFanOut with min_responses floor

| Field | Value |
|---|---|
| Task | CLO-259 |
| Design | docs/designs/clo-259-parallel-fanout.md |
| Branch | feat/clo-259-parallel |

## Summary

6 sub-tasks across 3 implementation phases:
1. Extend shared types (`StrategyKind`, `StrategyError`, `StrategyOutput`, `Attempt`)
2. Implement `ParallelFanOut` strategy
3. Wire module and re-exports
4. Unit tests (mock backends)
5. Integration tests (schema validation)
6. Manual verification (`make check`, cargo test)

## Sub-tasks

### T-1: Extend `StrategyKind` and `StrategyError`

**File**: `src/strategy/mod.rs`
**Effort**: S (~15 min)
**Depends on**: None

- Add `Parallel` variant to `StrategyKind`.
- Add `FloorViolation { successes, min_responses, output: Box<StrategyOutput> }` to `StrategyError`.
- Update `FutureVariantCompiles` test stub to include `ParallelFanOut` (compile-time object-safety check).

**AC**:
- `cargo check` passes.
- Existing tests still compile.

---

### T-2: Extend `StrategyOutput` serialization for parallel schema

**File**: `src/strategy/mod.rs`
**Effort**: M (~30 min)
**Depends on**: T-1

- Add `family: Option<String>` to `Attempt` (`#[serde(skip_serializing_if = "Option::is_none")]`).
- Add `aggregate_output_path: Option<String>` and `aggregator: Option<String>` to `StrategyOutput` (only for parallel).
- Implement conditional serialization:
  - `strategy == Parallel`: serialize `attempts` as `branches`, include `aggregator` and `aggregate_output_path`.
  - `strategy != Parallel`: retain existing behavior (`attempts` field name, skip new fields).
- Add `VerifyOutcome::skipped()` default for parallel `verify` field.

**AC**:
- `cargo test strategy::single_model` still passes (no schema regressions).
- `cargo test strategy::escalating_retry` still passes.
- `StrategyOutput` serializes correctly for `Single`, `Escalating`, and `Parallel` variants.

---

### T-3: Implement `ParallelFanOut` strategy module

**File**: `src/strategy/parallel_fanout.rs` (new)
**Effort**: M (~45 min)
**Depends on**: T-1, T-2

- Define `TargetSpec` struct.
- Define `Aggregator` enum (stub labels: `Concat`, `AnyFail`, `Vote`, `LLMJudge`).
- Define `ParallelFanOut` struct with `targets`, `min_responses`, `prompt_template`, `aggregator`.
- Implement `Strategy` trait:
  - Render prompt once.
  - Build `FuturesUnordered` of backend queries.
  - Poll loop with success/settle counters.
  - Short-circuit when `successes >= min_responses`.
  - Floor violation when all settled but `successes < min_responses`.
  - Return `StrategyOutput` with `branches` (completion order).
- Handle edge cases:
  - Empty targets → `StrategyError::NoBackends`
  - `min_responses == 0` → treated as invalid at construction (panic or assert)
  - Backend not found → `StrategyError::BackendNotFound`
  - Prompt render failure → `StrategyError::PromptRender` (before any dispatch)

**AC**:
- `cargo test strategy::parallel_fanout` passes (tests added in T-5).
- No regressions in existing strategy tests.

---

### T-4: Wire module tree

**File**: `src/strategy/mod.rs`
**Effort**: S (~5 min)
**Depends on**: T-3

- Add `pub mod parallel_fanout;`
- Re-export `ParallelFanOut`, `TargetSpec`, `Aggregator`.

**AC**:
- `use loker::strategy::parallel_fanout::{ParallelFanOut, TargetSpec, Aggregator};` compiles from integration tests.

---

### T-5: Unit + integration tests

**Files**:
- `src/strategy/parallel_fanout.rs` (`#[cfg(test)]` module)
- `tests/strategy_parallel_fanout.rs` (new)
**Effort**: M (~60 min)
**Depends on**: T-3, T-4

Unit tests (`#[cfg(test)]` in `parallel_fanout.rs`):
1. `happy_path_all_succeed` — 3 targets, min=2, all succeed → `Ok` with 3 branches
2. `one_fails_floor_still_met` — 2 targets, min=1, one fails → `Ok` with 2 branches
3. `floor_violation` — 3 targets, min=3, 2 fail → `FloorViolation` with shaped output
4. `fast_targets_cancel_slow` — 3 targets, min=2, one slow (5s) → only 2 calls, returns before slow completes
5. `outcomes_in_completion_order` — verify branches not in submission order
6. `prompt_render_failure_no_dispatch` — zero backend calls
7. `empty_targets_yields_no_backends`

Integration tests (`tests/strategy_parallel_fanout.rs`):
- Reuse `MockBackend` pattern from `tests/strategy_single_model.rs`
- Schema validation against `docs/schemas/phase_result_parallel.schema.json`
- Schema validation of `FloorViolation` error payload

**AC**:
- `cargo test strategy::parallel_fanout` covers all 7 unit tests.
- `cargo test --test strategy_parallel_fanout` covers integration tests.
- All schema validations pass.

---

### T-6: Manual verification

**Effort**: S (~10 min)
**Depends on**: T-5

- Run `make check` (fmt + clippy + test).
- Run `cargo test strategy::` (all strategy tests together).
- Confirm no regressions.

**AC**:
- `make check` exits 0.
- No new clippy warnings.
- All strategy tests green.

## Task Rollup

| # | Task | Effort | File(s) | Dependencies |
|---|---|---|---|---|
| T-1 | Extend `StrategyKind` + `StrategyError` | S | `src/strategy/mod.rs` | — |
| T-2 | Conditional serialization for parallel schema | M | `src/strategy/mod.rs` | T-1 |
| T-3 | Implement `ParallelFanOut` | M | `src/strategy/parallel_fanout.rs` | T-1, T-2 |
| T-4 | Wire module tree | S | `src/strategy/mod.rs` | T-3 |
| T-5 | Unit + integration tests | M | `src/strategy/parallel_fanout.rs`, `tests/strategy_parallel_fanout.rs` | T-3, T-4 |
| T-6 | Manual verification | S | — | T-5 |

**Total estimated effort**: ~2.5 hours of focused implementation + testing.
