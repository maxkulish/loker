# Pre-PR validation: CLO-312

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc parse error from backticks in `$BRANCH` ("\`loker trace …\`"); the runner shelled out before the model ever ran. Tooling artifact, not a code defect. |
| Gemini | REVIEW_FAILED | Same root cause — backticks in `$BRANCH` broke the heredoc. |
| Claude (fallback) | OK | Full review with 7 findings against `src/commands/trace.rs` and the design doc. |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 [HIGH] Error spans never render red/bold.** `colorize_line` matches strings (` ERROR `, `[error]`) that `format_span` never emits — real errors render as `[backend_error] …`. The design's headline goal ("errors visually highlighted") is unmet. Fix: drive coloring from the parsed `Status` / presence of `error.kind`, not substring sniffing. Add an ANSI-escape assertion (e.g. with `ColorChoice::Always`).
- **F2 [MEDIUM] `loker.min_responses_met=false` shortfall not highlighted.** Field never read; design-listed test `renders_min_responses_shortfall_highlighted` missing. Fix: read the field, force warn/error class, append `[shortfall]` marker, add the test.
- **F3 [MEDIUM] Truncation drops content with no `…`.** `fit("[strategy_failed] all attempts exhausted, …", 30)` silently chops to `[strategy_failed] all attempts` — exact opposite of the design's "append `…` so the operator sees it was truncated". Snapshot at `tests/snapshots/...:13` freezes the bug. Fix: make `fit` append `…` when it actually cuts; refresh the snapshot.
- **F4 [LOW] Dead `phase.finished` branch / misclassification.** `SpanKind::from_name` returns `Phase` for real `phase.<x>.finished` spans (which the writer at `src/trace/memory.rs:325` emits) because the generic `phase.*` arm fires first. Fix: check `ends_with(".finished")` before the generic `phase.` branch.
- **F5 [LOW] Byte/char mismatch on 80-col limit.** Impl uses `line.len()` (bytes) + appends 3-byte `…`; integration test asserts `chars().count() <= 80`. Pick one (recommend bytes, reserve room for the suffix) and align.
- **F6 [LOW] Missing color/status tests called out by design.** No assertions on ANSI escape codes — that's why F1 slipped through. Add `renders_backend_error_in_red_bold` and the `status_derivation_table` cases.

## Out of Scope / Deferred
- **F7 [INFO] `pub fn run` signature drift from design.** Functionally equivalent (`resolve_run_dir` lifted to `main.rs`, `Option<ColorChoice>` vs `ColorChoice`). Update the design doc in a separate doc-tidy pass; do not gate the PR on it.

## False Positives / Tooling Artifacts
- Both Codex and Gemini wrappers failed because the orchestrator embedded literal backticks (`` `loker trace <run_id>` ``) into the `BRANCH` shell variable inside a `cat <<EOF` heredoc, so the shell tried to execute `loker trace <run_id>` as command substitution. Not a finding against the branch. Recommend the orchestrator either single-quote `BRANCH`, escape backticks, or strip them before substitution before the next review run.

## Recommendation
PROCEED_WITH_FIXES. One bounded follow-up commit on this branch covering: (1) status-driven colorization with ANSI-escape tests, (2) `min_responses_met` shortfall highlighting + missing test, (3) truncation `…` suffix in `fit` plus snapshot refresh, (4) reorder `SpanKind::from_name` so `.finished` wins over the generic `phase.` arm, (5) reconcile byte-vs-char width budget. F7 (signature drift) and the reviewer-tooling backtick bug can be handled outside this PR.
