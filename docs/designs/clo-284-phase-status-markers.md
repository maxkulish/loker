# Design: CLO-284 — Phase status markers (started/completed/failed) with atomic write

## 1. Problem

The loker phase runner (T-028) and resume path (T-031) need crash-safe per-phase status markers to determine which phases have started, completed, or failed. Without markers, the resume path cannot distinguish "phase is in progress" from "phase is done" without guessing, violating the crash-injection contract (FR-23c). The D3 protocol (`docs/run-state.md`) fully specifies the marker JSON schemas, atomic write protocol, and the 14-row kill matrix, but no marker writer code exists yet. The atomic write primitive (`tmp → fsync → rename → parent-dir fsync`) exists in `src/manifest.rs` as a private `atomic_write()` function but is not accessible from other modules. The discovery report identified a baseline score of 6/10 — the protocol is well-specified but the implementation is entirely new code.

## 2. Goals / Non-goals

### Goals
- `MarkerWriter` API with `write_started`, `write_completed`, `write_failed` methods backed by the D3 atomic tmp+rename protocol
- `HeartbeatWriter` Tokio task that refreshes `runs/<id>/heartbeat.json` at `heartbeat_ttl_seconds / 3` cadence
- `next_attempt(phase) -> u32` helper deriving attempt number from existing `markers/<phase>.started.*` files
- `PhaseOrderGuard` state machine enforcing: started → artefact write → manifest append → completed
- `is_stale(heartbeat, now, ttl)` helper for staleness checks
- Shared `atomic_write` helper extracted from `src/manifest.rs` into `src/run_state/atomic.rs`
- All tests passing under `make check` (fmt + clippy + test)

### Non-goals
- Manifest writer (CLO-283 / T-024 — already done)
- Resume / load-time sweep (T-031)
- Attempt directories (T-027) — attempt number is derived from markers for now
- HITL state markers (M10)
- Advisory file lock on `runs/<id>/.lock` (T-031)

## 3. Architecture

### Module layout

```
src/
├── run_state/
│   ├── mod.rs              # Re-exports: MarkerWriter, HeartbeatWriter, PhaseOrderGuard
│   ├── atomic.rs           # pub(crate) atomic_write helper (extracted from manifest.rs)
│   ├── markers.rs          # MarkerWriter, StartedMarker, CompletedMarker, FailedMarker
│   ├── heartbeat.rs        # HeartbeatWriter, HeartbeatConfig, is_stale
│   └── order.rs            # PhaseOrderGuard state machine
├── lib.rs                  # pub mod run_state;
tests/
└── run_state_markers.rs    # TDD test contract (7 tests from issue body)
```

### Data flow

```
PhaseRunner (T-028) ──→ MarkerWriter::write_started(phase, attempt)
                    ──→ (artefact write)
                    ──→ (manifest append)
                    ──→ MarkerWriter::write_completed(phase, attempt, sha256)
                    ──→ on error → MarkerWriter::write_failed(phase, attempt, reason)

HeartbeatWriter (tokio::spawn) ──→ atomic_write(heartbeat.json)
                                     every heartbeat_ttl_seconds / 3

Resumer (T-031) ──→ is_stale(heartbeat, now, ttl)
                ──→ next_attempt(phase) → u32
                ──→ list markers/ directory for *.started / *.completed / *.failed
```

### File layout under `runs/<id>/`

```
runs/<id>/
└── markers/
    ├── design.started         # JSON body: { phase, attempt, started_at, writer_pid, writer_host, heartbeat_ttl_seconds }
    ├── design.completed       # JSON body: { phase, attempt, completed_at, manifest_entry_sha256, artefact_paths }
    └── design.failed          # JSON body: { phase, attempts_made, failed_at, error_class, last_attempt_path }
└── heartbeat.json             # JSON body: { writer_pid, writer_host, tick_at }
```

### Concrete Rust types

```rust
// --- atomic.rs ---
pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()>

// --- markers.rs ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StartedMarker {
    pub phase: String,
    pub attempt: u32,
    pub started_at: DateTime<Utc>,
    pub writer_pid: u32,
    pub writer_host: String,
    pub heartbeat_ttl_seconds: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompletedMarker {
    pub phase: String,
    pub attempt: u32,
    pub completed_at: DateTime<Utc>,
    pub manifest_entry_sha256: String,
    pub artefact_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FailedMarker {
    pub phase: String,
    pub attempts_made: u32,
    pub failed_at: DateTime<Utc>,
    pub error_class: String,
    pub last_attempt_path: String,
}

pub struct MarkerWriter {
    markers_dir: PathBuf,
}

impl MarkerWriter {
    pub fn new(run_dir: &Path) -> Self
    pub fn write_started(&self, phase: &str, attempt: u32) -> Result<StartedMarker>
    pub fn write_completed(&self, phase: &str, attempt: u32, manifest_entry_sha256: &str, artefact_paths: &[String]) -> Result<CompletedMarker>
    pub fn write_failed(&self, phase: &str, attempts_made: u32, error_class: &str, last_attempt_path: &str) -> Result<FailedMarker>

    // Internal helpers
    fn marker_path(&self, phase: &str, state: &str) -> PathBuf
    fn write_marker<T: Serialize>(&self, phase: &str, state: &str, body: &T) -> Result<()>
}

// --- heartbeat.rs ---

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeartbeatBody {
    pub writer_pid: u32,
    pub writer_host: String,
    pub tick_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    pub ttl_seconds: u64,           // default 300
    pub interval_seconds: u64,      // default ttl / 3 = 100
    pub markers_dir: PathBuf,
    pub writer_pid: u32,
    pub writer_host: String,
}

pub fn is_stale(heartbeat: &HeartbeatBody, now: &DateTime<Utc>, ttl_seconds: u64) -> bool

// --- order.rs ---

#[derive(Debug, Clone, PartialEq)]
pub enum PhaseState {
    Idle,
    Started,
    ArtefactWritten,
    ManifestAppended,
    Completed,
}

pub struct PhaseOrderGuard {
    state: PhaseState,
    phase: String,
    attempt: u32,
}

impl PhaseOrderGuard {
    pub fn new(phase: String, attempt: u32) -> Self
    pub fn state(&self) -> &PhaseState
    pub fn mark_started(&mut self)
    pub fn mark_artefact_written(&mut self)
    pub fn mark_manifest_appended(&mut self)
    pub fn mark_completed(&mut self)
}
```

### Attempt counter helper

```rust
/// Derives the next attempt number for a phase by listing marker files.
/// Returns 0 if no started markers exist.
/// Returns max(attempt_numbers) + 1 if any exist (gaps don't reduce the counter).
pub fn next_attempt(markers_dir: &Path, phase: &str) -> Result<u32>
```

The implementation lists files matching `<phase>.started.*` (the `.*` accounts for crash-debris tmp files that survived sweep, but the `.started` suffix is the exact final name after rename). If no files match, returns 0. Otherwise extracts the attempt number from the JSON body of each marker and returns `max + 1`.

> **Note for implementers**: Add a `// TODO(T-027)` or similar comment in the `next_attempt` function body pointing to the future attempt-directories task. When T-027 lands, this function should switch to directory listing for better performance with many retries.

### Marker file naming convention

```
markers/<phase>.started.<attempt>   # attempt-numbered so multiple attempts can coexist
markers/<phase>.completed           # terminal — only the latest completed marker is kept
markers/<phase>.failed              # terminal — only the latest failed marker is kept
```

Tmp files during write: `.markers/<phase>.<state>.tmp.<rand64>` — created in the same directory to ensure rename is atomic (POSIX requirement).

## 4. Public API surface

```rust
// src/run_state/mod.rs

mod atomic;
mod heartbeat;
mod markers;
mod order;

pub use heartbeat::{is_stale, HeartbeatBody, HeartbeatConfig, HeartbeatWriter};
pub use markers::{CompletedMarker, FailedMarker, MarkerWriter, StartedMarker};
pub use order::{PhaseOrderGuard, PhaseState};

/// Derive next attempt number by listing existing started markers.
/// Returns 0 for first attempt; max(existing) + 1 otherwise.
pub fn next_attempt(markers_dir: &Path, phase: &str) -> Result<u32, MarkerError>;

// src/lib.rs
pub mod run_state;
```

### MarkerError

```rust
#[derive(Debug, thiserror::Error)]
pub enum MarkerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}
```

### HeartbeatWriter (Tokio task)

```rust
pub struct HeartbeatWriter;

impl HeartbeatWriter {
    /// Spawn a tokio task that writes heartbeat.json every `interval_seconds`.
    /// The task runs until the returned JoinHandle is dropped/cancelled.
    pub fn spawn(config: HeartbeatConfig) -> tokio::task::JoinHandle<()>;
}
```

The spawned task:
1. Creates the markers directory if it doesn't exist
2. Enters a loop: sleep `interval_seconds`, build `HeartbeatBody` with current time, call `atomic_write` on `heartbeat.json`
3. Logs `tracing::warn!` with the error if `atomic_write` fails (disk full, permission denied) and continues to the next tick — a missed heartbeat is not fatal since 2 more ticks must be missed before staleness
4. Exits silently if the markers directory is deleted (run cleaned up)

### Clock trait for testability

```rust
pub(crate) trait Clock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
}

pub(crate) struct RealClock;
impl Clock for RealClock {
    fn now(&self) -> DateTime<Utc> { Utc::now() }
}

#[cfg(test)]
pub(crate) struct FakeClock {
    current: Arc<Mutex<DateTime<Utc>>>,
}
#[cfg(test)]
impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> { *self.current.lock().unwrap() }
}
#[cfg(test)]
impl FakeClock {
    pub fn advance(&self, delta: chrono::Duration);
}
```

The clock is injected into `MarkerWriter` and `HeartbeatWriter`. In production they use `RealClock`; in tests they use `FakeClock`.

## 5. Test plan

All tests live in `tests/run_state_markers.rs`. Tests use `tempfile::TempDir` for isolated filesystem access.

### Unit tests

| # | Test name | What it verifies |
|---|---|---|
| 1 | `marker_roundtrip_started` | Write a started marker, read it back via `serde_json::from_str`, assert all fields match |
| 2 | `marker_roundtrip_completed` | Same for completed marker |
| 3 | `marker_roundtrip_failed` | Same for failed marker |
| 4 | `atomic_rename_crash_between_tmp_and_rename` | Simulate crash: write tmp but don't rename. Assert final path doesn't exist and no tmp debris after sweep |
| 5 | `atomic_rename_tmp_cleaned_after_success` | After successful write, assert no `.tmp.*` files remain in markers/ |
| 6 | `heartbeat_ticks_under_fake_clock` | Spawn HeartbeatWriter with FakeClock advanced by TTL. Assert heartbeat file written exactly `ttl / interval` times |
| 7 | `is_stale_returns_true_when_expired` | `is_stale` with heartbeat at T-1s returns false; at T+1s returns true |
| 8 | `is_stale_boundary_exact_ttl` | At exactly `ttl` seconds, returns false (not stale yet); at `ttl + epsilon` returns true |
| 9 | `next_attempt_zero_markers` | No started markers → returns 0 |
| 10 | `next_attempt_single_marker` | One started marker with attempt=0 → returns 1 |
| 11 | `next_attempt_three_markers` | Three started markers (0, 1, 2) → returns 3 |
| 12 | `next_attempt_with_gaps` | Started markers for attempt 0 and 2 (missing 1) → returns 3 (max + 1) |
| 13 | `out_of_order_commit_panics_in_debug` | Call `write_completed` without prior `write_started` → panics in debug build |
| 14 | `out_of_order_commit_logs_in_release` | Same call in release build logs error, does not panic |
| 15 | `concurrent_writers_no_corruption` | Two threads writing different markers don't corrupt each other's files (tmp suffix uniqueness) |
| 16 | `phase_order_guard_valid_transitions` | Walk through Idle→Started→ArtefactWritten→ManifestAppended→Completed without error |
| 17 | `phase_order_guard_invalid_skip` | Attempt Idle→Completed directly → panics in debug |

### Integration tests

No integration tests for markers alone — they are tested at the unit level. The resume path integration (T-031) will cover multi-marker scenarios.

### Manual verification

```bash
# Run the full test suite
make check

# Run only marker tests
cargo test --test run_state_markers
```

## 6. Migration / Rollout

### Extraction of atomic_write

The `atomic_write` function in `src/manifest.rs` is private. During implementation:
1. Move it to `src/run_state/atomic.rs` as `pub(crate)`
2. Re-export it from `src/run_state/mod.rs`
3. Update `src/manifest.rs` to import `crate::run_state::atomic_write`
4. Verify existing manifest tests still pass

### No feature flags needed

This is additive code in a new module. No existing code is modified except the extraction of `atomic_write` and the addition of `pub mod run_state;` to `src/lib.rs`.

### Rollout order

1. Extract `atomic_write` into shared helper (with tests)
2. Implement `MarkerWriter` (with tests)
3. Implement `PhaseOrderGuard` (with tests)
4. Implement `next_attempt` helper (with tests)
5. Implement `HeartbeatWriter` + `is_stale` helper (with tests)
6. Wire `pub mod run_state;` into `src/lib.rs`
7. `make check` green

## 7. Open questions

1. **HeartbeatWriter cancellation semantics**: If the Tokio task detects the run directory has been cleaned up (markers dir deleted), should it log a warning and exit, or keep trying? Current design: exit silently. An alternative is to keep trying with a debounced backoff so the heartbeat survives temporary NFS disconnects.

2. **PhaseOrderGuard storage**: Should the guard be a standalone struct that the caller holds across the phase lifecycle, or should it be stored inside `MarkerWriter` and tracked per-phase? Current design: standalone — the caller creates one `PhaseOrderGuard` per phase invocation and holds it until the phase completes. This is simpler but requires the caller to not lose the guard across async boundaries. **Implementation note**: T-028 (PhaseRunner) must be careful not to hold the guard across `.await` points in a way that would allow re-entrant calls to skip steps.

3. **Error recovery in HeartbeatWriter**: If an `atomic_write` call fails (disk full, permission denied), should the heartbeat task retry on the next tick or abort? Current design: log the error and continue — a missed heartbeat is not fatal (TTL gives 2 more missed ticks before staleness). An alternative is to abort so the runner knows immediately.

4. **`next_attempt` with T-027**: When attempt directories land (T-027), should `next_attempt` also scan `attempts/<phase>/` directories? Current design: markers only. The T-027 task should update this function.

5. **Writer PID type**: `u32` is used for writer_pid — portable across Unix (pid_t is i32 on Linux, but u32 covers all positive values). For Windows, process IDs exceed u32 range on extremely long-running systems; `u64` would be safer but deviates from the D3 spec. Accept u32 for v0.
