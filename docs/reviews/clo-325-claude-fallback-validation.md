# Pre-PR validation: clo-325

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-15
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [medium] FailedPhase scenario diverges from design — synthetic marker, not real backend error
**Where:** tests/resume_roundtrip.rs:143-178 (InitialState::FailedPhase setup)
**What:** The design (§3.2 Scenario B, §6 applied suggestion) and plan (ST4 details) both call for the failed state to be produced by a MockBackend that returns `Err(BackendError::Provider("simulated failure"))` for attempt 0 so that the failure path is exercised end-to-end. The implementation instead hand-writes a synthetic `phase2.failed` JSON marker. That short-circuits the very runner path the design wanted to validate (real error → terminal-failure marker + attempt archival), so this scenario reduces to "does ResumePlanner see a Failed status" — already covered by `planner_failed_increments_attempt` unit test.
**Suggested fix:** Add a counter or `Mutex<u32>` on `MockBackend` (or a `FailingMockBackend`) that returns `Err(BackendError::Provider(...))` on the first call and `Ok(...)` thereafter. In `build_roundtrip_run_dir(FailedPhase)`, invoke `PhaseRunner::run(phase2, attempt=0)` so the runner itself emits `phase2.failed` and archives the failed attempt directory naturally.

### F2 [low] Empty placeholder file committed
**Where:** docs/designs/clo-325-resume-roundtrip.draft (0 bytes)
**What:** A zero-byte draft companion file ships alongside the finalized design. It's a workflow artifact, not content; will sit forever as a phantom doc.
**Suggested fix:** `git rm docs/designs/clo-325-resume-roundtrip.draft` before opening the PR.

### F3 [low] InterruptedPhase test doesn't assert what happens to the stale `started.0` marker
**Where:** tests/resume_roundtrip.rs:324-368 (test_resume_roundtrip_kill_phase2)
**What:** After resume runs phase 2 at attempt 1, the original `phase2.started.0` marker still exists on disk alongside the new `phase2.started.1` and `phase2.completed`. The design §7 Q2 explicitly says scenario B should assert the attempt archive / next-attempt marker layout, but the test asserts only `phase_status == Completed`. Without a positive check on the new attempt markers, a regression that silently kept attempt 0 active or skipped the increment would go undetected.
**Suggested fix:** After resume, assert that `markers/phase2.completed` exists, that the new `started.<n>` marker has `attempt >= 1`, and (optionally) that `attempts/phase2/0/` was created if the runner archives — even one explicit marker-file existence check is enough.

### F4 [low] FailedPhase test missing retry-path manifest/attempt assertions
**Where:** tests/resume_roundtrip.rs:374-419
**What:** Same gap as F3 on the retry path. The test confirms phase 2 ends `Completed` but doesn't assert the manifest now has a second `phase2` entry with `attempt: Some(1)`, nor that `phase2.failed` was cleared/superseded. These are the observable signals that the failure was actually archived and retried.
**Suggested fix:** Load manifest after resume and assert there's a phase-2 entry with `attempt == Some(1)`; assert `markers/phase2.completed` exists for the new attempt.

### F5 [low] Pre-merge gate (`make check`) is red on `main`
**Where:** src/strategy/verify/human_verifier.rs:1288, :1294 (pre-existing, NOT on this branch)
**What:** `cargo clippy --all-targets -- -D warnings` fails before this branch's changes (verified by reproducing on a clean main checkout). `make check` therefore won't pass on this PR through no fault of CLO-325, but CLAUDE.md explicitly names `make check` as the pre-merge gate.
**Suggested fix:** Out of scope for this PR, but flag it — either rebase on a main that fixes the two `useless_vec` / `assert_eq!(_, true)` lints in `human_verifier.rs`, or land a tiny pre-req cleanup commit first so the gate is green when this merges.

### F6 [info] Marker file existence not asserted directly
**Where:** tests/resume_roundtrip.rs (all three tests)
**What:** The tests infer state via `RunState::load`-derived `phase_status`, which is correct but indirect — a bug in marker scanning could produce the same status from the wrong marker files. The plan asks for explicit `.completed` marker assertions in ST3 step 7 and ST4 step 7.
**Suggested fix:** Add `assert!(run_dir.path().join("markers/phase2.completed").exists())` in the kill and failed-retry tests after resume.

## Verdict
approve_with_changes

The branch lands the requested round-trip integration test cleanly: three scenarios, no production code touched, all three tests pass deterministically in ~0.1s, no new clippy warnings introduced (the existing failures are inherited from main). The structural choices — direct marker fabrication, stale heartbeat to dodge live-writer guard, in-process MockBackend — are reasonable for the stated goal. The reason for "approve_with_changes" rather than "approve" is F1: the FailedPhase scenario was specifically redesigned in the design review to use a real backend error, and the implementation reverted to synthetic marker injection, weakening the most valuable scenario in the matrix. F3/F4/F6 are small assertion gaps that follow the plan's own ST3/ST4 acceptance text. F2 is a one-line cleanup; F5 is not blocking but the project's stated merge gate needs attention out-of-band.
