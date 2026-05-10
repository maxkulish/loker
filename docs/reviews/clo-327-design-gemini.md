Ripgrep is not available. Falling back to GrepTool.
(node:4700) [DEP0190] DeprecationWarning: Passing args to a child process with shell option true can lead to security vulnerabilities, as the arguments are not escaped, only concatenated.
(Use `node --trace-deprecation ...` to show where the warning was created)
Error executing tool run_shell_command: Tool "run_shell_command" not found. Did you mean one of: "update_topic", "grep_search", "replace"?
## 1. Completeness Check
- **Summary/Problem**: Present. Clearly states the gap with `loker run` silently dropping phase-based workflows.
- **Goals / Non-goals**: Present. Explicitly scopes what is and isn't being built.
- **Architecture**: Present. Details the execution fallback path, structs, and data flow.
- **Public API surface**: Present. Details modified and new functions.
- **Test plan**: Present. Good coverage of unit and integration testing.
- **Migration / Rollout**: Present.
- **Open questions**: Present.

The structure deviates slightly from the standard PRD template headings but provides all necessary functional context.

## 2. Architecture Assessment
**Strengths**:
- The decision point in `run_workflow()` attempting phase parsing before falling back to step-based TOML elegantly preserves backward compatibility without requiring explicit flags.
- The fail-fast approach for phase execution (stopping if phase N fails) is architecturally correct since phases depend strictly on earlier outputs.
- Data flow cleanly resolves backends through the existing `create_backend()` factory.

**Concerns**:
- **Re-inventing `TemplateEngine`**: The document proposes a new `TemplateEngine` struct with `spec_content`, `phase_outputs`, and `vars`. However, `src/workflow/template.rs` already exists, providing a strict substitution `Template` engine and a `TemplateContext` struct with exactly these three fields. Proposing to build a new one duplicates effort and diverges from existing components built in task CLO-289.
- **API Ergonomics**: `PhaseWorkflowRunner` specifies `config: Arc<config::Config>`, but `run_phase_workflow` is typed as `config: &config::Config`. This should be synchronized, preferably matching `Arc<config::Config>` if the underlying executor requires it.

## 3. Alignment with Handoff & Roadmap
- **Deviation from PRD**: The PRD `docs/prds/clo-327-wire-phase-based-runner.md` states in Scope: "7. Honour `--resume` for phase-based workflows." However, the design doc explicitly lists "Full `--resume` support for phase-based workflows" as a **Non-goal** and states "Decision: Defer" in Open Questions. This is a severe contradiction of the task's PRD.
- Fits within the intent of `docs/handoff.md` by adhering to TDD testing contracts and isolated execution. 

## 4. Security Review
- **Secure Substitution**: Reusing the existing simple-substitution template engine prevents unintended execution or evaluation vulnerabilities (no logic or expressions allowed inside templates).
- No new network calls or filesystem behaviors are introduced outside of the existing `PhaseRunner` boundary. The architecture maintains the current security posture.

## 5. Implementation Concerns
- **Backend Resolution Timing**: The document claims backend resolution at config building time is "same as the step-based runner." This is factually incorrect; the step-based runner resolves backends via `create_backend` inside the execution loop at runtime. While resolving early is better for failing fast, the comparison rationale is flawed.
- **Implementation Order**: Step 2 of the implementation plan ("Template engine - implement TemplateEngine") should be updated to "Wire existing TemplateEngine (`src/workflow/template.rs`)". 
- `make check` should explicitly cover `phase_detection_detects_phases_only` and `phase_detection_detects_steps_only`.

## 6. Concurrency & Async
- The execution loop walks phases sequentially and asynchronously via `PhaseRunner::run()`. This conforms correctly to `tokio` async expectations.
- No blocking filesystem or network calls appear in the proposed synchronous parts of the loop.
- **Cancellation Safety**: The `PhaseRunner` relies on the filesystem marker system (`phase.completed`). Because we are just invoking the existing `PhaseRunner`, crash-consistency and resumption invariants remain intact.

## 7. Blind Spots
- **`--resume` contradictions**: As noted, the deferral of `--resume` contradicts the PRD. Given `src/resume.rs` already exists and operates on `PhaseConfig`s, wiring it might be trivial and should not be deferred.
- **`--rerun phase=<name>` behavior**: If `--resume` support is deferred, it's unclear how the `--rerun` flag interacts with sequential phase execution in a fresh run. The document passes `rerun_phases` to `PhaseWorkflowRunner` but doesn't explain the logic if it encounters a phase it should skip.

## 8. Verdict
NEEDS_REVISION

## 9. Actionable Feedback
1. **Reconcile `--resume` Scope**: Resolve the contradiction with the PRD. Either design the wiring for `ResumeRunner` so `--resume` works for phase-based workflows, or get sign-off to formally amend the PRD.
2. **Reuse Existing Template Engine**: Remove the proposal to build a new `TemplateEngine`. Explicitly specify wiring `crate::workflow::template::Template` and `TemplateContext` instead of re-inventing them.
3. **Clarify `--rerun` Flag**: If resume/stateful runs are indeed deferred, clarify how `--rerun phase=<name>` is handled in `run_phase_workflow`.
4. **Fix API Signatures**: Ensure `config` parameter types match between `PhaseWorkflowRunner` constructor and internal helpers (e.g., standardizing on `Arc<config::Config>`).
