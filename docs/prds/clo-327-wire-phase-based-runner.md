# PRD: Wire Phase-Based Runner into `loker run`

## Problem

`loker run <workflow>` silently accepts phase-based workflow files (`[[phases]]` grammar)
but produces zero output and an empty manifest because the CLI only dispatches to the
step-based runner, which has no `phases` field and silently drops the `[[phases]]` blocks.

## Users Affected

Anyone writing phase-based `.loker/workflows/*.toml` files and running them with
`loker run`. Currently the only known workflow is the mentis `task-kickoff.toml`,
but this is the intended primary workflow authoring format going forward.

## Current Behaviour

- `loker run task-kickoff --spec docs/specs/MENTI-68.md` → produces `runs/<id>/manifest.json`
  with `"entries": []` and exits cleanly with no error.
- The step-based `Workflow` struct (deserialised from TOML) has no `phases` field, so
  `[[phases]]` blocks are silently dropped.
- The phase-based grammar parser (`grammar::Workflow`) and executor (`PhaseRunner`)
  both exist with full test coverage but are never connected to `loker run`.

## Desired Behaviour

- `loker run task-kickoff --spec docs/specs/MENTI-68.md` should detect the `[[phases]]`
  grammar, parse via `grammar::Workflow`, walk phases sequentially, and produce artefacts
  (design.md, review.md, plan.md) under the run directory with a non-empty `manifest.json`.
- Step-based workflows must continue to work unchanged.

## Scope

1. Detect phase-based workflow files (peek for `[[phases]]` or try phase-grammar parse
   before step-grammar parse).
2. Parse via `grammar::Workflow::from_str()` and run `validate()`.
3. Walk phases sequentially, building `PhaseConfig` + `PhaseInputs` per phase.
4. Resolve backends from `<backend>/<model>` references via the existing backend registry.
5. Render prompt templates with `{{ spec }}`, `{{ phase.NAME.output }}`, `{{ var.X }}` substitutions.
6. Persist artefacts to `runs/<wf>-<ts>-<id>/attempts/<phase>/<n>/<output>` and append manifest entries.
7. Honour `--resume` for phase-based workflows.

## Acceptance Criteria

1. `loker run task-kickoff --spec docs/specs/MENTI-68.md` from `~/Code/mentis/` produces
   design.md, review.md, plan.md under the run dir, and a non-empty `manifest.json`.
2. `make check` passes.
3. New integration test in `tests/` covers a phase-based workflow end-to-end through
   `loker run` (with mock backends, like `tests/phase_runner_integration.rs`).
4. Step-based workflows continue to work unchanged.
