# Design: CLO-259 — Strategy::ParallelFanOut with min_responses floor

| Field | Value |
|---|---|
| Task | CLO-259 |
| Status | Draft |
| Depends on | CLO-257 (Strategy::SingleModel), CLO-258 (Strategy::EscalatingRetry) |
| Discovery | docs/discovery/clo-259.md |

## 1. Problem

Workflow authors need a concurrent multi-backend execution primitive that tolerates partial failure. Today loker has `SingleModel` (one backend, one prompt) and `EscalatingRetry` (sequential ladder, stop-on-verify-pass), but no named strategy for dispatching the same rendered prompt to N targets in parallel, short-circuiting once a configured `min_responses` floor is met, and cancelling remaining in-flight requests.

This blocks the `design-doc-tdd` reference workflow (§4.2, `review` phase fans out to three families in parallel per PRD FR-6). Without `ParallelFanOut`, the only way to achieve concurrency is external orchestration, which loses the schema-shaped phase result and trace output that the strategy layer guarantees.

### Goals
- Add `ParallelFanOut` as a third `Strategy` implementation alongside `SingleModel` and `EscalatingRetry`.
- Keep the `Strategy` trait signature unchanged so existing call sites do not break.
- Satisfy `phase_result_parallel.schema.json` with per-branch metadata (backend, model, usage, finish reasons, output path, family).
- Introduce a minimal `Aggregator` enum stub so the schema validates; defer actual aggregation logic to M3 / a follow-up issue.
- Surface a structured floor-violation error when fewer than `min_responses` targets succeed.

### Non-goals
- Full `Aggregator` trait implementations (`Concat`, `LLMJudge`, `AnyFail`, `Vote`). The issue explicitly scopes aggregation to a follow-up.
- Cross-family enforcement (`require_judge_different_family`). This is an M3 concern that depends on `family_of(backend_id)` resolution.
- Step-level resumability for the parallel strategy. Phase-level granularity is sufficient for v0.
- Streaming or cancellation tokens beyond `drop(FuturesUnordered)`. The `Backend` trait has no cancellation handle; in-flight HTTP requests may continue in the background. This is a v0 limitation.

## 2. Architecture

### 2.1 Module layout

```
src/strategy/
  mod.rs              — trait, StrategyKind, StrategyOutput, Attempt, error types
  single_model.rs     — existing; unchanged (compile-time object-safety check preserved)
  escalating_retry.rs — existing; unchanged
  parallel_fanout.rs  — NEW: ParallelFanOut struct + TargetSpec + Aggregator enum
```

### 2.2 Data flow

```
PhaseContext (prompt_template + TemplateEngine + TemplateContext)
  │
  ▼
ParallelFanOut::execute
  ──► render prompt once via TemplateEngine
  │
  ├──► spawn FuturesUnordered<Future< (usize, Result<QueryOutput, BackendError>) >>
  │     each future: find backend by TargetSpec.backend name → Backend::query(rendered, cwd, model_override)
  │
  ├──► poll loop:
  │     * on success: push Attempt, increment successes
  │     * on error: push Attempt with finish_reasons=[Error], increment settles
  │     * break when successes >= min_responses  → drop remaining futures
  │     * break when settles == targets.len()    → decide floor violation
  │
  ├──► if floor violated:
  │     construct StrategyOutput (branches = attempts) → StrategyError::FloorViolation { output }
  │
  └──► else:
       construct StrategyOutput (branches = attempts) → Ok(StrategyOutput)
```

### 2.3 Serialization

`StrategyOutput` uses a conditional `Serialize` impl (or a serde field rename helper):
- When `strategy == StrategyKind::Parallel`, the `attempts` field serializes as `branches`.
- `aggregator`, `aggregate_output_path`, and `verify` are only emitted for `Parallel`.
- Other strategies (`Single`, `Escalating`) skip these fields via `#[serde(skip_serializing_if)]`.

For the parallel schema, `aggregate_output_path` can be a fixed sentinel (`"aggregated.txt"`) in v0 since actual aggregation is deferred. The `verify` object uses `VerifyOutcome::skipped()` since no verify hook runs at the strategy layer in v0.

## 3. Public API

### 3.1 New types (`src/strategy/parallel_fanout.rs`)

```rust
use crate::strategy::{Attempt, PhaseContext, Prompt, Strategy, StrategyError, StrategyKind, StrategyOutput, SCHEMA_VERSION};
use crate::backend::{Backend, BackendError, QueryOutput};
use async_trait::async_trait;
use serde::Serialize;
use std::sync::Arc;

/// Target specification for one branch of the fan-out.
#[derive(Debug, Clone)]
pub struct TargetSpec {
    pub backend: String,
    pub model: Option<String>,
}

impl TargetSpec {
    pub fn new(backend: impl Into<String>) -> Self {
        Self {
            backend: backend.into(),
            model: None,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
}

/// Aggregator label for schema compliance. Actual logic deferred to M3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregator {
    Concat,
    AnyFail,
    Vote,
    LLMJudge,
}

/// Parallel fan-out strategy.
#[derive(Debug, Clone)]
pub struct ParallelFanOut {
    pub targets: Vec<TargetSpec>,
    pub min_responses: usize,
    pub prompt_template: String,
    pub aggregator: Aggregator,
}

impl ParallelFanOut {
    pub fn new(
        targets: Vec<TargetSpec>,
        min_responses: usize,
        prompt_template: impl Into<String>,
        aggregator: Aggregator,
    ) -> Self {
        Self {
            targets,
            min_responses,
            prompt_template: prompt_template.into(),
            aggregator,
        }
    }
}

#[async_trait]
impl Strategy for ParallelFanOut {
    async fn execute(
        &self,
        backends: &[Arc<dyn Backend>],
        prompt: &Prompt,
        ctx: &PhaseContext,
    ) -> Result<StrategyOutput, StrategyError>;
}
```

### 3.2 Changes to existing types

```rust
// src/strategy/mod.rs

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    Single,
    Escalating,
    Parallel,   // NEW
}

#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum StrategyError {
    // ... existing variants ...

    /// Floor violation: fewer than min_responses succeeded before all targets settled.
    #[error("parallel floor violated: {successes}/{min_responses} targets succeeded")]
    FloorViolation {
        successes: usize,
        min_responses: usize,
        output: Box<StrategyOutput>,
    },
}
```

`StrategyOutput::attempts` is renamed to `branches` at serialization time when `strategy == StrategyKind::Parallel`. `aggregate_output_path` is only emitted when `strategy == StrategyKind::Parallel`.

`Attempt::family` is required by `phase_result_parallel.schema.json`. In v0, it defaults to `"local"` for all backends (family resolution is deferred to M3). For `Single`/`Escalating` strategies, `family` is omitted via `#[serde(skip_serializing_if)]`.

### 3.3 Compile-time check

The `mod.rs` `future_variant_compiles` test module should be extended with a second stub that implements `Strategy` using `TargetSpec` and `Aggregator` to prove the trait remains object-safe and accommodates the new data types.

## 4. Implementation Plan

### 4.1 Step 1 — Extend shared types
- Add `StrategyKind::Parallel` variant.
- Add `StrategyError::FloorViolation`.
- Modify `StrategyOutput` serialization to support `branches`/`aggregator`/`aggregate_output_path` for parallel strategy only.
- Add per-attempt `family` field to `Attempt` (optional, `#[serde(skip_serializing_if)]` for non-parallel strategies); default to `"local"` or omit if not populated.

### 4.2 Step 2 — Implement `parallel_fanout.rs`
- `ParallelFanOut` struct with `TargetSpec` + `Aggregator`.
- `execute` dispatches via `FuturesUnordered`.
- Poll loop tracks successes and settles; short-circuits on floor met.
- Floor violation constructs shaped output and returns `StrategyError::FloorViolation`.
- Happy path returns shaped output.
- All branches receive the same rendered prompt and `PhaseContext.cwd`; each target's `model` override (from `TargetSpec`) takes precedence over the `Prompt` model override if present.

### 4.3 Step 3 — Wire into module tree
- Add `pub mod parallel_fanout;` in `src/strategy/mod.rs`.
- Re-export `ParallelFanOut`, `TargetSpec`, `Aggregator`.

### 4.4 Step 4 — Tests
Unit tests in `src/strategy/parallel_fanout.rs` under `#[cfg(test)]`:

| Test | What it proves |
|---|---|
| `happy_path_all_succeed` | All targets return; `StrategyOutput` has N branches; schema validates; aggregator label present |
| `one_fails_floor_still_met` | 2 targets, min_responses=1; one errors; still returns `Ok` with 2 branches |
| `floor_violation` | 3 targets, min_responses=3; 2 error; returns `FloorViolation` with shaped output and schema validates |
| `fast_targets_cancel_slow` | 3 targets, min_responses=2; two fast (1ms), one slow (5s); loop returns before slow completes; only 2 backend calls counted |
| `outcomes_in_completion_order` | Different backend delays; branches appear in completion order, not submission order |
| `prompt_render_failure_no_dispatch` | Invalid template; `StrategyError::PromptRender` immediately; zero backend calls |
| `empty_targets_yields_no_backends` | `StrategyError::NoBackends` |
| `schema_validates_parallel` | Full `StrategyOutput` JSON against `phase_result_parallel.schema.json` |
| `floor_violation_schema_validates` | Error-carrying output JSON against parallel schema |

### 4.5 Step 5 — Compile-time object-safety check
- Extend `future_variant_compiles` test in `mod.rs` with `ParallelFanOut` stub.

## 5. Test Plan

### 5.1 Unit tests (target: 8+ tests)
- Mock backends with `Arc<dyn Backend>` and `AtomicUsize` call counters.
- `tokio::runtime::Runtime::new().unwrap().block_on` for sync-style test block.
- `jsonschema` validation against `docs/schemas/phase_result_parallel.schema.json`.
- No live network. All backends mocked.

### 5.2 Integration tests (target: 1 file)
- `tests/strategy_parallel_fanout.rs` — mirrors `tests/strategy_single_model.rs` and `tests/strategy_escalating_retry.rs` in structure.
- Reuses the `MockBackend` pattern from those files.
- Schema fixtures: `tests/fixtures/schemas/phase_result_parallel/positive/` and `negative/` if needed for edge cases.

### 5.3 Manual verification
- `make check` clean (fmt + clippy + test).
- `cargo test strategy::parallel_fanout` passes.
- No regressions in `cargo test strategy::single_model` or `strategy::escalating_retry`.

## 6. Migration / Rollout

- `SingleModel` and `EscalatingRetry` are untouched; no breaking changes.
- The `Strategy` trait signature is unchanged.
- Existing `StrategyOutput` consumers that deserialize JSON will see a new `"parallel"` value in `loker.strategy`; if they match exhaustively on `StrategyKind`, they'll need to add `Parallel`. `StrategyKind` is `#[non_exhaustive]`, so this is expected.
- No database or persisted-state migration needed; this is a new feature.

## 7. Open Questions (resolved or deferred)

| # | Question | Resolution |
|---|---|---|
| 1 | How does `StrategyOutput` serialize `attempts` vs `branches`? | Custom `Serialize` impl or `serialize_with` helper that reads `strategy` field. For non-parallel, existing behavior unchanged. |
| 2 | Where does `family` per branch come from? | Defer to M3. In v0, `Attempt::family` is `Option<String>` defaulting to `None` (omitted in JSON) or a hardcoded lookup table if trivial. No `Backend::family()` method needed yet. |
| 3 | Cancellation of slow backends when floor met? | Drop `FuturesUnordered`; dropped futures may continue their HTTP request in the background. Acceptable v0 limitation; document in rustdoc. |
| 4 | Should `min_responses == 0` be allowed? | No. Treat as construction-time panic or assert `min_responses >= 1`; in `execute`, if `targets.is_empty()` return `StrategyError::NoBackends`. |
| 5 | Does the aggregator enum need a `name` method for trace/logging? | Not for v0. Schema only needs the label string; tracing can use `format!("{:?}", aggregator)`. |

## 8. References

- Discovery report: `docs/discovery/clo-259.md`
- PRD: `docs/prds/clo-259-parallel-fanout.md`
- Canonical design: `loker-design.md` §4.2 (Strategy), §4.3 (Aggregator), §11 Q3 (parallel partial failure)
- Issue: CLO-259
- Depends on: CLO-257, CLO-258
- Schema: `docs/schemas/phase_result_parallel.schema.json`
