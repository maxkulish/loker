# Design Document: CLO-267 — Aggregator::AnyFail (first failure wins)

| Field | Value |
|-------|-------|
| Task | CLO-267 |
| Date | 2026-04-28 |
| Dependent on | CLO-259 (`ParallelFanOut`), CLO-258 (`EscalatingRetry`) |
| PRD | FR-11 |

## 1. Problem

`Strategy::ParallelFanOut` dispatches to N backends concurrently, but currently performs zero aggregation logic. The `aggregator` field in `StrategyOutput` is purely a schema label. Workflow authors who need pessimistic consensus — "fail the phase if any reviewer disagrees" — have no mechanism to evaluate per-branch verdicts, short-circuit on disagreement, or surface a structured failure carrying the offending branch. See `docs/discovery/clo-267.md` for full context.

## 2. Goals

- Implement `Aggregator::AnyFail` evaluation inside `ParallelFanOut::execute`.
- Lock the JSON verdict schema at `{ "pass": bool }` with forward-compatible tolerance for extra keys.
- Walker evaluates results in **arrival order** and short-circuits immediately on first `pass: false`.
- Surface success **only** when every target reports `pass: true`.
- Treat backend errors and schema mismatches as failures, never silent "missing votes".
- Carry the full `StrategyOutput` on failure so a future phase runner (T-029) can persist artefact metadata even on aggregation failure.

## 3. Non-Goals

- Extract a reusable aggregator trait (blocked by T-017 `LLMJudge`, which needs backend access; the abstraction will be designed there).
- Configurable tolerance for backend errors (future flag; v0 default is strict failure).
- Cross-family enforcement (T-015 / T-017).
- Step-level resumability for dropped futures (T-028 / T-031).

## 4. Architecture

### 4.1 Data flow

```
ParallelFanOut::execute
  ├─ FuturesUnordered over Backend::query
  ├─ per future:
  │    ├─ Ok(query)  → parse text as JSON verdict
  │    │              ├─ pass: true   → continue awaiting
  │    │              ├─ pass: false  → return Err(AnyFail)
    │    │              └─ schema mismatch → return Err(AnyFail) with contract flag
    │    └─ Err(_)     → return Err(AnyFail) treating error as failure
    └─ All pass        → return Ok(StrategyOutput) with verify.status = Pass
```

### 4.2 Type additions

#### `src/aggregator/mod.rs` (new thin module)

Per handoff rule: "New primitives (Strategy, Aggregator, VerifyHook) land as new modules." AnyFail is the first aggregator implementation; it gets its own module even though T-017 will redesign the abstraction into a trait. The module is thin: one function + one error enum, re-exported from `parallel_fanout.rs`.

```rust
//! Aggregator primitives. Currently a single-function module for
//! `AnyFail`; T-017 (LLMJudge) will introduce a proper `Aggregator` trait.

use crate::strategy::{Attempt, TokenUsageReport};
use serde_json::Value;

#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum AnyFailReason {
    VerdictRejected { payload: String },
    VerdictContract { message: String },
    BackendError { source: String },
}

/// Parse a backend response text as a JSON verdict and check `pass`.
///
/// Strips markdown fences (` ```json ... ``` `) before parsing so
/// LLM backends that wrap JSON in code blocks work out of the box.
/// Accepts extra keys (forward compatible). Missing or malformed `pass`
/// is a contract violation, not a panic.
pub fn any_fail_evaluate(text: &str) -> Result<(), AnyFailReason> {
    let stripped = strip_markdown_fences(text.trim());
    let value: Value = serde_json::from_str(stripped)
        .map_err(|e| AnyFailReason::VerdictContract {
            message: format!("JSON parse error: {e}"),
        })?;

    match value.get("pass") {
        Some(Value::Bool(true)) => Ok(()),
        Some(Value::Bool(false)) => Err(AnyFailReason::VerdictRejected {
            payload: stripped.to_string(),
        }),
        Some(other) => Err(AnyFailReason::VerdictContract {
            message: format!("expected bool, got {}", other),
        }),
        None => Err(AnyFailReason::VerdictContract {
            message: "missing required field 'pass'".to_string(),
        }),
    }
}

/// Strip leading/trailing markdown fences if present.
fn strip_markdown_fences(text: &str) -> &str {
    let text = text.strip_prefix("```json").unwrap_or(text);
    let text = text.strip_prefix("```").unwrap_or(text);
    let text = text.strip_suffix("```").unwrap_or(text);
    text.trim()
}
```

#### `src/strategy/mod.rs`

Extend `StrategyError`:

```rust
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum StrategyError {
    // … existing variants …

    #[error("aggregator any_fail: first failure from backend {backend}: {reason}")]
    AnyFail {
        backend: String,
        reason: AnyFailReason,
        offender: Box<Attempt>,
        output: Box<StrategyOutput>,
    },
}
```

`AnyFailReason` is re-exported from `crate::aggregator` for test assertions.

#### `src/family.rs`

Extend `PhaseError` (required by AC: "schema mismatch raises structured `PhaseError::AggregatorContract`"):

```rust
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum PhaseError {
    #[error("family overlap: found {family} on {count} backends")]
    FamilyOverlap { family: Family, count: usize },

    #[error("aggregator contract violation: {message}")]
    AggregatorContract { message: String },
}
```

#### `src/strategy/parallel_fanout.rs`

Import and call:

```rust
use crate::aggregator::any_fail_evaluate;
// inside the FuturesUnordered loop:
//   any_fail_evaluate(&query.text)?;
//   if let Err(reason) = result { return Err(StrategyError::AnyFail { ... }); }
```

### 4.3 Modified `execute` loop (AnyFail path only)

Inside the `while let Some((idx, result)) = futures.next().await` loop, when `self.aggregator == Aggregator::AnyFail`:

1. On `Ok(query)`: call `crate::aggregator::any_fail_evaluate(&query.text)`.
   - If `Err(reason)`: construct full `StrategyOutput` from attempts collected so far + current attempt, return `Err(StrategyError::AnyFail { backend, reason, offender, output })`.
   - If `Ok(())`: push attempt and continue (still need to await remaining branches).
2. On `Err(backend_err)`: same early-return path with `AnyFailReason::BackendError`.
3. After loop completes with zero failures: construct `StrategyOutput` with `verify: VerifyOutcome::passed("Aggregator::AnyFail")` and return `Ok`.

Note: `min_responses` semantics change under AnyFail. The Concat short-circuit (`successes >= min_responses`) is **disabled** when `aggregator == AnyFail`. Every branch must be awaited because one of the not-yet-settled branches could be the first failure. If a branch returns a backend error, that also triggers the AnyFail early-return.

Priority rule: when both `AnyFail` and `FloorViolation` are possible, AnyFail wins via early-return on the first failure. `FloorViolation` can only surface if every branch succeeds but the count falls below `min_responses` — which is impossible under AnyFail because we await every branch and only succeed when all pass.

### 4.4 Output shape

On success, `StrategyOutput.verify.status` transitions from `"skipped"` to `"pass"`:

```json
{
  "schema_version": 1,
  "loker.strategy": "parallel",
  "loker.phase": "review",
  "loker.run_id": "…",
  "branches": [ … ],
  "aggregator": "any_fail",
  "aggregate_output_path": "review/aggregated.txt",
  "verify": {
    "status": "pass",
    "hook": "Aggregator::AnyFail"
  }
}
```

On failure, the same shape is carried inside `StrategyError::AnyFail.output`, but the function returns `Err` so the caller knows the phase failed.

## 5. Public API surface

No new public traits or structs beyond the enum variants above. `AnyFailReason` is `pub` so test assertions can pattern-match the error details, but it is `#[non_exhaustive]`.

`Aggregator::AnyFail` was already a public enum variant; after this change it actually does something.

## 6. Test plan

### 6.1 Unit tests (all mock-backend, no I/O)

| Test | Fixture | Assertion |
|---|---|---|
| `all_pass` | 3 `MockBackend`s return `{"pass":true}` | `Ok`, `verify.status == Pass`, `attempts.len() == 3` |
| `first_fails` | backends `[b0,b1,b2]` with deterministic delays `[1ms,10ms,10ms]`; b0 returns `{"pass":false}` | `Err(AnyFail)`, `backend == "b0"`, `reason == VerdictRejected` |
| `mid_list_fails` | backends `[b0,b1,b2]` with delays `[1ms,5ms,10ms]`; b1 returns `{"pass":false}` | `Err(AnyFail)`, `backend == "b1"`, deterministic arrival order |
| `all_fail` | all return `{"pass":false}` with staggered delays | `Err(AnyFail)`, offender is first arrival |
| `backend_error_treated_as_fail` | one `MockBackend::fail` | `Err(AnyFail)`, `reason == BackendError` |
| `missing_pass_field` | `{"status":"ok"}` | `Err(AnyFail)`, `reason == VerdictContract` |
| `wrong_pass_type` | `{"pass":"yes"}` | `Err(AnyFail)`, `reason == VerdictContract` |
| `empty_query_text` | backend returns `""` | `Err(AnyFail)`, `reason == VerdictContract` |
| `markdown_fenced_json` | `` ```json\n{"pass":true}\n``` `` | `Ok` (strip succeeds) |
| `markdown_fenced_fail` | `` ```json\n{"pass":false}\n``` `` | `Err(AnyFail)`, `reason == VerdictRejected` |
| `valid_json_extra_keys` | `{"pass":true, "note":"lgtm"}` | `Ok` (forward compat) |
| `non_deterministic_offender` | `[b0,b1]` with same delay, both fail | `Err(AnyFail)`, assert offender is *one of* `{"b0","b1"}` (not a specific one) |

**Test determinism note**: `MockBackend` will gain `delayed_ok(name, text, delay_ms)` and `delayed_fail(name, err_fn, delay_ms)` constructors so arrival order is deterministic by delay. Where determinism cannot be enforced (same delay), assertions relax to "offender is one of the failing backends."

### 6.2 Schema validation

After each test case, serialize the `StrategyOutput` (on success) or the `output` field from the error (on failure) and assert it passes `phase_result_parallel.schema.json`. Use the existing JSON-schema test helper from `tests/strategy_single_model.rs` or T-002's validator if available.

Add a new `docs/schemas/verdict.schema.json` to formalise the forward-compat contract:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://loker.dev/schemas/verdict.schema.json",
  "title": "AnyFail Verdict",
  "description": "JSON payload expected by Aggregator::AnyFail",
  "type": "object",
  "required": ["pass"],
  "properties": {
    "pass": { "type": "boolean" }
  },
  "additionalProperties": true
}
```

### 6.3 Regression

Run `cargo test strategy::parallel_fanout` and `make check`. No existing tests should break because AnyFail is a new code path; Concat behaviour is untouched.

## 7. Migration / Rollout

No user-facing migration. This is a net-new capability on an enum variant that previously did nothing. Workflow authors who previously used `aggregator = "any_fail"` in their TOML will now see real evaluation instead of a no-op.

## 8. Open Questions

| Question | Resolution |
|---|---|
| Should `AnyFail` disable `min_responses` floor? | **Resolved:** Disable short-circuit entirely for AnyFail. Every branch must be awaited because any unsettled branch could fail. If a caller wants early exit on first success, that is `Concat` semantics, not AnyFail. |
| Should we add `AnyFailReason` to `PhaseError` or `StrategyError`? | **Resolved:** `AnyFailReason` lives in `src/aggregator/mod.rs`; `StrategyError::AnyFail` carries it. `PhaseError::AggregatorContract` is added for future phase-runner mapping. |
| Empty branch set + AnyFail? | Already handled: `ParallelFanOut::new` asserts `min_responses > 0`; empty targets path returns `StrategyError::NoBackends` before aggregation begins. |
| Priority between `AnyFail` and `FloorViolation`? | **Resolved:** `AnyFail` > `FloorViolation`. AnyFail short-circuits on first failure; `FloorViolation` can only surface if the loop completes with fewer successes than `min_responses`, which is impossible under AnyFail because we await every branch and only return `Ok` when all pass. |
| Module placement for `any_fail_evaluate`? | **Resolved:** New `src/aggregator/mod.rs` per handoff rule. Thin module re-exported from `parallel_fanout.rs` until T-017 redesigns the trait. |
| Markdown-fence stripping? | **Resolved:** `strip_markdown_fences` helper added to `src/aggregator/mod.rs`. |
| Secret leakage in `VerdictRejected.payload`? | Accepted risk for v0. `StrategyError::AnyFail` output needs redaction at the T-029 trace boundary; flagged as T-029 dependency. |

## 9. References

- `docs/discovery/clo-267.md`
- `docs/prds/clo-267-anyfail.md`
- PRD FR-11
- `src/strategy/parallel_fanout.rs`
- `src/strategy/mod.rs`
- `docs/schemas/phase_result_parallel.schema.json`
