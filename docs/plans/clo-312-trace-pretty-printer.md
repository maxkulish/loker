# Plan: CLO-312 `loker trace <run_id>` pretty-printer

## Context
- **Design:** docs/designs/clo-312-trace-pretty-printer.md
- **Discovery:** docs/discovery/clo-312.md
- **Linear:** https://linear.app/cloud-ai/issue/CLO-312/t-043-loker-trace-run-id-pretty-printer

## Sub-tasks

### ST1 — Scaffold `src/commands/trace.rs` module
**Files:** `src/commands/mod.rs` (new), `src/commands/trace.rs` (new), `src/lib.rs`
**Acceptance:** `cargo test commands::trace::unit::scaffold_compiles` stub passes + `make check` green.
**Estimate:** S

Create the module tree:
1. `src/commands/mod.rs` — `pub mod trace;`
2. `src/commands/trace.rs` — empty `TracePrinter<W>` struct + `ColorChoice` enum.
3. Re-export in `src/lib.rs` if needed for `tests/` access (discover during compile).

### ST2 — Implement `render_span` (core formatting)
**Files:** `src/commands/trace.rs`
**Acceptance:** `cargo test commands::trace::unit::render_span_happy_path` + `render_span_error` pass.
**Estimate:** M

- Implement `SpanKind::from_name()` inference (phase/backend/aggregator/verify/finished/error).
- Implement `Status::from_outcome_and_error()` derivation (pass/fail/repair/error/none).
- Implement `TracePrinter::render_span()` — format one `serde_json::Value` into a single `≤80` col line.
- Colorization via `colored` (red/bold for errors, green for pass).
- Truncate variable-length fields (backend id, hook name, error message) with `…`.
- `fit()` unicode-aware truncation helper.

### ST3 — Implement `render_file` (streaming reader)
**Files:** `src/commands/trace.rs`
**Acceptance:** `cargo test commands::trace::unit::render_file_fixture` passes.
**Estimate:** S

- Open `trace.jsonl` line-by-line via `BufReader`.
- Parse each line with `serde_json::from_str`.
- On parse failure: emit `<malformed>` to stdout + `eprintln!` warning to stderr; continue.
- Pipe parsed span into `render_span`.

### ST4 — Implement `--json` passthrough + CLI wiring
**Files:** `src/main.rs`, `src/commands/trace.rs`
**Acceptance:** `cargo test commands::trace::unit::passthrough_verbatim` passes.
**Estimate:** S

- Implement `passthrough(path, writer)` as `std::io::copy` from `File` to `W`.
- Add `Commands::Trace { run_id, json, color }` variant to the clap enum.
- Implement `commands::trace::run()` — resolve run_id to `trace.jsonl`, dispatch to passthrough or pretty-print.
- Handle `--color <auto|always|never>` override.

### ST5 — Create fixture trace + snapshot test
**Files:** `tests/fixtures/trace/happy_path_and_errors.jsonl` (new), `tests/trace_pretty.rs` (new)
**Acceptance:** `cargo test --test trace_pretty` passes (snapshot test green).
**Estimate:** M

- Fixture: ~8 spans covering happy path (design → backend → aggregator → verify pass → finished), backend error (claude timeout), verify failure (`make check` failed).
- Use fixed timestamps + span IDs for snapshot stability.
- Snapshot via `insta::assert_snapshot!` with `ColorChoice::Never`.

### ST6 — Integration tests + edge cases
**Files:** `tests/trace_pretty.rs`, `src/commands/trace.rs`
**Acceptance:** `cargo test --test trace_pretty` passes (all tests green).
**Estimate:** M

- `json_passthrough_streams_file_verbatim` — raw passthrough byte-for-byte match.
- `json_passthrough_errors_when_file_missing` — clean `anyhow::Error` output.
- `cli_smoke_run_id_resolution` — `assert_cmd` spawn of the binary on real RunDir.
- Unit tests from design doc: `fit_helper_unicode`, `malformed_with_stderr`, `span_kind_table`, `status_derivation_table`, `truncation_edge_cases`.

### ST7 — Manual verification + pre-merge gate
**Files:** —
**Acceptance:** `make check` clean.
**Estimate:** S

- Produce a real run: `cargo run --bin loker -- ask "hello"`.
- Pretty-print: `cargo run --bin loker -- trace <run_id>` — verify colors, 80-col fit.
- Passthrough: `cargo run --bin loker -- trace <run_id> --json | jq .name`.
- Error path: `cargo run --bin loker -- trace not_a_run` — clean message.
- `make check`.

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Risks
| # | Risk | Mitigation |
|---|---|---|
| 1 | Snapshot tests fail on CI if terminal width differs | Use `insta` inline snapshots, run with `ColorChoice::Never` |
| 2 | `resolve_run_dir` changed behavior since CLO-310 | Re-test bare name / relative / absolute paths in ST7 |
| 3 | Fixture spans drift from future trace schema changes | Fixture is a test artifact — update snapshot if schema changes intentionally |
