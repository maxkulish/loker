# PRD: CLO-268 — Aggregator::LLMJudge with cross-family enforcement

| Field | Value |
|-------|-------|
| Author | pi (discovery phase) |
| Status | Draft |
| Created | 2026-04-28 |
| Task | CLO-268 |
| Depends on | CLO-265 (family lookup — merged), CLO-259 (ParallelFanOut — done) |
| PRD Source | Linear issue CLO-268, PRD FR-10 |

## 1. Goal

Ship a judge aggregator that calls a separate-family LLM to evaluate N candidate responses from a `ParallelFanOut` phase. The judge produces a structured ballot (`chosen_index`, `reason`) that selects one candidate as canonical. The defining invariant is cross-family enforcement: the judge backend must belong to a different family than every candidate, preventing family-level collusion.

## 2. Scope

### In scope
- `Aggregator::LLMJudge` variant carrying `judge_backend: String`, `prompt_template: String`, `require_judge_different_family: bool` (default `true`).
- Judge prompt construction: candidates labelled by `backend_id` and `family`, injected into the user-provided template.
- Ballot JSON schema: `{ "chosen_index": usize, "reason": string }`. Missing or malformed fields raise `PhaseError::AggregatorContract`.
- Cross-family enforcement via `enforce_cross_family` from CLO-265, extended to check judge-vs-candidate overlap.
- Opt-out path: when `require_judge_different_family = false`, skip enforcement and log a warning.
- Wiremock-backed integration tests for the judge backend call (success, malformed JSON, transport error).
- Snapshot test on the produced phase result file.
- Prompt template unit tests that need no live backend.

### Out of scope (deferred)
- `Vote` aggregator implementation (T-019).
- Retry logic for judge backend failure (v0: single attempt, fail fast).
- Structured `chosen_index` validation against candidate count overflow (handled by normalising with saturating bounds).

## 3. Acceptance Criteria

1. `cargo test` covers family-overlap refusal AND the opt-out path.
2. Snapshot test on the produced phase result file (`chosen_index`, rationale, candidate metadata).
3. Judge prompt template is unit-testable in isolation (no live backend needed for template tests).
4. Malformed judge response raises a structured `PhaseError::AggregatorContract`, not a panic.
5. Judge backend transport error surfaces as `PhaseError::JudgeUnavailable`.
6. `make check` clean (fmt, clippy, test).

## 4. Design

### 4.1 Config model

Extend the behavioural `Aggregator` enum in `src/aggregator/concat.rs` (or migrate both to `src/aggregator/mod.rs` if the refactor is natural):

```rust
pub enum Aggregator {
    Concat { heading_template: String },
    LLMJudge {
        judge_backend: String,
        prompt_template: String,
        require_judge_different_family: bool,
    },
}
```

Default for `require_judge_different_family`: `true`.

### 4.2 Prompt construction

The `prompt_template` is rendered by the engine (tera/minijinja). The template context receives:
- `candidates: Vec<Candidate>` where each candidate has `index`, `backend_id`, `family`, `output`.
- `phase_name: String` for context.

Example default template (overridable by user):
```markdown
You are an impartial judge. Evaluate the following {{ candidates.len() }} candidate responses and pick the best one.

{% for c in candidates %}
## Candidate {{ c.index }} ({{ c.backend_id }} — {{ c.family }})

{{ c.output }}
{% endfor %}

Return ONLY JSON matching this schema:
{ "chosen_index": <0-based index>, "reason": "<1-sentence rationale>" }
```

### 4.3 Cross-family enforcement

Before calling the judge backend, resolve `family_of(judge_backend)` and compare against each candidate's family. If any overlap:
- If `require_judge_different_family == true` → return `Err(PhaseError::FamilyOverlap { ... })`.
- If `require_judge_different_family == false` → log a warning and continue.

This reuses `family_of` and `Family::eq` from `src/family.rs`.

### 4.4 Ballot parsing

After calling `backend.query(...)` on the judge backend:
1. Strip markdown fences via the existing `strip_markdown_fences` helper in `src/aggregator/mod.rs`.
2. Parse as JSON via `serde_json::from_str`.
3. Extract `chosen_index` (usize) and `reason` (string).
4. Clamp `chosen_index` to `0..candidates.len()`.
5. Produce `AggregatedArtifact` with `text = candidates[chosen_index].output` plus rationale metadata.

Malformed JSON, missing fields, or wrong types → `PhaseError::AggregatorContract { message }`.

### 4.5 Error model

Add to `PhaseError` in `src/family.rs`:
```rust
#[error("judge unavailable: {detail}")]
JudgeUnavailable { detail: String },
```

Judge backend transport errors (`BackendError::Network`, `BackendError::Unavailable`, `BackendError::Timeout`) are mapped to `PhaseError::JudgeUnavailable` at the aggregation layer.

### 4.6 Aggregation flow in `ParallelFanOut`

Currently `ParallelFanOut` uses `is_any_fail` branching. Extend to handle `LLMJudge`:

```rust
match self.aggregator {
    Aggregator::Concat { ... } => { /* existing concat logic */ }
    Aggregator::AnyFail => { /* existing any_fail logic */ }
    Aggregator::LLMJudge { ... } => {
        // 1. collect all attempts (already done by FuturesUnordered loop)
        // 2. enforce cross-family
        // 3. render ballot prompt
        // 4. call judge_backend.query(...)
        // 5. parse ballot
        // 6. produce AggregatedArtifact
        // 7. emit StrategyOutput with aggregate_output_path
    }
}
```

Because `LLMJudge` needs all candidates before it can judge, it does NOT short-circuit on `min_responses`; it always waits for the full set to settle (or for `FuturesUnordered` to complete naturally).

### 4.7 Test contract

**Unit tests** (no wiremock, no runtime):
- Prompt template renders correct candidate labels with markdown fences stripped from candidate outputs.
- Ballot parser handles valid JSON, markdown-fenced JSON, missing `chosen_index`, missing `reason`, wrong `chosen_index` type, negative index, out-of-bounds index.
- Family-overlap check passes with diverse families; fails with overlap; allows overlap when opted out.

**Integration tests** (wiremock backend):
- Mock judge backend returns valid ballot → chosen candidate selected.
- Mock judge backend returns malformed JSON → `PhaseError::AggregatorContract`.
- Mock judge backend returns network error → `PhaseError::JudgeUnavailable`.

**Snapshot test:**
- Serialised `StrategyOutput` for a successful judge run matches `phase_result_parallel.schema.json`.

## 5. Risks

| Risk | Mitigation |
|------|------------|
| Adding `PhaseError::JudgeUnavailable` changes the enum for downstream callers. | `PhaseError` is already `#[non_exhaustive]`; additive variant is safe. |
| `LLMJudge` needs access to `backends: &[Arc<dyn Backend>]` inside aggregation, blurring the strategy/aggregator boundary. | Acceptable for v0; T-029 phase runner may introduce an `Aggregator` trait with backend references later. Document the boundary debt in code comments. |
| Prompt template injection of raw candidate outputs could be exploited for prompt injection. | v0 assumes workflow authors control candidate backends. A future sanitisation layer can escape candidate boundaries if needed. |

## 6. References

- PRD FR-10 (LLMJudge aggregator)
- Design doc §7 aggregators
- `family_of` / `enforce_cross_family` from CLO-265 (`src/family.rs`)
- Roadmap task T-017 in `docs/plans/001-implementation-roadmap.md`
