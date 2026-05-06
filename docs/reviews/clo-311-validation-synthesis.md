# Pre-PR validation: clo-311

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc parsing error in invocation script — `unexpected EOF while looking for matching '` (tooling failure, no review produced) |
| Gemini | REVIEW_FAILED | Same shell heredoc parsing error in invocation script (tooling failure, no review produced) |
| Claude (fallback) | OK | Six findings (F1–F6), all minor/nit; verdict approve_with_changes |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 — Workflow lookup error swallowed for typo'd workflow names** (src/main.rs:1996-2005). When `target` looks like a workflow name but isn't found, `run_explain_unified` discards the "Workflow 'X' not found" error and falls through to codebase mode, which then fails with an unrelated `canonicalize`/path error. This is a UX regression on the new code path: a common user mistake (typo) produces a misleading error. Bounded fix: if `target` doesn't resolve as a workflow AND isn't an existing directory, return the original `find_workflow_in` error before delegating to `run_explain`.

## Out of Scope / Deferred
- **F2 — `[[phases]]` substring heuristic is brittle** (src/workflow/explain.rs:44-49). Produces misleading errors for edge cases (commented-out headers, unusual whitespace). Real concern but no correctness impact for valid phase-based workflows; cleaner refactor (parse-first, classify on failure) can land in a follow-up.
- **F3 — Double validation in `explain_workflow`** (src/workflow/explain.rs:86-89). `FromStr` already validated; the second `validate()` call is wasteful but harmless. Pure efficiency cleanup.
- **F4 — Snapshot path is OS-dependent** (tests/explain_cli.rs:29). Will diverge on Windows due to backslash separators. No Windows CI today; defer until/unless Windows CI is added, or fix in a separate test-stability pass.
- **F5 — CLI flag arity change not documented** (src/main.rs:439-454). `loker explain` now accepts a workflow name as first positional with `--dir/-d` for codebase mode. Worth a one-line entry in docs/handoff.md but not blocking.
- **F6 — Unused `Clone` on `WorkflowSource`** (src/workflow/mod.rs:3453). Minor API-surface trim per CLAUDE.md "don't add features beyond what the task requires." Drop in the same follow-up as F3.

## False Positives / Tooling Artifacts
- **Codex review** — invocation script has a heredoc quoting bug (`'EOF'` inside `$(cat <<EOF ... EOF)` got mangled by the shell wrapper). Not a real REVIEW_FAILED; the reviewer never ran. Tooling/script bug to fix in `.pi/` review harness.
- **Gemini review** — same heredoc quoting bug in its invocation script. Same tooling artifact, same fix needed.

## Recommendation
PROCEED_WITH_FIXES. Land one bounded fix iteration: address **F1** (return the workflow-not-found error when target doesn't resolve as either workflow or directory). Optionally bundle **F3** (drop double-validate) and **F6** (drop unused `Clone`) since they're trivial single-line changes in adjacent files. Defer F2/F4/F5 to follow-up tickets. Independently, the orchestrator's `.pi/` review scripts for Codex and Gemini have a shell quoting bug that prevented both external reviews from running — file a separate ticket to fix the heredoc nesting in the wrapper so future syntheses aren't single-source.

## Re-validation
- Applied Must Fix F1: `run_explain_unified` now returns the original workflow lookup error when the target is neither a resolvable workflow nor an existing directory, instead of falling through to codebase explanation.
- Added `test_explain_unknown_workflow_name_reports_lookup_error` to lock the behavior.
- Re-ran `cargo test --test explain_cli` and `make check`; both are green.
