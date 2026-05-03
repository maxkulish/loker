# Plan: CLO-284 — Phase status markers (started/completed/failed) with atomic write

## Context
- **Design**: docs/designs/clo-284-phase-status-markers.md
- **Discovery**: docs/discovery/clo-284.md
- **PRD**: docs/prds/clo-284-phase-status-markers.md
- **Linear**: https://linear.app/cloud-ai/issue/CLO-284/implement-phase-status-markers-startedcompletedfailed-with-atomic
- **D3 protocol**: docs/run-state.md
- **Branch**: feat/clo-284-phase-status-markers

## Sub-tasks

### ST1 — Extract atomic_write into shared module

**Description**: Move the private `atomic_write` helper from `src/manifest.rs` into a new `src/run_state/atomic.rs` module as `pub(crate)`. Re-export from `src/run_state/mod.rs`. Update `src/manifest.rs` to import via `crate::run_state::atomic_write`. Add `pub mod run_state;` to `src/lib.rs`.

**Files**:
- `src/run_state/atomic.rs` — **create**, extracted atomic write logic
- `src/run_state/mod.rs` — **create**, re-exports `pub(crate) use atomic::atomic_write;`
- `src/manifest.rs` — **modify**, replace private `atomic_write` with `crate::run_state::atomic_write` import
- `src/lib.rs` — **modify**, add `pub mod run_state;`

**Acceptance**: `cargo test --test manifest` passes (existing tests continue to work).

**Estimate**: S (~15 min)

---

### ST2 — Implement marker types + MarkerWriter + next_attempt

**Description**: Implement `StartedMarker`, `CompletedMarker`, `FailedMarker` types with `#[serde(deny_unknown_fields)]`. Implement `MarkerWriter` struct with `write_started`, `write_completed`, `write_failed` methods backed by `atomic_write`. Implement `next_attempt(markers_dir, phase) -> Result<u32>` helper. Implement `MarkerError` enum.

**Files**:
- `src/run_state/markers.rs` — **create**, all marker types + `MarkerWriter` + `next_attempt` + `MarkerError`

**Acceptance**: `cargo test --test run_state_markers` passes these tests:
- `marker_roundtrip_started`
- `marker_roundtrip_completed`
- `marker_roundtrip_failed`
- `atomic_rename_crash_between_tmp_and_rename`
- `atomic_rename_tmp_cleaned_after_success`
- `next_attempt_zero_markers`
- `next_attempt_single_marker`
- `next_attempt_three_markers`
- `next_attempt_with_gaps`
- `concurrent_writers_no_corruption`

**Depends on**: ST1 (needs `atomic_write`)

**Estimate**: M (~1-2 sessions)

---

### ST3 — Implement PhaseOrderGuard state machine

**Description**: Implement `PhaseState` enum (`Idle`, `Started`, `ArtefactWritten`, `ManifestAppended`, `Completed`) and `PhaseOrderGuard` struct. Enforce valid transitions: debug=panic, release=log. Wire into `src/run_state/mod.rs` re-exports.

**Files**:
- `src/run_state/order.rs` — **create**, `PhaseOrderGuard` + `PhaseState` + transition enforcement

**Acceptance**: `cargo test --test run_state_markers` passes:
- `phase_order_guard_valid_transitions`
- `phase_order_guard_invalid_skip`

**Depends on**: ST2 (uses `PhaseState` re-exported from run_state)

**Estimate**: S (~15-20 min)

---

### ST4 — Implement HeartbeatWriter + Clock trait + is_stale

**Description**: Implement `Clock` trait, `RealClock`, `FakeClock` for testability. Implement `HeartbeatBody`, `HeartbeatConfig`, `HeartbeatWriter::spawn` (Tokio task). Implement `is_stale(heartbeat, now, ttl)` helper.

**Files**:
- `src/run_state/heartbeat.rs` — **create**, all heartbeat components

**Acceptance**: `cargo test --test run_state_markers` passes:
- `heartbeat_ticks_under_fake_clock`
- `is_stale_returns_true_when_expired`
- `is_stale_boundary_exact_ttl`

**Depends on**: ST1 (needs `atomic_write`), ST2 (marker dir conventions)

**Estimate**: M (~1 session)

---

### ST5 — Wire state machine integration tests + `make check`

**Description**: Add remaining integration tests:
- `out_of_order_commit_panics_in_debug`
- `out_of_order_commit_logs_in_release`

Wire all re-exports in `src/run_state/mod.rs`. Run full `make check` and fix any clippy / fmt issues.

**Files**:
- `tests/run_state_markers.rs` — **modify**, ensure all 17 tests exist
- `src/run_state/mod.rs` — **verify** all re-exports are complete

**Acceptance**: `make check` (cargo fmt + cargo clippy + cargo test) green.

**Depends on**: ST2, ST3, ST4

**Estimate**: S (~15 min)

---

## Pre-merge gate

```
make check   # fmt + clippy + test
```

## Risks

1. **atomic_write extraction compatibility**: The `atomic_write` function in `src/manifest.rs` may have internal dependencies (path manipulation helpers). If tightly coupled, inlining or a refactor may be needed instead of a move. Mitigation: read the function carefully before extracting.

2. **Tokio runtime dependency for HeartbeatWriter**: The `HeartbeatWriter::spawn` returns a `JoinHandle` which requires an active Tokio runtime. Integration tests that call it need `#[tokio::test]`. The existing test file may need the `tokio` test attribute configured. Mitigation: add `use tokio;` and `#[tokio::test]` attributes as needed.

3. **Clock trait injection into MarkerWriter**: The design mentions injecting `Clock` into `MarkerWriter`, but the `Clock` trait is defined in `heartbeat.rs`. This creates a cross-module dependency (`markers.rs` needing heartbeat types). Mitigation: define `Clock` in `mod.rs` or a separate `clock.rs` within `run_state/`.

4. **serde(deny_unknown_fields) compatibility**: The `deny_unknown_fields` attribute requires all fields to be known at deserialization time. If marker files are ever read by old/new versions of the code with different fields, this will cause errors. Acceptable for v0 since only this crate writes and reads markers.
