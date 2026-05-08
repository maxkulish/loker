# Pre-PR validation: clo-323

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-08
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

All UI tests pass (42/42). `cargo clippy -- -D warnings` (the actual `make check` gate) is clean. The clippy errors I noticed earlier require `--all-targets` and are in pre-existing test code.

## Findings

### F1 [HIGH] XSS in live SSE event renderer (innerHTML interpolation)
**Where:** `templates/run_detail.html:104` (the live `addEventListener("trace_event", ...)` block)
**What:** The static path escapes via Askama. The live path builds HTML by string-interpolation: `div.innerHTML = \`${ts ? \`<strong>${ts}</strong> \` : ""}[${et}] ${sum}\``. `ts`/`et`/`sum` come from arbitrary JSON in `trace.jsonl`. Since orchestrator events can include LLM outputs, tool stdout, prompt text, or hook messages, an attacker (or simply an LLM emitting `<img onerror=...>`) can inject script into the daemon UI. This is a regression vs. the static render.
**Suggested fix:** Build DOM nodes with `document.createElement` + `textContent`, e.g.
```js
const ts_el = document.createElement("strong"); ts_el.textContent = ts;
div.append(ts_el, ` [${et}] `, sum); // textContent for sum
```
Never assign untrusted data to `innerHTML`.

### F2 [MEDIUM] Idle SSE clients leak `notify` watchers until next FS event
**Where:** `src/ui/sse.rs:69-79` (`watch` loop)
**What:** Disconnect is only detected when `tx.send(...)` fails, which only runs after a filesystem event. If the trace file is quiet (run finished, or just slow), each disconnected client leaves an inotify watcher + tokio task alive indefinitely. Directly contradicts the PRD acceptance criterion "100 sequential connections do not exhaust file descriptors" when the watched file is idle (the most common case for completed runs).
**Suggested fix:** Race the FS receiver against close detection:
```rust
loop {
    tokio::select! {
        _ = tx.closed() => return Ok(()),
        ev = event_rx.recv() => match ev { Some(_) => { /* read & send */ } None => return Ok(()) },
    }
}
```

### F3 [MEDIUM] Offset tracking can desync on partial-line writes
**Where:** `src/ui/sse.rs:73-79` and `src/ui/trace_reader.rs:read_from_offset`
**What:** The watch loop advances `self.offset = new_offset` (the file's current `len()`) regardless of whether the last line ended in `\n`. If `notify` fires after a writer has written half a line, `read_from_offset` reads to EOF, splits with `.lines()` (which yields the trailing partial line as a complete line), and pushes the offset past the partial bytes — so the rest of that line, when finally written, will be skipped. Also, the per-line `current_pos += line_len + 1` assumes UNIX `\n` separators and one newline per line; broken if `.lines()` swallows a `\r\n`.
**Suggested fix:** Read into a `Vec<u8>`, scan forward only to the last `\n`, advance the offset to that position, and buffer any trailing partial bytes for the next iteration.

### F4 [MEDIUM] Race window between page render and SSE connect drops events
**Where:** `src/ui/routes.rs:run_trace_sse` (initial offset choice) + `templates/run_detail.html` (no offset handoff)
**What:** Static render reads last N events at time T1. The browser opens EventSource at T2; the server then captures EOF as starting offset. Events written in the (T1, T2) window are lost. For the documented "<1s latency" goal, this means a writer hitting the file every few hundred ms can drop events on slow page loads.
**Suggested fix:** Have the static template emit the byte offset of the last event it rendered (e.g., into a `data-trace-offset` attribute), and pass it in the SSE URL or `Last-Event-ID` so the server starts from there.

### F5 [MEDIUM] EventSource auto-reconnects forever on completed/404 runs
**Where:** `templates/run_detail.html` (no terminal/close handling)
**What:** For runs whose `trace.jsonl` is missing or whose run has completed, the handler returns 404 HTML or eventually nothing useful. Browsers' `EventSource` retries indefinitely (default ~3s), which produces a steady pulse of failing requests for every viewed completed run — and the 404 path returns HTML, which the client will throw away on every retry.
**Suggested fix:** When the run is terminal (e.g., `summary.json` exists), either don't open EventSource at all client-side, or have the server emit `event: end\ndata: \n\n` and instruct the client to `es.close()` on that event. Also return SSE-formatted bodies (or empty) on the error paths instead of HTML.

### F6 [LOW] Error responses are HTML in an SSE endpoint
**Where:** `src/ui/routes.rs:run_trace_sse` lines that return `templates::ErrorTemplate { ... }.into_response()`
**What:** Writes HTML with the wrong content-type contract for an `/sse` endpoint. Functional, but unhelpful for clients (which will just observe an `error` event).
**Suggested fix:** Return `(StatusCode::X, "msg")` plaintext, or empty bodies. Keep status codes accurate so EventSource gives up on 4xx instead of reconnecting on 5xx.

### F7 [LOW] Pre-existing clippy errors under `--all-targets`
**Where:** `src/strategy/verify/human_verifier.rs:1269,1275` (not modified in this branch)
**What:** `cargo clippy --all-targets -- -D warnings` fails with `bool_assert_comparison` and `useless_vec`. The project gate (`make check` → `cargo clippy -- -D warnings` without `--all-targets`) does not catch these, so this PR is not regressing them, but external CI might.
**Suggested fix:** Out of scope here, but file a follow-up to either fix the asserts or run `--all-targets` in `make check`.

### F8 [LOW] Unused destructured binding in test
**Where:** `src/ui/sse.rs:178,184` — `let (offset, line) = rx.recv().await...` (offset unused) and `let Ok((offset, line))` similar.
**What:** Triggers `unused_variables` if the test build ever runs under `-D warnings --all-targets`.
**Suggested fix:** Rename to `_offset`.

### F9 [LOW] `runId` interpolated into JS via HTML escaping
**Where:** `templates/run_detail.html`: `const runId = "{{ run_id }}";`
**What:** Inside `<script>`, HTML entities are not decoded — so Askama's `&quot;`/`&amp;` escaping happens to be safe, but quirky and easy to break later. Not currently exploitable because `run_detail`'s sanitizer rejects `/`, `\`, `..`. Still, putting user-controlled data into JS via HTML escape is a bad pattern.
**Suggested fix:** Render via `JSON.parse('...')` with proper JSON escaping, or use a `data-` attribute on a DOM node and read it.

### F10 [LOW] Discovery doc typo
**Where:** `docs/discovery/clo-323.md:5` ("currenty")
**What:** Cosmetic.
**Suggested fix:** "currently".

## Verdict
**rework**

The feature works on the happy path and the test suite + the project's `make check` gate is green, so the foundation is sound. But F1 (XSS via innerHTML over LLM-derived strings) is unacceptable for a developer tool that displays orchestrator output, and the combination of F2 (watcher leak when the file is idle) and F3 (offset desync on partial writes) directly contradicts two of the PRD's three acceptance criteria ("100 sequential connections do not exhaust FDs" and "events appear in the UI within 1 second"). F4 also undermines the live-streaming guarantee. Each fix is small (under ~10 lines), so this is a quick rework — but as written I would not merge.
