# Gemini design review - CLO-310

## Context
- **Branch:** feat/clo-310-resume
- **Design:** docs/designs/clo-310-loker-resume.md
- **PRD:** docs/prds/clo-310-loker-resume.md
- **Discovery:** docs/discovery/clo-310.md

## Findings

### F1 [minor] `resolve_run_dir` should walk up to `lok.toml` for bare names
**Where:** design doc §4 — `resolve_run_dir()` helper
**What:** The proposed helper resolves bare `<run_id>` names relative to `$PWD/runs/<run_id>`. This breaks when the user's CWD is a subdirectory of the project (e.g., `src/`), which is a common working pattern when reading source code. `loker run` already walks up to the `lok.toml` root before creating run directories.
**Why it matters:** Users who cd into `src/` then type `loker resume design-20260505-abc123` will get "Run not found" even though the run directory exists one directory up.
**Suggested fix:** Use the same project-root detection that `loker run` already uses: walk ancestors looking for `lok.toml`, then resolve bare names relative to `<root>/runs/<run_id>`. This is documented as Open Question Q2 in the design doc — resolve it in favor of `lok.toml` walking, matching the existing `run` command's convention.

### F2 [minor] Round-trip test should gate on an opt-in env var
**Where:** design doc §5 — `test_resume_round_trip_kill_mid_phase()`
**What:** The test spawns `loker run` as a child process and sends SIGTERM. This is inherently slower and more brittle than the unit planner tests. Running it unconditionally in CI would add ~30s per test run.
**Why it matters:** The existing integration test convention in this repo is `LOKER_TZ_INTEGRATION=1` for optional tests (`tests/tensorzero_integration.rs`). The round-trip test should follow the same pattern.
**Suggested fix:** Gate the test behind `LOKER_RESUME_INTEGRATION=1` or similar env-var flag, matching the repo convention for opt-in integration tests. Add a comment at the top explaining the env var.

### F3 [nit] `completed` check should also verify manifest SHA consistency
**Where:** design doc §3 — guard clause for "all phases Completed"
**What:** The proposed guard checks `run_state.phase_status.values().all(|s| *s == PhaseStatus::Completed)` before loading the manifest. If a completed marker has a manifest-entry SHA mismatch (artefact corruption), the guard would exit 0 silently even though the run state is not truly clean.
**Why it matters:** The user might lose their completed artefacts to disk corruption. Exiting 0 on "all complete" without verifying artefact integrity gives false confidence.
**Suggested fix:** Reorder the checks: after `RunState::load` succeeds (which already validates manifest SHAs internally), then check `all Completed`. The guard is fine as-is because `RunState::load` will surface `ArtefactCorrupt` before the guard runs — but document this dependency explicitly so future readers don't move the guard before the load.

### F4 [nit] `run_id` rename breaks backward compatibility for scripted callers
**Where:** design doc §3 — renaming `run_dir: PathBuf` to `run_id: String`
**What:** Changing the struct field name breaks anyone who has scripted `loker resume /absolute/path/to/run`. The old positional argument was a `PathBuf`; the new one is a `String`. Clap handles positional arguments by position, not name, so the rename is transparent to CLI callers — but the design doc should state this explicitly.
**Suggested fix:** Add a note: "The CLI positional argument index (first positional) is unchanged. Callers passing an absolute or relative path continue to work without changes."

## Strengths
- Clean separation of concerns: the design is purely additive CLI surface changes with no internal plumbing modifications. Zero risk of regressions in the planner/runner stack.
- `resolve_run_dir` correctly handles both bare names and paths, and the resolution order (abs/relative first, then `runs/` fallback) is sensible.
- The guard clauses for "all complete" (exit 0) and "no state" (exit 1) correctly differentiate the two no-work scenarios.
- Test plan covers both the round-trip integration test AND new unit tests for error paths — good coverage.
- Open questions are genuine and well-framed with tradeoffs.

## Verdict
approve_with_changes

The design is sound and additive. Three concrete improvements needed before implementation:
1. Resolve Open Question Q2 in favor of `lok.toml` walking (F1)
2. Gate the round-trip integration test behind an opt-in env var (F2)
3. Document the implicit SHA-verification dependency in the all-complete guard (F3)
