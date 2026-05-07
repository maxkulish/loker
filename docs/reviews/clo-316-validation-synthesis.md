# Pre-PR validation: clo-316

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-07
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc quoting bug in invocation script (unmatched `'` on line 30) — script never reached the `codex exec` call. Tooling artifact, not a code finding. |
| Gemini | REVIEW_FAILED | Same shell heredoc quoting bug (unmatched `'` on line 38) — `gemini` never invoked. Tooling artifact. |
| Claude (fallback) | OK | Produced 5 findings; F1 verified independently (`src/commands/trace.rs` exists, `loker trace` shipped in 28c477d). |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 (critical) — Committed migration doc contains a factually wrong claim that `loker trace` is not present.** Verified: `src/commands/trace.rs` exists and was shipped by CLO-312 (commit 28c477d). HEAD's `docs/migration-from-lok.md:35` still states "loker trace is not present in the currently installed CLI in this branch." This contradicts the doc's own "Verification appendix" claim that commands were checked against the binary. Shipping this would mislead migrants and undermine source-of-truth credibility.
- **F2 (medium) — Substantive corrections live only in the working tree.** `git status` shows `M docs/migration-from-lok.md`; the unstaged diff already removes the false `trace` claim, adds a "New in loker" section listing `loker trace <run_id>`, and reorganizes the command translation table. These edits must be committed (folded into the CLO-316 commit chain) before opening the PR — otherwise the PR ships the broken HEAD copy.

## Out of Scope / Deferred
- **F3 (low) — Verification appendix asymmetry.** "New in loker" lists ~14 commands; appendix only shows `--help` evidence for a subset. Tighten in this PR if convenient, otherwise track as polish.
- **F4 (low) — Phases row missing from concept mapping.** Design Goal 1 enumerates "workflows, phases, backends, run artefacts, and config paths"; current table covers all but phases. Either add a one-row entry mapping legacy `lok`'s implicit single-phase model to loker's per-phase orchestration, or note phases are out of scope.
- **F5 (low) — README link placement.** The migration callout under "Design docs & roadmap" works; moving it closer to install/quickstart is optional.

## False Positives / Tooling Artifacts
- **Codex review failure** — heredoc with unescaped backticks inside single-quoted `$(cat <<EOF ... EOF)` produced a shell parse error before any model was called. Not a code defect; the runner script itself needs fixing.
- **Gemini review failure** — same root cause as Codex. Not a code defect.

## Recommendation
PROCEED_WITH_FIXES. Bounded fixes:
1. Stage and commit the existing working-tree edits to `docs/migration-from-lok.md` (resolves F1+F2) — recommend a single commit `docs(CLO-316): correct trace claim and reorganize migration table`.
2. Optionally fold in F3 (add missing `--help` lines to appendix) and F4 (add phases row) before pushing; both are small and align with design acceptance criteria.
3. Separately, fix the Codex/Gemini reviewer scripts' heredoc quoting (out-of-scope for this PR but worth a follow-up so future CLO tasks get real two-model coverage instead of falling through to Claude only).

After step 1, the diff is docs-only, scope-correct, and `make check` is unaffected — safe to open the PR.
