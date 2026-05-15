# Pre-PR validation: clo-325

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-15
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc syntax error in invocation script (unmatched quote on line 30); no review output produced |
| Gemini | REVIEW_FAILED | Same shell heredoc syntax error in invocation script (line 38); both primary and fallback models never invoked |
| Claude (fallback) | OK | Full review produced with 6 findings (1 medium, 4 low, 1 info); verdict `approve_with_changes` |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 [medium] FailedPhase scenario diverges from design** — `tests/resume_roundtrip.rs:143-178` fabricates a synthetic `phase2.failed` marker instead of driving the failure through a `MockBackend` that returns `Err(BackendError::Provider(...))` for attempt 0. The design §3.2 Scenario B and the §6 applied design-review suggestion explicitly require a real backend error so the runner emits the failed marker and archives the attempt naturally. As implemented, the scenario reduces to "ResumePlanner sees a Failed status" — already covered by `planner_failed_increments_attempt`. Fix: add a stateful `MockBackend` (e.g., `Mutex<u32>` call counter) that errors on attempt 0 and succeeds on attempt 1, then let `PhaseRunner::run` produce the failed marker.
- **F3/F4/F6 [low] Missing explicit marker/manifest assertions** — Plan ST3 step 7 and ST4 step 7 call for explicit `.completed` marker assertions and manifest re-checks. Both `test_resume_roundtrip_kill_phase2` (line 324) and the FailedPhase retry test (line 374) infer success only via `phase_status == Completed`. Add: (a) `assert!(run_dir.path().join("markers/phase2.completed").exists())` in both tests after resume; (b) load the manifest and assert a phase-2 entry with `attempt == Some(1)` exists; (c) for the kill test, assert the new `phase2.started.<n>` marker has `attempt >= 1`.
- **F2 [low] Empty placeholder file** — `docs/designs/clo-325-resume-roundtrip.draft` is a 0-byte workflow artifact. `git rm` before opening the PR.

## Out of Scope / Deferred
- **F5 [low] `make check` red on main** — Pre-existing clippy failures in `src/strategy/verify/human_verifier.rs:1288, :1294` (`useless_vec`, `assert_eq!(_, true)`) are inherited from main and unrelated to CLO-325. The branch itself introduces no new clippy warnings. Flag for a separate cleanup commit; do not block this PR.

## False Positives / Tooling Artifacts
- **Codex and Gemini "REVIEW_FAILED"** — Both failures are bugs in the invocation shell scripts (heredoc quoting), not signals about the change. The fallback review covered the same ground (design fidelity, test coverage, scope creep, Rust idioms). Treat the synthesis as single-reviewer (Claude fallback) and weight the verdict accordingly — but the in-scope findings stand on their own merits against the design and plan.

## Recommendation
PROCEED_WITH_FIXES. The branch is bounded (tests-only, no production code touched, all three tests pass deterministically) and the gaps are addressable in one iteration: (1) replace the synthetic `phase2.failed` marker in the FailedPhase scenario with a real `MockBackend` failure on attempt 0; (2) add explicit `markers/phase2.completed` existence assertions and manifest `attempt == Some(1)` checks in both kill and failed-retry tests; (3) `git rm` the empty `.draft` companion file. After these, the round-trip suite matches the design's intent rather than working around it. The Codex/Gemini script breakage should be fixed upstream in `.pi/scripts` (heredoc quoting), but is independent of this PR's merge decision.
