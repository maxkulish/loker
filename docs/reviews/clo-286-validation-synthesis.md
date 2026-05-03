# Validation Synthesis: CLO-286

**Date**: 2026-05-03
**Pipeline**: lok implement-gate (manual fallback — workflow template bug + external reviewer failures)
**Branch**: feat/clo-286-attempt

---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex (gpt-5.5) | OK | Reviewed diff; produced 3 findings on second pass |
| Gemini (primary) | FAILED | Sandbox trust rejection on macOS |
| Gemini (fallback) | FAILED | Same trust issue |
| Claude (fallback) | SKIPPED | Workflow skipped because both reviewers "failed" (dependency logic bug) |
| Synthesis | MANUAL | Produced by pi due to workflow infrastructure failure |

## Findings Classification

### F1 — `promote_to_canonical` fallback too broad [FIXED]
**Severity**: High  
**Where**: `src/run_state/attempt_dir.rs:48`  
**What**: Round 2 replaced `CrossesDevices` match with universal fallback for ANY rename error. Could silently merge into existing canonical dir on non-EXDEV errors.  
**Fix applied (Round 3)**: Restored targeted fallback using `raw_os_error() == Some(18)` (libc::EXDEV), which is MSRV-safe (works on Rust 1.80+). Added D3 parent-fsync after rename.

### F2 — `LatestPointer` stale symlink shadowing + contract drift [FIXED]
**Severity**: High  
**Where**: `src/run_state/latest.rs`  
**What**: Round 2 created `.` symlink for promoted attempts (self-referential). Existing symlink could shadow newer `latest.json`.  
**Fix applied (Round 3)**:
- For promoted attempts: write `latest.json` with canonical path; remove stale symlink first
- For in-progress/failed attempts: create symlink to attempt dir; remove stale `latest.json` first
- Design doc updated to document this contract

### F3 — Producer wiring not implemented [OUT OF SCOPE]
**Severity**: Medium  
**Where**: `src/` (no production callers)  
**What**: No production code path passes `attempt` into `ManifestEntry::from_payload`. The issue body mentions "All four producers write into attempts/" but the plan explicitly scopes this task to primitives only:  
> "This design provides the primitives (`AttemptDir`, `LatestPointer`, updated `next_attempt`). T-028 (PhaseRunner) will integrate these."

This is a deliberate scope boundary. Producers in the current codebase are not yet calling into `run_state`. Adding producer wiring would require touching M2–M4 execution paths and is tracked as T-028.

### F4 — `archive_on_failure()` missing [FIXED]
**Severity**: Low  
**Where**: `src/run_state/attempt_dir.rs`  
**What**: Design includes `AttemptDir::archive_on_failure()` no-op API; not in implementation.  
**Fix applied (Round 2)**: Added `archive_on_failure() -> Ok(())`.

### F5 — Test contract weaker than planned [ACCEPTABLE]
**Severity**: Low  
**Where**: `tests/run_state_attempts.rs`  
**What**: Some tests construct scenarios manually rather than through the full producer pipeline. This is expected since producers are not yet wired (T-028). All 9 tests pass and cover the 8-test TDD contract, plus one bonus test.

## Verdict
approve_with_changes

## Must Fix Before PR
- [x] F1: Targeted EXDEV fallback (Round 3 applied)
- [x] F2: Stale symlink cleanup + no self-reference (Round 3 applied)
- [x] F4: `archive_on_failure()` API (Round 2 applied)

## Out of Scope / Deferred
- F3: Producer wiring — deferred to T-028 (PhaseRunner integration)
- F5: Full producer pipeline tests — deferred to T-028

## False Positives / Tooling Artifacts
- Gemini failure is a macOS sandbox trust issue, not a code finding
- Workflow template bug (`steps.synthesis.output` undefined when all reviewers fail) is infrastructure, not code

## Recommendation
**PROCEED_WITH_FIXES**: All Must Fix items are addressed. The out-of-scope items (producer wiring) are explicitly documented in the plan and design as deferred to T-028. `make check` is green. 9 integration tests + 18 marker tests + 33 config tests pass.

## Re-validation
After Round 3 fixes: `make check` green (2026-05-03). No additional issues found.
