# Design: Wire Phase-Based Runner into `loker run`

**CLO-327** | **Design Document** | **v1 (draft)**

## Problem

`loker run <workflow>` has a single dispatch path that instantiates a step-based
`WorkflowRunner` and iterates `workflow.steps`. Phase-based workflow files written
with the `[[phases]]` grammar are silently accepted but produce zero output and an
empty manifest because the step-based `Workflow` struct has no `phases` field — the
`[[phases]]` blocks are dropped at deserialisation. The phase-based pieces
(`grammar::Workflow` parser + `PhaseRunner` executor) both exist with full test
coverage but are never glued to the `loker run` dispatch path. The gap was
identified in the discovery phase (see `docs/discovery/clo-327.md`, baseline score
4/10).

## Goals / Non-goals

### Goals
1. `loker run` detects phase-based workflows (peek for `[[phases]]`) and dispatches
   to the phase-based runner instead of the step-based runner.
2. Phase-based workflows are parsed via `grammar::Workflow::from_str()`, validated,
   and executed sequentially via `PhaseRunner::run()`.
3. Backend references (`<backend>/<model>` strings) are resolved to `Arc<dyn Backend>`
   via the existing `create_backend()` factory.
4. Prompt templates are rendered with `{{ spec }}`, `{{ phase.NAME.output }}`,
   `{{ var.X }}` substitutions.
5. Artifacts are persisted to `runs/<wf>-<ts>-<id>/attempts/<phase>/<n>/<output>`
   with manifest entries appended.
6. Step-based workflows continue to work unchanged.

### Non-goals
- Full `--resume` support for phase-based workflows (will be wired separately;
  the existing `ResumeRunner` in `src/resume.rs` can be adapted).
- Shell phases within phase-based workflows (phase-based grammar has no
  shell execution model — that's a future enhancement).
- Phase strategies beyond the three already implemented (Single, ParallelFanOut,
  EscalatingRetry).
- Template syntax beyond `{{ spec }}`, `{{ phase.NAME.output }}`, `{{ var.X }}`
  (no conditionals, loops, or filters in v1).

## Architecture

### Overview

The change is centred on a single decision point in `run_workflow()` (currently at
`src/main.rs:1413`). After loading the workflow source text, we attempt the phase
grammar parse first. If it succeeds, we build `PhaseConfig` values from the
`grammar::Workflow` AST and execute via `PhaseRunner`. If it fails (and the file
contains `[[steps]]` or no `[[phases]]`), we fall through to the existing step-based
path.

```
loker run <name> --spec <file>
│
├─ find_workflow(name) → source text
│
├─ try grammar::Workflow::from_str(&text)
│   ├─ Success → phase-based path:
│   │   ├─ validate()
│   │   ├─ for each phase in phases:
│   │   │   ├─ build_phase_config(phase, spec, vars, prev_outputs)
│   │   │   ├─ resolve_backends(phase.backends)
│   │   │   ├─ render_prompt_template(phase.prompt_template, spec, vars, prev_outputs)
│   │   │   ├─ PhaseRunner::run(&cfg, inputs, attempt)
│   │   │   ├─ persist output → attempt dir, append to manifest
│   │   │   └─ store output for next phase
│   │   └─ emit run summary → exit
│   │
│   └─ Failure (grammar errors) → step-based path:
│       ├─ toml::from_str::<Workflow>(&text)  [existing]
│       ├─ validate_with_capabilities()
│       ├─ WorkflowRunner::run()               [existing]
│       └─ exit
```

### New modules and types

```rust
// In src/workflow/mod.rs or a new src/workflow/phase_runner.rs:

/// Bridge between grammar::Workflow AST and phase_runner::PhaseConfig.
pub struct PhaseWorkflowRunner {
    config: Arc<config::Config>,
    cwd: PathBuf,
    spec_content: Option<String>,
    template_vars: HashMap<String, String>,
    rerun_phases: Vec<String>,
}

impl PhaseWorkflowRunner {
    pub fn new(
        config: Arc<config::Config>,
        cwd: PathBuf,
        spec_content: Option<String>,
        template_vars: HashMap<String, String>,
        rerun_phases: Vec<String>,
    ) -> Self;

    /// Run a grammar-based workflow, returning paths to the generated artifacts.
    pub async fn run(
        &self,
        workflow: &grammar::Workflow,
        run_dir: &RunDir,
    ) -> Result<Vec<PathBuf>>;
}
```

### Data flow (per phase)

```
grammar::Phase
├─ phase.name               → PhaseConfig.phase
├─ phase.strategy           → PhaseConfig.strategy (Strategy::Single → StrategyName::Single, etc.)
├─ phase.backends           → resolved via create_backend() → Vec<Arc<dyn Backend>>
├─ phase.prompt_template    → rendered via TemplateEngine → Prompt
├─ phase.inputs             → resolved to actual content (spec, prev phase outputs, vars)
├─ phase.output             → PhaseConfig.artefact_name
├─ phase.contract           → ignored (lint warning; reserved for post-v0)
│
PhaseInputs {
    backends: &[Arc<dyn Backend>],     // resolved from phase.backends
    prompt: Prompt,                      // rendered template
    ctx: PhaseContext,                   // phase name, run_id, cwd
    verify: None,                        // no verify hooks in v1
    run_dir: PathBuf,                    // run_dir/attempts/<phase>/
    trace: None,                          // no tracing in v1
}
```

### Template rendering

Reuse the existing `crate::workflow::template` module (built in CLO-289), which provides:

- `TemplateContext` with `spec: Option<String>`, `phase_outputs: HashMap<String, PhaseOutput>`,
  and `vars: HashMap<String, String>`
- `Template::render(&self, ctx: &TemplateContext)` for substitution of `{{ spec }}`,
  `{{ phase.<name>.output }}`, `{{ phase.<name>.output.path }}`, and `{{ var.<name> }}`
- Strict mode: every `{{ ... }}` must resolve or `TemplateError::UnresolvedPlaceholder` is raised

No new template engine is needed — the existing one is wired into `PhaseWorkflowRunner`
by building a `TemplateContext` per phase and calling `render()`.

### Run directory layout

```
runs/<wf>-<ts>-<id>/
├── manifest.json
├── attempts/
│   ├── design/
│   │   ├── 0/
│   │   │   └── design.md
│   │   └── markers/
│   │       ├── design.started.0
│   │       └── design.completed
│   ├── review/
│   │   ├── 0/
│   │   │   └── review.md
│   │   └── markers/
│   │       ├── review.started.0
│   │       └── review.completed
│   └── plan/
│       ├── 0/
│       │   └── plan.md
│       └── markers/
│           ├── plan.started.0
│           └── plan.completed
└── trace.jsonl          (if tracing is enabled)
```

Matches the existing `PhaseRunner` persistence pattern used by `ResumeRunner`.

## Public API surface

### New function: `run_phase_workflow` (internal, in `src/workflow/`)

```rust
/// Run a phase-based workflow, returning paths to generated artifacts.
///
/// This is the entry point for phase-based `loker run`. Callers must provide
/// the parsed grammar::Workflow, backend config, spec content, and template vars.
pub async fn run_phase_workflow(
    config: &Arc<config::Config>,
    cwd: &Path,
    workflow: &grammar::Workflow,
    spec_content: Option<String>,
    template_vars: HashMap<String, String>,
    rerun_phases: &[String],
    run_dir: &RunDir,
) -> Result<Vec<PathBuf>>;
```

### Modified function: `run_workflow` (in `src/main.rs`)

The existing `run_workflow` function gains phase detection logic. Its signature
does not change — callers are unaffected.

```rust
// Pseudocode for the modified logic:
async fn run_workflow(...) -> Result<()> {
    let source = workflow::find_workflow(name).await?;
    let (display_name, text) = read_source_text(&source).await?;

    // Attempt phase-based parse first
    if let Ok(grammar_wf) = text.parse::<grammar::Workflow>() {
        // Phase-based path
        let run_dir = RunDir::create(&cwd, name)?;
        let outputs = run_phase_workflow(
            config, &cwd, &grammar_wf, spec_content, template_vars,
            rerun_phases, &run_dir,
        ).await?;
        // Print summary
        return Ok(());
    }

    // Fall through to step-based path (existing code)
    let wf: Workflow = toml::from_str(&text)?;
    // ... existing validation + runner code ...
}
```

### Reused: `crate::workflow::template` (from CLO-289)

The existing template module at `src/workflow/template.rs` provides:

```rust
// Template context built per phase
use crate::workflow::template::{TemplateContext, PhaseOutput};

let mut ctx = TemplateContext::new()
    .with_spec(spec_content.clone())
    .with_phase_output("design", PhaseOutput {
        content: design_output.clone(),
        path: "attempts/design/0/design.md".into(),
    })
    .with_var("name", "value");

let rendered = template.render(&ctx)?;
```

No new template engine types needed.

### Builder: `grammar::Phase` → `PhaseConfig`

```rust
/// Build a PhaseConfig from the grammar AST, resolving backends and inputs.
pub fn build_phase_config(
    phase: &grammar::Phase,
    phase_index: usize,
    spec_content: Option<&str>,
    phase_outputs: &HashMap<String, String>,
    vars: &HashMap<String, String>,
    config: &config::Config,
) -> Result<(PhaseConfig, PhaseInputs)>;
```

## Test plan

### Unit tests (in `src/workflow/`)

| Test | What it covers |
|---|---|
| `phase_detection_detects_phases_only` | Text with `[[phases]]` → phase path |
| `phase_detection_detects_steps_only` | Text with `[[steps]]` → step path |
| `phase_detection_no_phases_no_steps` | Text with neither → step path (fallback) |
| `build_phase_config_single_strategy` | grammar::Phase with Single → correct PhaseConfig |
| `build_phase_config_parallel_strategy` | grammar::Phase with ParallelFanOut → correct PhaseConfig |
| `build_phase_config_escalating_strategy` | grammar::Phase with EscalatingRetry → correct PhaseConfig |
| `build_phase_config_resolves_backends` | Backend strings resolved via create_backend() |
| `build_phase_config_renders_template` | Prompt template rendered via existing TemplateEngine |
| `run_phase_workflow_three_phases` | Full workflow: design → review → plan with mock backends |
| `run_phase_workflow_phase_output_chaining` | `{{ phase.A.output }}` resolves to A's output in phase B |
| `run_phase_workflow_fail_fast` | Phase N failure prevents phase N+1 execution |
| `run_workflow_dispatch_phase_based` | `run_workflow()` dispatches to phase path for `[[phases]]` |
| `run_workflow_dispatch_step_based` | `run_workflow()` dispatches to step path for `[[steps]]` |
| `run_workflow_dispatch_no_phases` | `run_workflow()` dispatches to step path for no `[[phases]]` |

### Integration tests (in `tests/`)

| Test | What it covers |
|---|---|
| `loker_run_phase_workflow_emits_artifacts` | End-to-end `loker run` with phase-based workflow (mock backends) — validates manifest non-empty, artifacts exist |
| `loker_run_phase_workflow_emits_correct_manifest_entries` | Validates manifest `Kind`, `Producer`, `phase`, `attempt` fields |
| `loker_run_step_workflow_unchanged` | Step-based workflow still works after changes |
| `loker_run_phase_with_input_chaining` | Output of phase A becomes input to phase B |

### Manual testing

```bash
cd ~/Code/mentis
loker run task-kickoff --spec docs/specs/MENTI-68.md
# Verify:
#   - runs/task-kickoff-<ts>-<id>/manifest.json has "entries" length ≥ 3
#   - runs/task-kickoff-<ts>-<id>/attempts/design/0/design.md exists
#   - runs/task-kickoff-<ts>-<id>/attempts/review/0/review.md exists
#   - runs/task-kickoff-<ts>-<id>/attempts/plan/0/plan.md exists
```

## Migration / Rollout

### Implementation order

1. **Phase detection in `run_workflow()`** — modify `load_workflow_from_source` or
   the `run_workflow()` function to return raw text alongside the parsed AST.
   Attempt `grammar::Workflow::from_str()` before fallback.

2. **Template engine** — implement `TemplateEngine` with spec/phase/var substitution.
   Cover with unit tests.

3. **PhaseConfig builder** — implement `build_phase_config()` that converts
   `grammar::Phase` → `PhaseConfig` and resolves backends.

4. **Phase workflow runner** — implement `run_phase_workflow()` that walks phases
   sequentially, renders templates, calls `PhaseRunner::run()`, and persists outputs.

5. **Integration test** — end-to-end test with mock backends.

6. **Manual validation** — test with mentis `task-kickoff.toml`.

### Rollout

- The change is fully backward-compatible: step-based workflows are unaffected.
- No migration needed for existing run directories.
- Phase-based workflows go from "zero output" to "fully working" in one deployment.

## Open questions

1. **Resume support**: The PRD scope item 7 says "Honour `--resume` for phase-based
   workflows." The existing `ResumeRunner` at `src/resume.rs` already operates on
   `PhaseConfig` values and run directories — adapting it to phase-based workflows
   is a natural follow-on. However, `loker run` (fresh execution) and `loker resume`
   (stateful replay) are different entry points. For v1, `loker run` creates a new
   run directory and executes all phases fresh. Wiring `--resume` is deferred to a
   follow-up that connects `loker resume` to the phase-based workflow discovery path.
   **Decision: Defer to follow-up issue.** Update PRD scope item 7 to reflect this
   scoping.

2. **Template syntax**: Should we support `{{ phase.NAME.output }}` or just
   `{{ phase.NAME }}`? **Decision: `{{ phase.NAME.output }}`** — explicit is
   better, and the grammar already documents this form.

3. **Error handling**: If phase N fails, should subsequent phases be skipped?
   **Decision: Yes — fail-fast.** Unlike steps (which support `continue_on_error`),
   phases are sequential and later phases depend on earlier outputs.

5. **Backend resolution at config time or run time?** **Decision: At config building time**
   via `create_backend()` — this surfaces missing backends early and simplifies the
   per-phase execution loop. This differs from the step-based runner (which resolves
   backends inside the execution loop), but phases are fewer and backend resolution
   is a cheap operation.

5. **Template file resolution**: Should prompt templates be resolved relative
   to the workflow file or the working directory? **Decision: Relative to the
   workflow file's directory** (same as the existing step-based pattern).
