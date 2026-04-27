# PRD: CLO-259 — Strategy::ParallelFanOut with min_responses floor

| Field | Value |
|-------|-------|
| Author | pi (discovery phase) |
| Status | Draft |
| Created | 2026-04-27 |
| Task | CLO-259 |
| Depends on | CLO-257 (Strategy::SingleModel — done), CLO-258 (Strategy::EscalatingRetry — done) |

## 1. Goal

Add `Strategy::ParallelFanOut` that dispatches a single rendered prompt to N backend targets concurrently, collects per-target `Result<Output, BackendError>`, and yields a schema-shaped phase result. The strategy short-circuits once `min_responses` successful responses arrive; remaining in-flight requests are dropped. If fewer than `min_responses` complete before all targets settle, the strategy returns a structured floor-violation error carrying a fully-shaped `StrategyOutput` so callers can persist artefact metadata even on failure.

## 2. Scope

### In scope
- `ParallelFanOut` struct implementing the existing `Strategy` trait
- `StrategyKind::Parallel` discriminator variant
- Concurrent dispatch via `futures::stream::FuturesUnordered` over `Backend::query`
- `min_responses` floor with short-circuit cancellation
- `TargetSpec` type (backend name + optional model override)
- `Aggregator` enum stub (`Concat | AnyFail | Vote | LLMJudge`) wired into `ParallelFanOut` for schema compliance
- Per-branch `Attempt` records in `StrategyOutput` (serialized as `branches` for the parallel schema)
- Wiremock-style unit tests covering all four acceptance test cases
- Schema validation against `docs/schemas/phase_result_parallel.schema.json`

### Out of scope (deferred to M3 / follow-up)
- Full `Aggregator` trait implementations (actual concat logic, LLM judge prompt construction, vote counting)
- Cross-family enforcement (FR-13)
- Step-level resumability for parallel strategy (FR-21)
- `family_of(backend_id)` runtime lookup (open question from PRD §8)

## 3. Acceptance Criteria

1. `cargo test strategy::parallel_fanout` covers:
   - Happy path: all targets succeed, aggregator label recorded
   - One target fails, `min_responses` still satisfied, floor success
   - Too many targets fail, returns floor-violation error with shaped output
   - One target slow, fast targets satisfy floor, slow target dropped / its result not waited for
   - Outcomes arrive in `StrategyOutput.attempts` in completion order, not submission order
2. No additional public API beyond the new `Strategy` impl + the existing `Strategy::execute` entrypoint.
3. `make check` clean.

## 4. Design

### 4.1 Type additions

```rust
// src/strategy/parallel_fanout.rs
/// Backend target specification for a single branch of the fan-out.
#[derive(Debug, Clone)]
pub struct TargetSpec {
    pub backend: String,
    pub model: Option<String>,
}

/// Aggregator label for the parallel schema. Actual aggregation logic
/// is a follow-up (M3); the label ensures schema validation passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregator {
    Concat,
    AnyFail,
    Vote,
    LLMJudge,
}

/// Parallel fan-out strategy.
pub struct ParallelFanOut {
    pub targets: Vec<TargetSpec>,
    pub min_responses: usize,
    pub prompt_template: String,
    pub aggregator: Aggregator,
}
```

### 4.2 Implementation notes

- `execute` renders the prompt once, then spawns one `Backend::query` future per target.
- Uses `FuturesUnordered` to poll futures concurrently and receive results in arrival order.
- Maintains two counters: `successes` and `settles`.
- When `successes >= min_responses`: break the loop; remaining futures are dropped on return.
- When `settles == targets.len()` and `successes < min_responses`: construct the full `StrategyOutput` and return `StrategyError::FloorViolation { output: Box<StrategyOutput> }`.
- Each completed future produces an `Attempt`. For successful queries: `finish_reasons: [Stop]`. For errors: `finish_reasons: [Error]` and the `BackendError` is **not** propagated as a top-level `StrategyError`; it is recorded in the attempt so the aggregator can inspect it.
- `StrategyKind` gains `Parallel` variant. `StrategyOutput` serialization: when `strategy == StrategyKind::Parallel`, emit `branches` (renamed from `attempts`); otherwise emit `attempts`. This is handled with a custom `Serialize` impl or a serde helper.

### 4.3 Error model

New `StrategyError` variant:
```rust
#[error("parallel floor violated: {successes}/{min_responses} targets succeeded")]
FloorViolation {
    output: Box<StrategyOutput>,
},
```

This mirrors `StrategyError::Exhausted` used by `EscalatingRetry`, keeping the pattern consistent: failures that carry shaped output use a boxed output field.

### 4.4 Test contract

Mock backends follow the pattern from `tests/strategy_single_model.rs`:
- `MockBackend::ok(name, text)` — returns after configurable delay
- `MockBackend::fail(name, err_fn)` — returns error after delay
- `MockBackend::slow(name, text, delay_ms)` — delays to test cancellation

Tests run via `tokio::runtime::Runtime::new().unwrap().block_on` and validate:
- Exact number of backend calls (fast path + slow target not awaited if floor met)
- `StrategyOutput.attempts.len()`
- Schema validation against `phase_result_parallel.schema.json`
- `StrategyError::FloorViolation` pattern match

## 5. Risks

| Risk | Likigation |
|------|-----------|
| `Backend::query` takes `&self` with no cancellation handle; dropped futures may continue their HTTP request in the background. | Document as known v0 limitation; cancellation is cooperative via future drop. |
| `StrategyOutput` custom serialization adds complexity. | Keep serialization logic local to `StrategyOutput`; existing single/escalating schemas remain unaffected via `skip_serializing_if`. |
| `min_responses == 0` edge case. | Assert `min_responses >= 1` at construction time or in `execute`; treat `0` as `NoBackends` error. |

## 6. References

- PRD FR-6: `Strategy::ParallelFanOut` runs N backends concurrently with `min_responses` floor
- `docs/schemas/phase_result_parallel.schema.json`
- `src/strategy/mod.rs` trait definition
- `tests/strategy_single_model.rs` mock pattern
- `tests/strategy_escalating_retry.rs` multi-attempt output pattern
