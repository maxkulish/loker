# Design — CLO-293: Implement trace.jsonl writer with OpenTelemetry GenAI semantic conventions

## Problem

Loker's PhaseRunner executes backend calls, strategy decisions, aggregator folds, and verify-hook invocations, but emits no structured, machine-readable trace. Every run is opaque: there is no durable record of which model was called, how many tokens were consumed, whether a verify hook passed or failed, or how many retry attempts occurred. This blocks debugging, cost accounting, compliance, and any downstream observability pipeline.

The discovery established that PhaseRunner, Backend, Strategy, Aggregator, and VerifyHook all produce the required data. The gap is a sink abstraction and a file writer that turns this data into OTel GenAI-compatible spans.

## Goals / Non-goals

### Goals

1. Define a `TraceSink` trait that PhaseRunner can call at lifecycle points.
2. Implement `TraceWriter` that appends OTel GenAI spans as JSONL to `run_dir/trace.jsonl`.
3. Implement `InMemorySink` for test capture.
4. Wire `TraceSink` into `PhaseRunner::run()` at every significant lifecycle point.
5. Define `trace_span.schema.json` that validates every emitted line.
6. Deliver `tests/trace_jsonl.rs` satisfying the TDD contract.

### Non-goals

- Live OTLP export (post-v0).
- Metrics or logs — only spans.
- Compression / rotation.
- Replacing `trace_event.schema.json` — the span schema is additive.

## Architecture

### Module placement

```
src/
├── trace.rs               ← NEW: TraceSink trait + TraceSpan + SpanKind + helpers
├── trace/
│   ├── writer.rs           ← NEW: TraceWriter (file sink)
│   └── memory.rs           ← NEW: InMemorySink
├── phase_runner.rs         ← MODIFIED: accept Option<&dyn TraceSink>
├── phase_runner/
│   ├── dispatch.rs         ← NO CHANGE
│   └── persist.rs          ← NO CHANGE
└── lib.rs                  ← MODIFIED: re-export TraceSink, TraceWriter, InMemorySink
```

### Data flow

```
PhaseRunner::run()
  ├─ trace.phase_started()    → root span (phase span)
  ├─ Strategy::execute()
  │   ├─ Backend::query()   → trace.backend_call(child of phase span)
  │   └─ [retry]            → trace.backend_call(child of phase span)
  ├─ Aggregator::aggregate()  → trace.aggregator_fold(child of phase span)
  ├─ VerifyHook::verify()   → trace.verify_result(child of phase span)
  └─ trace.phase_finished() → terminal outcome
```

For **EscalatingRetry**, each attempt is an `Attempt` in `StrategyOutput`. The PhaseRunner iterates over these attempts after strategy execution completes. Each attempt's backend call gets a backend span whose `parent_span_id` is the phase span. No separate attempt span is needed; the attempt index is captured on the backend span (`loker.attempt`).

For **ParallelFanOut**, each branch backend call gets its own backend span, all children of the phase span. The aggregator fold span is also a child of the phase span.

### Span hierarchy (OTel parent-child)

```
trace_id = run_id (UUID)
  └─ span_id = <phase_span>    name="phase.{phase_name}"  parent=null
      ├─ span_id = <backend_0> name="backend.{backend_name}"  parent=phase_span  loker.attempt=0
      ├─ span_id = <backend_1> name="backend.{backend_name}"  parent=phase_span  loker.attempt=1
      ├─ span_id = <backend_n> ...
      ├─ span_id = <agg>       name="aggregator.{agg_name}"    parent=phase_span
      └─ span_id = <verify>    name="verify.{hook_name}"      parent=phase_span
```

The PhaseRunner is responsible for generating `trace_id` (from `ctx.run_id`) and `span_id` (random hex). Backend spans, aggregator spans, and verify spans all share the same `parent_span_id` = the phase span.

## Public API surface

### `src/trace.rs` — Core types and trait

```rust
use serde_json::Value;
use std::fmt;
use uuid::Uuid;

/// Trait for emitting trace spans. Implementations may write to disk,
/// memory, or a downstream OTel pipeline.
pub trait TraceSink: Send + Sync {
    fn phase_started(&self, ctx: &PhaseSpanContext);
    fn backend_call(&self, ctx: &PhaseSpanContext, attempt: &AttemptSpanContext, result: &BackendSpanResult);
    fn aggregator_fold(&self, ctx: &PhaseSpanContext, kind: &str, input_count: usize);
    fn verify_result(&self, ctx: &PhaseSpanContext, hook_name: &str, result: &VerifySpanResult);
    fn phase_finished(&self, ctx: &PhaseSpanContext, outcome: &str);
    fn error(&self, ctx: &PhaseSpanContext, kind: &str, message: &str);
}

/// Context shared by all spans within a phase.
#[derive(Clone, Debug)]
pub struct PhaseSpanContext {
    pub trace_id: Uuid,
    pub span_id: String,      // hex, 16+ chars
    pub phase: String,
    pub strategy: String,
    pub aggregator: String,
    pub verify_hook: Option<String>,
}

/// Per-attempt context for backend spans.
#[derive(Clone, Debug)]
pub struct AttemptSpanContext {
    pub span_id: String,
    pub attempt: usize,
    pub backend: String,
    pub model: Option<String>,
}

/// Result of a backend call, used to populate gen_ai.* fields.
pub struct BackendSpanResult {
    pub duration_ms: u64,
    pub usage_input_tokens: Option<u64>,
    pub usage_output_tokens: Option<u64>,
    pub finish_reasons: Vec<String>,
    pub error: Option<String>,
}

/// Result of a verify hook invocation.
pub struct VerifySpanResult {
    pub passed: bool,
    pub message: Option<String>,
    pub duration_ms: u64,
}

/// Generates random span IDs (16-byte hex).
pub fn new_span_id() -> String { /* ... */ }
```

### `src/trace/writer.rs` — File sink

```rust
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

pub struct TraceWriter {
    writer: Mutex<BufWriter<std::fs::File>>,
    fsync: bool,
}

impl TraceWriter {
    pub fn new(path: &Path, fsync: bool) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        Ok(Self {
            writer: Mutex::new(BufWriter::new(file)),
            fsync,
        })
    }
}

impl TraceSink for TraceWriter {
    // Each method builds a serde_json::Value, writes one JSON line,
    // and optionally calls self.flush_and_fsync().
}
```

### `src/trace/memory.rs` — In-memory sink for tests

```rust
use std::sync::Mutex;

pub struct InMemorySink {
    spans: Mutex<Vec<Value>>,
}

impl InMemorySink {
    pub fn new() -> Self { ... }
    pub fn spans(&self) -> Vec<Value> { ... }
    pub fn clear(&self) { ... }
}

impl TraceSink for InMemorySink { ... }
```

### `PhaseInputs` extension

```rust
pub struct PhaseInputs<'a> {
    pub backends: &'a [Arc<dyn Backend>],
    pub prompt: Prompt,
    pub ctx: PhaseContext,
    pub verify: Option<Arc<dyn VerifyHook>>,
    pub run_dir: PathBuf,
    pub trace: Option<&'a dyn TraceSink>,  // NEW
}
```

### `PhaseRunner` instrumentation points

Inside `PhaseRunner::run()`:

1. **Start of run** — generate `trace_id` from `ctx.run_id`, generate `phase_span_id`, build `PhaseSpanContext`, call `trace.phase_started()`.
2. **After strategy execution** — for each `Attempt` in `StrategyOutput.attempts`, call `trace.backend_call()` with attempt metadata. For error attempts, set `error` on `BackendSpanResult`.
3. **After aggregation** — call `trace.aggregator_fold()` with the aggregator name and input count.
4. **After verify** — call `trace.verify_result()` with the hook name and pass/fail.
5. **On success** — call `trace.phase_finished("success")`.
6. **On terminal failure** — call `trace.error()` with the failure kind and message, then `trace.phase_finished(kind)`.

## Test plan

### Unit tests

| File | What it tests |
|---|---|
| `src/trace/writer.rs` | `TraceWriter::new` creates file, appends valid JSONL lines, `fsync=true` flushes |
| `src/trace/memory.rs` | `InMemorySink` captures all span kinds; `spans()` returns clone |
| `src/trace.rs` | `new_span_id()` returns 32-char hex; `PhaseSpanContext` clones cleanly |

### Integration tests (`tests/trace_jsonl.rs`)

Each test uses the same `PhaseRunner` + mock backends + `InMemorySink` pattern established by `tests/phase_runner_integration.rs`.

| Test name | Assertion |
|---|---|
| `single_phase_two_spans_parented` | Exactly 2 spans captured; backend span `parent_span_id` == phase span `span_id` |
| `parallel_three_replicas_five_spans` | 1 phase + 3 backend + 1 aggregator spans; all backend `parent_span_id` == phase span |
| `escalating_retry_attempt_spans` | 1 phase + 2 backend spans; both backend `parent_span_id` == phase span; `loker.attempt` = 0,1 |
| `verify_failure_outcome` | Verify span `loker.outcome = "verify_failed"`; no `gen_ai.usage.*` on verify span |
| `token_counts_match_mock` | Mock backend `QueryOutput` with `usage` → matching `gen_ai.usage.input_tokens` / `output_tokens` |
| `file_valid_jsonl` | Every line in `run_dir/trace.jsonl` parses as JSON |
| `schema_validation` | Every emitted line validates against `docs/schemas/trace_span.schema.json` |

### Manual test

1. Run `cargo test --test trace_jsonl` — all 7 assertions pass.
2. Run `make check` — fmt, clippy, unit tests, integration tests all green.
3. Inspect `trace.jsonl` from a tempdir test with `cat` — confirm human readability.

## Migration / rollout

No migration needed. This is an additive feature:

- `PhaseInputs` gains a new optional field `trace: Option<&dyn TraceSink>`.
- Existing callers (integration tests, CLI) pass `None` and see no change in behaviour.
- The trace file is not manifest-registered, so no manifest schema migration.
- TODO(T-029) comments in `src/manifest.rs` and `src/run_state/load.rs` are removed during implementation.

## Open questions

| Question | Resolution |
|---|---|
| Should we emit an attempt span (parent of backend span) for EscalatingRetry? | **No.** The PRD says "one backend span per attempt; all attempts share the same phase parent span." Adding attempt spans would increase span count without adding useful OTel data. The `loker.attempt` field on the backend span is sufficient. |
| Should verify spans carry `gen_ai.usage.*`? | **No.** Verify hooks (`RunCommand`, `LLMVerifier`) may or may not call LLMs. The `RunCommand` path has no token usage. The `LLMVerifier` path *could* have usage, but the current `VerifyHook` trait does not return it. We leave `gen_ai.usage.*` absent on verify spans until a later milestone. |
| Should `trace.jsonl` be atomic-written like the manifest? | **No.** The manifest uses atomic tmp→rename because it is SHA-256-verified and append-only. `trace.jsonl` is explicitly mutable and not manifest-registered. Standard line-buffered append with `BufWriter` is sufficient. |
| How do we handle trace writes when the run directory does not exist? | `PhaseRunner` already ensures `run_dir` exists before calling persist helpers. `TraceWriter::new` should be called inside `PhaseRunner::run()` after directory creation, or it should `create_dir_all` itself. We'll let `PhaseRunner` create the directory and then open the writer. |
