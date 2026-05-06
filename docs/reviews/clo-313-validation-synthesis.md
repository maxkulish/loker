# Pre-PR validation: clo-313

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc syntax error (unmatched quote on line 30) — backticks inside the `cat <<EOF` body broke shell parsing; no review output produced |
| Gemini | REVIEW_FAILED | Same shell heredoc syntax error (unmatched quote on line 38); fallback model never invoked |
| Claude (fallback) | OK | Produced 3 findings (F1 Low, F2 Trivial, F3 Info); verdict `approve_with_changes` |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 — Single source of truth for `run_id`/`phase` in blocked listing** (`src/commands/ls_blocked.rs:117-130` vs existence check at `:68`). The existence check derives `response_path` from filesystem stems, but `response_display_path` derives them from the pending JSON body (with disk fallback). If the two ever drift, operators are told to write the response at a path the scanner won't recognize, so the row never clears. Won't bite under current HumanVerifier writes, but it's a few-line tightening and the dual-source pattern is exactly the kind of latent bug that costs an hour to debug later. Pick one: always use disk stems, or treat JSON/disk mismatch as malformed (warn + skip), matching the existing schema-mismatch behavior.

## Out of Scope / Deferred
- **F2 — Dead `else` branch in `response_path` normalization** (`src/commands/ls_blocked.rs:139-143`). Unreachable from the only caller. Pure cleanup; fine to fold into the F1 fix or leave for a follow-up.
- **F3 — Unused `response_path` field on `BlockedEntry`** (`src/commands/ls_blocked.rs:21`). Currently no consumer or test coverage. Either drop it or add a snapshot assertion when a consumer arrives. Not blocking.

## False Positives / Tooling Artifacts
- **Codex review failure** — tooling artifact in the wrapper script, not a code issue. The heredoc embeds a shell command in backticks (`` `git diff main...HEAD` ``) inside a `$(cat <<EOF ... EOF)`, which the outer shell tries to evaluate. Fix the wrapper (escape the backticks or switch to single-quoted heredoc `<<'EOF'`); does not reflect on this branch.
- **Gemini review failure** — same wrapper bug, same fix. Both reviewers should be re-runnable once the script is patched, but that's an orchestrator-side concern, not a clo-313 concern.

## Recommendation
PROCEED_WITH_FIXES. Land one bounded fix for F1 (collapse `run_id`/`phase`/display-path derivation to a single source — disk stems preferred, since the pending file's location on disk is the ground truth the existence check already trusts). F2 and F3 can ride along or defer. Both external reviewers failed on a wrapper-script syntax bug unrelated to this branch; the Claude fallback's verdict stands and the `make check` gate is green, so the PR is safe to open after F1 is addressed. Separately, flag the heredoc bug in the Codex/Gemini wrapper scripts to the orchestrator owner so future tasks get real dual-reviewer coverage.

## Re-validation

Applied the single permitted fix iteration for F1:

- `src/commands/ls_blocked.rs` now treats the pending file's disk location (`runs/<run_id>/pending/<phase>.json`) as the source of truth for `run_id`, `phase`, and `response_display_path`.
- Added `scan_blocked_uses_disk_stems_when_payload_ids_drift` to prove drifted JSON fields cannot point operators at a response path the scanner will not check.
- Folded in F2 cleanup by storing the already-computed response path directly.
- Added an assertion covering `BlockedEntry.response_path`, addressing F3's test-contract note.

Re-validation command: `make check` — green after the fix.
