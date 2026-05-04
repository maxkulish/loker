# Pre-PR validation: clo-309

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-04
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc syntax error in invocation script (unmatched quote in line 30); no review content produced |
| Gemini | REVIEW_FAILED | Same heredoc syntax error (line 38); both primary `gemini-3.1-pro-preview` and fallback `gemini-2.5-pro` never invoked |
| Claude (fallback) | OK | Full review delivered, 9 findings (2 medium, 6 low, 1 info), verdict approve_with_changes |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 (medium) — `Run` help text claims phase-based engine** at src/main.rs:350-353. Doc comment says "with the new phase-based engine (CLO-309)" but the command routes to the step-based `WorkflowRunner`; phase wiring is explicitly deferred to T-041 per the design. Replace with a string that names step-based as today's behavior and notes T-041.
- **F2 (medium) — `--rerun` prints "Forcing re-execution" while doing nothing** at src/main.rs:1342-1352. The loop emits `↻ Forcing re-execution of phase '<X>'` and an inline `// NOTE:` claims markers are cleared "directly here" — neither happens. This is a correctness/honesty issue that ships a code comment contradicting the code. Either drop the loop or rephrase to an honest "accepted, no-op for step-based runner; effective in T-041" message and delete the misleading comment.

## Out of Scope / Deferred
- F3 (rerun_phases field stored but unused) — reserved for T-041; add `#[allow(dead_code)]` + one-line comment in this PR is optional cleanup.
- F4 (`trailing_var_arg` parity with `WorkflowCommands::Run`) — behavioral nit, not a regression in CLO-309 scope; verify with quick parity test, fix in follow-up if divergence matters.
- F5 (rerun integration test doesn't actually verify rerun semantics) — accurate observation, but the design accepts no-op rerun for step-based runner; rename or add comment in T-041 when semantics arrive.
- F6 (test workflow uses `[[steps]]` named `phase_*`) — naming hygiene; resolve in T-041 when phase-based runner lands.
- F7 (spec double-clone per render) — perf micro-nit on a code path that runs per step; defer.
- F8 (no size guard on `--spec` file read) — defensive hardening; defer unless project has a sibling pattern to match.
- F9 (`{{ spec }}` in shell `echo` examples is injection-prone) — workflow-author responsibility; one-line caveat in design doc is sufficient and can be a docs follow-up.

## False Positives / Tooling Artifacts
- Codex and Gemini reviews are tooling failures (broken shell heredoc in the wrapper scripts at `.pi/agents/...` invocation), not signals about the diff. The wrapper itself is bugged — line 30 (Codex) and line 38 (Gemini) have unmatched single quotes inside the heredoc. Worth fixing in `.pi/` orchestrator scripts so future synthesis isn't single-reviewer.

## Recommendation
PROCEED_WITH_FIXES. Land two bounded edits before opening the PR: (1) rewrite the `Run` doc comment at src/main.rs:350-353 to name step-based-today + T-041-tomorrow honestly, and (2) at src/main.rs:1342-1352 either drop the rerun loop or rewrite the message to declare it a no-op and delete the false `// NOTE:` comment. Everything else is deferrable. Separately, file a quick fix for the `.pi/agents/codex-pre-pr.md` and `gemini-architect.md` invocation scripts so the next synthesis has three reviewers instead of one.
