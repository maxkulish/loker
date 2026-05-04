# Gemini Design Review — CLO-301: Wire ResumeRunner Execution End-to-End

## 1. Completeness Check

The design document is comprehensive and well-structured. It includes all required sections:
- **Problem**: Clearly identifies the stubbed `execute()` method and the five blockers preventing end-to-end wiring.
- **Goals / Non-goals**: Well-defined, specifically excluding manual rewind which fits the v0 scope.
- **Architecture**: Maps the changes to existing modules (`phase_runner.rs`, `resume.rs`, `workflow/mod.rs`, `main.rs`).
- **Detailed Design**: Provides concrete implementation details for blockers, derivation rules, and prompt reconstruction.
- **Implementation Plan**: Integrated into the roadmap and blocker resolution table.
- **Acceptance Criteria**: Defined via unit and integration tests.

## 2. Architecture Assessment

**Strengths**:
- **Leverages Existing Primitives**: Instead of inventing a new execution path, it correctly adapts `PhaseRunner::run` to accept an `initial_attempt`.
- **Stateless Planning**: The `Workflow::to_phase_configs()` adapter is a clean way to bridge the declarative workflow world with the execution-focused `PhaseRunner`.
- **Atomic Run-State Alignment**: The design adheres to the D3 protocol (`docs/run-state.md`) by correctly handling `initial_attempt` and archiving current attempts before re-execution.

**Concerns**:
- **Prompt Reconstruction Complexity**: While `with_artefact` is a good start, the design relies on loading manifest entries for all `completed` phases. It needs to ensure that the `PhaseInputs` injected into `ResumeRunner` have a fully populated `Prompt` that matches what `WorkflowRunner` would have built.

## 3. Alignment with Handoff & Roadmap

- **Intent**: Perfectly aligns with the "resumable runs" goal in `docs/handoff.md` and `docs/run-state.md`.
- **Milestone**: Directly addresses T-031 from the roadmap (`Phase 6 - Phase runner & trace`).
- **Conventions**: Follows the pattern of using `Arc<dyn Backend>` and `PhaseInputs` consistently with the rest of the M1-M5 implementation.

## 4. Security Review

- **Sandboxing**: The design assumes the use of `PhaseRunner`, which already integrates `RunCommand` for verify hooks (satisfying FR-14/FR-21).
- **Redaction**: Since it uses the existing `TraceWriter`, it inherits the redaction capabilities defined in the security NFRs.
- **Input Validation**: `Workflow::to_phase_configs()` should be called after `Workflow::validate()`, which ensures that the conversion only happens for valid workflows.

## 5. Implementation Concerns

- **Shell Steps Gap**: The design explicitly excludes shell steps from `to_phase_configs()`. While this simplifies the `PhaseRunner` wiring, it means a resumed workflow will skip shell steps if they were the point of failure. This is a known limitation of the current `PhaseRunner` (which focuses on LLM phases), but should be documented as a "resume-path limitation" if shell steps can fail.
- **Aggregator Mapping**: The mapping in Question 2 of the "Open questions" section (`Synthesis -> First`) is a simplification but acceptable for v0 given that `Synthesis` in `Workflow` is effectively just taking the first response if no explicit synthesis logic is wired yet.

## 6. Concurrency & Async

- **Locking**: The design correctly includes `lock::acquire(run_dir/.lock)`, which prevents two `resume` processes (or a `run` and a `resume`) from corrupting the state.
- **Cancellation**: Relies on `tokio` patterns already present in `PhaseRunner`.

## 7. Blind Spots

- **Variable Interpolation**: `WorkflowRunner` handles complex interpolation (steps.X.output, etc.). `ResumeRunner` needs to ensure that when it converts steps to `PhaseConfig`, it handles these dependencies correctly. The design touches on this in §4.4 (Prompt reconstruction) but the implementation of `to_phase_configs` needs to be careful not to lose the "templating" context.

## 8. Verdict

**APPROVE_WITH_SUGGESTIONS**

The design is solid and correctly bridges the gap between the persisted run state and the phase runner. The suggestions below focus on hardening the prompt reconstruction and documenting the shell-step limitation.

## 9. Actionable Feedback

### F1 [minor] Shell Step Resumability

- **Where**: docs/designs/clo-301-resume-runner-wiring.md §4.2
- **What**: Shell steps are excluded from `to_phase_configs()`. If a workflow fails on a shell step, `loker resume` will effectively skip it or fail to find the "next" phase if the shell step was the failure point.
- **Suggested fix**: Add a note to the design doc stating that shell steps are currently "pass-through" and resume will skip them, or plan to add a `PhaseConfig::shell()` variant in a future iteration.

### F2 [minor] Manifest Entry Sweeping

- **Where**: docs/designs/clo-301-resume-runner-wiring.md §4.3
- **What**: When adding `workflow_name` to `manifest.json`, ensure the `RunState::load` logic is updated to handle cases where the manifest might be partially written (matching the D3 protocol's "orphan-entry sweep").
- **Suggested fix**: Explicitly state in §4.3 that `ResumeRunner` will trigger the manifest orphan-sweep (defined in `docs/run-state.md` row 9) before execution.

### F3 [nit] Prompt Helper Placement

- **Where**: src/strategy/mod.rs
- **What**: `with_artefact` is proposed. Ensure it correctly handles binary content vs UTF-8 if the `artefact_kind` is not a text format.
- **Suggested fix**: Implementation should ensure `Prompt` storage for artefacts is `Vec<u8>` or appropriately encoded.
