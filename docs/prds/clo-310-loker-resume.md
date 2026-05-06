# PRD: `loker resume <run_id>` CLI subcommand (CLO-310 / T-041)

## Source

This PRD is extracted from the Linear issue body at https://linear.app/cloud-ai/issue/CLO-310/t-041-loker-resume-run-id-cli-subcommand

## Goal

Expose `loker resume <run_id>` as a first-class CLI surface that picks up an interrupted run from the last `phase.completed` marker.

Roadmap row: T-041 (Phase 9). PRD: FR-33.

## Scope

* `loker resume <run_id>` subcommand. `<run_id>` is either the directory name under `runs/` or an absolute path.
* Reuses the resume planner shipped in [CLO-295](https://linear.app/cloud-ai/issue/CLO-295/t-031-implement-run-resumability-via-status-markers) / [CLO-301](https://linear.app/cloud-ai/issue/CLO-301/t-031-follow-up-wire-resumerunner-execution-end-to-end).
* Round-trip integration test: kill mid-phase via signal, resume, assert the run completes with no duplicate work.
* Helpful error if `<run_id>` is not found, is fully complete, or has no resumable state.

## Acceptance Criteria

- [ ] Pause / resume round-trip integration test passes against a mocked workflow.
- [ ] Resuming a fully-complete run is a no-op with exit code 0 and a clear message.
- [ ] Resuming a corrupted run (missing manifest, stale lock) fails fast with diagnosable output.
- [ ] `loker resume` and `loker run --rerun phase=X` are clearly distinct in `--help`.

## Non-goals

* Resuming HITL-blocked phases — covered by Phase 11/12 work.
* Automatic resume on crash detection — explicit user invocation only for v0.

## Dependencies

* T-031 ([CLO-295](https://linear.app/cloud-ai/issue/CLO-295/t-031-implement-run-resumability-via-status-markers)) resumability via status markers — done.
* [CLO-301](https://linear.app/cloud-ai/issue/CLO-301/t-031-follow-up-wire-resumerunner-execution-end-to-end) ResumeRunner wiring — done.
* T-040 ([CLO-309](https://linear.app/cloud-ai/issue/CLO-309/t-040-loker-run-workflow-cli-subcommand-flags)) CLI conventions — done.

## References

* PRD FR-33
* `docs/plans/001-implementation-roadmap.md` Phase 9 row T-041
* `docs/discovery/clo-310.md`

## Post-implementation note (CLO-310)

The round-trip integration test (kill mid-phase via signal, resume, assert completion)
was **descoped** from CLO-310. The original test design required spawning `loker run` and
sending SIGTERM, but `loker run` uses the step-based runner which does not write phase
markers. The phase-based runner that produces resume-able state is a separate execution
path (CLO-301).

**Shipped instead:**
- Binary-level guard clause test (`test_resume_via_binary_all_complete_guard`), opt-in
  behind `LOKER_RESUME_INTEGRATION=1`, validates the full data path through the binary
  (resolve_run_dir -> RunLock -> RunState::load -> guard clause).
- Guard clause tests cover all error paths (not found, all complete, no resumable state).

**Follow-up:**
- [CLO-325](https://linear.app/cloud-ai/issue/CLO-325) tracks the runner-level round-trip
  integration test once the phase runner is wired into `loker run`.
