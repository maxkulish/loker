# PRD — CLO-292: PhaseRunner composing Strategy + Aggregator + VerifyHook

## Problem

Loker has independent implementations for strategy execution, aggregators, verify hooks, manifest persistence, and status markers, but no single phase-level coordinator that composes them into a durable execution loop. Developers and downstream orchestration tasks cannot run an end-to-end phase from input artefacts to verified output, so M5/M6 tasks such as tracing, run directory plumbing, resumability, and the reference workflow remain blocked. We need a `PhaseRunner` API that drives one phase to a terminal state and persists enough filesystem state for recovery without live in-memory state.

## Users and impact

- Workflow authors need a phase invocation to reliably produce one canonical artefact from a configured strategy and aggregator.
- Resume/run orchestration code needs durable started/completed/failed markers plus manifest entries to decide what can be skipped after a crash.
- Test and verification authors need verify-hook results to be captured and surfaced as typed errors when terminal failures occur.

## Requirements

- `PhaseRunner::run(phase, inputs) -> Result<PhaseOutcome, PhaseError>` returns only after the phase reaches success or terminal failure.
- Dispatch strategy names `single`, `parallel`, and `escalating_retry` to the existing M2/M3 strategy implementations.
- Dispatch aggregators `first`, `concat`, `vote`, `any_fail`, and `all_pass`, bridging existing aggregator modules and simple verification folds where needed.
- Dispatch verify hooks `run_command` and `llm_verifier` through existing M4 hook implementations/test doubles.
- On success, write exactly one canonical artefact, append/register exactly one manifest entry with `producer` set to the strategy, and write `<phase>.completed`.
- On every attempt, write durable markers and use `attempts/<phase>/<n>/` for failed retry debris so the live attempt can be recovered from disk alone.
- Surface typed failure variants for phase failure, verify failure, and strategy failure.
- No CLI/run-level orchestration, tracing, or cost accounting in this slice.

## Acceptance tests

- Single + first + no verify emits one artefact, one manifest entry, and `design.completed`.
- Parallel + concat + any_fail with a passing command verifier merges three replicas, captures verify exit, and writes `review.completed`.
- Escalating retry + all_pass with LLM verifier failing twice then passing creates attempts `0..2`, fails attempts 0/1, and completes attempt 2.
- Terminal verify failure writes a failed marker with reason and propagates `PhaseFailed`.
- All tests use mocked backend/verifier clients; `make check` remains clean without live network.
