# Design: CLO-311 - `loker explain <workflow>` DAG and per-phase strategy summary

## 1. Problem

Discovery for CLO-311 found that workflow authors and maintainers currently must inspect phase-based TOML by hand to understand a workflow's DAG, strategy selection, backend use, verify hooks, and output artefacts before executing it. The phase grammar, lookup path, validators, canonical `design-doc-tdd` workflow, and snapshot tooling already exist, but the CLI does not expose a static workflow explanation command. FR-34 now requires `loker explain <workflow>` as part of the Phase 9 CLI surface so users can catch configuration mistakes and reason about execution without backend calls or run history.

## 2. Goals / Non-goals

### Goals

- Add top-level `loker explain <workflow>` behavior while preserving existing codebase explanation behavior.
- Reuse the same workflow lookup semantics as `loker run`: explicit path, project-local `.lok/workflows`, global workflows, then embedded workflows.
- Parse phase-based TOML workflows with `workflow::grammar::Workflow` and report validation errors before rendering summaries.
- Return a clear unsupported-workflow error if the target resolves to legacy step-only TOML rather than phase-based `[[phases]]` TOML.
- Render a stable, readable text summary containing workflow metadata, phase order, dependencies, strategy, backends, verify status, and declared output paths.
- Add snapshot/integration coverage for `design-doc-tdd` and invalid workflow references.

### Non-goals

- No backend calls and no execution of workflow phases.
- No run-history, resume-state, trace, or manifest lookup.
- No graphviz, Mermaid, JSON, or machine-readable export in v0.
- No new workflow grammar fields for verify hooks in this task; current grammar renders `verify: none` unless parsed contract data becomes available later.
- No support for legacy step-based workflow summaries in this task; those can be added later if product demand exists.
- No rename of `.lok/workflows` or `lok.toml` paths in this task.

## 3. Architecture

### Module layout

Implement the workflow explanation as a small formatter module behind the CLI:

```text
src/
  main.rs                    # CLI shape and dispatch
  workflow/
    mod.rs                   # existing WorkflowSource/find_workflow/load helpers
    grammar.rs               # existing phase grammar and validation
    explain.rs               # new summary model + text renderer
  lib.rs                     # expose workflow::explain with existing workflow module

tests/
  explain_cli.rs             # command-level snapshots and error tests
  fixtures/workflows/
    explain-missing-ref.toml # invalid workflow fixture
    explain-forward-ref.toml # cycle/forward-ref fixture
```

`workflow::explain` should depend on `workflow::grammar`, not on `WorkflowRunner`, `PhaseRunner`, or backend construction. This keeps the command static and ensures no backend clients are initialized.

### CLI data flow

```text
loker explain <target> [--dir <path>] [--backend ...] [--focus ...]
  |
  |-- Try workflow::find_workflow(target) from the selected working directory.
  |     |
  |     |-- Found workflow source
  |     |     |-- Read source as TOML text.
  |     |     |-- Parse as workflow::grammar::Workflow.
  |     |     |-- Run grammar.validate() and grammar.lint().
  |     |     |-- If validation errors exist: print clear error and exit non-zero.
  |     |     |-- Build WorkflowExplanation.
  |     |     `-- Print render_workflow_explanation().
  |     |
  |     `-- Not found
  |           `-- Fall back to existing codebase run_explain() behavior.
  |
  `-- Exit 0 only if workflow explanation or codebase explanation succeeds.
```

The chosen discovery approach is workflow-first detection. A target that resolves as a workflow is always treated as a workflow, even if a same-named directory exists. If no workflow resolves, the command preserves current codebase explanation semantics.

### Working directory handling

`workflow::find_workflow()` currently searches relative to process cwd. The CLI must preserve `--dir` for existing codebase explanation while also making workflow lookup deterministic. Add `workflow::find_workflow_in(name, dir)` and use it from `run_explain_unified`; do not use process-wide `std::env::set_current_dir` in async code.

`find_workflow_in` should follow the same precedence as `find_workflow`: explicit path, `<dir>/.lok/workflows`, global config, then embedded workflows. Direct relative paths should resolve against `dir`.

### Explanation model

```rust
pub struct WorkflowExplanation {
    pub name: String,
    pub description: Option<String>,
    pub source: String,
    pub phases: Vec<PhaseExplanation>,
    pub warnings: Vec<String>,
}

pub struct PhaseExplanation {
    pub index: usize,
    pub name: String,
    pub dependencies: Vec<String>,
    pub strategy: StrategyExplanation,
    pub backends: Vec<String>,
    pub prompt_template: String,
    pub output: String,
    pub verify: VerifyExplanation,
}

pub enum StrategyExplanation {
    Single,
    Parallel { min_responses: usize },
    Escalating { pass_failure_context: bool },
}

pub enum VerifyExplanation {
    None,
    ContractReserved,
}
```

Dependencies are derived from `phase.inputs` entries of the form `phase:<name>`. Other inputs such as `spec` and `var:<name>` should be displayed as inputs, but they are not DAG dependencies. If the renderer only has room for one field, include both by rendering `depends_on` for phase references and `inputs` for all raw inputs.

### Text output

Use stable, plain text without colors by default so snapshots are deterministic. Example shape:

```text
Workflow: design-doc-tdd
Source: .lok/workflows/design-doc-tdd.toml
Description: Four-phase design → review → implement → verify pipeline.

Phase order:
  1. design
  2. review (depends on: design)
  3. implement (depends on: design, review)
  4. verify (depends on: implement, design)

Phases:
  design
    strategy: single
    backends: ollama/qwen3-coder-next
    inputs: spec
    prompt_template: ../prompts/design-doc-tdd/design.md.tmpl
    output: design.md
    verify: none

  review
    strategy: parallel (min_responses: 2)
    backends: claude/, gemini/, codex/, ollama/qwen3-coder-next
    inputs: phase:design
    prompt_template: ../prompts/design-doc-tdd/review.md.tmpl
    output: review.md
    verify: contract reserved
```

Validation errors should be joined with one error per line:

```text
Invalid workflow: tests/fixtures/workflows/explain-missing-ref.toml
  - Phase 'review' references unknown phase 'design'
```

## 4. Public API surface

Add a new private/public-in-crate module under `workflow` and keep CLI plumbing in `main.rs`.

```rust
// src/workflow/mod.rs
pub mod explain;
```

```rust
// src/workflow/explain.rs
use anyhow::Result;
use std::path::Path;

use crate::workflow::WorkflowSource;
use crate::workflow::grammar;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowExplanation {
    pub name: String,
    pub description: Option<String>,
    pub source: String,
    pub phases: Vec<PhaseExplanation>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseExplanation {
    pub index: usize,
    pub name: String,
    pub dependencies: Vec<String>,
    pub inputs: Vec<String>,
    pub strategy: StrategyExplanation,
    pub backends: Vec<String>,
    pub prompt_template: String,
    pub output: String,
    pub verify: VerifyExplanation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrategyExplanation {
    Single,
    Parallel { min_responses: usize },
    Escalating { pass_failure_context: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyExplanation {
    None,
    ContractReserved,
}

pub async fn explain_workflow_source(source: WorkflowSource) -> Result<WorkflowExplanation>;

pub async fn find_workflow_in(name: &str, dir: &Path) -> Result<WorkflowSource>;

pub fn explain_workflow(
    workflow: &grammar::Workflow,
    source: impl Into<String>,
) -> Result<WorkflowExplanation>;

pub fn render_text(explanation: &WorkflowExplanation) -> String;
```

`explain_workflow_source` must read raw TOML text from `WorkflowSource` and parse it with `workflow::grammar::Workflow`; it must not call `load_workflow_from_source()`, which returns the legacy step-based `workflow::Workflow`. Add an internal helper:

```rust
async fn read_source_text(source: WorkflowSource) -> Result<(String, String)>;
```

If parsing as phase grammar fails because the source is legacy step-only TOML or lacks `[[phases]]`, return a clear error such as `workflow explanation supports phase-based workflows only`.

CLI updates in `src/main.rs`:

```rust
Commands::Explain {
    target,
    dir,
    backend,
    focus,
} => {
    run_explain_unified(
        target.as_deref(),
        &dir,
        backend.as_deref(),
        focus.as_deref(),
        &config,
        cli.verbose,
    ).await?;
}

async fn run_explain_unified(
    target: Option<&str>,
    dir: &Path,
    backend: Option<&str>,
    focus: Option<&str>,
    config: &config::Config,
    verbose: bool,
) -> Result<()>;
```

Clap shape:

```rust
Explain {
    /// Workflow name/path to explain, or directory for existing codebase explain behavior.
    target: Option<String>,

    /// Directory used for workflow lookup and codebase explanation.
    #[arg(short, long, default_value = ".")]
    dir: PathBuf,

    #[arg(short, long)]
    backend: Option<String>,

    #[arg(short, long)]
    focus: Option<String>,
}
```

`run_explain_unified` should use `target.unwrap_or(".")` for the codebase fallback so existing `loker explain` with no argument still explains the current directory.

## 5. Test plan

### Unit tests

Add tests in `src/workflow/explain.rs`:

- `explain_builds_phase_dependencies_from_phase_inputs`: builds a `grammar::Workflow` with `phase:design` inputs and asserts dependencies are extracted.
- `explain_preserves_non_phase_inputs`: verifies `spec` and `var:name` remain in `inputs` but are not dependencies.
- `render_text_is_stable_for_design_doc_tdd_shape`: parses fixture TOML and snapshots rendered text, or checks key sections if the command-level snapshot is preferred.
- `render_strategy_variants`: covers `single`, `parallel`, and `escalating` formatting.
- `render_verify_contract_reserved`: verifies phases with `contract` render `contract reserved`; phases without it render `none`.

### Integration tests

Add `tests/explain_cli.rs` using the existing `cargo run --quiet --bin loker -- ...` pattern from `tests/run_cli.rs`:

- `test_explain_design_doc_tdd_snapshot`: runs `loker explain design-doc-tdd` from repo root and snapshots stdout with `insta::assert_snapshot!`.
- `test_explain_workflow_path_snapshot`: runs `loker explain tests/fixtures/workflows/design-doc-tdd.toml` and snapshots or asserts the same phase details.
- `test_explain_missing_reference_errors`: runs invalid fixture with `phase:missing` and asserts non-zero plus `references unknown phase`.
- `test_explain_forward_reference_errors`: runs invalid fixture where a phase references a later phase and asserts non-zero plus `references phase ... declared later`.
- `test_explain_falls_back_to_codebase_mode_for_directory`: use a narrow assertion that `loker explain . --focus workflow` does not print workflow-summary headers or validation errors; avoid snapshotting full codebase-explain output because it can vary by backend and repository state.

Use `insta` inline snapshots for stable stdout. Do not rely on colors; ensure workflow rendering uses plain strings.

### Manual verification

```bash
cargo run --bin loker -- explain design-doc-tdd
cargo run --bin loker -- explain tests/fixtures/workflows/design-doc-tdd.toml
cargo run --bin loker -- explain tests/fixtures/workflows/explain-missing-ref.toml
cargo test --test explain_cli
make check
```

Expected manual behavior:

- Valid workflow prints no run directory and initializes no backend clients.
- Invalid workflow exits non-zero with grammar validation errors.
- Existing codebase explanation still works with `loker explain .` and with no target.

## 6. Migration / rollout

No data migration is required. This is an additive CLI behavior on an existing command name. Rollout order:

1. Add `workflow::explain` model, renderer, and unit tests.
2. Add `run_explain_unified` and update `Commands::Explain` to accept an optional target.
3. Wire workflow-first detection while preserving codebase fallback.
4. Add invalid workflow fixtures and snapshot tests.
5. Run `cargo fmt`, targeted tests, and `make check`.

Backward compatibility notes:

- `loker explain` with no positional argument should keep explaining the current codebase.
- `loker explain .` should keep codebase behavior unless `.` is an explicit workflow TOML file path, which it is not.
- `--backend` and `--focus` remain codebase-explain options; they are ignored or rejected for workflow explanation only if needed. Prefer ignoring them with workflow output unchanged to avoid surprising users who have global aliases.
- The top-level command name remains `Explain`; no `workflow explain` alias is required for v0.

## 7. Open questions

None. The design resolves the lookup-root detail by requiring `workflow::find_workflow_in(name, dir)` so `--dir` behavior is deterministic without process-wide cwd changes.
