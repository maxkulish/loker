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

## Re-validation

All five Must Fix Before PR items were already implemented in the branch commits (74f9bad, bda7a77, 767d9f1, fa1351d, 192fa62) and confirmed present:

- **F1 (XSS)**: DOM nodes constructed with `createElement` + `textContent`; no `innerHTML` with untrusted data.
- **F2 (watcher leak)**: `tokio::select!` races `tx.closed()` against `event_rx.recv()` — watchers tear down on disconnect even for idle files.
- **F3 (offset desync)**: `read_from_offset` scans to last `\n` via `rposition`, advances offset only to that boundary.
- **F4 (render→connect race)**: Template emits `data-last-offset`; SSE URL includes `?offset=` to fill the gap.
- **F5 (EventSource reconnect)**: Template guards EventSource on `is_terminal`; server emits `event: end` for terminal runs; client calls `es.close()` on receipt.

Additional cleanup applied: replaced the one remaining `innerHTML = ""` with `textContent = ""` for clearing the empty-state placeholder.

`make check` is green (fmt + clippy + test, all passing).

**Re-validation verdict**: approve
