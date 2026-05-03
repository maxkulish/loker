# PRD: CLO-284 — Phase status markers (started/completed/failed) with atomic write

| Field | Value |
|-------|-------|
| Author | pi (discovery phase) |
| Status | Draft |
| Created | 2026-05-02 |
| Task | CLO-284 / T-025 |
| Depends on | CLO-245 (D3 atomic write protocol — done) |
| Blocks | T-031 (resumability), T-027 (attempt directories) |

## 1. Goal

Implement per-phase status markers and a heartbeat writer as described in
`docs/run-state.md` (D3). The `<phase>.completed` marker is the single source
of truth for "this phase is done"; resume logic (T-031) and the orphan-entry
sweep (T-024) both depend on these markers being written atomically and in the
correct order relative to manifest entries.

## 2. Scope

### In scope

**MarkerWriter API** (`src/run_state/markers.rs`):
- `write_started(phase, attempt)` — body includes phase name, attempt number,
  ISO-8601 timestamp, host/pid for crash forensics, heartbeat TTL.
- `write_completed(phase, attempt, manifest_entry_sha256)` — body references
  the manifest entry that this phase produced, so the orphan sweep (kill
  matrix row 9) can verify the link.
- `write_failed(phase, attempt, reason)` — body captures the failure class
  (e.g., `verify_unrecoverable`, `hitl_declined`).
- All writes use the tmp+rename atomic protocol from D3: write to
  `runs/<id>/markers/.<phase>.<state>.tmp.<rand>`, fsync the file fd, rename
  to final path, fsync the parent directory fd.

**HeartbeatWriter**:
- Tokio task that rewrites `runs/<id>/heartbeat.json` every
  `heartbeat_ttl_seconds / 3` (default 100s for 300s TTL).
- Each rewrite uses the same atomic protocol.
- Body: writer host/pid, last-tick timestamp, current phase.
- Provide a fake-clock injection point so tests don't sleep.

**Attempt-counter helper**:
- `next_attempt(phase) -> u32` — derives the next attempt number by listing
  `markers/<phase>.started.*` (or by scanning the attempt directory once T-027
  lands; for now, just markers).
- Returns 0 for first attempt.

**Per-phase commit order enforcement**:
- Helper API that wraps a phase: `started` marker → artefact write → manifest
  entry append → `completed` marker.
- Out-of-order calls (e.g., `completed` before `started`) panic in debug,
  log+error in release.

### Out of scope (deferred)
- Manifest writer (CLO-283 / T-024 — already done).
- Resume / load-time sweep (T-031).
- Attempt directories (T-027) — derive attempt number from markers for now.
- HITL state markers — those land separately in M10.

## 3. Acceptance Criteria

1. All marker writes survive `kill -9` between any two syscalls (verified via
   fault injection in tests).
2. HeartbeatWriter cadence holds under fake clock; real-clock smoke test with
   1s TTL passes.
3. `make check` (fmt + clippy + test) is green.
4. Per-phase commit order is enforced or asserted; out-of-order writes are loud
   failures, not silent successes.
5. Public API is documented with rustdoc; private helpers can stay terse.

## 4. Design

### 4.1 Module layout

```
src/run_state/
├── mod.rs          # Re-exports, module structure
├── markers.rs      # MarkerWriter, marker types, mark_phase_started/completed/failed
├── heartbeat.rs    # HeartbeatWriter, HeartbeatConfig
└── order.rs        # PhaseOrderGuard (started → artefact → manifest → completed)
src/lib.rs          # pub mod run_state;
tests/
└── run_state_markers.rs  # TDD test contract from issue body
```

### 4.2 Marker JSON schemas (from docs/run-state.md)

`<phase>.started`:
```json
{
  "phase": "design",
  "attempt": 1,
  "started_at": "2026-04-25T20:45:00Z",
  "writer_pid": 12345,
  "writer_host": "loker-runner-3",
  "heartbeat_ttl_seconds": 300
}
```

`<phase>.completed`:
```json
{
  "phase": "design",
  "attempt": 1,
  "completed_at": "2026-04-25T20:48:13Z",
  "manifest_entry_sha256": "ab12...",
  "artefact_paths": ["design/design.md"]
}
```

`<phase>.failed`:
```json
{
  "phase": "design",
  "attempts_made": 3,
  "failed_at": "2026-04-25T20:51:02Z",
  "error_class": "BackendTimeout",
  "last_attempt_path": "attempts/design/3/"
}
```

### 4.3 Atomic write primitive

Reuse the `atomic_write` helper from `src/manifest.rs`. Extract it into a shared
helper (e.g., `src/run_state/atomic.rs`) so both the manifest writer and the
marker writer use the same code path. This prevents protocol divergence.

### 4.4 HeartbeatWriter

A Tokio task spawned with a `tokio::time::interval` running at
`heartbeat_ttl_seconds / 3`. Each tick reads the current phase from shared state,
builds the JSON body, and calls `atomic_write` on `runs/<id>/heartbeat.json`.

For testability, accept a `Clock` trait that can be swapped for a fake clock.
The real clock wraps `tokio::time::Instant::now()`; the fake clock advances
on explicit calls.

### 4.5 Per-phase commit order guard

A struct `PhaseOrderGuard` with an enum state machine:

```rust
enum PhaseState {
    Idle,
    Started,
    ArtefactWritten,
    ManifestAppended,
    Completed,
}
```

Methods panic (debug) / log error (release) on invalid transitions:
- `mark_started` → Idle → Started
- `mark_artefact_written` → Started → ArtefactWritten
- `mark_manifest_appended` → ArtefactWritten → ManifestAppended
- `mark_completed` → ManifestAppended → Completed

### 4.6 Test contract (`tests/run_state_markers.rs`)

Per the issue body TDD spec:

1. **Round-trip**: write each marker kind, read back, assert body fields match.
2. **Atomic rename**: inject a crash between tmp-write and rename; assert the
   final marker path does not exist and no partial file is left in the markers
   directory.
3. **Heartbeat ticks under fake clock**: advance fake clock by ttl, assert
   heartbeat file mtime/body advanced exactly the expected number of times
   (ttl/3 cadence).
4. **Staleness helper**: `is_stale(heartbeat, now, ttl)` returns true when
   `now - heartbeat.timestamp > ttl`, false otherwise; boundary cases at
   exactly `ttl` and `ttl + epsilon`.
5. **Attempt counter**: with 0/1/3 prior `started` markers, `next_attempt`
   returns 0/1/3 respectively. Gaps in attempt numbers (started.0 + started.2,
   no started.1) return 3 (max + 1).
6. **Out-of-order commit**: calling `write_completed` without a prior
   `write_started` for the same (phase, attempt) panics in debug.
7. **Concurrent writers** (best-effort): two threads writing different markers
   don't corrupt each other (tempfile names must be unique per writer).

## 5. Risks

| Risk | Mitigation |
|------|-----------|
| `atomic_write` is private to `manifest.rs`; duplicating it would cause protocol drift. | Extract into shared helper (`src/run_state/atomic.rs`), make `pub(crate)`. |
| Marker file naming convention (`<phase>.started`) overlaps with marker type (`started`). | Use clear `MarkerState` enum with serde rename. |
| HeartbeatWriter tests require real time. | Inject `Clock` trait; fake clock for tests, real clock for production. |
| `PhaseOrderGuard` panics in debug but the caller might not catch it. | Also log a `tracing::error!` so release builds are audible. |

## 6. References

- PRD FR-23c, FR-21
- `docs/run-state.md` — D3 protocol, kill matrix, marker JSON schemas
- `docs/plans/001-implementation-roadmap.md` — Phase 5 row T-025
- `src/manifest.rs` — existing `atomic_write` and `Manifest` implementations
- `Cargo.toml` — `tempfile`, `serde`, `chrono`, `sha2`, `tokio`, `rand`, `uuid`
