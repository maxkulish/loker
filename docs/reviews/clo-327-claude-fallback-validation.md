# Pre-PR validation: clo-327

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-10
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [HIGH] Template render errors are silently swallowed, breaking strict-mode contract
**Where:** `src/workflow/phase_bridge.rs:115-116`
**What:** `Template::render(&phase.prompt_template, &tmpl_ctx).unwrap_or_else(|_| phase.prompt_template.clone())` falls back to the unrendered template on any error. The design specifies strict-mode rendering where unknown variables (e.g., a misspelled `{{ phase.foo.output }}` or `{{ var.missing }}`) must fail loudly. Today a typo silently ships the raw `{{ ... }}` string to the LLM, producing useless output and zero feedback to the user.
**Suggested fix:** Propagate the error: `let rendered_prompt = Template::render(&phase.prompt_template, &tmpl_ctx).with_context(|| format!("template render failed for phase '{}'", phase.name))?;` and bubble the `Result` through `build_phase_config`.

### F2 [HIGH] ST5 integration tests not implemented — pre-merge gate not met
**Where:** `tests/` (no `runner_phase_integration.rs`)
**What:** The plan's pre-merge gate requires four ST5 tests: `loker_run_phase_workflow_emits_artifacts`, `loker_run_phase_workflow_emits_correct_manifest_entries`, `loker_run_step_workflow_unchanged`, `loker_run_phase_with_input_chaining`. None exist. Unit tests in `phase_bridge.rs` exercise the builder but not the full `loker run` path, so manifest layout, artifact paths, output chaining and step-workflow regression are untested end-to-end.
**Suggested fix:** Add `tests/runner_phase_integration.rs` per ST5 with mock backends; do not merge until those four cases pass plus `make check`.

### F3 [MEDIUM] Manifest layout diverges from design — each phase writes its own manifest
**Where:** `src/workflow/phase_bridge.rs` (phase loop) + `src/phase_runner/persist.rs`
**What:** `run_phase_workflow` passes `phase_dir = run_dir/attempts/<phase>` as `inputs.run_dir`. `persist::commit_success` then writes `manifest.json` and the artefact directly under that phase dir. The design specifies a single workflow-level `runs/<wf>-.../manifest.json` and per-attempt artefacts at `attempts/<phase>/<n>/<output>`. Current layout yields N manifests, one per phase, and no `<n>/` subdir — making it harder to aggregate the run and breaking the documented contract consumers may rely on.
**Suggested fix:** Either (a) keep `inputs.run_dir = run_dir` (workflow root) and have persist namespace into `attempts/<phase>/<n>/`, or (b) keep per-phase dirs but emit and merge a top-level workflow manifest after each phase. Document whichever you pick in the design.

### F4 [MEDIUM] Prompt template is rendered twice (strict workflow render, then strategy MiniJinja render)
**Where:** `src/workflow/phase_bridge.rs:115` → `src/strategy/single_model.rs` (`ctx.template_engine.render(&self.prompt_template, ...)`)
**What:** `phase_bridge` renders the prompt with the workflow `Template` engine, then stores the rendered string in `PhaseConfig.prompt_template`. The strategy layer then runs it through MiniJinja again. Any literal `{{`/`}}` surviving the first pass (e.g., user examples in prompts) or any variable named the same in both engines will produce inconsistent behavior. It also makes the source of a render failure ambiguous.
**Suggested fix:** Choose one render site. Simplest: stop pre-rendering in phase_bridge and pass the raw template plus the variable bag through to the strategy; or, mark the strategy template engine as a no-op when the phase came from grammar workflows.

### F5 [LOW] `_rerun_phases` is accepted but unused while error/help text references `--rerun`
**Where:** `src/workflow/phase_bridge.rs` (function signature) and `src/main.rs` (dispatch site)
**What:** The plan calls out `--rerun` as a flag accepted transparently. The runner takes `_rerun_phases: &[String]` and discards it, yet user-facing strings still reference the flag. Either the feature is in or it isn't; silent ignore is the worst of both worlds.
**Suggested fix:** Document explicitly that `--rerun` is a no-op in phase mode (warn at startup if non-empty), or implement it by skipping phases whose name is not in the rerun list and reusing the prior artifact for chaining.

### F6 [LOW] `artefact_kind` inferred from filename suffix
**Where:** `src/workflow/phase_bridge.rs` (kind selection)
**What:** Mapping `.md` → Markdown, `.json` → Json, else opaque-bytes is fragile (`.MD`, `.yaml`, `.txt`). A wrong kind affects downstream parsing/aggregation.
**Suggested fix:** Add an explicit `kind` field to `grammar::Phase` (default Markdown), or lowercase + extend the suffix table and reject unknown extensions with a clear error.

### F7 [LOW] `make check` not verified in this review
**Where:** repo root
**What:** I did not run `make check`. Given F1 and F3 likely require signature changes, fmt/clippy/test status is unknown.
**Suggested fix:** Run `make check` before pushing; address any new clippy lints introduced by `phase_bridge.rs`.

## Verdict
**rework**

The phase-detection dispatch and `PhaseRunner` wiring are structurally sound and the unit tests in `phase_bridge.rs` cover the strategy mapping. But two blockers prevent merge: (F1) the silent template-render fallback directly contradicts the design's strict-mode requirement and will mask user errors in production prompts, and (F2) ST5 — the integration test suite the plan names as part of the pre-merge gate — is entirely absent, so there is no end-to-end verification that artifacts, manifests, chaining, or step-workflow regression actually work. F3 and F4 are real but smaller divergences from the design that should be reconciled either by fixing the code or amending the design before this lands.
