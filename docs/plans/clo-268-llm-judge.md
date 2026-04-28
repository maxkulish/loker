# Plan: CLO-268 — Aggregator::LLMJudge with cross-family enforcement

## Context
- Design: docs/designs/clo-268-llm-judge.md
- Discovery: docs/discovery/clo-268.md
- PRD: docs/prds/clo-268-llm-judge.md
- Linear: https://linear.app/cloud-ai/issue/clo-268/implement-aggregatorllmjudge-with-cross-family-enforcement

## Sub-tasks

### ST1 Expand `Aggregator` enum and `PhaseError`
**Files:** `src/aggregator/concat.rs`, `src/family.rs`, `src/strategy/parallel_fanout.rs`
**Acceptance:** `cargo test` compiles and existing tests pass (no behavioural change yet).
**Estimate:** S

Add `LLMJudge` variant to the behavioural `Aggregator` enum:
```rust
LLMJudge {
    judge_backend: String,
    prompt_template: String,
    require_judge_different_family: bool,
}
```
Add `JudgeUnavailable` to `PhaseError` in `src/family.rs`.
Update `Aggregator::kind()` to map `LLMJudge` → `crate::strategy::Aggregator::LLMJudge`.
Update `Aggregator::aggregate()` signature to accept `backends: &[Arc<dyn Backend>]` and `ctx: &PhaseContext` (additive only; `Concat` ignores them).
Adjust `ParallelFanOut` call site to pass the extra args.

### ST2 Create `src/aggregator/llm_judge.rs` — family check + prompt builder
**Files:** `src/aggregator/llm_judge.rs`, `src/aggregator/mod.rs`
**Acceptance:** `cargo test llm_judge_family_ llm_judge_prompt_` passes.
**Estimate:** M

Implement:
- `Candidate` struct (serialisable for Tera).
- `check_cross_family(judge_backend, candidates, require_different) → Result<(), LLMJudgeError>` reusing `family_of`.
- `render_ballot_prompt(template, candidates, ctx) → String` via `ctx.template_engine`.
- Unit tests:
  - `llm_judge_family_overlap_blocks`
  - `llm_judge_family_overlap_opt_out_warns`
  - `llm_judge_family_diverse_ok`
  - `llm_judge_prompt_renders_candidates`

### ST3 Create ballot parser + `LLMJudgeError` contract errors
**Files:** `src/aggregator/llm_judge.rs`
**Acceptance:** `cargo test llm_judge_parse_` passes.
**Estimate:** S

Implement:
- `Ballot` struct (`chosen_index: usize`, `reason: String`).
- `parse_ballot(text) → Result<Ballot, LLMJudgeError>` with markdown-fence stripping.
- Unit tests:
  - `llm_judge_parse_valid_ballot`
  - `llm_judge_parse_markdown_fenced_ballot`
  - `llm_judge_parse_missing_chosen_index`
  - `llm_judge_parse_negative_chosen_index`
  - `llm_judge_parse_out_of_bounds_index`

### ST4 Wire `LLMJudge` into `ParallelFanOut::execute`
**Files:** `src/strategy/parallel_fanout.rs`
**Acceptance:** `cargo test` passes; `make check` green.
**Estimate:** M

After the `FuturesUnordered` loop, add a branch for `Aggregator::LLMJudge`:
1. Map successful `BranchOutcome::Success` attempts into `Vec<BranchSuccess>`.
2. Call `llm_judge::check_cross_family(...)`.
3. Render ballot prompt.
4. Resolve judge backend from `backends` slice; call `query(...)`.
5. Parse ballot; clamp index; build `AggregatedArtifact`.
6. Return `StrategyOutput` with `aggregator: Some(self.aggregator)`, `verify: passed("LLMJudge")`.

Map errors at the call site:
- `LLMJudgeError::FamilyOverlap` → `PhaseError::FamilyOverlap`
- `LLMJudgeError::Contract` → `PhaseError::AggregatorContract`
- `LLMJudgeError::JudgeCall` → `PhaseError::JudgeUnavailable`
- `LLMJudgeError::BackendNotFound` → `PhaseError::JudgeUnavailable` or `StrategyError::BackendNotFound`

### ST5 Integration tests: Wiremock-backed judge call
**Files:** `tests/aggregator_llm_judge.rs` (new)
**Acceptance:** `cargo test llm_judge_` in `tests/aggregator_llm_judge.rs` passes.
**Estimate:** M

Use `MockBackend` pattern from `tests/strategy_parallel_fanout.rs`:
- `llm_judge_success` — 2 candidates + 1 judge returning valid ballot → correct `chosen_index` and rationale.
- `llm_judge_malformed_json` — judge returns `"not json"` → `PhaseError::AggregatorContract`.
- `llm_judge_backend_error` — judge returns `BackendError::Network` → `PhaseError::JudgeUnavailable`.
- `llm_judge_family_overlap_refused` — judge == candidate family, require=true → `PhaseError::FamilyOverlap`.
- `llm_judge_family_overlap_opt_out` — same overlap, require=false → succeeds with warning.

### ST6 Snapshot test + schema validation
**Files:** `tests/aggregator_llm_judge.rs`, `src/aggregator/snapshots/`
**Acceptance:** `cargo test llm_judge_snapshot` passes.
**Estimate:** S

- Serialise `StrategyOutput` from `llm_judge_success` to JSON.
- Validate against `docs/schemas/phase_result_parallel.schema.json` using `jsonschema::Validator`.
- Use `insta` to snapshot the `AggregatedArtifact::text` content.

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Risks
- **AggregateInput signature change** touches `Concat` and `AnyFail` call sites. Mitigation: ST1 is standalone; compile check catches missed call sites.
- **MockBackend test determinism** — judge call order is sequential (not FuturesUnordered), so no arrival-order nondeterminism.
- **Index clamping edge case** — if judge returns index 0 and there are 0 candidates (impossible because min_responses ≥ 1 and floor violation would have fired earlier), `.saturating_sub(1)` yields 0, which is safe for an empty slice. In practice this cannot happen due to the floor check.
