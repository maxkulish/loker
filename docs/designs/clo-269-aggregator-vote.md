# Design: CLO-269 - Aggregator: Vote

## 1. Problem

Per `docs/designs/clo-269-aggregator-vote.draft` discovery, workflow authors using `Strategy::ParallelFanOut` cannot resolve N candidate responses by majority count today. The schema-level label `crate::strategy::Aggregator::Vote` already round-trips through `phase_result_parallel.schema.json`, but `src/aggregator/concat.rs` has no behavioural variant for it, so any workflow that selects `vote` falls through unhandled. Authors who want a yes/no classification across `loker_d1_anthropic`, `loker_d1_openai`, and `loker_d1_zhipu` are forced to hand-craft an LLMJudge prompt for what is mechanical counting. CLO-269 is task T-019 on the M3 critical path, with `family_of` (CLO-265) and the `LLMJudge` wiring (CLO-268) both already merged - this closes the M3 aggregator vocabulary.

## 2. Goals / Non-goals

### Goals

- New behavioural variant `Aggregator::Vote { ballot_schema, tie_break, abstain_policy }` in `src/aggregator/concat.rs`, mapped through `Aggregator::kind()` to the schema label.
- New module `src/aggregator/vote.rs` owning a pure, synchronous `compute_vote(...)` function that operates on collected branch outcomes (successes + failures).
- `BallotSchema::FreeText { case_fold: bool, trim: bool }` and `BallotSchema::Enum { variants: Vec<String> }`.
- `TieBreak::ClosestToFamily(Family)`, `TieBreak::Random { seed: u64 }`, `TieBreak::FirstResponder` - all deterministic given the same inputs.
- Abstention-aware quorum: backend errors and ballot parse failures count as abstains; exceeding the configured `AbstainPolicy` raises `PhaseError::QuorumLost`.
- New `PhaseError::QuorumLost { abstains, total, policy }` variant in `src/family.rs`.
- `ParallelFanOut` collects all branches (no early short-circuit) and dispatches to `vote::compute_vote` synchronously after the loop.

### Non-goals

- Weighted voting (already handled by `src/consensus.rs::weighted_vote` under a different orchestration path).
- Recursive or adaptive tie-breaking (re-prompting tied candidates).
- JSON-schema-driven ballot validation.
- Prompt engineering for the ballot question itself - rendering remains the responsibility of `ParallelFanOut`'s existing template engine.
- Refactoring `src/consensus.rs::majority_vote` to share code (rejected as Approach C in discovery).
- Mutating `Aggregator::aggregate()`'s async signature - vote logic is pure, so it gets a direct call path from `ParallelFanOut` similar to `AnyFail`.

## 3. Architecture

### Module layout

```
src/
├── aggregator/
│   ├── mod.rs           (existing - re-exports)
│   ├── concat.rs        (existing - extend Aggregator enum + kind())
│   ├── llm_judge.rs     (existing - reference pattern)
│   └── vote.rs          (NEW)
├── strategy/
│   ├── mod.rs           (existing - schema enum already has Vote)
│   └── parallel_fanout.rs (existing - add is_vote dispatch path)
└── family.rs            (existing - add PhaseError::QuorumLost)
```

### Data flow

```
ParallelFanOut::execute(...)
   │
   │  FuturesUnordered loop completes (no short-circuit for Vote)
   │  -> Vec<BranchSuccess>, Vec<BranchFailure>
   │
   ├─ if matches!(aggregator, Aggregator::Vote { .. }):
   │      vote::compute_vote(VoteInput {
   │          successes: &[BranchSuccess],
   │          failures:  &[BranchFailure],
   │          ballot_schema,
   │          tie_break,
   │          abstain_policy,
   │          arrival_order: Vec<BackendId>,   // index into successes by completion order
   │      }) -> Result<VoteOutcome, PhaseError>
   │
   └─ wrap VoteOutcome into AggregatedArtifact
```

`VoteOutcome` carries the winning response text plus per-bucket counts, abstain count, the tie-break rule actually applied (or `None` if a strict majority was reached), and a `seed_used: Option<u64>` for reproducibility logging.

### Vote computation pipeline

1. **Bucket** each `BranchSuccess.content` through the `BallotSchema`:
   - `FreeText { trim, case_fold }`: apply trim/case-fold, bucket by exact string match.
   - `Enum { variants }`: match the normalised text against `variants`; on miss, classify as abstain.
2. **Combine** abstains from `failures.len()` + parse-failure count.
3. **Apply `AbstainPolicy`** (see §4): if the policy is exceeded, return `PhaseError::QuorumLost`.
4. **Strict majority check**: a bucket wins outright if its count > `votes_cast / 2` (votes_cast excludes abstains).
5. **Tie-break dispatch** when no strict majority:
   - `ClosestToFamily(family)`: among the top-tied buckets, pick the candidate whose `family_of(backend_id)` matches `family`. If multiple match, fall through to `FirstResponder` semantics within those.
   - `Random { seed }`: deterministic shuffle via `rand::SeedableRng` (e.g. `StdRng::seed_from_u64`) over the top-tied bucket list sorted lexicographically; log the seed.
   - `FirstResponder`: among top-tied buckets, pick the bucket whose earliest contributor appears first in `arrival_order`.

### `arrival_order` plumbing

`ParallelFanOut` already drains a `FuturesUnordered`; we capture each `BackendId` into a `Vec<BackendId>` as branches resolve (regardless of success/failure) and pass that into `VoteInput`. This adds one `Vec::push` per resolved future - no extra storage that would not already exist.

### ASCII relationship diagram

```
+---------------------+      +-------------------+
| ParallelFanOut      |----->| Aggregator::Vote  |
|  - FuturesUnordered |      |  ballot_schema    |
|  - arrival_order    |      |  tie_break        |
+---------------------+      |  abstain_policy   |
        | collects all       +---------+---------+
        v                              |
+----------------------+               |
| Vec<BranchSuccess>   |               |
| Vec<BranchFailure>   |               v
+----------+-----------+     +-------------------+
           +---------------->| vote::compute_vote|
                             |  (pure, sync)     |
                             +---------+---------+
                                       |
                  +--------------------+--------------------+
                  v                    v                    v
           VoteOutcome           PhaseError::          family_of() lookup
           (winner, counts,      QuorumLost            (only for
            seed_used,                                  ClosestToFamily)
            tie_rule_used)
```

## 4. Public API surface

### `src/family.rs`

```rust
#[derive(Debug, thiserror::Error)]
pub enum PhaseError {
    // ... existing variants: FamilyOverlap, AggregatorContract, JudgeUnavailable

    #[error("quorum lost: {abstains} of {total} branches abstained (policy: {policy:?})")]
    QuorumLost {
        abstains: usize,
        total: usize,
        policy: AbstainPolicy,
    },
}
```

### `src/aggregator/concat.rs` (extension to existing enum)

```rust
pub enum Aggregator {
    Concat,
    AnyFail,
    LLMJudge {
        judge_backend: BackendId,
        prompt_template: String,
        require_judge_different_family: bool,
    },
    Vote {
        ballot_schema: BallotSchema,
        tie_break: TieBreak,
        abstain_policy: AbstainPolicy,
    },
}

impl Aggregator {
    pub fn kind(&self) -> crate::strategy::Aggregator {
        match self {
            // ... existing arms
            Aggregator::Vote { .. } => crate::strategy::Aggregator::Vote,
        }
    }
}
```

### `src/aggregator/vote.rs` (new module)

```rust
use crate::backend::BackendId;
use crate::family::{Family, PhaseError};
use crate::strategy::parallel_fanout::{BranchFailure, BranchSuccess};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BallotSchema {
    FreeText {
        trim: bool,
        case_fold: bool,
    },
    Enum {
        variants: Vec<String>,
    },
}

impl Default for BallotSchema {
    fn default() -> Self {
        BallotSchema::FreeText { trim: true, case_fold: true }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TieBreak {
    ClosestToFamily(Family),
    Random { seed: u64 },
    FirstResponder,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbstainPolicy {
    /// Fail when abstains >= max.
    MaxCount(usize),
    /// Fail when abstains / total >= fraction.
    MaxFraction(f64),
}

#[derive(Debug, Clone)]
pub struct VoteInput<'a> {
    pub successes: &'a [BranchSuccess],
    pub failures: &'a [BranchFailure],
    pub arrival_order: &'a [BackendId],
    pub ballot_schema: &'a BallotSchema,
    pub tie_break: &'a TieBreak,
    pub abstain_policy: &'a AbstainPolicy,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoteOutcome {
    pub winner_text: String,
    pub winner_backend: BackendId,
    pub counts: Vec<(String, usize)>,
    pub abstains: usize,
    pub votes_cast: usize,
    pub tie_rule_used: Option<TieBreak>,
    pub seed_used: Option<u64>,
}

pub fn compute_vote(input: VoteInput<'_>) -> Result<VoteOutcome, PhaseError>;
```

### `src/strategy/parallel_fanout.rs` (dispatch extension)

```rust
// Inside execute(), after the FuturesUnordered loop:
let is_vote = matches!(self.aggregator, Aggregator::Vote { .. });
if is_vote {
    let Aggregator::Vote {
        ballot_schema,
        tie_break,
        abstain_policy,
    } = &self.aggregator
    else {
        unreachable!()
    };
    let outcome = crate::aggregator::vote::compute_vote(VoteInput {
        successes: &successes,
        failures: &failures,
        arrival_order: &arrival_order,
        ballot_schema,
        tie_break,
        abstain_policy,
    })?;
    return Ok(AggregatedArtifact::from_vote(outcome));
}
```

## 5. Test plan

### Unit tests (`src/aggregator/vote.rs`)

All synchronous, no `tokio` runtime, no `wiremock` (vote is pure):

- `vote_strict_majority_freetext_wins` - 3 successes, 2 say "yes", 1 says "no" -> winner is "yes", `tie_rule_used` is `None`.
- `vote_strict_majority_after_normalisation` - inputs `" Yes "`, `"yes"`, `"NO"` with `trim+case_fold` -> "yes" wins 2-1.
- `vote_enum_schema_classifies_invalid_as_abstain` - schema variants `["yes","no"]`, one branch returns `"maybe"` -> 1 abstain, vote between remaining two.
- `vote_tie_closest_to_family_picks_anthropic` - 1 anthropic vote, 1 openai vote, both for different answers; `ClosestToFamily(Family::Anthropic)` -> anthropic answer wins.
- `vote_tie_random_is_deterministic_for_seed` - same `seed: 42`, two runs over identical input return identical winner; `seed_used == Some(42)`.
- `vote_tie_first_responder_uses_arrival_order` - tied buckets `[A,B]`; `arrival_order = [b_id, a_id]` -> B wins.
- `vote_quorum_lost_on_max_count_threshold` - 2 abstains, `AbstainPolicy::MaxCount(2)` -> `Err(PhaseError::QuorumLost { .. })`.
- `vote_quorum_lost_on_max_fraction_threshold` - 2 abstains of 3 total, `AbstainPolicy::MaxFraction(0.5)` -> `QuorumLost`.
- `vote_zero_successes_all_failures_yields_quorum_lost` - 0 successes, 3 failures, any policy -> `QuorumLost`.
- `vote_kind_round_trips_to_schema_label` - `Aggregator::Vote { .. }.kind() == strategy::Aggregator::Vote`.

### Integration tests (`tests/parallel_fanout_vote.rs`)

Use `wiremock` for the three branch backends; one full `ParallelFanOut::execute` round-trip per case:

- `parallel_fanout_vote_majority_wins_end_to_end` - three mocked TensorZero responses, vote aggregator selected, asserts the artifact carries the majority text and the schema-labelled `aggregator: "vote"` field.
- `parallel_fanout_vote_branch_error_counts_as_abstain` - one of three mocks returns 500, remaining two return identical text -> winner is that text with `abstains == 1`.
- `parallel_fanout_vote_quorum_lost_propagates_phase_error` - two of three mocks return malformed ballots under `BallotSchema::Enum`, `AbstainPolicy::MaxCount(1)` -> `Err(PhaseError::QuorumLost)` surfaces from `ParallelFanOut::execute`.

### Snapshot tests

- `snapshot_vote_phase_result_matches_schema` - serialize a `VoteOutcome` into the phase-result envelope and validate against `docs/schemas/phase_result_parallel.schema.json` using the existing schema validator harness (mirrors the `LLMJudge` snapshot test from CLO-268).

### Manual verification

1. `make check` passes (fmt + clippy + test).
2. Author a workflow in `.lok/workflows/` that fans out a yes/no question to `loker_d1_anthropic`, `loker_d1_openai`, `loker_d1_zhipu` with `Aggregator::Vote { ballot_schema: Enum { variants: ["yes","no"] }, tie_break: FirstResponder, abstain_policy: MaxFraction(0.5) }`.
3. `LOKER_TZ_INTEGRATION=1 cargo run --bin loker -- run <workflow>` against the local TensorZero gateway; confirm the phase result file matches the snapshot shape and the chosen winner is logged.

## 6. Migration / rollout

- **Backward compatibility**: zero existing workflows can select `Vote` because no behavioural variant existed. Any workflow that did try would have failed at `Aggregator::kind()` mapping or earlier. Adding the variant is purely additive.
- **Schema**: `phase_result_parallel.schema.json` already accepts `"vote"` - no schema bump needed.
- **`PhaseError::QuorumLost` addition**: matches against `PhaseError` in callers must remain non-exhaustive (current pattern in the codebase). Confirm no `match` on `PhaseError` in `src/strategy/parallel_fanout.rs` or `src/family.rs` is exhaustive before merge; if any are, add the arm.
- **Feature flags**: none. The variant is gated only by config selection.
- **Rollout order**: 
  1. Add `PhaseError::QuorumLost` + tests.
  2. Add `BallotSchema`, `TieBreak`, `AbstainPolicy`, `VoteInput`, `VoteOutcome`, `compute_vote` in `vote.rs` with unit tests (TDD-first per `docs/handoff.md`).
  3. Extend `Aggregator` enum and `kind()` in `concat.rs`.
  4. Wire dispatch in `parallel_fanout.rs` + integration tests.
  5. Snapshot test against the schema.
  6. `make check` clean -> single PR.
- **Legacy `src/consensus.rs`**: untouched. Per handoff Intent ("don't mutate-in-place"), the legacy path stays working until the new aggregator vocabulary fully subsumes it.

## 7. Open questions

These are unresolved from discovery and should be closed during implementation review, not assumed:

1. **Seed source for `TieBreak::Random`**. Discovery debt item 2 lists two options:
   - workflow-level config (seed lives in `.lok/workflows/<name>.toml`) - reproducible per workflow definition, but the same seed reused across runs makes "random" deterministically identical every time.
   - run-level UUID (derived from a per-execution UUID hashed to `u64`) - varies per run but still logged for replay.
   The PRD says "seed sourced from manifest / workflow config" but the discovery report flags this as open. Tradeoff: reproducibility vs. genuine variability. The proposed `TieBreak::Random { seed: u64 }` carries the seed inline so either source can populate it; the surrounding config layer is what is unresolved.

2. **Quorum threshold spelling**: discovery debt item 3 - absolute `MaxCount(usize)` vs. fractional `MaxFraction(f64)`. This design proposes `AbstainPolicy` as an enum with both, leaving the call site to choose. If the project prefers a single spelling, collapse to one variant and document why in the PR.

3. **Ballot schema lock-in for v0**. PRD recommends `FreeText` as default with `Enum` "strongly desired"; discovery flags this as open. This design ships both, but if `Enum` is deferred to a follow-up issue, the `BallotSchema` enum should still land with both variants today (so the public API is stable) even if `Enum` is unimplemented behind a `todo!()` - or be deferred entirely. Tradeoff: API stability now vs. shipping less surface area. Recommend shipping both since the implementation cost of `Enum` matching is trivial.

4. **Tie-break fallback chaining**. When `ClosestToFamily` matches multiple tied buckets (e.g. two anthropic candidates tied), this design falls through to `FirstResponder` semantics. The PRD does not specify this. Alternative: surface as `PhaseError::AggregatorContract`. Tradeoff: pragmatic resolution vs. strict configuration. Decision deferred to TDD review.

5. **Abstain on `BallotSchema::FreeText`**. Free text in principle never fails to parse, so abstains under `FreeText` come only from backend errors. Should empty-string responses count as abstains? Currently this design treats `""` (after trim) as a valid (empty-string) bucket. If the desired semantics is "empty = abstain", that needs a flag on `BallotSchema::FreeText`.
