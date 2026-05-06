# PRD — CLO-312: `loker trace <run_id>` pretty-printer

## Problem

Loker writes an append-only `trace.jsonl` to every run directory (`runs/<run_id>/trace.jsonl`).
The file contains OpenTelemetry GenAI-compatible spans — one JSON object per line —
recording every phase start, backend call, aggregator fold, verify hook result, and
timeout/error. Today, users must read this file with `cat`, `jq`, or raw `grep` to
understand what happened during a run. This is opaque, slow, and error-prone for
anyone who is not fluent in JSON.

## Users and impact

- **Developers and operators** debugging a failed workflow run need to quickly see
  which phase failed, which backend returned an error, how many tokens were consumed,
  and whether a `min_responses` shortfall occurred.
- **Test authors** need a fixture trace and a known-good render to guard against
  regressions in the trace format.

## Requirements

### CLI surface

- `loker trace <run_id>` — pretty-print the trace in chronological order.
- `--json` flag — print raw JSONL lines unchanged (for piping to `jq` / `grep`).
- `<run_id>` resolution uses the same `resolve_run_dir()` logic as `loker resume`:
  bare name → `<project_root>/runs/<run_id>`, relative path, or absolute path.

### Rendered output (default mode)

For each span, print a single line containing:
- **Timestamp** — wall-clock time, compact ISO-8601 or HH:MM:SS.
- **Phase** — the `loker.phase` value.
- **Span kind** — inferred from `name` (phase, backend, aggregator, verify, finished, error).
- **Backend** — `gen_ai.system` for backend spans, or the hook name for verify spans.
- **Latency** — `duration_ms` if present.
- **Tokens** — `input+output` if `gen_ai.usage.*` present.
- **Status** — one of: pass, fail, repair, error — derived from `loker.outcome`, `error.kind`, and `error.message`.

Errors and `min_responses` shortfalls must be visually highlighted (red / bold).

### 80-column constraint

No single line may exceed 80 characters. If a field would overflow, truncate with `…`.

### Snapshot test

A fixture file `tests/fixtures/trace/happy_path_and_errors.jsonl` must cover:
- Happy path: phase start → backend call → aggregator → verify pass → phase finished.
- Error path: phase start → backend error → phase error.
- Verify failure path: phase start → backend call → verify fail.

Snapshot the default-mode output via `insta::assert_snapshot!`.

### `--json` passthrough

When `--json` is passed, stream each line from `trace.jsonl` to stdout verbatim.
No parsing, no coloring, no truncation. Fail if the file does not exist.

## Acceptance criteria

1. `loker trace <run_id>` produces readable, colorized output on an 80-column terminal.
2. `loker trace <run_id> --json` passes through every line of `trace.jsonl` raw.
3. Snapshot test passes against the fixture trace.
4. `make check` clean.

## Non-goals

- Real-time follow (`-f` mode) — belongs to Phase 12 SSE work.
- Span filtering DSL — just full chronological output for v0.
- Sorting by anything other than chronological order.
- Computing aggregate statistics (total tokens, total latency). That belongs to the summary module.

## Dependencies

- T-029 (CLO-293) trace.jsonl writer — **done**.
- `resolve_run_dir()` helper — **done** (CLO-310).

## References

- PRD FR-36
- `docs/plans/001-implementation-roadmap.md` Phase 9 row T-043
- `docs/schemas/trace_span.schema.json`
