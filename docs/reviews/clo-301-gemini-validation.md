YOLO mode is enabled. All tool calls will be automatically approved.
YOLO mode is enabled. All tool calls will be automatically approved.
Ripgrep is not available. Falling back to GrepTool.
## Verdict
rework

## Findings

1. **[CRITICAL] Prompt Reconstruction Missing**: The design doc (§4.1, §4.4) explicitly requires `ResumeRunner` to reconstruct the prompt context for resumed phases by loading artefacts of upstream completed phases from the manifest. The current implementation in `src/resume.rs` passes an empty `Prompt` to `PhaseRunner::run()`, which will cause resumed phases to lose all context from previous steps.

2. **[HIGH] Prompt Model Missing `artefacts` Field**: `src/strategy/mod.rs` was not updated to include the `artefacts` field in the `Prompt` struct as specified in the design (§4.4). This prevents any strategy from receiving or using reconstructed context.

3. **[HIGH] Trace and Verify Wiring Missing**: The design doc (§4.5) and plan (P2) require wiring `TraceWriter` and the `verify` hook into `PhaseInputs` during resume. `src/resume.rs` currently hardcodes `trace: None` and `verify: None` in `run_phase`, and `main.rs` does not resolve these from the config.

4. **[MEDIUM] Integration Tests Do Not Verify Execution**: `tests/resume.rs` assertions only cover `ResumePlanner::plan` (the planning phase). There are no integration tests verifying that `ResumeRunner::execute` actually drives the `PhaseRunner` or that the prompt is correctly reconstructed with upstream data.

5. **[LOW] ResumeRunner API Mismatch**: The `ResumeRunner::execute` method in `src/resume.rs` does not accept the `trace` and `verify` parameters as suggested in the plan (P2). This lack of propagation is a direct cause of the missing wiring found in the code.

## Missing Items

- `Prompt::with_artefact()` helper and the underlying `artefacts` storage in `src/strategy/mod.rs`.
- Logic in `ResumeRunner` to load manifest entry content for all `completed` upstream phases and inject them into the `Prompt`.
- Resolution and injection of `TraceWriter` and `verify` hook in `main.rs` and their propagation through `ResumeRunner`.
- End-to-end integration tests that verify execution and context preservation.

## Recommendations

1. **Update `Prompt` struct**: Add `pub artefacts: HashMap<String, Vec<u8>>` to `Prompt` in `src/strategy/mod.rs` and implement the `with_artefact` helper method.
2. **Implement Reconstruction in `ResumeRunner`**: Update `ResumeRunner::run_phase` to iterate through all `completed` phases in the `RunState`, load their artefacts from disk using the paths in the manifest, and add them to the `Prompt`.
3. **Complete CLI Wiring**: Update `main.rs` to initialize the `TraceWriter` (at `run_dir/trace.jsonl`) and resolve the global verify hook. Pass these into `ResumeRunner`.
4. **Fix `PhaseInputs` in Resume**: Ensure `run_phase` populates `PhaseInputs` with the resolved `trace` and `verify` hook instead of `None`.
5. **Add Execution Tests**: Add at least one test case in `tests/resume.rs` that calls `ResumeRunner::execute` and uses a mock backend to verify that the prompt contains the expected artefacts from "completed" phases.
