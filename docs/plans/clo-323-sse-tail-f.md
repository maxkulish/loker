# Plan: CLO-323 [T-054] SSE tail-f of trace.jsonl

## Context
- Design: docs/designs/clo-323-sse-tail-f.md
- Discovery: docs/discovery/clo-323.md
- Linear: https://linear.app/cloud-ai/issue/CLO-323

## Sub-tasks

### ST1 Implement SSE event types and formatting
**Files:** `src/ui/sse.rs` (new)
**Acceptance:** `cargo test ui::sse::test_sse_event_formatting` passes.
**Estimate:** S

### ST2 Implement the tail reader (offset-based read)
**Files:** `src/ui/trace_reader.rs`
**Acceptance:** `cargo test ui::trace_reader::test_tail_reader_offset` passes.
**Estimate:** S

### ST3 Implement the `notify` file watcher integration
**Files:** `src/ui/sse.rs`
**Acceptance:** Integration test `test_notify_watcher_triggers_event` passes.
**Estimate:** M

### ST4 Implement the `GET /runs/:id/trace/sse` Axum handler
**Files:** `src/ui/routes.rs`, `src/ui/sse.rs`
**Acceptance:** `curl -N http://localhost:port/runs/<id>/trace/sse` opens and stays open.
**Estimate:** M

### ST5 Implement heartbeat and reconnection logic (`Last-Event-ID`)
**Files:** `src/ui/sse.rs`
**Acceptance:** Integration test `test_sse_heartbeat_and_recon` passes.
**Estimate:** M

### ST6 Frontend `EventSource` integration
**Files:** `src/ui/run_detail.tsx` (or equivalent frontend component)
**Acceptance:** Manual verification: live events appear in the trace timeline without page refresh.
**Estimate:** M

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Risks
- **File System Noise**: `notify` might send multiple events for a single append. This will be mitigated by strict byte-offset tracking in the reader.
- **FD Exhaustion**: Each SSE connection holds a file handle. We must ensure that when the client disconnects, the Axum stream is dropped and the file is closed immediately.
