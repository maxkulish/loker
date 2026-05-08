# Design: clo-323 - [T-054] SSE tail-f of trace.jsonl

## Problem
As identified in the discovery report, users of the Loker daemon UI currently experience a "blind spot" when monitoring long-running workflows. Because trace events in `trace.jsonl` are viewed statically or via manual page refresh, the a real-time sense of progress is missing, and failures are not noticed immediately. This hinders the developer experience during the most critical phase of a run.

## Goals / Non-goals
### Goals
- Implement a Server-Sent Events (SSE) endpoint `GET /runs/:id/trace/sse`.
- Use the `notify` crate to watch `trace.jsonl` for new writes.
- Stream new trace events to the browser in real-time (< 1s latency).
- Support reconnection via `Last-Event-ID` to prevent data loss during transient drops.
- Implement a heartbeat to maintain long-lived connections.
- Ensure file descriptors are properly closed on client disconnect.

### Non-goals
- Backfilling historical events upon initial connection (the UI already loads the last N events on the initial page request).
- Transitioning the entire UI to WebSockets.
- Modifying the `TraceWriter` in the core orchestration engine.

## Architecture

### Components
1. **`SseTraceHandler`**: The Axum handler that manages the SSE stream.
2. **`TraceWatcher`**: A wrapper around `notify` that monitors the run directory's `trace.jsonl`.
3. **`SseEventStream`**: A `Stream` implementation that yields SSE-formatted strings.

### Data Flow
1. Client requests `GET /runs/:id/trace/sse`.
2. Server validates `run_id` and locates `runs/<id>/trace.jsonl`.
3. Server initializes a `TraceWatcher` and opens the file at the current EOF.
4. `notify` triggers a `Write` event when `TraceWriter` appends a new line.
5. The handler reads the new bytes from the last known offset.
6. The bytes are parsed as JSON and wrapped in an SSE event (e.g., `event: trace_event\ndata: {...}\n\n`).
7. The event is pushed to the Axum response stream.

### Core Types
```rust
pub struct TraceSseState {
    pub project_root: PathBuf,
}

pub struct TraceEvent {
    pub timestamp: String,
    pub event_type: String,
    pub summary: String,
}
```

## Public API Surface

### Routes (`src/ui/routes.rs`)
Add the following route to `ui_routes`:
```rust
.route("/runs/:id/trace/sse", get(run_trace_sse))
```

### Handler (`src/ui/routes.rs` or new module `src/ui/sse.rs`)
```rust
async fn run_trace_sse(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> impl IntoResponse {
    // SSE implementation using axum::response::Sse
}
```

## Test Plan

### Unit Tests
- `test_sse_event_formatting`: Verify that a `TraceEvent` is correctly converted to an SSE wire-format string.
- `test_tail_reader_offset`: Ensure that reading from a specific offset correctly captures new lines.

### Integration Tests
- `test_sse_stream_end_to_end`:
    1. Start the daemon UI.
    2. Open an SSE connection to a dummy run.
    3. Append a line to `trace.jsonl`.
    4. Verify the SSE stream receives the event.
- `test_heartbeat_delivery`: Verify that a heartbeat is sent every 15-30 seconds.

### Manual Verification
- Open the run detail view in Chrome/Firefox.
- Run a workflow that produces trace events every few seconds.
- Verify that events appear in the timeline without page refresh.
- Manually disconnect network and reconnect to verify `Last-Event-ID` behavior.

## Migration / Rollout
- **Backward Compatibility**: No existing API is modified. This is a purely additive endpoint.
- **Rollout**: Deploy the backend endpoint first, followed by the frontend `EventSource` integration.

## Open Questions
- **Event Deduplication**: `notify` can sometimes send multiple events for a single write. Should we rely on the file offset for strict linear reading, or is a simple "read everything since last offset" sufficient? (Decision: Use file offset for correctness).
- **Last-Event-ID Format**: Since we are streaming JSON lines, should the `Last-Event-ID` be the byte offset of the file or a custom sequence number? (Decision: Use byte offset for simplicity and reliability in file-tailing).
