# PRD: CLO-318 — Severity ladder for HumanVerifier gates

## Problem

Workflow authors can now insert a `HumanVerifier` gate, but the gate has only scaffold-level severity metadata and always behaves like an indefinite pending failure when no response file exists. Operators need low-, medium-, and high-severity gates to communicate urgency and to apply predictable timeout behavior without custom wrapper code.

## Goals

- Allow a spec/configured HumanVerifier gate to declare severity: `low`, `medium`, or `high`.
- Map severities to default timeout/escalation policies: low auto-approve after 1 hour, medium auto-fail after 24 hours, high block indefinitely.
- Keep timeout duration and escalation behavior configurable from the workflow/spec plumbing.
- Surface severity in pending payloads, trace events, and phase status markers so downstream tooling can prioritize HITL work.
- Cover each severity path with deterministic unit/integration tests, using a fake clock for timeout behavior.

## Non-goals

- UI rendering of severity.
- Notification, paging, or external escalation integrations.
- Per-phase advisory response locking.

## Acceptance criteria

1. A workflow/spec can declare severity per HumanVerifier gate, and the runner honors the configured timeout/escalation behavior.
2. Defaults are documented in the workflow/spec reference.
3. Trace events include severity for HITL downstream tooling.
4. Status markers include enough severity/timeout detail for run inspection.
5. Tests cover low timeout auto-approve, medium timeout auto-fail, high indefinite block, and explicit responses before timeout.

## Dependencies

- T-013 timeout/budget infrastructure.
- T-048 HumanVerifier hook scaffold.
