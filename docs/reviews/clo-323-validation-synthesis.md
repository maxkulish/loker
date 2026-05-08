# Pre-PR validation: clo-323

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-08
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc quoting error (`unexpected EOF while looking for matching '` in invocation script — backticks inside heredoc broke the `$(cat <<EOF ... EOF)` capture). No model output produced. |
| Gemini | REVIEW_FAILED | Same shell heredoc quoting error in invocation script. Both `gemini-3.1-pro-preview` and fallback `gemini-2.5-pro` were never reached. |
| Claude (fallback) | OK | Full findings produced; UI tests pass 42/42; `make check` clippy gate clean. |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 [HIGH] XSS via `innerHTML` in live SSE renderer** (`templates/run_detail.html:104`). Static path escapes via Askama; live path interpolates arbitrary JSONL fields (LLM output, tool stdout, hook messages) into `div.innerHTML`. This is a security regression vs. the static render and is unacceptable for a daemon UI that displays orchestrator output. Replace with `document.createElement` + `textContent`.
- **F2 [MEDIUM] Idle SSE clients leak `notify` watcher + tokio task** (`src/ui/sse.rs:69-79`). Disconnects only surface after `tx.send` fails, which only runs on a filesystem event; for completed/idle runs (the common case), watchers persist indefinitely. Directly contradicts PRD acceptance "100 sequential connections do not exhaust FDs". Fix: `tokio::select!` between `tx.closed()` and `event_rx.recv()`.
- **F3 [MEDIUM] Offset desync on partial-line writes** (`src/ui/sse.rs:73-79`, `src/ui/trace_reader.rs:read_from_offset`). `notify` can fire mid-line; `.lines()` yields the partial line, offset advances past it, remaining bytes are lost on next write. Also assumes `\n` separators / one newline per line. Fix: scan to last `\n`, advance offset only to that point, buffer trailing bytes.
- **F4 [MEDIUM] Render→connect race drops events** (`src/ui/routes.rs:run_trace_sse`, `templates/run_detail.html`). Static render at T1, EventSource opens at T2; events in (T1, T2) are silently dropped. Contradicts PRD "<1s latency" guarantee under load. Fix: emit last-rendered byte offset in template, pass via `Last-Event-ID` or query param.
- **F5 [MEDIUM] EventSource reconnects forever on terminal/404 runs** (`templates/run_detail.html`). Browsers retry every ~3s indefinitely; for every viewed completed run this becomes a steady pulse of failing requests, and 404 returns HTML the client discards each retry. Fix: server emits `event: end` and client closes; on terminal runs (summary.json present), do not open EventSource at all.

## Out of Scope / Deferred
- **F6 [LOW] HTML error bodies on SSE endpoint** — small content-type cleanup; nice to do but not a correctness blocker.
- **F9 [LOW] `runId` injected into `<script>` via HTML-entity escape** — currently safe because `run_id` is sanitized upstream; refactor to JSON-escaped data attribute can be a follow-up.
- **F10 [LOW] Typo "currenty" in `docs/discovery/clo-323.md:5`** — cosmetic.

## False Positives / Tooling Artifacts
- **F7 [LOW] Pre-existing clippy errors in `src/strategy/verify/human_verifier.rs:1269,1275`** — surfaces only under `--all-targets`, which is not part of `make check`. Not introduced by this branch; file a separate ticket.
- **F8 [LOW] Unused `offset` binding in tests in `src/ui/sse.rs:178,184`** — only triggers under `-D warnings --all-targets`, which the project's gate does not run. Trivially `_offset` rename if/when CI tightens.
- **Codex / Gemini "review failed"** — both failures are bash heredoc-quoting bugs in the wrapper scripts, not signal about the code under review. Worth fixing the `.pi/` invocation scripts (escape backticks inside the `$(cat <<EOF...EOF)` block) so future synthesis isn't single-reviewer.

## Recommendation
PROCEED_WITH_FIXES. The branch is structurally sound — `make check` is green, all 42 UI tests pass, and every Must Fix item is a localized change of <10 lines confined to `templates/run_detail.html`, `src/ui/sse.rs`, `src/ui/trace_reader.rs`, and `src/ui/routes.rs`. The bounded fix iteration is: (1) replace `innerHTML` with `createElement`/`textContent` in the live renderer, (2) add `tokio::select!` on `tx.closed()` to the watch loop, (3) make offset advancement newline-anchored with a partial-line buffer, (4) pass last-rendered offset from template to SSE handler, (5) emit a terminal `event: end` and close client-side when runs are complete. Separately, fix the bash heredoc bugs in `.pi/` codex/gemini invocation wrappers so synthesis isn't reduced to a single reviewer next round.
