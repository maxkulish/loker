# Validation Synthesis: CLO-322

**Synthesized**: 2026-05-07
**Design**: docs/designs/clo-322-sessions-ui.md
**Plan**: docs/plans/clo-322-sessions-ui.md

## Reviewer Status

| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini | OK | Gemini 3.1 Pro — verdict: approve_with_changes |
| Codex | OK | GPT-5.5 high effort via `codex review` — verdict: approve_with_changes |

## Agreement (High Confidence)

Both reviewers independently flagged the missing `insta` snapshot tests.

## Findings

| # | Finding | Source | Severity |
|---|---------|--------|----------|
| 1 | Approve/reject handlers lack `run_id` path traversal validation — SECURITY | Codex | CRITICAL |
| 2 | Pending gate discovery doesn't filter resolved gates (response files ignored) | Codex | HIGH |
| 3 | Missing `insta` snapshot tests for HTML views (design doc requirement) | Both | MEDIUM |
| 4 | `tail_trace_file` reads entire file into memory | Gemini | LOW |

## Verdict

**approve_with_changes**

## Must Fix Before PR

1. **Add `run_id` sanitization** to approve/reject handlers (same as `run_detail`).
2. **Filter resolved gates** in `gate_discovery.rs` — skip pending files that have
   a sibling response file in `responses/<phase>.json`.
3. **Add `insta` snapshot tests** for `/`, `/runs/:id`, `/pending` HTML output.

## Out of Scope / Deferred

- `tail_trace_file` memory efficiency — LOW, defer to follow-up.

## Recommendation

**Proceed** — Apply the 3 fixes (1 iteration), re-run tests, then transition to PR.
