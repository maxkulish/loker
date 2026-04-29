# Review Synthesis: CLO-269

**Synthesized**: 2026-04-29
**Pipeline**: lok design-review
**Reviewers**: Gemini 3.1 Pro, Codex/Ollama (glm-5.1:cloud), Claude (fallback if needed)

---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini | OK | Returned full structured review with verdict APPROVE_WITH_SUGGESTIONS |
| Ollama | OK | Returned full structured review with verdict APPROVE_WITH_SUGGESTIONS |
| Claude Fallback | SKIPPED | External reviewers succeeded |

## Agreement (High Confidence)
| # | Finding | Severity |
|---|---------|----------|
| 1 | `ParallelFanOut::execute` short-circuits via `min_responses` and lacks an `is_vote` guard analogous to `is_llm_judge`; Vote must wait for all branches before aggregating, otherwise quorum/tie results are incorrect | P0 / Critical |
| 2 | Determinism / ordering bug in tie-break paths: `HashMap` iteration order plus `FuturesUnordered` arrival order make `TieBreak::Random { seed }`, `ClosestToFamily`, and `FirstResponder` non-deterministic without an explicit canonical ordering (sorted keys, documented arrival semantics) | P0 / High |
| 3 | `VoteError` -> `PhaseError` / `StrategyError` mapping is undefined: `NoCandidates`, `NoOpinion`, `QuorumLost` need an explicit conversion analogous to `LLMJudgeError`, so the orchestrator knows whether to retry or fail | P1 / High |
| 4 | `VoteError::NoOpinion` is defined but never returned in the §4.2 pseudocode - either implement the trigger condition or remove it | P2 / Medium |
| 5 | `ClosestToFamily` lacks a defined fallback when multiple tied answers contain candidates from the target family - needs explicit secondary rule (e.g., `FirstResponder` over the matching subset) | P1 / Medium |

## Disagreement (Needs Human Decision)
| # | Topic | Position A (Reviewer) | Position B (Reviewer) |
|---|-------|----------------------|----------------------|
| 1 | HTML comment injection in `AggregatedArtifact` metadata | Gemini: real concern - sanitize backend `winner`/`tie_break_rule` against `-->` to prevent comment escape | Ollama: notes the metadata comment is freeform/unschemaed but treats it as "probably fine for v0" with no security action |
| 2 | Cross-family enforcement for Vote (FR-13) | Ollama: must address - either add `require_cross_family: bool` to `VoteConfig` or document the exemption | Gemini: did not raise this concern at all |
| 3 | `BranchOutcome::Abstain` variant | Ollama: pseudocode references a variant that doesn't exist - must add (with `#[non_exhaustive]`) or map onto `Failure` | Gemini: did not flag; reads the design as compatible with current `BranchOutcome` |

## Novel Insights (Single Reviewer)
| # | Finding | Source | Severity |
|---|---------|--------|----------|
| 1 | `VoteConfig` needs `Serialize`/`Deserialize` derives for `lok.toml` parsing (T-033) | Ollama | P1 |
| 2 | `BallotSchema` should be `#[non_exhaustive]` so future `Enum` variant doesn't break semver | Ollama | P1 |
| 3 | `compute_vote` signature uses `(&str, String, usize)` tuples while §4.2 builds `VoteCandidate` structs - pick one canonical type | Ollama | P1 |
| 4 | `VoteResult.winner` stores normalised (trim/lowercase) text; downstream consumers lose the original casing - either store both or document the normalisation | Ollama | P2 |
| 5 | `phase_result_parallel.schema.json` aggregator enum must add `"vote"` - cross-cutting change not flagged in design | Ollama | P1 |
| 6 | `VoteResult.vote_counts` sort order (descending by count) should be documented for snapshot determinism | Ollama | P2 |
| 7 | `rand` dependency footprint - import only `StdRng` + `SliceRandom` (or `rand_core`) instead of full `rand` facade | Ollama | P2 |
| 8 | Interaction between `min_responses` floor and `abstain_threshold` is undefined - clarify they are independent gates | Ollama | P2 |
| 9 | `VoteResult` Serialize requirement depends on whether it lands in `trace.jsonl`/`summary.json` vs only the HTML comment - clarify | Ollama | P2 |
| 10 | No explicit Acceptance Criteria section restating FR-12 ACs, and no rollback plan | Ollama | P3 |
| 11 | Naming: `concat.rs` housing four aggregator variants is a code smell; consider `aggregator.rs` | Ollama | P3 |
| 12 | HTML comment metadata injection via raw `-->` in winner text | Gemini | P1 (security) |

## Consolidated Verdict
**APPROVE_WITH_SUGGESTIONS** - both reviewers independently arrived at the same verdict; no NEEDS_REVISION.

## Priority Actions
1. **P0 - Disable `min_responses` short-circuit for Vote.** In `src/strategy/parallel_fanout.rs`, add `is_vote` guard to the early-break condition: `if !is_any_fail && !is_llm_judge && !is_vote && successes >= self.min_responses { break; }`. (Both reviewers, agreement.)
2. **P0 - Make tie-break deterministic.** Replace `HashMap` with `BTreeMap<String, Vec<usize>>` in `aggregate_vote`, or sort tied keys before applying `TieBreak::Random { seed }`, `ClosestToFamily`, and `FirstResponder`. Explicitly document that `FirstResponder` uses `FuturesUnordered` completion order, not dispatch order. (Both reviewers, agreement.)
3. **P1 - Resolve `BranchOutcome::Abstain` ambiguity.** Either add the variant (relying on `#[non_exhaustive]`) or map abstentions onto `BranchOutcome::Failure` with a distinguishing field; update §4.2 pseudocode accordingly. (Ollama.)
4. **P1 - Define `VoteError` -> `PhaseError`/`StrategyError` mapping.** Document conversions for `NoCandidates`, `NoOpinion`, `QuorumLost` analogous to `LLMJudgeError`. (Both reviewers.)
5. **P1 - Address cross-family enforcement (FR-13).** Add `require_cross_family: bool` to `VoteConfig` or explicitly document the exemption with rationale. (Ollama.)
6. **P1 - Fix `ClosestToFamily` ambiguity.** Specify fallback (e.g., `FirstResponder` over the matching subset) when multiple tied answers contain candidates from the target family. (Gemini.)
7. **P1 - Sanitize HTML comment metadata.** Replace `-->` with `-- >` in `winner`/`tie_break_rule` before formatting into the `<!-- loker: ... -->` block. (Gemini.)
8. **P1 - Add `"vote"` to `docs/schemas/phase_result_parallel.schema.json` aggregator enum.** (Ollama.)
9. **P1 - Add `Serialize`/`Deserialize` to `VoteConfig` and `#[non_exhaustive]` to `BallotSchema`.** (Ollama.)
10. **P1 - Unify `compute_vote` signature** on the `VoteCandidate` struct (drop the tuple form). (Ollama.)
11. **P2 - Resolve `VoteError::NoOpinion`** - implement the trigger or remove it. (Both reviewers.)
12. **P2 - Document `winner` normalisation, `vote_counts` sort order, `min_responses`/`abstain_threshold` independence, and `VoteResult` Serialize requirement.** (Ollama.)
13. **P2 - Trim `rand` import surface** to `StdRng` + `SliceRandom`. (Ollama.)
14. **P3 - Add Acceptance Criteria section restating FR-12 ACs and a rollback plan.** Optional rename of `concat.rs` -> `aggregator.rs`. (Ollama.)

## Decision Recommendation
**PROCEED_WITH_FIXES.** Both reviewers approve the core architecture (pure synchronous `vote.rs`, `BranchOutcome` reuse, deterministic tie-break vocabulary). Before merging implementation, resolve at minimum the P0 items (short-circuit guard, deterministic ordering) and the P1 items where reviewers agree (error mapping, schema enum update, `Abstain` variant decision, cross-family stance). The disagreement items (HTML comment sanitization, cross-family enforcement, `Abstain` variant) need explicit human decisions but each has a low-effort safe default - apply Gemini's sanitization and Ollama's `Abstain`/cross-family clarifications unless there's a reason to deviate.
