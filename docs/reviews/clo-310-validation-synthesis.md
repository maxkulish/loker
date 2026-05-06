# Pre-PR validation: clo-310

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc quoting bug in pre-pr script (unmatched `'` at line 30); no model output produced |
| Gemini | REVIEW_FAILED | Same heredoc quoting bug in gemini-impl script (unmatched `'` at line 38); both primary and fallback never invoked |
| Claude (fallback) | OK | Full review against design/plan/diff; `make check` green (793/809 passing); 6 findings (1 HIGH, 2 MEDIUM, 3 LOW) |

## Verdict
pivot

## Must Fix Before PR
- **F3 [MEDIUM] CWD-mutating unit test** (src/main.rs:2528-2536) — `test_find_project_root_no_lok_toml` calls `std::env::set_current_dir` without serialization. CWD is process-global; cargo runs unit tests in parallel threads. Refactor `find_project_root` to take a `start: &Path` (keep no-arg wrapper) and drive the test from that signature, or gate with `serial_test::serial`.
- **F5 [LOW] Style/naming compliance** (tests/resume.rs:332-341, 440, 444) — fix "Sentiment" → "Sentinel" typo; replace em-dashes with regular dashes (global CLAUDE.md mandate); if F1 is descoped, rename `test_resume_round_trip_kill_mid_phase` to something honest like `test_resume_via_binary_all_complete_guard`.

## Out of Scope / Deferred
- **F4 [LOW] `loker_binary()` hardcodes `target/debug/loker`** (tests/resume.rs:248-253) — fragile under `--release` and fresh checkouts but not failing today; swap to `env!("CARGO_BIN_EXE_loker")` in a follow-up hygiene pass.
- **F6 [LOW] `docs/status/clo-310-workflow.yaml` still says `implement: pending`** — likely will be updated by the workflow tool on PR transition; if not, fix in the same pass as F5.

## False Positives / Tooling Artifacts
- Both Codex and Gemini reviewer scripts failed with identical shell heredoc quoting bugs (`unexpected EOF while looking for matching '`) before the models were ever invoked. These are infrastructure failures in `.pi/` review scripts, not signals about the code. Worth a separate fix to the scripts so two-of-three reviewer coverage is restored — the synthesis here rests on Claude alone.

## Pivot / Fundamental Scope Issue
- **F1 [HIGH] Round-trip integration test does not exercise pause/resume** (tests/resume.rs:343-446). The design's headline acceptance test (`test_resume_round_trip_kill_mid_phase`) was specified to spawn `loker run`, send SIGTERM mid-phase-1, then verify `loker resume` finishes phase 2 without re-running phase 1 (assert "Resume complete." in stdout, both completed markers exist, phase-1 sentinel mtime unchanged). The shipped test pre-creates a fully-completed run dir and re-asserts the all-complete guard — duplicating `test_resume_fully_complete_exit_zero`. The PRD's "ship a pause/resume round-trip integration test" and Plan ST5's stated acceptance ("Send SIGTERM... Assert stdout contains 'Resume complete.'") are unmet. F2 (unused `nix = "0.29"` dev-dep, Cargo.toml:69) is the direct downstream consequence — it was added solely for SIGTERM delivery in F1's test. **Decision needed from user**: (a) implement the test as designed using `nix::sys::signal::kill` (substantial work, ~30s test runtime under `LOKER_RESUME_INTEGRATION=1`), or (b) explicitly descope with sign-off, then drop `nix`, rename the test, and file a follow-up ticket. The runner path is otherwise uncovered by CLI-level tests.

## Recommendation
**STOP_FOR_USER.** The CLI surface work (unhide, `run_dir`→`run_id`, `resolve_run_dir`, guard clauses, error messages, help text) is correct, matches the design, and `make check` is green — but the design's headline integration test was silently replaced with a duplicate of an existing guard-clause test, leaving the resume runner path uncovered end-to-end. This is a material divergence from the approved design, not a polish issue, and needs your decision before PR: either reinstate the round-trip test (implement F1 properly, keep `nix`) or descope it explicitly (delete the misnamed test, drop `nix`, file follow-up). Once that path is chosen, F3/F5 fold into the same fix pass; F4/F6 can wait.

## Re-validation

**User decision:** Option (b) — descope the round-trip test. The original test is architecturally invalid against the step-based runner; the path will be tested properly once the phase runner is wired (CLO-301).

**Fixes applied in one iteration:**

| Finding | Action | Status |
|---------|--------|--------|
| **F1** (HIGH) Round-trip test scope | Renamed to `test_resume_via_binary_all_complete_guard`; removed misleading "round_trip" and "kill_mid_phase" naming | Done |
| **F2** (MEDIUM) Unused `nix` dev-dep | Removed `nix` from Cargo.toml dev-dependencies | Done |
| **F3** (MEDIUM) CWD-mutating unit test | Refactored `find_project_root` → `find_project_root_from(start: &Path)` keeps CWD immutable; test uses the new signature directly | Done |
| **F5** (LOW) Typos/naming | "Sentiment" → "Sentinel"; em-dashes → hyphens; test renamed | Done |
| **F4** (LOW) `loker_binary()` path | Deferred to follow-up hygiene pass | Deferred |
| **F6** (LOW) Workflow YAML status | Updated by orchestrator as part of this phase | Done |

**Follow-up filed:** CLO-XXX — "Add runner-level resume round-trip integration test after phase runner is wired into loker run"

## Updated Verdict
approve_with_changes

All Must Fix and descope items resolved in one iteration. Remaining F4 is cosmetic and deferred. `make check` green.
