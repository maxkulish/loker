# PRD: CLO-313 - `loker ls --blocked`

## Background

Loker workflows can pause at HITL verification gates by writing `runs/<run_id>/pending/<phase>.json`. Operators currently need to know the exact run directory or inspect the filesystem manually to find runs waiting for a human decision.

## Requirement

Implement `loker ls --blocked` to enumerate HITL-pending runs from the project `runs/` tree. A blocked entry is any `runs/*/pending/<phase>.json` file that does not have a matching `runs/*/responses/<phase>.json` response.

## Users

- Developers and operators running local or CI-assisted Loker workflows.
- Human reviewers who need to find pending verification gates.

## Acceptance criteria

- Lists every run with at least one unmatched `pending/<phase>.json`.
- Prints run id, blocked phase, severity, age, and decision URL/path.
- Empty case prints `no blocked runs` and exits 0.
- Stable default sort is oldest blocked first.
- Snapshot test covers mixed blocked and completed runs.

## Non-goals

- Decision submission or mutation of pending/response files.
- UI rendering for blocked runs.
- Replacing the per-phase advisory lock implementation.
