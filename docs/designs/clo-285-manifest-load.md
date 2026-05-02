# Design: CLO-285 — Manifest-driven artefact load with orphan-entry sweep

## 1. Problem

`CLO-283` added manifest persistence, but the read path still performs only schema checks and a marker-based orphan drop. T-031 resumability needs a stricter loader that (a) verifies each manifest entry against current on-disk artefacts, (b) reports dropped orphans separately, and (c) exposes phase progress plus writer heartbeat state. Without this, resume logic cannot reliably choose whether to skip, rerun, or block on an active writer.

## 2. Goals / Non-goals

### Goals
- Introduce a dedicated loader surface (`src/run_state/load.rs`) that returns a typed `RunState` for downstream resume paths.
- Preserve existing `src/manifest.rs` append/write semantics and reuse its existing types/helpers (`Manifest`, `ManifestEntry`, `Kind`, `dir_digest`, `sha256_hex`).
- Add typed load errors (`LoadError`) that distinguish schema mismatch, missing artefacts, corrupt artefacts, and heartbeat state (`StaleWriter` / `LiveWriter`).
- Keep orphan handling deterministic: only keep manifest entries whose sha256 appears in `markers/*.completed`.
- Keep docs updated with a resume-path hint in rustdoc.

### Non-goals
- Implementing full phase resume orchestration (`T-031`).
- Mutating `manifest.json` to delete orphan rows from disk.
- Reworking marker writing (`CLO-284`).

## 3. Architecture

### Modules

- `src/manifest.rs` (existing): owns manifest data model and persistence primitives.
- `src/run_state/load.rs` (new): owns load-time verification, heartbeat and marker interpretation, and `RunState` output.
- `src/run_state/mod.rs` (new): re-export loader types for integration tests and downstream phase modules.
- `tests/run_state_load.rs` (new): TDD contract from issue body.

### Data flow

```
runs/<id>/manifest.json  --> parse manifest + schema_version check --> parse heartbeat marker files --> parse markers --> per-entry verify --> orphan sweep
                                   |                                                             |
                                   +--------------------> RunState(entries, dropped_orphans, phase_status, heartbeat)
```

### `Load` algorithm

1. Read and parse `manifest.json` into `Manifest`.
2. Enforce manifest-level schema version and per-entry `entry.schema_version == 1`.
3. Read phase markers from `runs/<id>/markers/*.completed`, collect referenced sha256 set.
4. Split manifest entries into:
   - `entries` (sha256 present in completed set, or no completed markers exist)
   - `dropped_orphans` (sha256 not present).
5. For every surviving entry, resolve its artefact path relative to run directory and verify SHA-256.
6. Detect heartbeat (`runs/<id>/heartbeat.json`) freshness using `heartbeat_ttl_seconds`.
   - missing heartbeat file -> no warning and continue as `NoHeartbeat`.
   - stale -> `HeartbeatStatus::StaleWriter`
   - live -> `HeartbeatStatus::LiveWriter`
7. For each marker file set (`*.started`, `*.completed`, `*.failed`) compute a per-phase status map.

### Phase status precedence

If multiple markers exist, precedence is:
`Completed` > `Failed` > `Started` > `None`.

## 4. Public API surface

```rust
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("manifest schema mismatch: expected {expected}, found {found}")]
    ArtefactSchemaMismatch { expected: u32, found: u32, path: String },

    #[error("artefact missing: {path}")]
    ArtefactMissing { path: String },

    #[error("artefact corrupt: {path} (expected {expected}, found {found})")]
    ArtefactCorrupt { path: String, expected: String, found: String },

    #[error("live writer at pid={writer_pid}, host={writer_host}")]
    LiveWriter { writer_pid: i64, writer_host: String },

    #[error("stale writer: last_tick={last_tick}, ttl={ttl_seconds}s")]
    StaleWriter { last_tick: chrono::DateTime<chrono::Utc>, ttl_seconds: u64 },

    #[error("IO: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseStatus { Started, Completed, Failed, None }

#[derive(Debug)]
pub struct RunState {
    pub run_id: String,
    pub entries: Vec<ManifestEntry>,
    pub dropped_orphans: Vec<ManifestEntry>,
    pub phase_status: std::collections::HashMap<String, PhaseStatus>,
    pub heartbeat: Option<HeartbeatStatus>,
}

#[derive(Debug, Clone, Copy)]
pub enum HeartbeatStatus { Live(Heartbeat), Stale, Missing }

pub struct Heartbeat {
    pub writer_pid: i64,
    pub writer_host: String,
    pub tick_at: chrono::DateTime<chrono::Utc>,
}

impl RunState {
    pub fn load(
        run_dir: &std::path::Path,
        heartbeat_ttl_seconds: u64,
    ) -> Result<Self, LoadError>;
}
```

### Logging requirement

Each dropped orphan should log at `WARN` level with `phase`, `kind`, and `sha256` (use `eprintln!` with TODO comment until a logger exists).

## 5. Test plan

- Unit/integration tests in `tests/run_state_load.rs`:
  1. happy path with completed markers -> all entries retained
  2. manifest schema mismatch -> `ArtefactSchemaMismatch`
  3. changed file bytes -> `ArtefactCorrupt`
  4. deleted file -> `ArtefactMissing`
  5. orphan sweep -> dropped entries listed
  6. stale heartbeat -> `StaleWriter`
  7. fresh heartbeat -> `LiveWriter`
  8. empty manifest -> no entries, no dropped
  9. phase-status derivation from started/completed/failed markers
  10. missing `markers/` directory tolerated (all entries survive)
  11. `changes/` entry verifies via deterministic `dir_digest`

- Keep existing manifest tests unchanged; extend if needed once `run_state` API is consumed.

## 6. Migration / rollout

1. Add module `src/run_state/mod.rs` and `src/run_state/load.rs`.
2. Keep `src/manifest.rs` APIs stable.
3. Add `tests/run_state_load.rs` and validate all paths.
4. Use `#[doc = "... resume path ..."]` comment on `RunState::load`.

## 7. Open questions

- **Heartbeat missing**: should missing heartbeat be treated as `LiveWriter`-safe or neutral? This design treats it as neutral (`None`)
  and leaves resume orchestration to decide, to avoid false positives.
- **Marker conflict**: if a phase has both `started` and `completed`, use `Completed` and if both `failed` and `completed` exist, use `Completed`.
- **Directory ownership**: keep orphan logging as `eprintln!` until centralized logging exists (`CLO-029`/trace logger landings).
