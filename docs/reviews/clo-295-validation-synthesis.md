# Pre-PR validation: clo-295

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-03
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc syntax error in wrapper script (unmatched quote on line 30); never invoked the model |
| Gemini | REVIEW_FAILED | Same heredoc syntax error pattern (unmatched quote on line 38); never invoked the model |
| Claude (fallback) | OK | 8 findings; verdict `rework` |

## Verdict
pivot

## Must Fix Before PR
- **F3 (high) — `archive_current_attempt` mislabels attempt directory.** `src/resume.rs:170-175` passes `next_attempt` to a function the design says expects the *current* attempt; archives partial output of attempt N as `attempts/<phase>/<N+1>/`. Bounded fix: pass `attempt - 1` (or rename field to `next_attempt`) and add a regression test where a canonical `<phase>/` + `.started.1` marker exists.
- **F6 (low) — Sweep TTL boundary inconsistent with `is_stale`.** `src/resume/sweep.rs:46` uses `<=`; `heartbeat.rs:212` uses `<`. Pick one (recommend `<`) or extract a shared helper.
- **F7 (low) — `RunLock.file` `#[allow(dead_code)]` masks load-bearing field.** Replace the allow with a one-line comment documenting Drop semantics, or implement explicit `Drop`.
- **F8 (low) — `clippy::result_large_err` blanket-allowed.** `Box` the heavy `LoadError`/`PhaseError` variants and remove the file-level allow.

## Out of Scope / Deferred
- **F5 (medium) — Cross-filesystem `EXDEV` fallback missing.** Design §3.5 specified it; in practice run_dir + attempts share a parent. Either drop from design or schedule as follow-up — don't pull `fs_extra` in for one rare path.

## False Positives / Tooling Artifacts
- Both Codex and Gemini wrappers crashed before model invocation due to a heredoc quoting bug in the review scripts (`unexpected EOF while looking for matching '`). Not a code finding — wrapper-script defect under `.pi/agents/` or the orchestrator that runs them. Should be fixed before relying on dual-model review for the next gate.

## Pivot / Fundamental Scope Issue
- **F1 + F2 + F4 together** — the CLI `resume` subcommand is a status printer (`src/main.rs:864-898`), the integration tests in `tests/resume.rs` import `ResumeRunner` but never invoke it (compiler dead-code warnings on `fake_backends`/`write_failed_marker` confirm), and `ResumeRunner::run_phase` (`src/resume.rs:189-209`) builds an empty `Prompt::new()` with no manifest/verify/trace and ignores its `_attempt` parameter. The branch ships a green planner + scaffolding, but the user-visible contract from the design ("`loker resume <run-dir>` actually resumes the run; manifest ends with 3 entries; phase 2 re-runs in `attempts/phase2/2/`") is not met. This is a scope decision, not a one-iteration bug fix:
  - **Option A (wire it up):** reuse `WorkflowRunner` plumbing inside `ResumeRunner`, derive `Vec<PhaseConfig>` from the persisted workflow, build a mock `Backend` for tests, and add the design §5.2 integration assertions. Substantial — likely a week of work plus design clarification on how the runner gets phase configs at resume time (open question #2 in the design was never resolved).
  - **Option B (descope):** rename the PR/PRD/Linear scope to "planner + library scaffolding for resume", remove the half-wired CLI subcommand (or gate it behind a hidden flag with a clear "not yet executable" message), delete the dead imports in `tests/resume.rs`, and split runner-wiring into a follow-up issue.

## Recommendation
**STOP_FOR_USER.** Decision needed: *Option A — finish the runner end-to-end in this PR (substantial, plus resolving design open question #2 on phase-config derivation at resume time)*, or *Option B — descope CLO-295 to "planner + library scaffolding" and split runner integration into a follow-up issue*. The branch should not merge as-is: `loker resume <run-dir>` would silently print status and exit while looking like it works. Once scope is decided, F3/F6/F7/F8 are the small Must-Fix list to clean up before PR; F5 can be deferred. Also worth a separate fix: the Codex and Gemini review wrappers under `.pi/agents/` are syntactically broken and never ran their models — both review legs were pure tooling failures, not silent approvals.
