# Plan: CLO-266 Implement Aggregator::Concat with per-source headings

## Context

- Design: `docs/designs/clo-266-aggregator-concat.md`
- Discovery: `docs/discovery/clo-266.md`
- PRD: `docs/prds/clo-266-aggregator-concat.md`
- Linear: https://linear.app/cloud-ai/issue/CLO-266/implement-aggregatorconcat-with-per-source-headings

## Sub-tasks

### ST1 Add concat aggregator module and public API

**Files:**
- `src/aggregator.rs`
- `src/lib.rs`

**Work:**
- Add `pub mod aggregator;` to the crate root.
- Define `EMPTY_CONCAT_SENTINEL`.
- Define behavioral `aggregator::Aggregator::Concat { heading_template: String }`.
- Define `AggregateInput`, `BranchOutcome`, `BranchSuccess`, `BranchFailure`, `AggregatedArtifact`, and `AggregatorError`.
- Add `Aggregator::concat(...)`, `Aggregator::kind()`, and `Aggregator::aggregate(...)` signatures.
- Implement concat rendering per design: supported placeholders `{backend_id}`, `{family}`, `{index}`; unknown placeholders preserved; success sections in input order; failures in a `## Errors` footer; exactly one trailing newline.

**Acceptance:**
- `cargo test aggregator:: --lib` passes

**Estimate:** M

### ST2 Add deterministic concat unit tests

**Files:**
- `src/aggregator.rs`

**Work:**
- Add unit tests for success section rendering in input order.
- Add unit test that unknown placeholders remain literal.
- Add unit test for empty input returning `EMPTY_CONCAT_SENTINEL` and zero counts.
- Add unit test for mixed success/failure counts.
- Add unit test that behavioral concat maps to `loker::strategy::Aggregator::Concat`.

**Acceptance:**
- `cargo test aggregator::tests --lib` passes

**Estimate:** S

### ST3 Add insta snapshot coverage for mixed success/failure

**Files:**
- `Cargo.toml`
- `src/aggregator.rs`
- `src/snapshots/` or generated insta snapshot location

**Work:**
- Add `insta` as a dev-dependency.
- Add snapshot test for the 3-target mixed fixture:
  - success `claude` / `anthropic` / index `1`
  - failure `codex` / `openai` / index `2` / reason `network: timeout`
  - success `gemini` / `google` / index `3`
- Snapshot must capture heading placeholder rendering and the full `## Errors` footer shape.

**Acceptance:**
- `cargo test aggregator::tests::concat_mixed_success_failure_snapshot --lib` passes

**Estimate:** S

### ST4 Verify D2 parallel phase-result schema compatibility

**Files:**
- `tests/strategy_parallel_fanout.rs` or existing schema validation test file
- `docs/schemas/phase_result_parallel.schema.json` (read-only unless a regression is found)

**Work:**
- Keep the existing `strategy::Aggregator::Concat` serialization path stable.
- Add or adjust a focused schema test showing a parallel `StrategyOutput` with `aggregator: concat`, non-empty `aggregate_output_path`, branches, and verify object validates against `docs/schemas/phase_result_parallel.schema.json`.
- Ensure the new behavioral aggregator module does not change existing JSON shape.

**Acceptance:**
- `cargo test --test strategy_parallel_fanout phase_result_validates_against_parallel_schema` passes

**Estimate:** S

### ST5 Documentation and full gate

**Files:**
- `src/aggregator.rs`
- `docs/designs/clo-266-aggregator-concat.md` if implementation discovers a small doc correction

**Work:**
- Add rustdoc documenting supported concat heading placeholders and the empty-input sentinel.
- Run formatting, clippy, and full tests.
- Fix any import ambiguity with explicit aliases where behavioral and schema aggregators are both in scope.

**Acceptance:**
- `make check` passes

**Estimate:** S

## Pre-merge gate

- `make check` (fmt + clippy + test)

## Risks

- **Aggregator naming ambiguity:** `loker::strategy::Aggregator` and `loker::aggregator::Aggregator` may both be in scope. Use explicit imports or aliases in tests and implementation.
- **Snapshot churn:** Markdown whitespace rules must be implemented exactly: two newlines between sections and one final trailing newline.
- **Schema regression:** Do not alter `StrategyOutput` serialization while adding behavior; D2 compatibility is proven by the parallel schema test.
- **Scope creep:** Do not add `{model}` or phase-runner disk writing in this task; both are outside CLO-266 scope.
