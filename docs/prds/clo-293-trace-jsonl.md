# PRD — CLO-293: Implement trace.jsonl writer with OpenTelemetry GenAI semantic conventions

## Problem

Loker's PhaseRunner executes backend calls, strategy decisions, aggregator folds, and verify-hook invocations, but emits no structured, machine-readable trace. Every run is opaque: there is no durable record of which model was called, how many tokens were consumed, whether a verify hook passed or failed, or how many retry attempts occurred. This blocks debugging, cost accounting, compliance, and any downstream observability pipeline. We need an append-only `trace.jsonl` per run that captures every significant lifecycle event as an OpenTelemetry GenAI-compatible span.

## Users and impact

- **Developers and operators** need to inspect runs after the fact to understand why a phase failed, which backend produced which output, and how many tokens were consumed.
- **Test authors** need an in-memory trace capture mechanism to assert on span counts, parent-child relationships, and field values (the TDD contract).
- **Downstream observability tooling** can ingest `trace.jsonl` directly because it follows OTel GenAI semantic conventions; no custom parser is needed.

## Requirements

### TraceSink trait

- Define `TraceSink: Send + Sync` with methods to emit spans at lifecycle points: `phase_started`, `backend_call`, `aggregator_fold`, `verify_invoked`, `verify_result`, `phase_finished`, `error`.
- Each method receives structured context (run_id, phase, attempt, backend, model, usage, outcome, etc.) so the sink decides the serialized shape.

### TraceWriter

- Implement `TraceWriter` with an append-only, line-buffered writer to `run_dir/trace.jsonl`.
- Per-line fsync gate: configurable via constructor; default `false` for speed, `true` under tests.
- Each line is one JSON object (valid JSONL).
- Uses `loker.*` namespace for Loker-specific fields and `gen_ai.*` for standard OTel GenAI fields.
- Required span fields:
  - `trace_id`, `span_id`, `parent_span_id` (for lineage)
  - `timestamp` (ISO-8601)
  - `name` (span name: e.g. `phase.design`, `backend.mock`)
  - `gen_ai.system`, `gen_ai.request.model`
  - `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`
  - `gen_ai.response.id` (optional)
  - `loker.run_id`, `loker.phase`, `loker.attempt`
  - `loker.strategy`, `loker.aggregator`, `loker.verify_hook`, `loker.outcome`

### PhaseRunner instrumentation

- Add an optional `trace: Option<&dyn TraceSink>` to `PhaseInputs`.
- `PhaseRunner::run()` emits spans at these points:
  1. **phase_started** — when `start_attempt(0)` is called.
  2. **backend_call** — for every `Backend::query()` result (success or error), one span per call with `parent_span_id` pointing to the phase span.
  3. **aggregator_fold** — after aggregation completes, one span with `loker.aggregator` set.
  4. **verify_invoked** / **verify_result** — before and after verify hook execution.
  5. **phase_finished** — on success before returning; or **error** span on terminal failure.
- For `EscalatingRetry`, emit one backend span per attempt; all attempts share the same phase parent span.
- For `ParallelFanOut`, emit one backend span per branch plus one aggregator span; all share the same phase parent span.

### Trace file contract

- `trace.jsonl` lives in `run_dir/` alongside the manifest and markers.
- It is **not** registered in the manifest (it is mutable; manifest is append-only-with-sha256).
- On `PhaseRunner::run()` entry, if `trace.jsonl` does not exist, create it. If it exists, append.
- File rotation is not in scope; one file per run.

### Schema

- Create `docs/schemas/trace_span.schema.json` that validates:
  - Required top-level keys: `trace_id`, `span_id`, `timestamp`, `name`
  - OTel GenAI fields when `gen_ai.system` is present
  - `loker.*` namespace fields
  - Forward-compatible: unknown `loker.*` keys allowed via `patternProperties`

### Testability

- `InMemorySink` implementation of `TraceSink` that appends spans to a `Vec<serde_json::Value>`.
- `tests/trace_jsonl.rs` integration tests:
  1. Single phase emits exactly two spans: phase span + backend span, parented correctly.
  2. Parallel n=3 emits 1 phase span + 3 backend spans + 1 aggregator span.
  3. Escalating retry with one retry emits 1 phase span + 2 attempt spans + 2 backend spans.
  4. Verify-hook failure span has `loker.outcome = "verify_failed"` and `gen_ai.usage.*` absent on the verify span (or the run_command path where no LLM usage exists).
  5. Token counts from mocked backend match span fields.
  6. File is valid JSONL: every line parses as a JSON object.
  7. Schema validation: every emitted line passes `docs/schemas/trace_span.schema.json`.

## Acceptance tests

- `single_first_no_verify` with `InMemorySink` captures exactly 2 spans; file on disk has 2 lines.
- `parallel_concat_three_replicas` with `InMemorySink` captures exactly 5 spans (1 phase + 3 backend + 1 aggregator); file on disk has 5 lines; all backend spans share the same `parent_span_id` as the phase span.
- `escalating_retry_one_recovery` with `InMemorySink` captures exactly 5 spans (1 phase + 2 attempt + 2 backend); attempt spans are child spans of the phase span; backend spans are children of their respective attempt spans.
- `terminal_verify_failure` emits an error span with `loker.outcome = "verify_failed"`; `gen_ai.usage.*` fields are absent on the verify span.
- `token_counts_from_mocked_backend` — mocked `QueryOutput` with `usage` produces matching `gen_ai.usage.input_tokens` and `gen_ai.usage.output_tokens`.
- `file_valid_jsonl` — every line in `trace.jsonl` parses as JSON; the file as a whole is valid JSONL.
- `schema_validation` — every emitted line validates against `docs/schemas/trace_span.schema.json`.
- `make check` clean with no live network.

## Non-goals

- Live OTLP export (post-v0).
- Metrics or logs — only spans.
- Compression / rotation.
- Replacing the existing `trace_event.schema.json` event model (the span schema is additive; both can coexist).
