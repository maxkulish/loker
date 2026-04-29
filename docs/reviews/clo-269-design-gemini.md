# Design Review: CLO-269

**Reviewer**: Gemini 3.1 Pro
**Reviewed**: 2026-04-29
**Pipeline**: lok design-review

---

## 1. Completeness Check
- **Problem & Goals**: Present and clear. Good scoping of v0 features versus deferred features (e.g., Enum schema).
- **Architecture & Data Flow**: Present. Accurately identifies that `vote.rs` should be pure and synchronous, contrasting well with `LLMJudge`.
- **Types & Public API**: Present. Clean enums and config structs. `BranchOutcome` reuse is appropriate.
- **Implementation Details**: Present. Properly identifies that arrival order dictates a new tracking approach in the `ParallelFanOut` loop.
- **Test Plan**: Present and thorough. Both unit and integration scenarios are covered.
- **Migration & Debt**: Present. Safe additive change.

## 2. Architecture Assessment
**Strengths**:
- Pushing the aggregation logic into a pure, synchronous `vote.rs` module makes it highly testable without async overhead or mock backends.
- Reusing the existing `BranchOutcome` types respects the established domain boundaries between strategies and aggregators.
- Clear separation of `TieBreak` policies gives workflow authors strong control over non-deterministic edge cases.

**Concerns**:
- **Determinism Bug**: In section 4.2, `buckets` is defined as a `HashMap`. Rust's `HashMap` uses a randomized hasher by default. This means `buckets.iter()` yields the tied candidates in a random order on every execution. In `resolve_tie`, `TieBreak::Random { seed }` shuffles this randomly-ordered list, entirely defeating the fixed seed determinism.
- **Short-circuiting Interaction**: `ParallelFanOut::execute` currently short-circuits via `break` when `successes >= self.min_responses`. The design doc does not specify altering this. If a Vote phase is stopped early, it might falsely declare a tie or miss a quorum when a tie-breaking vote is still in flight.

## 3. Alignment with Handoff & Roadmap
- The design perfectly matches the intent of the handoff document and PRD (specifically FR-12, Vote aggregator).
- By limiting normalisation to `trim().to_lowercase()`, it honors the strict "no semantic-similarity scoring in v0" constraint while delivering exactly what's needed for mechanical consensus.
- Additive changes strictly fit into the M1/M3 milestones without breaking existing functionality.

## 4. Security Review
- **HTML Comment Injection**: The `AggregatedArtifact` embeds untrusted backend output inside a markdown comment (`<!-- loker: Vote aggregator metadata ... -->`). If a backend generates the text `-->` inside its winning answer, it will escape the comment block and inject the metadata payload directly into the visible markdown document. 
- The use of `rand` with `StdRng` for `TieBreak::Random` is perfectly safe. Seed extraction from config is a standard reproducible-run pattern. No other boundaries are affected.

## 5. Implementation Concerns
- **Error Mapping**: The design introduces `VoteError::NoCandidates` and `VoteError::NoOpinion` but doesn't explicitly map them to `StrategyError` or `PhaseError`. These should map cleanly so the orchestrator knows whether to retry or fail the run.
- **`ClosestToFamily` Ambiguity**: If two different answers tie for the most votes, and *both* answers have at least one candidate matching the target family, `ClosestToFamily` currently picks whichever the iterator yields first (which is random due to `HashMap`). It needs a defined fallback (e.g., `FirstResponder` among the matching subsets).

## 6. Concurrency & Async
- Excellent approach. By passing `&[BranchOutcome]` to `aggregate_vote`, the entire vote counting and tie-breaking process remains synchronous and CPU-bound, which for small strings is virtually instantaneous and will not block the tokio runtime. 
- Using `FuturesUnordered` implicitly collects in arrival order, making `FirstResponder` trivial to implement.

## 7. Blind Spots
- **`min_responses` short-circuiting**: Vote aggregators typically require all votes to be cast to determine a definitive majority. The design misses the `!is_vote` condition needed in `ParallelFanOut::execute` to prevent premature short-circuiting.
- **Iterating HashMaps**: The assumption that a `HashMap` can be used to feed a deterministic pseudo-random number generator (PRNG) or order-dependent tie-breaker.

## 8. Verdict
APPROVE_WITH_SUGGESTIONS

## 9. Actionable Feedback
1. **Fix non-determinism**: In `aggregate_vote`, use a `BTreeMap<String, Vec<usize>>` instead of `HashMap`, OR explicitly sort the `winners` slice alphabetically before applying any tie-breakers. This ensures `TieBreak::Random` and other policies are perfectly deterministic across runs.
2. **Disable short-circuiting for Vote**: Update the loop condition in `src/strategy/parallel_fanout.rs` to ensure all branches resolve before aggregating: `if !is_any_fail && !is_llm_judge && !is_vote && successes >= self.min_responses { break; }`. 
3. **Prevent HTML comment injection**: In `build_aggregated_text`, sanitize `result.winner` and `result.tie_break_rule` by replacing `-->` with `-- >` before formatting them into the metadata comment block.
4. **Stabilize `ClosestToFamily`**: If multiple tied answers contain candidates from the target family, `ClosestToFamily` should explicitly fall back to `FirstResponder` among those matching answers, rather than returning the first match from the iterator.
5. **Complete Error Mapping**: Specify in the PR how `VoteError::NoCandidates` and `VoteError::NoOpinion` are converted into the encompassing `PhaseError` (e.g. `PhaseError::AggregatorRejected`).
