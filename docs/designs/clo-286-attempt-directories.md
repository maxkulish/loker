# Design: CLO-286 — Attempt Directories (`attempts/<phase>/<n>/`)

| Field | Value |
|-------|-------|
| Author | pi (design phase) |
| Status | Draft |
| Created | 2026-05-03 |
| Task | CLO-286 / T-027 |
| Depends on | CLO-283 (manifest), CLO-284 (markers), CLO-245 (D3 protocol) |
| Blocks | T-028 (PhaseRunner), T-031 (resumability) |

## 1. Goal

Implement the attempt-directory subsystem so that:
1. Every phase attempt gets a scoped temporary directory (`attempts/<phase>/<n>/`).
2. On success, the artefact is atomically promoted from the attempt dir to the canonical path (`<phase>/<artefact>`).
3. On failure, the attempt dir is left in place as debris for postmortem.
4. `next_attempt()` derives the next attempt number from both markers and existing attempt directories.
5. The manifest entry's `attempt` field is populated.
6. A best-effort `latest` convenience link points to the latest completed attempt.

## 2. Scope

### In scope

- **`AttemptDir` helper** (`src/run_state/attempt_dir.rs`):
  - `AttemptDir::new(run_dir, phase, attempt) → Self`
  - `fn path() -> &Path` — the attempt-scoped directory.
  - `fn create() -> io::Result<()>` — idempotent mkdir.
  - `fn promote_to_canonical(canonical_path: &Path) -> io::Result<()>` — `rename(attempt_path, canonical_path)`. On cross-device rename (impossible within same run dir), falls back to copy+remove.
  - `fn archive_on_failure() -> io::Result<()>` — noop; the attempt dir is already at the archive location.

- **`next_attempt` enhancement** (`src/run_state/markers.rs`):
  - Extend `next_attempt()` to also scan `attempts/<phase>/` directory entries.
  - Returns `max(marker_max, dir_max) + 1`. If neither exists, returns 0.
  - Rationale: after a crash between marker write and attempt creation (or vice versa), the two sources may disagree. The higher value wins.

- **Producer wiring** (`src/run_state/producer.rs` — new module):
  - `AttemptProducer { attempt_dir: PathBuf }` trait or struct.
  - Existing producers (single/parallel/escalating/verify) accept an `attempt_dir` and write all per-attempt artefacts there.
  - After successful manifest append, the producer calls `AttemptDir::promote_to_canonical()`.
  - On failure, the producer leaves the attempt dir in place.

- **Manifest `attempt` field population** (`src/manifest.rs`):
  - `ManifestEntry::from_payload()` already accepts `attempt: Option<u32>`.
  - Producers pass `Some(attempt_number)` when creating entries.
  - No schema migration needed (`attempt` was always in the schema).

- **Latest pointer** (`src/run_state/latest.rs` — new module):
  - `LatestPointer::update(run_dir, phase, attempt) -> io::Result<()>`
  - On Unix: creates/replaces symlink `run_dir/<phase>/latest → ../attempts/<phase>/<n>/`
  - On Windows / if symlink fails: writes `run_dir/<phase>/latest.json`:
    ```json
    { "attempt": 2, "path": "attempts/design/2/", "updated_at": "..." }
    ```
  - Best-effort: failures are logged (not fatal).

- **`AttemptConfig` stub** (`src/config.rs`):
  - Add `keep_attempts: AttemptRetention` (default `Unbounded`).
  - Enum: `Unbounded | Keep(usize)` — enforcement deferred to follow-up task.

### Out of scope (deferred)

- Cleanup / pruning of old attempts — config flag stubbed only.
- HITL-driven attempts (M6).
- Producer-side atomicity changes — they already use `atomic_write`.
- Changes to the canonical path layout (D3 Approach A is fixed).

## 3. Design

### 3.1 Module layout

```
src/run_state/
├── mod.rs           # Re-export AttemptDir, LatestPointer
├── atomic.rs        # existing
├── attempt_dir.rs   # NEW — AttemptDir, promotion, archive
├── latest.rs        # NEW — LatestPointer (symlink or json fallback)
├── markers.rs       # UPDATED — next_attempt scans attempt dirs
├── heartbeat.rs     # existing
├── order.rs         # existing
└── load.rs          # existing (no changes needed)
src/manifest.rs      # UPDATED — attempt field populated
src/config.rs        # UPDATED — AttemptRetention stub
tests/
└── run_state_attempts.rs  # NEW — TDD contract
```

### 3.2 AttemptDir

```rust
use std::path::{Path, PathBuf};

/// A scoped directory for one phase attempt.
pub struct AttemptDir {
    path: PathBuf,
}

impl AttemptDir {
    pub fn new(run_dir: &Path, phase: &str, attempt: u32) -> Self {
        Self {
            path: run_dir
                .join("attempts")
                .join(phase)
                .join(attempt.to_string()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Idempotent create.
    pub fn create(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.path)
    }

    /// Atomically promote the attempt dir to the canonical path.
    /// On same-filesystem, this is a single `rename(2)` call.
    /// On cross-device, falls back to copy + remove.
    pub fn promote_to_canonical(&self,
        canonical_dir: &Path,
    ) -> std::io::Result<()> {
        // Ensure canonical parent exists
        if let Some(parent) = canonical_dir.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Try atomic rename first
        match std::fs::rename(&self.path, canonical_dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
                Self::copy_tree(&self.path, canonical_dir)?;
                std::fs::remove_dir_all(&self.path)?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Copy a directory tree recursively.
    fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                Self::copy_tree(&src_path, &dst_path)?;
            } else {
                std::fs::copy(&src_path, &dst_path)?;
            }
        }
        Ok(())
    }
}
```

**Design note**: The attempt dir serves as the producer's working directory during the attempt. On success, the entire directory is atomically renamed to the canonical phase directory (e.g. `attempts/design/0/` → `design/`). On failure, the attempt dir remains at `attempts/design/0/` as debris. This matches D3's directory-rename atomicity guarantee and avoids per-file rename complexity.

### 3.3 `run_state/mod.rs` changes

Add the new modules to the public API surface:

```rust
pub(crate) mod atomic;
pub(crate) mod attempt_dir;   // NEW
pub(crate) mod heartbeat;
pub(crate) mod latest;        // NEW
pub(crate) mod markers;
pub(crate) mod order;

pub mod load;

pub(crate) use atomic::atomic_write;
pub use attempt_dir::AttemptDir;              // NEW
pub use heartbeat::{is_stale, HeartbeatBody, HeartbeatConfig, HeartbeatWriter};
pub use latest::LatestPointer;                  // NEW
pub use load::{Heartbeat, HeartbeatStatus, LoadError, PhaseStatus, RunState};
pub use markers::{
    next_attempt, CompletedMarker, FailedMarker, MarkerError, MarkerWriter, StartedMarker,
};
pub use order::{PhaseOrderGuard, PhaseState};
```

### 3.4 Config wiring (`src/config.rs`)

Introduce a `RunStateConfig` sub-struct and add it to the root `Config`:

```rust
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct RunStateConfig {
    #[serde(default)]
    pub keep_attempts: AttemptRetention,
}

impl Config {
    // Add field:
    #[serde(default)]
    pub run_state: RunStateConfig,
}
```

Because `Config` uses `#[serde(deny_unknown_fields)]`, adding `run_state` to the root struct is safe as long as existing TOML configs omit the key (the `#[serde(default)]` handles this). Existing run-state config files do not exist yet, so there is no backward-compatibility concern.

### 3.5 Attempt-aware producer pattern

```rust
// Pseudocode for a producer using AttemptDir
let attempt = next_attempt(markers_dir, phase)?;
let attempt_dir = AttemptDir::new(run_dir, phase, attempt);
attempt_dir.create()?;

// Write all attempt artefacts into attempt_dir.path()
let artefact_path = attempt_dir.path().join("design.md");
std::fs::write(&artefact_path, content)?;

// Compute sha256 on the attempt-scoped file
let payload = std::fs::read(&artefact_path)?;
let entry = ManifestEntry::from_payload(
    format!("design/design.md"), // canonical name in manifest
    Kind::DesignMd,
    1,
    Producer::Single,
    Some(phase.to_string()),
    Some(attempt),              // populate attempt field
    &payload,
);

// PhaseOrderGuard: mark_artefact_written
// Write manifest
manifest.append(entry, &manifest_path)?;

// Promote attempt to canonical
let canonical_dir = run_dir.join("design");
attempt_dir.promote_to_canonical(&canonical_dir)?;

// Update latest pointer
LatestPointer::update(run_dir, phase, attempt)?;
```

### 3.4 next_attempt enhancement

```rust
pub fn next_attempt(run_dir: &Path, phase: &str) -> Result<u32, MarkerError> {
    let markers_dir = run_dir.join("markers");
    let attempts_dir = run_dir.join("attempts").join(phase);

    let marker_max = next_attempt_from_markers(&markers_dir, phase)?;
    let dir_max = next_attempt_from_dirs(&attempts_dir)?;

    Ok(std::cmp::max(marker_max, dir_max))
}

fn next_attempt_from_dirs(attempts_dir: &Path) -> Result<u32, MarkerError> {
    let dir = match std::fs::read_dir(attempts_dir) {
        Ok(d) => d,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };

    let mut max_attempt: Option<u32> = None;
    for entry in dir {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Ok(n) = name.parse::<u32>() {
            if max_attempt.map_or(true, |m| n > m) {
                max_attempt = Some(n);
            }
        }
    }
    // Same +1 logic as next_attempt_from_markers
    Ok(max_attempt.map_or(0, |m| m + 1))
}
```

The function takes `run_dir` instead of `markers_dir` because it now needs both sources.

### 3.6 Latest pointer

```rust
pub struct LatestPointer;

impl LatestPointer {
    pub fn update(run_dir: &Path, phase: &str, attempt: u32) -> io::Result<()> {
        let latest = run_dir.join(phase).join("latest");
        let target = PathBuf::from(format!("../attempts/{phase}/{attempt}/"));

        // Best-effort: if symlink fails, fall back to json pointer
        #[cfg(unix)]
        {
            if let Some(parent) = latest.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let _ = std::fs::remove_file(&latest);
            if std::os::unix::fs::symlink(&target, &latest).is_ok() {
                return Ok(());
            }
        }

        // Fallback: write latest.json
        let pointer = run_dir.join(phase).join("latest.json");
        if let Some(parent) = pointer.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = serde_json::json!({
            "attempt": attempt,
            "path": format!("attempts/{phase}/{attempt}/"),
            "updated_at": chrono::Utc::now().to_rfc3339(),
        });
        crate::run_state::atomic_write(&pointer,
            body.to_string().as_bytes(),
        )?;
        Ok(())
    }
}
```

### 3.7 Config stub

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttemptRetention {
    Unbounded,
    Keep(usize),
}

impl Default for AttemptRetention {
    fn default() -> Self {
        Self::Unbounded
    }
}
```

## 4. Public API Surface Changes

### New public items (in `loker::run_state`)
- `pub use attempt_dir::AttemptDir;`
- `pub use latest::LatestPointer;`
- Updated: `pub fn next_attempt(run_dir: &Path, phase: &str)` (signature change from `markers_dir` to `run_dir`)

### Updated items
- `ManifestEntry::from_payload` — no change to signature; callers pass `Some(attempt)`.
- Existing tests in `tests/run_state_markers.rs` that call `next_attempt(markers_dir, phase)` will need updating to pass `run_dir`.

## 5. Test Plan (`tests/run_state_attempts.rs`)

Matches the issue-body TDD spec with D3 Approach A semantics:

1. **First attempt**: with no prior markers, `next_attempt("design")` returns 0, attempt dir created at `attempts/design/0/`, producer writes there.
2. **Second attempt after failure**: write `design.failed` for attempt 0, then `next_attempt("design")` returns 1, producer writes to `attempts/design/1/`, attempt-0 files are untouched.
3. **Manifest entry pins attempt**: after producer completes attempt 1, the manifest entry has `attempt: 1` and `name: "design/design.md"` (canonical path, attempt as metadata).
4. **Latest pointer**: after attempts 0,1,2 with 2 completing successfully, `latest` resolves to the canonical `design/` path (via `latest.json`, since the attempt dir was promoted). For in-progress or failed attempts, `latest` points to `attempts/design/<n>/` (symlink or json).
5. **Attempt counter survives restart**: simulate process restart by re-deriving `next_attempt` from disk only (no in-memory state); result matches markers + attempt dirs. Test creates attempt dirs without markers and vice versa.
6. **Cross-phase isolation**: design and review attempts are numbered independently.
7. **Promotion is atomic**: attempt-0 file exists at `attempts/design/0/design.md`, after promotion, canonical `design/design.md` exists and attempt-0 file is gone.
8. **Archive on failure**: failed attempt leaves files in `attempts/design/0/` intact.

## 6. Risks

| Risk | Mitigation |
|------|-----------|
| `next_attempt` signature change breaks existing callers | Single caller exists in tests only; update tests. |
| Directory promotion (`rename`) fails on some filesystems | `AttemptDir::promote_to_canonical` has `CrossesDevices` fallback to copy+remove. |
| Symmlink creation fails on Windows without admin | `LatestPointer` falls back to `latest.json` automatically. |
| Producers not yet wired to `run_state` system (T-028) | This task only implements the helper; T-028 wires it. Issue scope is "Update existing producers" but in current codebase, producers are not yet calling into `run_state`. We implement the helper and a stub integration. |

## 7. Migration / Rollout

### Backward compatibility

- **Existing runs without `attempts/` directory**: `next_attempt` falls back to markers-only (as before). The new code is fully backward-compatible.
- **Existing manifest entries without `attempt` field**: `load.rs` already handles `Option<u32>` as `None` for legacy entries. Orphan sweep behaviour is unchanged.
- **No schema migration**: `attempt` was always in `ManifestEntry`; this task only populates it.

### PhaseRunner wiring (T-028)

This design provides the primitives (`AttemptDir`, `LatestPointer`, updated `next_attempt`). T-028 (PhaseRunner) will integrate these into the actual producer execution loop. The interface contract is:

1. Before running a producer: call `next_attempt(run_dir, phase)`.
2. Create `AttemptDir::new(run_dir, phase, attempt)` and `attempt_dir.create()`.
3. Pass `attempt_dir.path()` to the producer as its working directory.
4. On success: call `attempt_dir.promote_to_canonical(canonical_dir)`, then `LatestPointer::update(...)`.
5. On failure: the attempt dir is already archived; write a `FailedMarker`.

## 9. Open Questions

1. **Cleanup policy**: `AttemptRetention` is stubbed but the actual enforcement (pruning old attempts) is explicitly deferred. Should there be a `loker gc` CLI subcommand for manual cleanup?
2. **Latest pointer semantics**: Should `latest` point to the latest *started* attempt or latest *completed* attempt? Current design uses **completed** attempts only (updated on promotion). If a phase fails multiple times, `latest` points to the last successful one, which may be confusing during debugging.
3. **Orphan sweep**: Should `load.rs` orphan sweep also remove stale attempt directories whose markers have been swept? Deferred per non-goals, but worth tracking.

## 8. Acceptance Criteria

- [ ] `AttemptDir` creates, promotes, and archives correctly.
- [ ] `next_attempt` derives correctly from markers + attempt dirs after restart.
- [ ] `LatestPointer` creates symlink (Unix) or `latest.json` (fallback).
- [ ] Manifest entries populate `attempt` field.
- [ ] `AttemptRetention` config stub exists.
- [ ] `make check` is green.
- [ ] `tests/run_state_attempts.rs` TDD contract passes (8 tests).
- [ ] Existing run smoke tests still pass.

## 10. References

- `docs/run-state.md` — D3 protocol, directory layout, kill matrix
- `docs/schemas/manifest.schema.json` — manifest schema
- `docs/prds/clo-284-phase-status-markers.md` — marker writer PRD
- `docs/discovery/clo-286.md` — discovery report
