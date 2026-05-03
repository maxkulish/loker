# Pre-PR validation: clo-293

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-03
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [high] InMemorySink emits a different span shape than TraceWriter

**Where:** `src/trace/memory.rs:46-241` (all six methods)
**What:** `InMemorySink` builds its JSON map inline and omits `timestamp` and `loker.run_id`, while `TraceWriter` routes through `build_span_skeleton` (so it always emits `timestamp` + `loker.run_id`). Two consequences: (1) the sink trait makes no shape guarantee — consumers/tests can't rely on parity, and (2) `tests/trace_jsonl.rs::schema_validation` only validates the writer path, so an `InMemorySink` capture would silently fail JSON-Schema validation (missing required `timestamp`).
**Suggested fix:** Make `InMemorySink::push_*` use the same `build_span_skeleton` helper, then merge the sink-specific extras. This eliminates the divergence, removes ~150 lines of duplicated key-building, and lets you add an `InMemorySink` round-trip to `schema_validation` to keep both sinks honest.

### F2 [high] `loker.outcome` written by phase_runner drifts from the schema enum

**Where:** `src/phase_runner.rs:240-241,291-292,363-364` (calls `phase_finished(&trace_ctx, phase_err.error_class())`); enum values defined at `src/phase_runner.rs:170-183`; schema enum at `docs/schemas/trace_span.schema.json:111-114`
**What:** Schema declares `loker.outcome ∈ {success, verify_failed, aggregator_failed, strategy_failed, backend_error, timeout}`. `PhaseError::error_class()` can return `manifest_failed`, `marker_failed`, `io_failed`, `phase_failed` — none of which are in the enum. Any phase that fails on manifest write, marker write, IO, or invalid config will produce a span that fails strict schema validation. With `additionalProperties: false` already enforced, this is a real interop hazard.
**Suggested fix:** Either (a) extend the schema enum to include `manifest_failed | marker_failed | io_failed | phase_failed`, or (b) map `error_class()` outputs not in the schema to `phase_failed` (already in your code) and add `phase_failed` to the schema enum. Option (a) is cleaner since the labels are useful diagnostic signal.

### F3 [med] `TraceWriter::fsync` only flushes the `BufWriter` — it never `fsync`s the file

**Where:** `src/trace/writer.rs:54-58` (and rustdoc at `:18-21` claiming "every line is flushed + fsynced")
**What:** The flag is named `fsync` and the doc comment says "flushed + fsynced before returning", but the code only calls `BufWriter::flush()` — it never calls `File::sync_all()`. After a power loss the line could still be lost in the page cache. Also, this is the test path (`fsync=true`) so tests are weaker than they look.
**Suggested fix:** After `guard.flush()?`, call `guard.get_ref().sync_all()` and propagate (or log) the error. Alternatively rename the field `flush_each_line` and update the rustdoc to drop the fsync claim.

### F4 [med] Backend-attempt errors are never surfaced in their span

**Where:** `src/phase_runner.rs:266-276`
**What:** The runner unconditionally sets `BackendSpanResult.error = None` and emits `gen_ai.response.finish_reasons` even when the attempt finished with `FinishReason::Error` / `Refused`. Trace consumers can't tell a successful attempt from a failed one without parsing the finish-reason strings, and `error.message` will never be populated for a backend failure that the strategy still managed to swallow.
**Suggested fix:** When `attempt.finish_reasons` contains an error variant (or when an `attempt.error` field exists), populate `result.error` with the message (or at minimum a kind label). If `Attempt` doesn't carry that today, plumb a `last_error: Option<String>` through `StrategyOutput::attempts` so this isn't lost.

### F5 [med] `duration_ms` is plumbed everywhere but never written to JSON

**Where:** `src/trace/writer.rs:90-151` (`backend_call` ignores `result.duration_ms`); same in `src/trace/memory.rs:73-139`; `verify_result` similarly ignores `VerifySpanResult.duration_ms`; schema declares `duration_ms` at `docs/schemas/trace_span.schema.json:34-38`
**What:** `BackendSpanResult.duration_ms` and `VerifySpanResult.duration_ms` are public struct fields that callers populate (verify path measures `vstart.elapsed()` correctly) but neither sink emits them. The schema documents `duration_ms` and tests don't catch the omission. Result: every span on disk is missing the latency data that's the main reason traces exist.
**Suggested fix:** In both sinks, insert `"duration_ms": Number(result.duration_ms)` for backend and verify spans. Also wire phase-level duration: capture `let phase_start = Instant::now()` at the top of `PhaseRunner::run` and pass it to `phase_finished` / `error` (extend signatures to take a duration or compute from a passed `Instant`).

### F6 [med] Phase-runner does not measure backend attempt durations

**Where:** `src/phase_runner.rs:266-276` (hard-codes `duration_ms: 0`)
**What:** Even after F5 is fixed, every backend span will report `duration_ms: 0` because the runner never measures the attempt. Strategies own the actual timings; without surfacing them the trace is misleading.
**Suggested fix:** Either (a) extend `Attempt` (in `crate::strategy`) with a `duration: Duration` field that strategies populate as they execute, then pass `attempt.duration.as_millis() as u64` here; or (b) emit backend spans inside the strategy (where the timing is known) rather than after-the-fact in PhaseRunner. (a) is the smaller change.

### F7 [low] `verify_hook` name casing is inconsistent between phase span and verify span

**Where:** `src/phase_runner.rs:215-219` (phase span sets `loker.verify_hook = "run_command"` snake_case via `verify_hook_name`); `src/phase_runner.rs:354` (verify span sets `loker.verify_hook = hook.name()` — implementations typically return `"RunCommand"` PascalCase); span name `verify.RunCommand` vs phase metadata `loker.verify_hook: "run_command"`
**What:** The same hook is identified by two different strings on the same trace. Joining/filtering across spans by `loker.verify_hook` won't work.
**Suggested fix:** Pick one canonical form. Easiest: pass `verify_hook_name(cfg.verify)` (snake_case) into `t.verify_result` instead of `hook.name()`, and use it in the span name (`format!("verify.{snake}")` -> `verify.run_command`). Match what already lives on the phase span.

### F8 [low] Sink implementations duplicate ~150 lines of map-building

**Where:** `src/trace/memory.rs:46-241` mirrors `src/trace/writer.rs:62-256` field-for-field
**What:** Both sinks do the exact same `extras.insert("loker.run_id", ...)`, `extras.insert("loker.phase", ...)`, etc. for each method, only differing in destination (push vs writeln). Future schema additions will need to be made in both places (and forgetting one is what's caused F1).
**Suggested fix:** Extract a private `fn span_extras_*(...) -> serde_json::Map<String, Value>` helper per emit kind (or make `TraceSink` a thin trait that just consumes a fully-built `Map` and split the building into one shared module). After F1, this is largely free.

### F9 [low] `phase_finished` is called twice on failure paths

**Where:** `src/phase_runner.rs:239-242, 290-293, 362-365`
**What:** On any failure the runner calls `t.error(...)` *and then* `t.phase_finished(...)` with the error class. Per the design doc the "error" event itself is the terminal record and `phase_finished` is the success-path closer. Emitting both produces two spans for one terminal event, and the `phase_finished` outcome value re-uses an error class (worsens F2).
**Suggested fix:** Either drop the `phase_finished` calls on the error branches (let `error` be the closer), or drop the separate `error` calls and use `phase_finished(outcome=error_class)` exclusively. Pick one and document it.

## Verdict

**approve_with_changes**

The PR delivers the documented contract — TraceSink trait, two sinks, PhaseRunner wiring, JSONL on disk, integration tests pass, schema file is in place — and the writer-path output validates against the schema. But there's real polish missing before this is production-grade: InMemorySink emits a schema-incompatible shape (F1), `loker.outcome` can write enum values the schema rejects on common failure paths (F2), the test-mode `fsync` flag doesn't actually fsync (F3), and `duration_ms` — arguably the most useful field on a trace — is plumbed but never emitted, with backend timings not even measured (F5+F6). F1, F2, F5, and F6 are worth addressing in this PR or a fast follow-up; F3, F4, F7, F8, F9 can land separately. Not a rework — the bones are right and the tests cover the happy path — but I'd want F1/F2/F5/F6 fixed before marking CLO-293 closed.
