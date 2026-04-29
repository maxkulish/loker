# Design Review: CLO-269

**Reviewer**: Codex via Ollama (glm-5.1:cloud)
**Reviewed**: 2026-04-29
**Pipeline**: lok design-review

---

Now I have all the context needed. Let me compose the review.

---

## 1. Completeness Check

| Section | Present | Assessment |
|---------|---------|------------|
| Problem | Yes | Clear, traces to discovery report |
| Goals / Non-goals | Yes | Well-scoped; Enum deferred, weighted voting excluded |
| Architecture (modules, data flow, types, API) | Yes | Thorough; module layout, data flow, new types, public API all specified |
| Implementation details | Yes | Detailed pseudocode for all paths including tie-breakers |
| Test plan | Yes | Unit + integration + snapshot test matrix |
| Migration / rollout | Yes | Additive-only changes to existing enums |
| Discovery debt resolution | Yes | Three items resolved |
| Open questions | Yes | Three items, all with proposed resolutions |

**Missing**: No explicit Acceptance Criteria section (the PRD FR-12 has AC, but this design doc should restate/concretize them). No rollback plan section (what to revert if Vote causes a regression mid-milestone).

---

## 2. Architecture Assessment

**Strengths**:
- Vote is a **pure, synchronous** aggregator - no backend call, no `PhaseContext`, no `async`. This is the single best design decision in the doc. It means full unit-testability without mocking.
- Reuses the existing `BranchOutcome` / `BranchSuccess` / `BranchFailure` types from `concat.rs` rather than inventing parallel types.
- `PhaseError::QuorumLost` is additive on an already `#[non_exhaustive]` enum - no breakage risk.
- `TieBreak` variants are well-chosen: `ClosestToFamily` leverages the existing `family_of` infrastructure, `Random { seed }` is deterministic-for-reproducibility, `FirstResponder` maps to arrival order.
- The `abstain_threshold` semantics (strict greater-than) are clearly documented with examples.
- `VoteResult` carries full metadata (vote_counts, tie_broken, tie_break_rule) for traceability in the aggregated artefact.

**Concerns**:

1. **`BranchOutcome::Abstain` does not exist** (design §4.2 line 323). The current `BranchOutcome` enum has only `Success` and `Failure` variants. The pseudocode references `BranchOutcome::Abstain` as an arm, implying a new variant. Either add it (breaking: `#[non_exhaustive]` protects downstream matches) or map abstention cases onto `Failure` with a distinguisher. The design should state which path explicitly.

2. **Cross-family enforcement for Vote is absent** (PRD FR-13). The PRD states `LLMJudge` and `Vote` both enforce cross-family by default. For Vote, which has no judge, the enforcement would mean: "all voting backends must be from distinct families." The design doc is silent on this. Either argue that Vote is inherently counting diversity (so enforcement is less critical) or add a `require_cross_family: bool` to `VoteConfig` and enforce it analogous to `LLMJudge`.

3. **The `compute_vote` function signature is inconsistent** with the `VoteCandidate` struct. §3.4 defines `compute_vote(candidates: &[(&str, String, usize)], ...)` using an anonymous tuple, while §4.2 builds `VoteCandidate` structs. Pick one; `VoteCandidate` is clearer and should be the canonical type.

4. **`winner` in `VoteResult` stores *normalised* text**, not the original. Downstream phases consuming the aggregated artefact will see lowercase-trimmed text, not the original response. Consider storing both the normalised key and the original winning text, or clarifying that `winner` is intentionally the normalised form.

---

## 3. Alignment with Handoff & Roadmap

| Check | Result |
|-------|--------|
| Matches handoff WHY (cross-family aggregation) | Yes - Vote is an aggregator primitive |
| Matches active milestone M1 (TensorZero backend) | **No, but correctly** - T-019 (Vote) is M3 (Aggregator vocabulary), and the roadmap explicitly lists it in Phase 3. The design doc is aligned with the *correct* milestone, not the *active* milestone. CLAUDE.md says M1 is active; this design is for a Phase 3 task that depends on T-015 (already shipped). |
| Follows TDD-first convention (handoff §Intent) | Partially - the test plan is good, but there is no "failing test contract" written first as handoff mandates. The design lists test cases but doesn't define the contract before implementation. |
| `make check` compatibility | Yes - all proposed tests are unit/integration tests runnable under `cargo test` |
| PRD FR-12 alignment | Yes - ballot schema, tie-breakers, abstentions all specified; property test mention aligns with FR-12 AC |

**Contradiction**: The design says the `Vote` variant goes into `src/aggregator/concat.rs` (the file that houses the behavioral `Aggregator` enum). This is correct per the current code layout but the module name "concat.rs" is misleading once it houses four non-concat variants. Consider whether `Aggregator` should migrate to its own `aggregator.rs` or `mod.rs`-inline module, though this is a style concern, not a blocking issue.

---

## 4. Security Review

- **No shell execution**, no network calls, no secret handling in Vote. The module is pure computation on already-collected branch outputs. Low risk.
- **`TieBreak::Random { seed }`** receives seed from `lok.toml` config (not from user CLI input or environment variables). If the seed were ever user-controlled (e.g., from a prompt template), it would be a denial-of-reproducibility vector, not a security vulnerability per the handoff threat model. The design correctly notes the seed is logged in the artefact metadata.
- **`rand` crate**: The design notes `rand` 0.8 is already an indirect dependency via `uuid`. Adding it to direct dependencies is acceptable. Ensure it's added only as needed (`rand::rngs::StdRng`, `rand::seq::SliceRandom`, `rand::SeedableRng`) and not the full `rand` facade.
- **No input from attacker-controlled sources**: The ballot text comes from backend `stdout`, which is already trusted-at-source (backend responses are consumed verbatim by other aggregators).

**Verdict**: No security concerns for this module.

---

## 5. Implementation Concerns

1. **`VoteConfig` missing `Serialize`/`Deserialize`**. It must be parsed from `lok.toml`. Add `#[derive(Serialize, Deserialize)]` or implement custom deserialization. Current `Aggregator::Vote { config: VoteConfig }` will need it for the TOML workflow parser (T-033).

2. **`BallotSchema` is not `#[non_exhaustive]`**. The design says `Enum` is "reserved for v0+1" but the enum itself needs `#[non_exhaustive]` to permit adding `Enum` without a semver break.

3. **`VoteError::NoOpinion` is a niche case**. When every backend returns identical text after normalisation, `NoOpinion` fires. But the pseudocode in §4.2 doesn't handle it - it's defined but never returned from `aggregate_vote`. Either remove it or implement it (if all buckets have count 1 and only one bucket exists, it's technically a "unanimous" result, not "no opinion").

4. **Vote dispatcher in `parallel_fanout.rs` must not short-circuit**. The current code has `if !is_any_fail && !is_llm_judge && successes >= self.min_responses { break; }` (line 194). For Vote, this short-circuit must NOT kick in because Vote needs *all* branch outcomes (including failures as abstentions) to count correctly. The design doc doesn't call out this interaction explicitly. The `is_llm_judge` guard already prevents short-circuit for LLMJudge; a similar `is_vote` guard is needed, or the short-circuit condition must be generalised.

5. **Arrival order for `FirstResponder` is not guaranteed by `FuturesUnordered`**. The design assumes `attempts` preserves absolute arrival order, but `FuturesUnordered` yields in completion order, not dispatch order. The current `ParallelFanOut` code builds `attempts` by pushing in completion order. For `FirstResponder`, the design notes "arrival order" means *completion* order (which is what `FuturesUnordered` gives). This should be explicitly clarified: `FirstResponder` picks the bucket whose first *completed* branch arrived earliest, not the dispatch order.

6. **Snapshot test against schema** (§5, Integration tests): The design says "Validate against `docs/schemas/phase_result_parallel.schema.json`". That schema must already include an `aggregator: "vote"` enum value. Currently the schema defines `"aggregator": {"enum": ["concat", "any_fail", "llm_judge"]}` - it needs `"vote"` added. This is a cross-cutting concern that the design doc doesn't flag.

7. **Test file path**: The design references `tests/strategy_parallel_fanout.rs` for integration tests, which matches the existing file. Good.

---

## 6. Concurrency & Async

Vote is purely synchronous and runs after the `FuturesUnordered` loop completes. No tokio concerns. No blocking calls in async paths. The function is `fn aggregate_vote(...)` not `async fn`. This is correct and well-designed.

One interaction point: the vote dispatch in `parallel_fanout.rs` currently runs inside `async fn execute()`. The call to `aggregate_vote` will block the async task briefly (it iterates over branch outcomes and builds hash maps). For the expected N (typically 2-5 branches), this is negligible and not worth `spawn_blocking`. No action needed.

---

## 7. Blind Spots

1. **Cross-family enforcement for Vote** (reiterated from §2). FR-13 mandates it. The design doesn't address it. Either add enforcement or document why Vote exempts.

2. **Interaction with `min_responses` floor**. If `min_responses=2` and 2 of 3 backends succeed, Vote runs with 2 votes and 1 abstention. If `abstain_threshold=0`, this could trigger `QuorumLost` with a majority still present. The design should clarify: does `abstain_threshold` interact with `min_responses`, or are they independent gates? (They should be independent: `min_responses` gates whether the strategy returns results at all; `abstain_threshold` gates whether Vote refuses to count.)

3. **What happens when Vote is the aggregator and `min_responses` short-circuits?** If `min_responses=2` and 2 of 5 succeed, `ParallelFanOut` may short-circuit before late branches complete. Vote would then operate on incomplete data. The design must either (a) disable short-circuit for Vote (like LLMJudge), or (b) state that Vote operates on whatever branches have completed by floor time. The existing code already disables short-circuit for LLMJudge; Vote should get the same treatment.

4. **`loker: Vote aggregator metadata` HTML comment format** is not schema-validated. The design specifies a structured comment but it's freeform text inside the aggregated output. The `phase_result_parallel.schema.json` schema validates `StrategyOutput` (JSON), not the content of `aggregated.txt`. This is probably fine for v0 but should be noted.

5. **`VoteError` vs `PhaseError` duplication**. The design defines `VoteError::QuorumLost` (in `vote.rs`) AND adds `PhaseError::QuorumLost` (in `family.rs`). The design doc §3.5 shows `PhaseError::QuorumLost`, but the `aggregate_vote` function returns `VoteError`. The parallel_fanout code will need to map `VoteError::QuorumLost` to `StrategyError::Phase(PhaseError::QuorumLost)` (analogous to how `LLMJudgeError` maps). This mapping should be documented.

6. **`VoteResult.vote_counts` order**. The pseudocode sorts `vote_counts` descending at the end (`§4.2 line 381: result.vote_counts.sort_by(|a, b| b.1.cmp(&a.1))`). This destroys insertion order, making snapshot tests deterministic. Good - but document that `vote_counts` is sorted by count descending, not by normalised text.

7. **`rand` crate feature gating**. If `rand` is added, it should be behind a feature flag or at minimum only import the minimal subset. The `rand` crate is large; consider `rand_core` + `rand_rngs` for just `StdRng::seed_from_u64` and `SliceRandom`.

8. **No `impl Serialize` on `VoteResult`**. The design doesn't specify whether `VoteResult` is serialized into `trace.jsonl` or `summary.json`. If it is, it needs `Serialize`. If it's only in the HTML comment, `Serialize` is not needed. Clarify.

---

## 8. Verdict

**APPROVE_WITH_SUGGESTIONS**

The design is thorough, well-aligned with the codebase, and introduces a clean, testable pure function. The core architecture (synchronous, no backend call, `BranchOutcome` reuse, deterministic tie-breakers) is sound. The concerns above are fixable without redesigning the module.

---

## 9. Actionable Feedback

| Priority | Item | Action |
|----------|------|--------|
| P0 | **Short-circuit guard for Vote** | Add `is_vote` check alongside `is_llm_judge` in `parallel_fanout.rs` to prevent `min_responses` short-circuit. Vote must collect all branches. |
| P0 | **`BranchOutcome::Abstain` either add or remove** | The pseudocode references `BranchOutcome::Abstain` (§4.2 line 323). Either add it to the enum (with `#[non_exhaustive]` protection) or map abstention to `Failure` with a distinguisher. |
| P0 | **Cross-family enforcement for Vote** | Address FR-13. Add `require_cross_family: bool` to `VoteConfig` or document in non-goals why Vote exempts. |
| P1 | **`VoteConfig` needs `Serialize`/`Deserialize`** | Add derives for TOML config parsing. |
| P1 | **`BallotSchema` needs `#[non_exhaustive]`** | Required for v0+1 `Enum` variant addition without semver break. |
| P1 | **Consistent `VoteCandidate` type** | Replace the `(&str, String, usize)` tuple in `compute_vote` with the `VoteCandidate` struct. |
| P1 | **Map `VoteError` to `StrategyError`** | Document the mapping from `VoteError::QuorumLost` to `StrategyError::Phase(PhaseError::QuorumLost)` in parallel_fanout dispatch. |
| P1 | **Add "vote" to `phase_result_parallel.schema.json`** | The JSON schema's `aggregator` enum must include `"vote"`. |
| P2 | **Clarify `winner` is normalised text** | Add a comment or field: `original_text` alongside `winner`, or document that downstream consumers receive only the normalised form. |
| P2 | **Remove or implement `VoteError::NoOpinion`** | It's defined but never returned. Either implement it (all buckets single-entry with count 1) or remove it. |
| P2 | **Document `vote_counts` sort order** | Note in the type or doc comment that `vote_counts` is sorted by count descending. |
| P2 | **Minimise `rand` dependency** | Import only `rand::rngs::StdRng` and `rand::seq::SliceRandom`, not the full `rand` crate. Consider `rand_core` for a slimmer dependency. |
| P3 | **Add acceptance criteria section** | Restate FR-12's AC as concrete, testable criteria in the design doc. |
| P3 | **Rename `concat.rs` or extract `Aggregator`** | Four variants in a file named for one variant is a smell. Not blocking, but consider `aggregator.rs` for the enum and keeping `concat.rs` for just the `aggregate_concat` function. |
