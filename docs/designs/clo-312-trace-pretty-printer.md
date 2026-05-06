# Design: CLO-312 - `loker trace <run_id>` pretty-printer

## Problem

Per the discovery report, every loker run writes an append-only `trace.jsonl` to its run directory containing OpenTelemetry GenAI-compatible spans, but the CLI offers zero way to read them back — operators currently rely on `cat`, `jq`, and `grep` to figure out which phase failed, which backend errored, or whether a `min_responses` shortfall occurred. This blocks debug workflows for developers and operators today and prevents test authors from guarding against trace-format regressions; T-043 is the scheduled Phase 9 unblock, with the writer (CLO-293) and `resolve_run_dir()` (CLO-310) both already shipped.

## Goals / Non-goals

### Goals

- New `loker trace <run_id>` subcommand that pretty-prints `runs/<run_id>/trace.jsonl` in chronological (file) order, one span per line, fitting in 80 columns.
- `--json` flag streams the file verbatim to stdout for piping to `jq` / `grep`.
- Bare-name, relative, and absolute `<run_id>` resolved through the existing `resolve_run_dir()` helper.
- Errors (`error.kind` / `error.message`) and `min_responses` shortfalls are visually highlighted (red / bold) using the existing `colored` dependency.
- Snapshot test (`insta`) covers happy path, backend error, and verify-failure span sequences.
- Unit-testable formatter so iteration does not require spawning the binary.
- Streaming line-by-line read (constant memory regardless of trace size).
- `make check` clean.

### Non-goals

- Real-time follow / `-f` mode (Phase 12 SSE work).
- Span filtering DSL or query language.
- Sorting modes other than chronological/file order.
- Aggregate statistics (totals, p95s) — owned by the future summary module.
- Reusable web/TUI rendering surface beyond what falls out of module separation.
- New dependencies — everything (`clap`, `colored`, `serde_json`, `chrono`, `insta`) is already in the workspace.

## Architecture

The implementation lives in a new module `src/commands/trace.rs` reachable via a `src/commands/mod.rs` entry. `main.rs` adds a `Commands::Trace` variant and dispatches to a thin handler; all parsing, formatting, and I/O live in the module so they can be unit-tested without spawning the binary.

Data flow:

```
+----------------------+
| CLI: loker trace ID  |
| [--json]             |
+----------+-----------+
           |
           v
+----------------------+
| resolve_run_dir(id)  |  (existing, src/main.rs / run_state)
+----------+-----------+
           |
           v
+----------------------+      --json     +-----------------+
| open trace.jsonl     +---------------> | copy stream     |
| BufReader<File>      |                 | to stdout raw   |
+----------+-----------+                 +-----------------+
           | default
           v
+----------------------+
| TracePrinter::render |   line-by-line: read -> parse Span -> format -> writeln!
+----------+-----------+
           |
           v
+----------------------+
|  RenderedLine (<=80) |
|  colored on stderr-  |
|  aware stdout        |
+----------------------+
```

Concrete Rust types in `src/commands/trace.rs`:

- `pub struct TraceArgs { pub run_id: String, pub json: bool }` — clap-derived args sub-struct (or fields inlined into the `Commands::Trace` variant — see Open Questions).
- `pub struct TracePrinter<W: Write> { writer: W, color: ColorChoice }` — owns the output sink and color policy. The generic `W` lets unit tests pass a `Vec<u8>` and assert on the rendered string.
- `enum SpanKind { Phase, Backend, Aggregator, Verify, Finished, Error }` — derived from the span's `name` field.
- `enum Status { Pass, Fail, Repair, Error, None }` — derived from `loker.outcome`, `error.kind`, `error.message`, and (for shortfalls) `loker.min_responses_met`.
- `struct ParsedSpan<'a>` — a thin view over the parsed `serde_json::Value` that pulls the specific fields the formatter needs. We do not introduce a strongly-typed mirror of the schema — the schema lives in `docs/schemas/trace_span.schema.json` and the writer/sink already enforces it; this consumer needs only a handful of fields and tolerating unknown extra keys is desirable.

Module placement:

```
src/
  commands/
    mod.rs        # pub mod trace;
    trace.rs      # TracePrinter + run_trace() entry point
  main.rs         # Commands::Trace { run_id, json } dispatch -> commands::trace::run(...)
  lib.rs          # add `pub mod commands;` (or keep crate-private if main.rs is the only consumer)
```

Per the project rule that the public surface lives in `src/lib.rs`, the new module is added there only if it needs to be exported; otherwise it stays crate-private and is reached from `main.rs` via the binary's normal module tree. See Open Questions.

Field extraction (from the discovery report and the schema):

- Timestamp source: `start_time_unix_nano` (preferred) or top-level `time` if present — formatted as `HH:MM:SS` to keep within 80 cols.
- Phase: `attributes."loker.phase"`.
- Span kind: substring match on `name` (`phase.start`, `phase.finished`, `backend.call`, `aggregator.fold`, `verify.<hook>`, `phase.error`).
- Backend / hook: `attributes."gen_ai.system"` for backend spans, or the verify hook name parsed out of `name` (`verify.shell`, `verify.judge`, `verify.test_runner`).
- Latency: `attributes."duration_ms"`.
- Tokens: `attributes."gen_ai.usage.input_tokens"` + `attributes."gen_ai.usage.output_tokens"`.
- Status: `attributes."loker.outcome"`, falling back to error fields for `error` and to `loker.min_responses_met` for shortfall highlighting.

Truncation: a small helper `fn fit(s: &str, max: usize) -> String` truncates with a trailing `…` when the rendered line would exceed 80 cols. Truncation is applied to the most variable fields first (backend id, hook name, error message) before the line is assembled, not after, so we never produce a >80 byte line under any input.

Color policy: respect `NO_COLOR` and non-tty stdout — `colored::control::SHOULD_COLORIZE` is already the rule used elsewhere in the codebase. Snapshot tests force colors off so the rendered output is stable. A `--color <auto|always|never>` CLI flag overrides both auto-detection and the env var for scripting.

## Public API surface

```rust
// src/commands/mod.rs
pub mod trace;

// src/commands/trace.rs
use std::io::Write;
use std::path::Path;

use anyhow::Result;

pub struct TracePrinter<W: Write> {
    writer: W,
    color: ColorChoice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

impl<W: Write> TracePrinter<W> {
    pub fn new(writer: W, color: ColorChoice) -> Self;

    /// Read each line of `trace.jsonl` from `path`, parse, format, and write
    /// one rendered line per span. Malformed lines are emitted as a `<error>`
    /// marker and processing continues.
    pub fn render_file(&mut self, path: &Path) -> Result<()>;

    /// Render a single already-parsed JSON span. Exposed for unit tests.
    pub fn render_span(&mut self, span: &serde_json::Value) -> Result<()>;
}

/// Stream `path` to `writer` byte-for-byte. Used by `--json` mode.
pub fn passthrough<W: Write>(path: &Path, writer: &mut W) -> Result<()>;

/// CLI entry point invoked from `main.rs`. Resolves the run id, picks
/// between pretty-print and `--json` modes, and writes to stdout.
pub fn run(run_id: &str, json: bool, color: Option<ColorChoice>) -> Result<()>;
```

The `Commands` enum in `src/main.rs` gains:

```rust
// src/main.rs (clap-derived)
#[derive(clap::Subcommand)]
enum Commands {
    // ... existing variants ...

    /// Pretty-print the trace.jsonl from a run directory.
    Trace {
        /// Run id (bare name, relative, or absolute path), as accepted by `loker resume`.
        run_id: String,

        /// Stream raw JSONL to stdout instead of pretty-printing.
        #[arg(long)]
        json: bool,

        /// Force color output (overrides auto-detection and NO_COLOR).
        /// Accepts `auto`, `always`, or `never`.
        #[arg(long, value_enum)]
        color: Option<ColorChoice>,
    },
}
```

Dispatch in `main.rs` is a one-liner:

```rust
Commands::Trace { run_id, json, color } => commands::trace::run(&run_id, json, color),
```

## Test plan

All tests live in `src/commands/trace.rs` (unit) plus one integration file under `tests/`. `wiremock` is not relevant — there are no backend calls in this code path.

Unit tests in `src/commands/trace.rs`:

- `fn renders_happy_path_phase_backend_aggregator_verify_finished()` — feeds five hand-built `serde_json::Value` spans into `render_span`, asserts the rendered string contains expected fields and is <=80 cols per line.
- `fn renders_backend_error_in_red_bold()` — asserts that the status and error message use the red/bold codes when `ColorChoice::Always` is set.
- `fn renders_min_responses_shortfall_highlighted()` — span with `loker.min_responses_met = false` triggers the same highlight as a hard error.
- `fn truncates_overlong_backend_with_ellipsis()` — synthetic backend id of 200 chars produces a line <=80 cols ending in `…`.
- `fn fit_helper_handles_unicode_boundaries()` — multi-byte chars do not split mid-codepoint.
- `fn malformed_line_does_not_abort_render()` — a line of garbage between two good spans yields a `<malformed>` placeholder (with an `eprintln!` warning to stderr) and the next span still renders.
- `fn span_kind_is_inferred_from_name()` — table-driven over the six `SpanKind` variants.
- `fn status_derivation_table()` — table-driven over `loker.outcome` x `error.*` combinations.

Integration / snapshot test: `tests/trace_pretty.rs`

- `fn snapshot_default_render_against_fixture()` — reads `tests/fixtures/trace/happy_path_and_errors.jsonl`, runs `TracePrinter::render_file` with `ColorChoice::Never`, snapshots via `insta::assert_snapshot!`. The fixture contains 6–10 spans covering happy path, backend error, and verify failure (the discovery debt item — created in the implement phase, not invented here).
- `fn json_passthrough_streams_file_verbatim()` — calls `passthrough` against the same fixture, asserts byte-for-byte equality with the file contents.
- `fn json_passthrough_errors_when_file_missing()` — non-existent run dir returns an `anyhow::Error` with a clear message.
- `fn cli_smoke_run_id_resolution()` — invokes the binary with `assert_cmd` against a temp run dir created via the existing `RunDir::create` helper, confirms exit 0 for both modes. (This mirrors `tests/explain_cli.rs`.)

Manual verification:

1. `cargo run --bin loker -- ask "hello"` to produce a real run.
2. `cargo run --bin loker -- trace <run_id>` — confirm one line per span, colors visible in tty, no line wraps under 80 cols.
3. `cargo run --bin loker -- trace <run_id> --json | jq .name` — confirm raw JSONL passthrough works.
4. `NO_COLOR=1 cargo run --bin loker -- trace <run_id>` — confirm colors are suppressed.
5. `loker trace not_a_run_id` — confirm clean error message from `resolve_run_dir`.
6. `make check` — must be clean.

## Migration / rollout

Nothing to migrate. This is a pure addition:

- New subcommand on the `clap` enum — no existing command's behavior changes.
- No new dependencies.
- No config schema changes (`lok.toml` untouched).
- No changes to `trace.jsonl` writer (`src/trace/writer.rs`) or schema (`docs/schemas/trace_span.schema.json`).
- No feature flag needed; the command is on by default in the next release.
- Rollout order: land the module + handler + fixture + tests in one PR; release piggybacks on the next regular `make release`.

## Open questions

- **Module exposure in `src/lib.rs`.** The project convention is that the public surface lives in `lib.rs` with private modules using `#![allow(dead_code)]` at the lib root. Should `commands::trace` be re-exported from `lib.rs` (so the formatter can be reused by a future `loker tui` or web dashboard, as Approach B's discovery rationale suggests), or stay crate-private and reachable only from the binary? Tradeoff: re-exporting commits us to a public API surface; keeping it private is cheaper now but means a future TUI must duplicate or extract later.
- **Argument shape: inline vs. struct.** `Commands::Trace { run_id, json, color }` inlines the args in the enum variant (matches `loker resume`'s style per CLO-310). Defining a separate `TraceArgs` struct is cleaner if we expect more flags soon (e.g., `--width`). For v0 there are only three args; if Phase 12 follow-mode lands as `loker trace -f` later, we will refactor at that point. Confirm we are happy with the inline shape now.
- **Color override flag.** The PRD did not originally specify a `--no-color` flag. The `--color <auto|always|never>` CLI flag (applied in review F1) closes the scripting gap. Resolve this question in favor of the three-state flag.
- **Span kind inference source.** The PRD says "inferred from `name`", but the schema may also expose a discriminator under `attributes`. We default to parsing the dotted prefix of `name` (`phase.*`, `backend.*`, `aggregator.*`, `verify.*`). If the schema gains a dedicated kind field later, we will switch — but is the dotted-prefix assumption stable for v0?
- **Fixture authorship boundary.** Discovery debt explicitly says the fixture is created during implementation. Confirm reviewers do not expect the fixture to be specified byte-for-byte in this design — only the three paths it must cover (happy, backend error, verify failure) and the snapshot stability requirement (fixed timestamps and span ids).
