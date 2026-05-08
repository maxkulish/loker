# PRD: SSE tail-f of trace.jsonl (CLO-323)

## Goal
Stream live trace events to the daemon UI via Server-Sent Events so users see runs progress without manual refresh.

## Problem Statement
Currently, users must manually refresh the run detail view or wait for the process to complete to see the trace events. This lacks the "live" feeling and makes debugging real-time behavior difficult.

## Scope
- **Backend**:
    - New endpoint `GET /runs/:id/trace/sse`.
    - Implementation of a file tailing mechanism using the `notify` crate (inotify/kqueue) to monitor `trace.jsonl`.
    - Streaming new JSON lines as SSE events.
    - Heartbeat mechanism to prevent connection timeout.
    - Resource management to ensure file descriptors are not leaked.
- **Frontend**:
    - Integration of `EventSource` in the run detail view.
    - Real-time appending of events to the timeline.
    - Handling of reconnections using `Last-Event-ID`.

## Acceptance Criteria
- **Real-time Delivery**: Events appear in the UI within 1 second of being written to `trace.jsonl`.
- **Robust Recon**: Client recovers state after drop via `Last-Event-ID`.
- **Stability**: 100 sequential connections do not exhaust file descriptors.

## Non-goals
- Backfilling historical events on connect (page already renders the last N).
- Using WebSockets.

## Dependencies
- T-053 (sessions list + per-run trace + pending panel).

## References
- Roadmap: `docs/plans/001-implementation-roadmap.md` Phase 12.
- PRD M11.
