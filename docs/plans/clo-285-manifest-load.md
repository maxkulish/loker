# Plan: CLO-285 Manifest-driven artefact load with orphan-entry sweep

## Context
- Design: docs/designs/clo-285-manifest-load.md
- Discovery: docs/discovery/clo-285-manifest-load.md
- Linear: https://linear.app/cloud-ai/issue/CLO-285/implement-manifest-driven-artefact-load-with-orphan-entry-sweep

## Sub-tasks

### ST1 Add run-state module scaffold and public API
**Files:** `src/run_state/mod.rs`, `src/run_state/load.rs`
- Create `PhaseStatus`, `HeartbeatStatus`, `RunState`, and `LoadError` types.
- Add `RunState::load(run_dir, heartbeat_ttl_seconds)` API and re-export via `src/lib.rs` if needed.

**Files:** `src/family.rs` (if new cross-module status enum references require updates)

**Acceptance:** `cargo test --test run_state_load -- --nocapture` compiles due type-level assertions in test scaffolding (after tests are added).

**Estimate:** M

### ST2 Implement typed load orchestration logic
**Files:** `src/run_state/load.rs`
- Parse `manifest.json`, enforce manifest/schema version checks.
- Collect completed-marker SHA set from `markers/*.completed` and derive phase status map from `*.started|*.completed|*.failed`.
- Split entries into `entries` and `dropped_orphans`.
- Verify each surviving entry against file bytes or `dir_digest`.
- Evaluate heartbeat freshness and produce `HeartbeatStatus`.
- Log each dropped orphan entry with phase/kind/sha256.

**Acceptance:** `cargo test --test run_state_load --run-only orphan_sweep_drops_orphans` passes.

**Estimate:** L

### ST3 Add `tests/run_state_load.rs` contract tests
**Files:** `tests/run_state_load.rs`
- Add tests for happy path, schema mismatch, corrupt entry, missing entry, orphan sweep, stale/live heartbeat, empty manifest, phase status derivation.
- Include changes-dir digest verification test.

**Acceptance:** `cargo test --test run_state_load -- --nocapture` passes.

### ST4 Wire module surface and docs
**Files:** `src/lib.rs`
- Export `run_state` module publicly for integration tests.
- Add rustdoc note on `RunState::load` with resume-path behavior.

**Acceptance:** `cargo test` compiles all integration tests referencing `run_state` and `cargo test --test run_state_load`.

### ST5 Keep manifest tests intact and check compatibility
**Files:** `tests/manifest.rs`, existing module surface
- Ensure `src/manifest.rs` APIs remain backward-compatible and continue to pass.

**Acceptance:** `cargo test --test manifest` passes.

### ST6 Full check gate
**Files:** none (repo-wide)

**Acceptance:** `make check` (fmt + clippy + test) passes.

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Risks
- `tests/run_state_load.rs` needs a stable marker schema contract from `CLO-284`; the plan assumes fields in `docs/run-state.md`.
- Heartbeat status semantics are conservative; resume orchestration must decide how to treat missing heartbeat files.
