# PRD: CLO-269 — Aggregator::Vote with ballot schema and tie-breakers

## Problem

Workflow authors using `Strategy::ParallelFanOut` need a way to ask each
backend a structured ballot question and pick the winner by majority, not
just join outputs (`Concat`) or judge them externally (`LLMJudge`). The
`Vote` enum variant exists as a schema label but has zero behavioural
implementation. Without it, majority-based consensus (e.g. “which approach
is simpler: A or B?”) requires hand-crafting an LLMJudge prompt, which is
over-engineered for mechanical counting.

## Goal

Implement `Aggregator::Vote { ballot_schema: BallotSchema, tie_break: TieBreak }`
that:

1. Collects free-text or enum-style responses from each successful branch.
2. Abstains on malformed responses or backend errors (does not count them
   as votes).
3. Declares a winner when one response commands a strict majority (> 50 %).
4. Applies a deterministic tie-break rule when no strict majority exists.
5. Fails the phase with `PhaseError::QuorumLost` when abstentions exceed a
   configurable threshold (or when too few votes are cast to reach majority).

## Scope (in)

- `Aggregator` variant `Vote { ballot_schema, tie_break }` added to the
  behavioural enum in `src/aggregator/concat.rs`.
- `BallotSchema` enum:
  - `FreeText` (default, v0): each backend returns free text; text is
    normalised (trimmed, case-folded by config) before bucket counting.
  - `Enum { variants: Vec<String> }` (optional but strongly desired): each
    backend must pick one variant; anything outside the set is treated as
    abstain.
- `TieBreak` enum:
  - `ClosestToFamily(Family)` — resolve toward the first candidate whose
    `family_of(backend_id)` matches the given `Family`.
  - `Random { seed: u64 }` — deterministic shuffle from a per-run seed
    (seed sourced from manifest / workflow config).
  - `FirstResponder` — choose the candidate that arrived first in
    `ParallelFanOut` branch completion order.
- Abstention handling:
  - Backend errors during parallel execution → abstain (not a vote).
  - Malformed ballot (garbled text, invalid enum choice) → abstain.
  - Configurable `abstain_threshold: usize` (or `max_abstain_fraction: f64`):
    if abstentions exceed the threshold, return `PhaseError::QuorumLost`.
- Unit tests for every tie-break path with fixed seeds.
- Snapshot of phase-result file shape matching
  `docs/schemas/phase_result_parallel.schema.json`.

## Scope (out)

- Weighted voting (already exists in `src/consensus.rs` as a distinct
  `ConsensusStrategy`, not an `Aggregator`).
- Adaptive or recursive tie-breaking (e.g. re-prompt tied candidates).
- Ballot validation using a JSON schema or external parser.
- Prompt engineering for the ballot question itself (the question is
  rendered by `ParallelFanOut`'s existing template engine; Vote only
  interprets answers).

## Acceptance Criteria

- [ ] Tests pin ballot parsing, majority math, abstention handling, and
      each of the three tie-break rules.
- [ ] Random tie-break is reproducible from a logged seed (assert in a
      test).
- [ ] Snapshot of phase result file shape.
- [ ] `PhaseError::QuorumLost` raised when abstentions exceed threshold.
- [ ] `Vote` aggregator registered in `src/aggregator/concat.rs`
      `Aggregator::kind()` so the schema label round-trips.

## Demotion clause

If no concrete first use case lands by M3 start, close as Won't-do (v0)
and document the deferral in the roadmap. (Per roadmap; M3 date not yet
fixed.)

## Dependencies

- `family_of` lookup from [CLO-265](https://linear.app/cloud-ai/issue/CLO-265)
  is merged to main and exercised by LLMJudge.
- `PhaseError::QuorumLost` variant may need to be added to
  `src/family.rs` if it does not already exist.

## Related

- PRD FR-12 (Vote aggregator, Should)
- Design doc §7 aggregators, §8 open question on ballot schema
- Roadmap task T-019 in `docs/plans/001-implementation-roadmap.md`
- Existing `majority_vote` in `src/consensus.rs` (different concern,
  but shares normalisation logic).
- CLO-268 (LLMJudge) demonstrates how to wire a new aggregator into
  `ParallelFanOut`.
