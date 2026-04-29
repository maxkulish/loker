# Plan: CLO-269 — Implement Aggregator::Vote with ballot schema and tie-breakers

## Context
- **Design**: `docs/designs/clo-269-aggregator-vote.md`
- **Discovery**: `docs/discovery/clo-269.md`
- **PRD**: `docs/prds/clo-269-aggregator-vote.md`
- **Linear**: https://linear.app/cloud-ai/issue/clo-269/implement-aggregatorvote-with-ballot-schema-and-tie-breakers
- **Branch**: `feat/clo-269`

## Sub-tasks

### ST1 — Add `PhaseError::QuorumLost` to error hierarchy
**Files:** `src/family.rs`
**Acceptance:** `cargo test --lib family` compiles and passes; new variant is `#[non_exhaustive]` safe.
**Estimate:** S

### ST2 — Expand `Aggregator` behavioural enum with `Vote` variant
**Files:** `src/aggregator/concat.rs`
**Acceptance:** `cargo test --lib aggregator::concat` compiles and passes; `Aggregator::kind()` returns `strategy::Aggregator::Vote` for the new variant.
**Estimate:** S

### ST3 — Author `src/aggregator/vote.rs` module
**Files:** `src/aggregator/vote.rs` (new), `src/aggregator/mod.rs` (re-export)
**Acceptance:** `cargo test --lib aggregator::vote` passes all unit tests:
- `free_text_clear_winner` — strict majority
- `free_text_tie_first_responder` — arrival-order tie-break
- `free_text_tie_closest_family` — family-preference tie-break
- `free_text_tie_random` — deterministic random from fixed seed
- `abstain_backend_error` — errors count as abstentions
- `quorum_lost` — abstentions exceed threshold
- `empty_input` — no candidates
- `all_abstain` — every branch fails
- `normalise_case` — case-folding buckets
- `normalise_whitespace` — trim buckets
- `closest_family_no_match_fallback` — falls back to `FirstResponder`
**Estimate:** M

### ST4 — Wire `Vote` into `ParallelFanOut` strategy
**Files:** `src/strategy/parallel_fanout.rs`
**Acceptance:** `cargo test --test strategy_parallel_fanout` passes:
- `vote_success` — 3 backends → majority winner
- `vote_tie_random_deterministic` — identical winner on repeat runs
- `vote_quorum_lost` — `PhaseError::QuorumLost` bubbles up correctly
**Estimate:** M

### ST5 — Update schema and add snapshot tests
**Files:** `docs/schemas/phase_result_parallel.schema.json`, `tests/snapshots/` (new), `Cargo.toml` (add `rand` if needed)
**Acceptance:** `cargo test --test strategy_parallel_fanout vote_snapshot` passes; schema file includes `"vote"` in the `aggregator` enum.
**Estimate:** S

### ST6 — Pre-merge gate
**Acceptance:** `make check` (fmt + clippy + test) is green on `feat/clo-269`.
**Estimate:** S

## Risk: dependencies
- Depends on [CLO-265](https://linear.app/cloud-ai/issue/CLO-265) (`family_of` lookup) — already merged.
- `rand` crate is already an indirect dependency; adding it to `Cargo.toml` is low risk.

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Rollback plan
If Vote causes regressions mid-milestone:
1. Revert `src/strategy/parallel_fanout.rs` changes (additive, no merge conflicts).
2. Revert `src/aggregator/concat.rs` variant addition.
3. Delete `src/aggregator/vote.rs` and remove re-export in `mod.rs`.
4. Revert `src/family.rs` `PhaseError::QuorumLost` addition.
All changes are additive; no downstream code depends on Vote yet.
