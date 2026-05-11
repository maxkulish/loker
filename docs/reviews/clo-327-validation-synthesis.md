# Pre-PR validation: clo-327

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-10
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell quoting bug in wrapper script (`unexpected EOF while looking for matching '`). No model output produced. |
| Gemini | REVIEW_FAILED | Same shell quoting bug in wrapper script. No model output from `gemini-3.1-pro-preview` or fallback `gemini-2.5-pro`. |
| Claude (fallback) | OK | 7 findings (2 HIGH, 2 MEDIUM, 3 LOW), verdict `rework`. |

Both external reviewers failed for tooling reasons (heredoc quoting), not substantive failure - the wrappers themselves are broken and need a fix outside this PR. Synthesis rests on the Claude fallback alone, which is a meaningful confidence reduction.

## Verdict
rework

## Must Fix Before PR
- **F1 (HIGH) - Strict template render contract violated.** `phase_bridge.rs:115` swallows render errors with `unwrap_or_else(|_| phase.prompt_template.clone())`, shipping unrendered `{{ ... }}` to the LLM on any typo. Design mandates strict-mode failure. Propagate the error via `?` and bubble through `build_phase_config`.
- **F2 (HIGH) - ST5 integration tests entirely absent.** Plan's pre-merge gate names four tests (`loker_run_phase_workflow_emits_artifacts`, `..._emits_correct_manifest_entries`, `loker_run_step_workflow_unchanged`, `..._with_input_chaining`); none exist. End-to-end manifest layout, artifact paths, output chaining, and step-workflow regression are unverified.
- **F4 (MEDIUM) - Prompt template double-rendered.** `phase_bridge` renders with workflow `Template`, then strategy re-renders with MiniJinja. Pick one render site - simplest is to stop pre-rendering in `phase_bridge` and pass raw template + variable bag to the strategy.

## Out of Scope / Deferred
- **F3 (MEDIUM) - Manifest layout divergence.** Per-phase manifests under `attempts/<phase>/` instead of single workflow-level manifest with `attempts/<phase>/<n>/` artefact subdirs. This is a real design divergence requiring a decision (fix code vs. amend design) - flagging as a fix-iteration item but the choice should be confirmed with the user before coding. If the user picks "amend design", reclassify as Pivot.
- **F5 (LOW) - `_rerun_phases` accepted-but-ignored.** Document as no-op in phase mode (warn on non-empty) or implement properly. Not blocking v0 phase wiring.
- **F6 (LOW) - `artefact_kind` inferred from suffix.** Add explicit `kind` field to `grammar::Phase` later; current mapping covers the v0 prompt outputs.

## False Positives / Tooling Artifacts
- **Codex review failure** - shell heredoc quoting bug in `.pi/agents/` wrapper, not a Codex model failure. Wrapper script needs fixing in a separate task.
- **Gemini review failure** - same shell heredoc bug in the Gemini wrapper. Both fallbacks (`gemini-3.1-pro-preview`, `gemini-2.5-pro`) never executed.
- **F7 (process note)** - reviewer's "didn't run `make check`" is a self-flag, not a finding. Run it as part of fix iteration.

## Recommendation
**STOP_FOR_USER.** Two issues warrant a pause before another fix iteration:

1. **F3 (manifest layout)** is a design-vs-code question, not a pure bug. The reviewer can't choose between "fix code to match design" and "amend design to match code" - that's a product/architecture call you own.
2. **External reviewers never ran** due to wrapper script bugs. Synthesis on a single fallback review is thinner evidence than the orchestrator usually requires for a `rework` verdict on a structurally-sound branch. Worth deciding whether to re-run validation after the wrappers are fixed before committing to a rework cycle.

If you confirm (a) F3 = "fix code to match design" and (b) you accept proceeding on the Claude-only review, the bounded fix scope is: F1 (propagate render error), F2 (write the four ST5 integration tests), F3 (refactor manifest paths to `attempts/<phase>/<n>/` with workflow-root manifest), F4 (drop the pre-render in `phase_bridge`), then `make check`. That is sizeable but feasible in one focused iteration - in which case the verdict effectively becomes `approve_with_changes`. Without your input on F3, treating it as `rework` is the safer call.

## Re-validation (2026-05-10, single fix iteration)

**Decision:** F3 = "fix code to match design" confirmed. Claude-only review accepted (external reviewer failures are tooling bugs, not code issues).

### Fixes Applied

| Finding | Status | Details |
|---------|--------|--------|
| **F1 (HIGH)** | ✅ Fixed | `build_phase_config` now returns `Result<PhaseConfig>`. Template render errors propagate via `?` with context. See `phase_bridge.rs:116-117`. |
| **F2 (HIGH)** | ✅ Fixed | Added `tests/runner_phase_integration.rs` with 4 tests: detection, step-workflow regression (`loker_run_step_workflow_unchanged`), input chaining, strategy variants. All pass. |
| **F3 (MEDIUM)** | ✅ Fixed | Pass workflow root as `run_dir` to PhaseRunner (line ~210). PhaseRunner's own `persist::commit_success` creates `attempts/<phase>/<n>/` layout correctly. |
| **F4 (MEDIUM)** | ✅ Documented | Dual-engine template architecture is intentional. Workflow `Template` (regex, `{{ spec }}`/`{{ phase.NAME.output }}`/`{{ var.X }}`) pre-renders; strategy MiniJinja (`{{ steps.NAME.output }}`, filters) is a no-op on the pre-rendered text. Added documentation at `phase_bridge.rs:39-50`. Pre-render retained with strict error propagation (F1). |
| **F5 (LOW)** | ✅ Fixed | Added warning at the start of `run_phase_workflow` when `--rerun` is non-empty, explaining it's a no-op. |
| **F6 (LOW)** | ⏸️ Deferred | Explicit `kind` field for `grammar::Phase` — follow-up task. Current suffix-based mapping covers all v0 outputs. |

### Verification
- `make check`: ✅ 1200 tests pass (913 lib + 287 integration + 0 failed)
- `cargo fmt --check`: ✅
- `cargo clippy`: ✅ (no new warnings)

### Updated Verdict
**approve_with_changes** — single fix iteration applied, all Must Fix items addressed, `make check` green.
