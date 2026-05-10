# Plan: CLO-327 — Wire Phase-Based Runner into `loker run`

## Context
- **Design:** `docs/designs/clo-327-wire-phase-based-runner.md`
- **Discovery:** `docs/discovery/clo-327.md`
- **PRD:** `docs/prds/clo-327-wire-phase-based-runner.md`
- **Linear:** https://linear.app/cloud-ai/issue/CLO-327/wire-phase-based-runner-into-loker-run
- **Approach:** Detect-and-dispatch inside `run_workflow()` (Approach A)
- **Branch:** `feat/clo-327-wire-phase-based-runner`

## Sub-tasks

### ST1 Phase detection — make source text available for dual-parsing

**Goal:** Modify `run_workflow()` (`src/main.rs:1413`) so that after loading a workflow, the raw source text is available, enabling a first-attempt parse via `grammar::Workflow::from_str()` before falling back to the existing step-based TOML parse.

**Design detail:**
Currently `run_workflow()` calls `workflow::load_workflow_from_source()` which returns a parsed `Workflow` struct. For ST1, we add a complementary function `read_workflow_source_text()` that returns the raw `String` content, or modify the existing function to also return text. The `run_workflow()` function then does:

```
text → try grammar::Workflow::from_str() → if success, dispatch to phase path
                                       ↓ if fail (parse error or no [[phases]])
                                       → fall through to existing toml::from_str::<Workflow>()
```

No actual phase execution is wired yet — just the detection gate.

**Files:**
- `src/main.rs` (modify `run_workflow`, ~20 lines)
- `src/workflow/mod.rs` (add `read_workflow_source_text`, ~10 lines)

**Acceptance:**
```
cargo test phase_detection_detects_phases_only
cargo test phase_detection_detects_steps_only
cargo test phase_detection_no_phases_no_steps
```

**Estimate:** S

---

### ST2 PhaseConfig builder — convert `grammar::Phase` → `PhaseConfig`

**Goal:** Implement `build_phase_config()` that converts a parsed `grammar::Phase` struct into the concrete types needed by `PhaseRunner::run()`: `PhaseConfig` + `PhaseInputs`.

**Design detail:**
- Map `grammar::Phase.strategy` → `phase_runner::StrategyName` (Single, ParallelFanOut, EscalatingRetry)
- Resolve `grammar::Phase.backends` strings → `Arc<dyn Backend>` via `create_backend()`
- Render `grammar::Phase.prompt_template` through the existing `Template::render()` from `src/workflow/template.rs` using a `TemplateContext` built from spec content, prior phase outputs, and `--var` values
- Map `grammar::Phase.output` → `PhaseConfig.artefact_name`
- Ignore `grammar::Phase.contract` (lint-only warning for now)

**Files:**
- `src/workflow/mod.rs` or new `src/workflow/phase_runner.rs` (add `build_phase_config`, ~80 lines)

**Acceptance:**
```
cargo test build_phase_config_single_strategy
cargo test build_phase_config_parallel_strategy
cargo test build_phase_config_escalating_strategy
cargo test build_phase_config_resolves_backends
cargo test build_phase_config_renders_template
```

**Estimate:** S

---

### ST3 Phase workflow runner — implement `run_phase_workflow()`

**Goal:** Implement the core `run_phase_workflow()` function that walks phases sequentially, rendering prompt templates, calling `PhaseRunner::run()` for each phase, persisting outputs, and chaining outputs to downstream phases via `{{ phase.<name>.output }}`.

**Design detail:**
```rust
pub async fn run_phase_workflow(
    config: &Arc<config::Config>,
    cwd: &Path,
    workflow: &grammar::Workflow,
    spec_content: Option<String>,
    template_vars: HashMap<String, String>,
    rerun_phases: &[String],
    run_dir: &RunDir,
) -> Result<Vec<PathBuf>>
```

Execution loop:
1. Create `RunDir` at `runs/<wf>-<ts>-<id>/`
2. For each phase in `workflow.phases`:
   a. Build `TemplateContext` with spec, prior outputs, vars
   b. Render prompt template → prompt string
   c. Build `PhaseConfig` via ST2 builder
   d. Resolve backends via `create_backend()`
   e. Call `PhaseRunner::run(&cfg, inputs, 0)`
   f. Read output artifact → store in phase_outputs map for next phase
   g. Append manifest entry
3. Return paths to all generated artifacts

On any phase failure, stop (fail-fast) and return error.

**Files:**
- `src/workflow/mod.rs` or new `src/workflow/phase_runner.rs` (add `run_phase_workflow`, ~100 lines)

**Acceptance:**
```
cargo test run_phase_workflow_three_phases
cargo test run_phase_workflow_phase_output_chaining
cargo test run_phase_workflow_fail_fast
```

**Estimate:** M

---

### ST4 Wire into `run_workflow()` dispatch

**Goal:** Connect the detection gate (ST1) with the phase runner (ST3) so that `loker run <phase-workflow>` actually produces output.

**Design detail:**
In `run_workflow()` after `grammar::Workflow::from_str()` succeeds:
- Create `RunDir`
- Call `run_phase_workflow()`
- Print run summary with phase names, status, artifact paths
- Skip all step-based validation and runner instantiation

Also accept existing `--spec`/`--var`/`--rerun` flags transparently.

**Files:**
- `src/main.rs` (modify `run_workflow` in the phase-based branch, ~30 lines)

**Acceptance:**
```
cargo test run_workflow_dispatch_phase_based
cargo test run_workflow_dispatch_step_based
cargo test run_workflow_dispatch_no_phases
```

**Estimate:** S

---

### ST5 Integration test — end-to-end with mock backends

**Goal:** Write integration tests that validate the full `loker run` path with a phase-based workflow file using mock backends.

**Design detail:**
Follow the pattern from `tests/phase_runner_integration.rs` which constructs `PhaseConfig` directly. For integration tests:
- Create a `.lok/workflows/test-phase.toml` with `[[phases]]`
- Configure mock backends (no actual LLM calls)
- Run `loker run test-phase --spec <spec_file>` (or invoke the internal path)
- Verify manifest has entries, artifacts exist in correct locations, output chaining works

Also add a regression test that step-based workflows still work.

**Files:**
- `tests/runner_phase_integration.rs` (new, ~150 lines)

**Acceptance:**
```
cargo test loker_run_phase_workflow_emits_artifacts
cargo test loker_run_phase_workflow_emits_correct_manifest_entries
cargo test loker_run_step_workflow_unchanged
cargo test loker_run_phase_with_input_chaining
```

**Estimate:** S

---

## Pre-merge gate
- `make check` (fmt + clippy + test)
- All 15+ new unit/integration tests pass
- Existing step-based test suite remains green
- Manual verification with `~/Code/mentis/.loker/workflows/task-kickoff.toml`

## Risks
1. **`grammar::Workflow` parse may overlap with step-based TOML** — A file with valid `[[phases]]` *and* valid `[[steps]]` sections would be handled by the phase path, silently ignoring `[[steps]]`. This is consistent with the design intent (phase-based grammar is the primary path), but we should document this behavior.
2. **Backend resolution may fail late** — Backends are resolved in ST2 (per-phase config building), which happens before `PhaseRunner::run()` executes the phase. If a backend reference is invalid, the error surfaces at config-build time rather than at execution time. This is safer than the step-based runner (which fails at runtime), but different behavior should be documented.
3. **Existing `TemplateEngine` API compatibility** — The current `src/workflow/template.rs` may need minor adjustments (e.g., making `Template::render()` public) or its constructor/context API may not perfectly match the phase runner's needs. Estimate includes buffer for these tweaks.
4. **Run directory collides with step-based naming** — Both phase and step runs use `runs/<wf>-<ts>-<id>/`. If the same workflow name is used for both phase and step runs, they'll share a namespace. This is fine since the run-id includes a hash, but should be noted.
