# PRD: CLO-311 `loker explain <workflow>`

## Requirement

`loker explain <workflow>` prints a static, human-readable analysis of a phase-based workflow without executing any backend calls.

## Users and need

Workflow authors and maintainers need a quick way to inspect the phase graph before running a workflow. Today they must read TOML manually or discover dependency/configuration mistakes only when `loker run` or lower-level validators fail.

## Functional requirements

- Accept a workflow name or path using the same lookup semantics as `loker run`.
- Print phases in execution order.
- For each phase, show dependencies, strategy, backends, verify hooks/contracts if declared, and output path.
- Validate missing references, forward/cyclic phase references, duplicate phases, malformed backends, and strategy constraints before rendering.
- Return a clear non-zero error for invalid workflows.
- Keep output stable enough for snapshot testing.

## Non-goals

- No live run history or status lookup.
- No backend calls.
- No graphviz, Mermaid, or other diagram export in v0.

## Acceptance criteria

- `loker explain design-doc-tdd` renders a stable, readable summary.
- Snapshot coverage is pinned with `insta` or equivalent.
- Workflows with cycles or missing references produce clear errors.
