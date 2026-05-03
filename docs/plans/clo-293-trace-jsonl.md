# Plan: CLO-293 Implement trace.jsonl writer with OpenTelemetry GenAI semantic conventions

## Context
- Design: `docs/designs/clo-293-trace-jsonl.md`
- Discovery: `docs/discovery/clo-293.md`
- PRD: `docs/prds/clo-293-trace-jsonl.md`
- Schema: `docs/schemas/trace_span.schema.json`
- Linear: https://linear.app/cloud-ai/issue/clo-293/t-029-implement-tracejsonl-writer-with-opentelemetry-genai-semantic

## Sub-tasks

### ST1 Define TraceSink trait and core span types
**Files:** `src/trace.rs`, `src/lib.rs`

Create the `trace` module with:
- `TraceSink` trait (6 methods: `phase_started`, `backend_call`, `aggregator_fold`, `verify_result`, `phase_finished`, `error`)
- `PhaseSpanContext`, `AttemptSpanContext`, `BackendSpanResult`, `VerifySpanResult`
- `new_span_id()` helper (32-char hex from `rand`)
- Re-export in `src/lib.rs`

**Acceptance:** `cargo test trace::` unit tests pass; `make check` green.
**Estimate:** S

### ST2 Implement InMemorySink for test capture
**Files:** `src/trace/memory.rs`, `src/lib.rs`

Create `InMemorySink` that captures all spans into a `Vec<serde_json::Value>` behind a `Mutex`. Implement `TraceSink` for it. Add unit tests in `src/trace/memory.rs`.

**Acceptance:** `cargo test memory_sink` passes (tests: `capture_phase_span`, `capture_backend_span`, `capture_all_kinds`).
**Estimate:** S

### ST3 Implement TraceWriter (file sink)
**Files:** `src/trace/writer.rs`, `src/lib.rs`

Create `TraceWriter` that:
- Opens `run_dir/trace.jsonl` with `OpenOptions::create(true).append(true)`
- Uses `Mutex<BufWriter<File>>` for line-buffered append
- Configurable `fsync` gate (default `false`, `true` in tests)
- Implements `TraceSink` by serializing each span as one JSONL line with `loker.*` and `gen_ai.*` fields
- Generates `trace_id` from `ctx.run_id` (UUID), `span_id` from `new_span_id()`
- Populates OTel GenAI fields: `gen_ai.system`, `gen_ai.request.model`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`

**Acceptance:** `cargo test trace_writer` passes (tests: `creates_file`, `appends_valid_jsonl`, `fsync_flushes`, `gen_ai_fields_present`).
**Estimate:** M

### ST4 Wire TraceSink into PhaseRunner
**Files:** `src/phase_runner.rs`, `src/phase_runner/dispatch.rs`

- Add `trace: Option<&'a dyn TraceSink>` to `PhaseInputs`
- In `PhaseRunner::run()`:
  1. Generate `PhaseSpanContext` at start → `trace.phase_started()`
  2. For each `Attempt` in `StrategyOutput.attempts`, emit `trace.backend_call()` with attempt metadata
  3. After `Aggregator::aggregate()`, emit `trace.aggregator_fold()`
  4. After `VerifyHook::verify()`, emit `trace.verify_result()`
  5. On success → `trace.phase_finished("success")`
  6. On terminal failure → `trace.error()` + `trace.phase_finished("verify_failed"|"error")`
- Update existing integration tests to pass `trace: None` (no behavioural change)
- Remove TODO(T-029) comments from `src/manifest.rs` and `src/run_state/load.rs`

**Acceptance:** `cargo test --test phase_runner` still passes (no regressions); `cargo test trace::phase_runner` passes (new trace-wiring unit test).
**Estimate:** M

### ST5 Deliver integration test contract (trace_jsonl.rs)
**Files:** `tests/trace_jsonl.rs`

Integration tests using `PhaseRunner` + mock backends + `InMemorySink` + `TraceWriter`:

| Test | Assertion |
|---|---|
| `single_phase_two_spans_parented` | 2 spans; backend `parent_span_id` == phase `span_id` |
| `parallel_three_replicas_five_spans` | 1 phase + 3 backend + 1 aggregator; all backend `parent_span_id` == phase span |
| `escalating_retry_attempt_spans` | 1 phase + 2 backend; `loker.attempt` = 0,1 |
| `verify_failure_outcome` | `loker.outcome = "verify_failed"`; no `gen_ai.usage.*` on verify |
| `token_counts_match_mock` | `gen_ai.usage.input_tokens` / `output_tokens` match mock backend |
| `file_valid_jsonl` | Every line in `trace.jsonl` parses as JSON |
| `schema_validation` | Every line validates against `docs/schemas/trace_span.schema.json` |

**Acceptance:** `cargo test --test trace_jsonl` passes all 7 tests; `make check` green.
**Estimate:** M

## Pre-merge gate
- `make check` (fmt + clippy + unit tests + integration tests)
- No live network required

## Risks
- **PhaseRunner instrumentation complexity** — the run method is the hottest path. Adding trace calls adds ~6 method invocations per phase. Mitigation: `trace: Option<&dyn TraceSink>` is `None` for existing callers, so the overhead is a null check per lifecycle point.
- **Span ID generation** — using `rand` adds a dependency. Mitigation: `rand` is likely already in the dependency tree; if not, it's small and standard.
- **Schema drift** — OTel GenAI semantic conventions may change. Mitigation: schema uses `patternProperties` for unknown `loker.*` keys; `gen_ai.*` keys match the current spec.
- **EscalatingRetry span count** — the PRD says 1 phase + N backend spans. If we discover we need attempt spans during implementation, we flag it and return to design. Mitigation: `loker.attempt` on backend spans is the agreed compromise.
