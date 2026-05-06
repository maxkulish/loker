# Plan: CLO-311 [T-042] `loker explain <workflow>` - DAG and per-phase strategy summary

## Context
- Design: docs/designs/clo-311-loker-explain-workflow.md
- Discovery: docs/discovery/clo-311.md
- PRD: docs/prds/clo-311-loker-explain-workflow.md
- Linear: https://linear.app/cloud-ai/issue/CLO-311/t-042-loker-explain-workflow-dag-and-per-phase-strategy-summary

## Sub-tasks

### ST1 Add workflow lookup from an explicit base directory
**Files:** `src/workflow/mod.rs`
**Acceptance:** `cargo test workflow::tests::find_workflow_in_resolves_project_local_workflow --lib` passes
**Estimate:** M

Add `workflow::find_workflow_in(name, dir)` with the same precedence as `find_workflow`: direct path, `<dir>/.lok/workflows`, global workflows, then embedded workflows. Direct relative paths must resolve against `dir`; existing `find_workflow(name)` should remain available and can delegate to `find_workflow_in(name, Path::new("."))` if that preserves behavior.

### ST2 Add the workflow explanation model, analyzer, renderer, and unit tests
**Files:** `src/workflow/mod.rs`, `src/workflow/explain.rs`, `src/workflow/grammar.rs`
**Acceptance:** `cargo test workflow::explain --lib` passes
**Estimate:** M

Create `workflow::explain` with `WorkflowExplanation`, `PhaseExplanation`, `StrategyExplanation`, `VerifyExplanation`, `explain_workflow_source`, `explain_workflow`, and `render_text`. Parse raw source text as `workflow::grammar::Workflow`, run validation/linting, derive DAG dependencies from `phase:<name>` inputs, preserve non-phase inputs, and render stable plain text for metadata, phase order, strategies, backends, prompt templates, outputs, and verify status.

### ST3 Wire unified top-level CLI behavior with workflow-first detection
**Files:** `src/main.rs`, `src/workflow/explain.rs`, `src/workflow/mod.rs`
**Acceptance:** `cargo run --bin loker -- explain design-doc-tdd` exits 0 and prints `Workflow: design-doc-tdd`
**Estimate:** M

Update `Commands::Explain` to accept an optional target plus `--dir`, `--backend`, and `--focus`. Add `run_explain_unified` that first tries `workflow::find_workflow_in(target, dir)` when a target is provided, renders workflow output if found, and otherwise falls back to existing codebase `run_explain`. Preserve `loker explain` and `loker explain .` codebase behavior.

### ST4 Add invalid workflow fixtures and CLI integration coverage
**Files:** `tests/explain_cli.rs`, `tests/fixtures/workflows/explain-missing-ref.toml`, `tests/fixtures/workflows/explain-forward-ref.toml`, `tests/fixtures/workflows/design-doc-tdd.toml`
**Acceptance:** `cargo test --test explain_cli` passes
**Estimate:** M

Add command-level tests for `loker explain design-doc-tdd`, explicit workflow path explanation, missing phase references, forward references, and directory fallback to codebase mode. Use stable assertions or inline snapshots for deterministic workflow stdout and narrow assertions for codebase fallback.

### ST5 Run formatting and full pre-merge validation
**Files:** `src/main.rs`, `src/workflow/mod.rs`, `src/workflow/explain.rs`, `tests/explain_cli.rs`, `tests/fixtures/workflows/*.toml`
**Acceptance:** `make check` passes
**Estimate:** S

Run `cargo fmt`, targeted test commands from ST1-ST4, and the repository pre-merge gate. Address any clippy, formatting, or snapshot issues without changing the approved CLI semantics.

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Risks
- `WorkflowSource` may not expose enough raw source data for phase-grammar parsing; mitigate by adding a small internal helper in `workflow::explain` rather than reusing legacy step workflow loading.
- Existing codebase `loker explain` behavior must remain backward compatible; mitigate with a dedicated fallback integration test.
- Validation error wording from `workflow::grammar` may differ from the design examples; tests should assert stable, meaningful substrings rather than over-constraining internal phrasing for invalid workflows.
- Snapshot output can become brittle if paths are absolute; renderer should prefer `WorkflowSource::display_name()` or normalized repo-relative paths where available.
