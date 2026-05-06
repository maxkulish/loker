# Pre-PR validation: CLO-312

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

Confirmed: the colorize patterns don't match anything `format_span` actually emits, and `min_responses_met` isn't referenced anywhere in the new code.

## Findings

### F1 [HIGH] Error spans never render in red/bold — colorize matches strings the formatter never produces
**Where:** src/commands/trace.rs:105-122 (vs. format_span at src/commands/trace.rs:243-249)
**What:** `colorize_line` looks for ` ERROR `, ` FAIL `, `[error]`, or a leading `<error>`. None of those appear in real output: the kind label is lowercase `error`/`fail`-free, and the error status is formatted as `[<error.kind>] <msg>` (e.g. `[backend_error] timeout`, `[strategy_failed] …`), not `[error]`. So the red/bold branch is unreachable for actual spans, breaking the design goal "Errors … visually highlighted (red / bold)". Tests don't catch it because `renders_error_span` only asserts substring presence, not ANSI codes.
**Suggested fix:** Drive coloring from the parsed `Status` enum (or from the presence of `error.kind` / `error.message` / non-success outcome), not from substring sniffing of the rendered line. Add an explicit assertion such as `assert!(out.contains("\x1b[31"))` (or `colored::control::set_override(true)` + a stripped-vs-raw comparison) in the error-span test.

### F2 [MEDIUM] `loker.min_responses_met` shortfall highlighting not implemented
**Where:** src/commands/trace.rs:171-249
**What:** The design (Goals + "Status" derivation in Architecture) lists `loker.min_responses_met = false` as a highlighted condition, with a planned test `renders_min_responses_shortfall_highlighted`. The field is never read in `format_span` and the test does not exist. Operators won't see the shortfall they specifically need to spot.
**Suggested fix:** Read `obj.get("loker.min_responses_met")`; if `Some(false)`, force the status branch into the warn/error visual class (yellow or red+bold) and append a marker like `[shortfall]`. Add the planned unit test.

### F3 [MEDIUM] Per-field truncation drops content silently — no `…` until the whole line overflows
**Where:** src/commands/trace.rs:266 and `fit` at 278-291
**What:** `fit(&status, 30)` chops `[strategy_failed] all attempts exhausted, no passing verify` to `[strategy_failed] all attempts` with no ellipsis. The fixture snapshot freezes this in (tests/snapshots/...:13). The design states truncation should be applied with `…` to variable fields *before* line assembly so the user can see truncation occurred. As-is, an operator reading the trace can't tell the message was cut off.
**Suggested fix:** Have `fit` (or a wrapper) append `…` when it actually truncates, accounting for its byte width when computing the cut. Update the snapshot intentionally.

### F4 [LOW] Dead branch in `SpanKind::from_name`
**Where:** src/commands/trace.rs:137-156
**What:** Line 137 already returns `Finished` for `phase.finished`, and lines 139-144 return either `Phase` or `Error` for any other `phase.*` name. The later `name == "phase.finished" || name.ends_with(".finished")` branch (line 151) is unreachable for `phase.*` inputs. As a side effect, real `phase.x.finished` spans (the writer emits these — see src/trace/memory.rs:325) are classified as `Phase`, not `Finished`, contradicting the dead-code intent.
**Suggested fix:** Reorder so `name.ends_with(".finished")` is checked before the generic `phase.` branch, or treat `phase.finished` and `phase.<x>.finished` symmetrically. Drop the duplicate `phase.finished` literal.

### F5 [LOW] 80-column limit mixes bytes (impl) and chars (test)
**Where:** src/commands/trace.rs:270-272 and tests/trace_pretty.rs:16-21
**What:** The integration test asserts `line.chars().count() <= 80`, but `format_span` checks `line.len() > 80` (bytes) and then appends `…` (3 bytes). For pure-ASCII input this can yield a 82-byte / 80-char line that satisfies the test but exceeds the design's "we never produce a >80 byte line under any input" guarantee.
**Suggested fix:** Pick one — either widen the spec to "≤80 columns" and keep the char-based test, or compute and trim using bytes plus reserve room for the `…` suffix.

### F6 [LOW] Test coverage gap on color codes and design-listed cases
**Where:** src/commands/trace.rs:309-466
**What:** Design enumerates `renders_backend_error_in_red_bold`, `renders_min_responses_shortfall_highlighted`, and `status_derivation_table`; none are present. No test asserts ANSI escapes are produced for any branch, which is exactly why F1 slipped through.
**Suggested fix:** Add the missing tests, asserting raw ANSI escape substrings against `ColorChoice::Always`.

### F7 [INFO] `pub fn run` signature drift from design
**Where:** src/commands/trace.rs:84 vs. design §"Public API surface"
**What:** Design declares `run(run_id: &str, json: bool, color: Option<ColorChoice>)`; impl uses `run(path: &Path, json: bool, color: ColorChoice)` with `resolve_run_dir` lifted into `main.rs`. Functional outcome is the same; flagging only because reviewers were told to expect the design shape. Either is fine; pick one and update the doc.

## Verdict
rework

Scope, schema fidelity, streaming behavior, and test scaffolding are all correct, and the new module is cleanly isolated with no dependency churn — but the central design promise that operators can spot errors at a glance is unmet (F1: red/bold path is dead code) and a documented highlight (F2: `min_responses_met`) was dropped on the floor; F3 compounds this by silently truncating the very error messages the user is trying to read. None of these need a redesign — they're a small follow-up commit plus a snapshot refresh — but they should land before merge so we don't ship a pretty-printer that under-prints exactly the cases it was built for.
