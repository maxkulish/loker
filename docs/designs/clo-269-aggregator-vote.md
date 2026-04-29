# Design: CLO-269 — Aggregator::Vote with ballot schema and tie-breakers

| Field | Value |
|-------|-------|
| Task | CLO-269 |
| Date | 2026-04-29 |
| Phase | design |
| Discovery | docs/discovery/clo-269.md |
| PRD | docs/prds/clo-269-aggregator-vote.md |

## 1. Problem

`ParallelFanOut` currently supports `Concat`, `AnyFail`, and `LLMJudge` aggregators. The
`Vote` label exists in the schema-facing `Aggregator` enum but has zero behavioural
implementation. Workflow authors who want N backends to answer the same ballot
question and pick the majority winner cannot express this today — they must hand-craft
an LLMJudge prompt, which is over-engineered for mechanical counting. This design
closes the gap by adding a pure, self-contained `vote.rs` module under
`src/aggregator/` that counts normalised responses, abstains on malformed or
error-time answers, applies tie-breakers, and produces a structured
`AggregatedArtifact`.

Reference: discovery report §Problem Framing.

## 2. Goals & Non-goals

### Goals
- Add `Aggregator::Vote { ballot_schema, tie_break, abstain_threshold }` to the
  behavioural enum in `src/aggregator/concat.rs`.
- Define `BallotSchema::FreeText` (v0 default) and reserve `BallotSchema::Enum`
  (deferred).
- Define `TieBreak::ClosestToFamily(Family)`, `TieBreak::Random { seed }`, and
  `TieBreak::FirstResponder`.
- Normalise free-text answers for comparison (`trim().to_lowercase()` in v0).
- Treat backend errors during parallel execution as abstentions (not votes).
- Return `PhaseError::QuorumLost` when abstentions exceed `abstain_threshold`.
- Produce an `AggregatedArtifact` containing the winning response text plus a
  structured metadata comment (vote counts, abstain count, tie-break used).
- Unit-test every tie-break path with fixed seeds / deterministic inputs.
- Snapshot-test serialised `StrategyOutput` against the parallel schema.

### Non-goals
- Weighted voting (already exists in `src/consensus.rs` as a distinct concern,
  not an `Aggregator`).
- Recursive tie-breaking (e.g. second-round runoff between tied candidates).
- Ballot validation using a JSON schema or external parser.
- Prompt engineering for the ballot question itself (Vote only interprets answers,
  not the question rendering).
- `BallotSchema::Enum` variant (deferred to v0+1).
- Configurable normalisation function beyond v0 `trim().to_lowercase()`.
- Cross-family enforcement at the Vote aggregator level. `LLMJudge` enforces a
  judge from a different family than the reviewers; `Vote` has no "judge" and is
  inherently counting diversity. Cross-family selection for the *parallel pool* is
  the strategy's responsibility, not the aggregator's. FR-13 enforcement remains
  scoped to `LLMJudge` and is documented out-of-scope for Vote.

## 2.5 Acceptance Criteria

Restated from PRD FR-12 as concrete, testable criteria for the design phase:

1. `Aggregator::Vote { config: VoteConfig }` is accepted by the TOML parser and
   round-trips through `Aggregator::kind()` → `strategy::Aggregator::Vote`.
2. `aggregate_vote` returns a strict-majority winner in O(N) time for N branches.
3. `TieBreak::Random { seed }` produces identical winners across repeated runs
   with the same seed and inputs.
4. `TieBreak::FirstResponder` selects the bucket whose earliest-completed branch
   arrived first in the `FuturesUnordered` completion order.
5. `abstain_threshold` semantics use strict greater-than (`abstain_count > threshold`).
6. Backend errors, empty responses, and malformed ballots are all counted as
   abstentions (not votes).
7. `PhaseError::QuorumLost` maps cleanly from `VoteError::QuorumLost` →
   `StrategyError::Phase(PhaseError::QuorumLost)` → `PhaseError`, so the
   orchestrator treats lost quorum as a terminal phase failure, not a retryable
   error.
8. The aggregated artefact metadata comment is deterministic (sorted keys,
   sanitised text) so snapshot tests are stable across runs.

## 3. Architecture

### 3.1 Modules

```
src/
  aggregator/
    mod.rs          # AnyFail logic, re-exports, shared helpers
    concat.rs       # Behavioural Aggregator enum + Concat (expanded with Vote variant)
    llm_judge.rs    # LLMJudge config + behaviour (unchanged)
    vote.rs         # NEW: Vote config, normalisation, counting, tie-breakers
  strategy/
    mod.rs          # Aggregator schema label enum (unchanged)
    parallel_fanout.rs  # Extended: post-collection vote dispatch
  family.rs         # Add PhaseError::QuorumLost
```

### 3.2 Data flow

```
ParallelFanOut::execute
  │
  ├─ FuturesUnordered loop collects all attempts (same as today)
  │
  ├─ match self.aggregator
  │    ├─ Concat  → concat::aggregate_concat(...)
  │    ├─ AnyFail → handled inline (short-circuit)
  │    ├─ LLMJudge → llm_judge::aggregate_llm_judge(...)
  │    └─ Vote    → vote::aggregate_vote(
  │                     branches,       // &amp;[BranchOutcome] — successes + failures
  │                     ballot_schema,
  │                     tie_break,
  │                     abstain_threshold,
  │                  )
  │
  └─ Returns StrategyOutput with
       aggregator: Some(Aggregator::Vote),
       aggregate_output_path: "{phase}/aggregated.txt",
       verify: VerifyOutcome::passed("Vote")
```

**Key difference from LLMJudge:** Vote does not need `backends` or `PhaseContext`
because it is pure (no secondary backend call). It only needs the collected branch
outcomes and the config. This keeps the call synchronous and unit-testable.

### 3.3 New types

```rust
// src/aggregator/vote.rs
use crate::family::Family;

/// How a ballot is normalised and interpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BallotSchema {
    /// Free text: each backend returns prose; normalise before bucketing.
    /// v0 normalisation: trim whitespace + lowercase.
    FreeText,
    // Enum { variants: Vec&lt;String&gt; } — reserved for v0+1
}

/// How to resolve a tie when no strict majority exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TieBreak {
    /// Pick the candidate whose backend family matches the given family.
    /// If multiple candidates match, first occurrence in arrival order wins.
    ClosestToFamily(Family),
    /// Deterministic shuffle from a fixed seed.
    /// The seed is sourced from the workflow config (`lok.toml` or per-phase
    /// override) and emitted in the aggregated artefact for reproducibility.
    Random { seed: u64 },
    /// Pick the candidate whose successful response arrived first.
    FirstResponder,
}

/// Config payload for the Vote aggregator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoteConfig {
    pub ballot_schema: BallotSchema,
    pub tie_break: TieBreak,
    /// Number of abstentions (errors + malformed answers) that triggers
    /// `PhaseError::QuorumLost`. If `abstain_threshold` == `n_targets`,
    /// a single error does not abort; this only fires when **strictly more**
    /// than `abstain_threshold` are abstentions.
    ///
    /// Example: 3 targets, threshold = 1, 0 abstentions → ok.
    /// 3 targets, threshold = 1, 2 abstentions → `QuorumLost`.
    pub abstain_threshold: usize,
}

/// Result of a vote aggregation, including metadata for traceability.
#[derive(Debug, Clone)]
pub struct VoteResult {
    /// The winning text (normalised key). If downstream consumers need the
    /// original casing, they must look up the chosen candidate's raw text.
    pub winner: String,
    /// vote_counts is always sorted descending by count for snapshot determinism.
    pub vote_counts: Vec&lt;(String, usize)&gt;,
    pub abstain_count: usize,
    pub total_branches: usize,
    pub tie_broken: bool,
    pub tie_break_rule: String,
}

/// Errors specific to Vote aggregation.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VoteError {
    #[error("quorum lost: {abstains} abstentions exceed threshold {threshold}")]
    QuorumLost { abstains: usize, threshold: usize },

    #[error("no candidates available")]
    NoCandidates,
}

// Note: `NoOpinion` was considered (identical responses from all branches) but
// removed because a unanimous single-bucket result is a valid strict majority,
// not an error.
```

### 3.4 Public API surface

```rust
// src/aggregator/concat.rs (behavioural Aggregator enum expanded)
pub enum Aggregator {
    Concat { heading_template: String },
    AnyFail,
    LLMJudge { judge_backend: String, prompt_template: String, require_judge_different_family: bool },
    Vote { config: VoteConfig },
}

impl Aggregator {
    pub fn kind(&self) -> crate::strategy::Aggregator { ... }
    pub fn vote(config: VoteConfig) -> Self { Self::Vote { config } }
}
```

```rust
// src/aggregator/vote.rs
pub fn aggregate_vote(
    branches: &[BranchOutcome],
    config: &VoteConfig,
) -> Result&lt;(AggregatedArtifact, VoteResult), VoteError&gt;;

pub fn normalise_ballot(text: &str) -> String;

pub fn compute_vote(
    candidates: &[(&str, String, usize)], // (backend_id, normalised, original_arrival_order)
    tie_break: &TieBreak,
) -> Option&lt;VoteResult&gt;;
```

### 3.5 PhaseError expansion and StrategyError mapping

```rust
// src/family.rs
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum PhaseError {
    // ... existing variants ...

    #[error("quorum lost: {abstains} abstentions exceed threshold {threshold}")]
    QuorumLost { abstains: usize, threshold: usize },
}
```

`VoteError` maps into the orchestrator's error hierarchy as follows:
- `VoteError::QuorumLost` → `StrategyError::Phase(PhaseError::QuorumLost)`
- `VoteError::NoCandidates` → `StrategyError::Phase(PhaseError::AggregatorRejected)`

This mapping is performed in `parallel_fanout.rs` at the dispatch site, analogous to `LLMJudgeError` conversion.

## 4. Implementation details

### 4.1 Short-circuit guard

`ParallelFanOut::execute` currently short-circuits when `successes >= min_responses`.
Vote must collect **all** branches (including failures as abstentions) before it can
compute a majority or detect a quorum loss. Therefore the early-break condition is
updated:

```rust
if !is_any_fail && !is_llm_judge && !is_vote && successes >= self.min_responses {
    break;
}
```

`is_vote` is computed from `self.aggregator.kind() == Aggregator::Vote`.

### 4.2 Branch outcome → vote candidates

In `parallel_fanout.rs`, after the `FuturesUnordered` loop:

```rust
let vote_branches: Vec&lt;BranchOutcome&gt; = attempts
    .iter()
    .enumerate()
    .map(|(arrival_order, attempt)| {
        // If the attempt succeeded, we already pushed a BranchSuccess
        // into successful_candidates. We need to map both successes
        // and failures into BranchOutcome for the aggregator.

        // For Vote specifically, we must retain arrival order
        // (for FirstResponder) and backend_id (for ClosestToFamily).
    })
    .collect();

// Actually, ParallelFanOut already has:
// - successful_candidates: Vec&lt;BranchSuccess&gt;
// - attempts: Vec&lt;Attempt&gt; with error metadata
// We convert both into BranchOutcome::Success / BranchOutcome::Failure.
```

A better approach: instead of building a new vector, pass `&attempts` plus
`&successful_candidates` directly to the vote helper. But `vote.rs` should
only know about `BranchOutcome` (the domain of aggregators), not `Attempt`
(the domain of strategies).

So `ParallelFanOut` will construct `Vec&lt;BranchOutcome&gt;` from the loop:

```rust
let mut vote_branches = Vec::with_capacity(self.targets.len());
for (idx, attempt) in attempts.iter().enumerate() {
    if attempt.finish_reasons.contains(&FinishReason::Error) {
        vote_branches.push(BranchOutcome::Failure(BranchFailure {
            backend_id: attempt.backend.clone(),
            family: attempt.family.clone().unwrap_or_default(),
            index: idx + 1,
            reason: format!("backend error: {:?}", attempt.finish_reasons),
        }));
    } else {
        vote_branches.push(BranchOutcome::Success(BranchSuccess {
            backend_id: attempt.backend.clone(),
            family: attempt.family.clone().unwrap_or_default(),
            index: idx + 1,
            output: /* read from output_path or from stdout in Attempt? */,
        }));
    }
}
```

Wait — `Attempt` currently does not carry the stdout text directly; it carries
`output_path`. For `Concat`, the aggregator reads from disk. For `AnyFail`, the
evaluator reads `query.stdout` inline during the loop. For `LLMJudge`, the
`successful_candidates` vector was built with `output: query.stdout.clone()`.

For `Vote`, we'll build `BranchSuccess` entries at the same point in the loop
where `LLMJudge` builds them:

```rust
successes += 1;
successful_candidates.push(BranchSuccess {
    backend_id: target.backend.clone(),
    family: family_of(&target.backend).to_string(),
    index: successes, // 1-based, NOT arrival_order
    output: query.stdout.clone(),
});
```

But `Vote` cares about **arrival order** for `FirstResponder`, while
`BranchSuccess.index` is currently `successful_candidates.len() + 1` (i.e. the
1-based index among *successful* branches only). This is wrong for
`FirstResponder`, which needs the absolute arrival order among *all* branches
(successes and failures).

**Resolution:** We will NOT reuse `successful_candidates` for Vote. Instead,
we'll build a separate `vote_branches: Vec<BranchOutcome>` inside the loop,
preserving the absolute branch order (0..N). Each `BranchOutcome::Success`
will carry the actual `query.stdout` text. Each `BranchOutcome::Failure` will
carry an error reason. The `Vote` module then normalises the successful answers
and counts them.

### 4.3 Vote counting algorithm

```rust
pub fn aggregate_vote(
    branches: &[BranchOutcome],
    config: &VoteConfig,
) -> Result<(AggregatedArtifact, VoteResult), VoteError> {
    let mut abstain_count = 0;
    let mut candidates: Vec<VoteCandidate> = Vec::new();
    let total = branches.len();

    for (arrival_order, branch) in branches.iter().enumerate() {
        match branch {
            BranchOutcome::Success(success) => {
                let normalised = normalise_ballot(&success.output);
                if normalised.is_empty() {
                    abstain_count += 1;
                } else {
                    candidates.push(VoteCandidate {
                        backend_id: success.backend_id.clone(),
                        family: success.family.clone(),
                        normalised,
                        arrival_order,
                    });
                }
            }
            BranchOutcome::Failure(_) => {
                abstain_count += 1;
            }
        }
    }

    if abstain_count > config.abstain_threshold {
        return Err(VoteError::QuorumLost {
            abstains: abstain_count,
            threshold: config.abstain_threshold,
        });
    }

    if candidates.is_empty() {
        return Err(VoteError::NoCandidates);
    }

    // Count votes by normalised bucket
    // BTreeMap ensures deterministic iteration order for tie-break determinism.
    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new(); // normalised → candidate indices
    let mut first_seen: BTreeMap<String, usize> = BTreeMap::new();   // normalised → first arrival_order
    for (idx, c) in candidates.iter().enumerate() {
        buckets.entry(c.normalised.clone()).or_default().push(idx);
        first_seen.entry(c.normalised.clone()).or_insert(c.arrival_order);
    }

    let max_votes = buckets.values().map(|v| v.len()).max().unwrap_or(0);
    let winners: Vec<&str> = buckets
        .iter()
        .filter(|(_, v)| v.len() == max_votes)
        .map(|(k, _)| k.as_str())
        .collect();

    let mut result = if winners.len() == 1 {
        let winner_text = winners[0];
        let chosen_idx = buckets[winner_text][0];
        VoteResult {
            winner: candidates[chosen_idx].normalised.clone(),
            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
            abstain_count,
            total_branches: total,
            tie_broken: false,
            tie_break_rule: "none (strict majority)".into(),
        }
    } else {
        // TIE: apply tie_breaker
        let chosen_text = resolve_tie(&winners, &candidates, &buckets, &config.tie_break);
        let chosen_idx = buckets[chosen_text][0]; // first occurrence of that bucket
        VoteResult {
            winner: chosen_text.into(),
            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
            abstain_count,
            total_branches: total,
            tie_broken: true,
            tie_break_rule: format!("{:?}", config.tie_break), // or a dedicated helper
        }
    };

    // Sort vote_counts descending for stable output
    result.vote_counts.sort_by(|a, b| b.1.cmp(&a.1));

    let text = build_aggregated_text(&result, &candidates);
    let artifact = AggregatedArtifact {
        text,
        successful: candidates.len(),
        failed: abstain_count,
    };

    Ok((artifact, result))
}
```

### 4.4 Tie-breaker resolution

```rust
fn resolve_tie(
    tied_buckets: &[&str],
    candidates: &[VoteCandidate],
    buckets: &BTreeMap<String, Vec<usize>>,
    tie_break: &TieBreak,
) -> &str {
    match tie_break {
        TieBreak::FirstResponder => {
            // Pick the bucket whose first candidate arrived earliest.
            tied_buckets
                .iter()
                .min_by_key(|&bucket| {
                    buckets[*bucket]
                        .iter()
                        .map(|&ci| candidates[ci].arrival_order)
                        .min()
                        .unwrap_or(usize::MAX)
                })
                .copied()
                .unwrap_or(tied_buckets[0])
        }

        TieBreak::ClosestToFamily(target_family) => {
            // Collect all tied buckets that contain at least one candidate
            // whose family matches target_family.
            let matching: Vec<&str> = tied_buckets
                .iter()
                .copied()
                .filter(|&bucket| {
                    buckets[bucket]
                        .iter()
                        .any(|&ci| family_of(&candidates[ci].backend_id) == *target_family)
                })
                .collect();

            if matching.is_empty() {
                // Fallback: if no bucket matches target family, use FirstResponder.
                resolve_tie(tied_buckets, candidates, buckets, &TieBreak::FirstResponder)
            } else if matching.len() == 1 {
                matching[0]
            } else {
                // Multiple matching buckets: apply FirstResponder among the matching subset.
                resolve_tie(&matching, candidates, buckets, &TieBreak::FirstResponder)
            }
        }

        TieBreak::Random { seed } => {
            use rand::{seq::SliceRandom, SeedableRng};
            use rand::rngs::StdRng;
            let mut rng = StdRng::seed_from_u64(*seed);
            let mut choices = tied_buckets.to_vec();
            choices.shuffle(&mut rng);
            choices[0]
        }
    }
}
```

### 4.5 Aggregated text format

The winning text goes first. A trailing HTML comment block carries metadata:

```markdown
&lt;answer text here&gt;

&lt;!-- loker: Vote aggregator metadata
  winner: &lt;text&gt;
  total_branches: N
  vote_counts:
    answer_a: 2
    answer_b: 1
  abstain_count: 0
  tie_broken: false
  tie_break_rule: none
--&gt;
```

Before writing the metadata comment, `result.winner` and `result.tie_break_rule`
are sanitised by replacing `-->` with `-- >` to prevent premature comment closure.
Answer text itself is emitted **before** the comment, so even an adversarial backend
response cannot escape the metadata block.

### 4.6 Normalisation

```rust
pub fn normalise_ballot(text: &str) -> String {
    text.trim().to_lowercase()
}
```

v0 normalisation is intentionally simple. Future work (post-v0) may add:
- Unicode normalisation (NFKC)
- Stop-word removal
- Stemming
- Deduplication of whitespace

## 5. Test plan

### Unit tests (`src/aggregator/vote.rs`)

| Test | Input | Expected |
|------|-------|----------|
| `free_text_clear_winner` | 3 branches: "yes", "yes", "no" | winner="yes", tie_broken=false |
| `free_text_tie_first_responder` | 3 branches: "yes", "no", "yes" + tie=FirstResponder | winner="yes" (arrival 0) |
| `free_text_tie_closest_family` | 3 branches: claude/"a", gemini/"b", openai/"a" + tie=ClosestToFamily(Anthropic) | winner="a" (claude is Anthropic) |
| `free_text_tie_random` | 2 branches: "a", "b" + tie=Random { seed=42 } | deterministic winner from seed |
| `abstain_backend_error` | 2 successes + 1 failure | abstain_count=1, 2 valid votes |
| `quorum_lost` | 1 success + 2 failures, threshold=1 | VoteError::QuorumLost |
| `empty_input` | no branches | VoteError::NoCandidates |
| `all_abstain` | 3 failures | VoteError::QuorumLost (or NoCandidates if threshold ≥ 3) |
| `normalise_case` | "YES", "yes", "Yes" | all bucketed together |
| `normalise_whitespace` | "  yes  ", "yes\n" | all bucketed together |
| `closest_family_no_match_fallback` | tie with no matching family | falls back to FirstResponder |

### Integration tests (`tests/strategy_parallel_fanout.rs`)

| Test | Setup | Expected |
|------|-------|----------|
| `vote_success` | 3 mock backends returning "A", "A", "B" with Vote(FreeText) | StrategyOutput with correct path, schema validates |
| `vote_tie_random_deterministic` | 2 backends returning "A", "B", Random seed=123 | Same winner on multiple runs |
| `vote_quorum_lost` | 1 ok + 2 fail, threshold=0 | PhaseError::QuorumLost |
| `vote_snapshot` | fixture branches + Vote config | `insta::assert_snapshot!` on aggregated text |

### Snapshot test

1. Run `vote_success`.
2. Serialise `StrategyOutput` to JSON.
3. Validate against `docs/schemas/phase_result_parallel.schema.json`.
4. Use `insta::assert_snapshot` on `aggregated.txt` content.

## 6. Migration / rollout

- No breaking changes to existing `Concat`, `AnyFail`, or `LLMJudge` aggregators.
- `PhaseError` gains `QuorumLost` (additive, `#[non_exhaustive]`).
- `Aggregator` enum gains `Vote` variant (schema label already existed; behavioural payload is new).
- `src/aggregator/concat.rs` expands slightly (new variant in behavioural enum + `kind()` match arm).
- `parallel_fanout.rs` loop must be extended to build `vote_branches` and dispatch to `vote::aggregate_vote`. This is additive; existing branches for `AnyFail` and `LLMJudge` are untouched.
- **`docs/schemas/phase_result_parallel.schema.json`** must add `"vote"` to the
  `aggregator` string enum so JSON schema validation passes for Vote outputs.
  This is a one-line additive schema change.

## 7. Resolving discovery debt

| Debt item | Resolution |
|-----------|------------|
| Ballot schema shape | `BallotSchema::FreeText` in v0. `BallotSchema::Enum { variants: Vec<String> }` reserved. The decision is: enum-style choices are useful but require ballot-prompt engineering that is out of scope for Vote (it lives at the prompt/template layer). Vote only normalises the free-text answer it receives. |
| Seed source for `TieBreak::Random` | Configured in `lok.toml` under `[workflow.vote]` or per-phase `[[phase]] vote_seed = 0x1234`. If absent, it defaults to `0` in v0. The seed is logged in the aggregated artefact metadata block for traceability. |
| Quorum threshold semantics | Absolute count: `abstain_threshold: usize`. The phase fails when `abstain_count > abstain_threshold`. This matches the AC: "document the threshold where abstain-majority returns PhaseError::QuorumLost". Example: 3 targets, threshold=1, 2 abstentions → QuorumLost. |

## 8. Open questions

| Question | Resolution |
|----------|------------|
| Should `Vote::aggregate()` be added to the `Aggregator::aggregate()` method? | For now, Vote is called inline from `parallel_fanout.rs`, matching the `AnyFail` pattern. Future T-029 may formalise a proper `Aggregator` trait. |
| What does `FirstResponder` mean for a branch that backend-errored? | Errors are abstentions, so they never win. `FirstResponder` applies among *successful* branches only, ranked by their absolute arrival order in the FuturesUnordered loop. |
| Do we need a new `rand` dependency? | `rand` 0.8 is already an indirect dependency via `uuid` (see Cargo.lock). Adding it to `Cargo.toml` is acceptable if we use it for `TieBreak::Random`. Import only `rand::rngs::StdRng` and `rand::seq::SliceRandom` (or `rand_core` + `rand_rngs`) for a minimal footprint. |
| How do `min_responses` and `abstain_threshold` interact? | They are independent gates. `min_responses` controls when `ParallelFanOut` stops collecting additional branches; `abstain_threshold` controls whether Vote rejects the result after all branches are collected. Vote disables the `min_responses` short-circuit so it always sees the full set. |

## 9. References

- Discovery report: `docs/discovery/clo-269.md`
- PRD: `docs/prds/clo-269-aggregator-vote.md`
- CLO-265 `family.rs`: `src/family.rs`
- CLO-266 concat aggregator: `src/aggregator/concat.rs`
- CLO-267 AnyFail: `src/aggregator/mod.rs`
- CLO-268 LLMJudge: `src/aggregator/llm_judge.rs`, `docs/designs/clo-268-llm-judge.md`
- loker-design.md §4.3 (Aggregator trait overview)
- PRD FR-12 (Vote aggregator)
