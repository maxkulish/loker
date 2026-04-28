# PRD: CLO-267 — Aggregator::AnyFail (first failure wins)

| Field | Value |
|-------|-------|
| Author | pi (discovery phase) |
| Status | Draft |
| Created | 2026-04-28 |
| Task | CLO-267 |
| Depends on | CLO-259 (ParallelFanOut — done), CLO-258 (EscalatingRetry — done) |
| PRD Source | Linear issue CLO-267, PRD FR-11 |

## 1. Goal

Ship a pessimistic aggregator that evaluates JSON verdicts produced by parallel branches. Any target reporting `pass: false` short-circuits the phase as a failure. Used when the safe default is "refuse to proceed if any reviewer disagrees."

## 2. Scope

### In scope
- `Aggregator::AnyFail` variant operating on JSON verdicts containing `pass: bool`.
- Walker logic that evaluates results in arrival order (first `pass: false` wins).
- Success only when every target reports `pass: true`.
- Treat backend errors as failures (no silent demotion to "missing vote").
- Verdict-schema mismatch raises `PhaseError::AggregatorContract` rather than panic.
- JSON-fixture-driven unit tests: all-pass, mid-list-fail, first-target-fail, all-fail.
- Snapshot of produced phase result file against `docs/schemas/phase_result_parallel.schema.json`.

### Out of scope (deferred to T-017 / T-019)
- `LLMJudge` and `Vote` aggregator implementations.
- Cross-family enforcement (handled by T-015, consumed by T-017).
- Configurable tolerance flag for backend errors (v0 default is strict failure).

## 3. Acceptance Criteria

1. `cargo test` covers all four fixture scenarios with mock backends:
   - All targets pass: phase result has `aggregator: "any_fail"`, `verify.status: "pass"`.
   - First target fails: short-circuit returns structured failure with offending target's payload.
   - Mid-list target fails: earlier passes do not mask the failure.
   - All targets fail: first failure in arrival order is reported.
2. Verdict JSON missing `pass: bool` raises a structured contract error, not a panic.
3. `StrategyOutput` snapshot matches `phase_result_parallel.schema.json`.
4. `make check` clean.

## 4. Design

### 4.1 Baseline

`ParallelFanOut` currently records `aggregator: AnyFail` in `StrategyOutput` but performs **zero** aggregation logic. Every attempt's `verify` is `Skipped`. This task closes that gap for the `AnyFail` variant.

### 4.2 Verdict schema

v0 locks the schema at a single required field:
```json
{ "pass": true }
```
Any additional fields are allowed (forward compatible). Missing `"pass"` or a non-boolean value is a contract violation.

### 4.3 Walker (streaming evaluation)

`ParallelFanOut::execute` uses `FuturesUnordered`, which yields results in completion order. For `AnyFail`:
- As each result arrives, parse the backend response text as JSON.
- If `pass: false` → immediately return `Err(StrategyError::AnyFail { ... })` carrying the first offending `Attempt` and the raw payload.
- If `pass: true` → continue (still need to await remaining branches).
- If backend error (or JSON parse error, or missing `pass` field) → treat as failure (same as `pass: false`).
- Only when every branch reports `pass: true` → return `Ok(StrategyOutput)` with `verify.status` set to `pass`.

This naturally implements "order is arrival order" without buffering all results first.

### 4.4 Error model

Add to `StrategyError`:
```rust
#[error("aggregator any_fail: first failure from backend {backend}: {reason}")]
AnyFail {
    offender: Box<Attempt>,
    payload: String,
    output: Box<StrategyOutput>,
},
```

Add to `PhaseError`:
```rust
#[error("aggregator contract violation: {message}")]
AggregatorContract { message: String },
```

The `StrategyError::AnyFail` variant carries the full `StrategyOutput` so callers (future T-029 phase runner) can persist the schema-shaped JSON even on aggregation failure, mirroring `FloorViolation` and `Exhausted`.

### 4.5 Test contract

Mock backends return JSON verdict text (e.g. `{"pass": true}`). Tests assert:
- Exact `AnyFail` pattern match with correct `offender.backend`.
- Snapshot of `StrategyOutput` serialised JSON against schema.

## 5. Risks

| Risk | Mitigation |
|------|------------|
| Adding `AnyFail` variant to `StrategyError` changes the enum for all strategy callers. | Variant is additive; `#[non_exhaustive]` already on `StrategyError`. |
| `FuturesUnordered` short-circuit on AnyFail drops remaining futures, but they may have already completed internally. | Acceptable v0 limitation (same as Concat floor short-circuit). |
| LLMJudge later needs a different interface (backend calls during aggregation). | Extract aggregator trait in T-017; AnyFail helper moves cleanly. |

## 6. References

- Linear issue CLO-267
- PRD FR-11: `Aggregator::AnyFail`
- `docs/schemas/phase_result_parallel.schema.json`
- `src/strategy/parallel_fanout.rs`
- `src/strategy/mod.rs`
