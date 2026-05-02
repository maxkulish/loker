# Design: CLO-283 — Append-only manifest writer with crash-safe rewrite

## 1. Problem

Every downstream loker phase (T-026 artefact load, T-029 trace writer, T-031 resumability) needs a trustworthy, content-addressed index of the artefacts a run has produced. Today there is no such index: each strategy writes artefacts ad-hoc, and resumability must rely on file-existence heuristics that cannot detect partial writes, manual edits, or schema drift. The D3 write protocol (`docs/run-state.md`) already specifies how to write atomically (tmp+rename+fsync), and the D2 schema (`docs/schemas/manifest.schema.json`) already defines the JSON envelope. What remains is the Rust module that implements the protocol — serialising, atomically rewriting, and validating the manifest file.

## 2. Goals / Non-goals

### Goals
- **G1**: `Manifest` and `ManifestEntry` structs that round-trip with `docs/schemas/manifest.schema.json` (envelope `loker.run_id`, `schema_version: 1`, `entries: [...]`).
- **G2**: `Manifest::append(entry)` that is crash-safe via temp-file + fsync + rename + parent-dir-fsync per D3 protocol.
- **G3**: sha256 helpers for byte payloads and deterministic directory digests for `kind: changes/`.
- **G4**: `Manifest::load()` that rejects non-v1 schema versions and drops orphan entries (entries unreferenced by any `markers/<phase>.completed` marker).
- **G5**: Add `PhaseError::ArtefactSchemaMismatch` to the existing error taxonomy in `src/family.rs`.
- **G6**: All produced manifests validate against `docs/schemas/manifest.schema.json` using the existing T-002 harness.

### Non-goals
- **NG1**: Manifest compaction (entries accumulate for the life of the run; this is v0 behaviour).
- **NG2**: Streaming / incremental reader API (full-file load is acceptable for v0 sizes).
- **NG3**: Phase marker writers themselves (owned by T-025).
- **NG4**: Resumability walk / resume logic (owned by T-031).
- **NG5**: HITL pending/response file writers (T-048).

## 3. Architecture

### Module layout

```
src/manifest.rs          # Manifest, ManifestEntry, Kind, Producer, Helpers, error types
src/family.rs            # PhaseError gains ArtefactSchemaMismatch variant
src/lib.rs               # mod manifest; re-export public surface
tests/manifest.rs        # TDD test contract (9 tests from issue body)
tests/fixtures/schemas/manifest/  # positive + negative JSON fixtures (already exist)
```

### Data flow

```
+---------+     append(entry)      +---------------+     atomic_write(path)
| Manifest|  -----------------------> | NamedTempFile |  -------------------->
| (in-    |                          | in same dir   |  fsync(file)
| memory) |                          |               |  rename to manifest.json
+---------+                          +---------------+  fsync(parent dir)
     |
     | load(path)
     v
+---------+     orphan sweep         +------------------+
| JSON    | <----------------------- | markers/*.completed |
| parse   |    (drop unreferenced)   +------------------+
+---------+
```

### Dependencies

| Crate | Version | Role |
|-------|---------|------|
| `serde` / `serde_json` | workspace | JSON (de)serialisation |
| `sha2` | workspace | sha256 of payloads and directory digests |
| `chrono` | workspace | `created_at` timestamps (ISO 8601) |
| `thiserror` | workspace | `ManifestError` and `PhaseError` |
| `tempfile` | dev-dep | `TempDir` for test scaffolding |
| `jsonschema` | dev-dep | T-002 harness (already present) |

## 4. Public API surface

### 4.1 Enums and structs (src/manifest.rs)

```rust
use serde::{Deserialize, Serialize};

/// Artefact kind. Serialises to the bare strings defined in manifest.schema.json.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    #[serde(rename = "design.md")]
    DesignMd,
    #[serde(rename = "review.md")]
    ReviewMd,
    #[serde(rename = "verify.json")]
    VerifyJson,
    #[serde(rename = "phase_result.json")]
    PhaseResultJson,
    #[serde(rename = "pending.json")]
    PendingJson,
    #[serde(rename = "response.json")]
    ResponseJson,
    #[serde(rename = "summary.json")]
    SummaryJson,
    #[serde(rename = "changes/")]
    ChangesDir,
    #[serde(rename = "trace.jsonl")]
    TraceJsonl,
}

/// Producer backend that created the artefact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Producer {
    #[serde(rename = "single")]
    Single,
    #[serde(rename = "parallel")]
    Parallel,
    #[serde(rename = "escalating")]
    Escalating,
    #[serde(rename = "verify")]
    Verify,
    #[serde(rename = "hitl")]
    Hitl,
}

/// A single entry in the manifest — one artefact produced by one phase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestEntry {
    pub name: String,
    pub kind: Kind,
    pub schema_version: u32,
    pub sha256: String,
    pub producer: Producer,
    pub phase: Option<String>,
    pub attempt: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Manifest envelope — the artefact registry for a single run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    #[serde(rename = "loker.run_id")]
    pub run_id: String,
    pub schema_version: u32,
    pub entries: Vec<ManifestEntry>,
}
```

### 4.2 `Manifest` methods

```rust
use std::path::Path;

impl Manifest {
    /// Create an empty manifest for a given run id.
    pub fn new(run_id: impl Into<String>) -> Self { ... }

    /// Load a manifest file from disk, validate schema version, run orphan sweep.
    pub fn load(path: &Path) -> Result<Self, ManifestError> { ... }

    /// Append an entry and atomically rewrite the manifest file on disk.
    pub fn append(&mut self, entry: ManifestEntry, path: &Path) -> Result<(), ManifestError> { ... }

    /// Look up the sha256 of an entry by its name.  O(N) — fine for v0 sizes.
    pub fn sha256_for(&self, name: &str) -> Option<&str> { ... }

    /// Content-addressed verification: does the on-disk payload match the recorded sha256?
    pub fn verify(&self, name: &str, payload: &[u8]) -> Result<(), PhaseError> { ... }
}
```

### 4.3 Error types

```rust
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Phase(#[from] PhaseError),
}
```

In `src/family.rs`, add to the existing `PhaseError` enum:

```rust
#[error("artefact schema/version/hash mismatch: {detail}")]
ArtefactSchemaMismatch { detail: String },
```

### 4.4 Sha256 helpers

```rust
/// sha256 hex string of arbitrary bytes.
pub fn sha256_hex(bytes: &[u8]) -> String { ... }

/// Deterministic directory digest for `kind: changes/`.
/// Walks the directory recursively, collects (relative_path, sha256_hex(content))
/// for every regular file, sorts by path, produces "<path>\t<sha256>\n" per line,
/// then sha256_hex of the whole concatenation.
pub fn dir_digest(root: &Path) -> Result<String, std::io::Error> { ... }
```

### 4.5 Atomic write helper (private)

```rust
fn atomic_write(path: &Path, contents: &[u8]) -> std::io::Result<()> { ... }
```

**Implementation sequence**: NamedTempFile::new_in(parent) → write_all → sync_all(file) → persist(path) → sync_all(parent_dir).  On failure at any step, the tmp file is left for a future sweep.  No `std::fs::rename` directly — `tempfile::NamedTempFile::persist` handles portability.

**Portability note**: Parent-directory fsync (`File::open(parent_dir).sync_all()`) is POSIX-only and a no-op on Windows. The design accepts this: on Windows the rename itself is atomic; the parent dir sync merely ensures the directory entry is durably on disk for crash recovery.

### 4.6 Orphan sweep (private)

On load, after JSON parse and schema version check:

1. Collect all `manifest_entry_sha256` values from `markers/*.completed` files under the run directory.
2. Filter `self.entries` to retain only entries whose `sha256` is present in that set.
3. For each dropped entry, emit a log line: `orphan manifest entry dropped: <name> (<sha256>)`.

Note: The marker file format is defined in `docs/run-state.md` §Phase markers.  The sweep code deserialises each marker with serde and extracts the `manifest_entry_sha256` field.

## 5. Test plan

### Unit tests (`tests/manifest.rs`) — matches the TDD contract from the issue body
1. **`empty_manifest_roundtrips`** — `Manifest::new` → `append` empty via `atomic_write` → `load` → `assert_eq!`.
2. **`append_and_reload_preserves_entries`** — append two entries, reload, compare.
3. **`atomic_crash_before_rename_leaves_tmp`** — simulate abort between file write and rename: write tmp manually, assert `manifest.json` is old, assert `*.tmp` exists.
4. **`atomic_crash_after_rename_before_parent_fsync`** — simulate abort after rename: assert `manifest.json` exists and `*.tmp` is gone.
5. **`sha256_mismatch_returns_schema_error`** — create entry with sha256 A, provide payload with sha256 B, call `verify`, assert `ArtefactSchemaMismatch`.
6. **`schema_version_mismatch_rejected`** — construct JSON with `"schema_version": 2`, assert `ArtefactSchemaMismatch` on load.
7. **`orphan_sweep_drops_unreferenced_entries`** — write manifest with two entries, only one referenced by a `markers/design.completed` marker, load, assert only one remains.
8. **`changes_dir_digest_is_deterministic`** — two identical trees → equal digests; change one file → different digest.
9. **`generated_manifest_validates_against_schema`** — produce manifest, run through T-002 harness (`jsonschema` validator).

### CI integration
- `cargo test --test manifest` must be green (AC-1).
- `make check` must be clean (fmt + clippy + test) (AC-2).
- Schema validation test reuses existing `tests/schema_validation.rs` — no new test harness needed.

## 6. Migration / Rollout

Backward compatibility: **N/A**. This is a brand-new module; nothing depends on it yet. T-026, T-029, T-031 will wire into it after it lands.

Rollout order:
1. Land `src/manifest.rs` + `tests/manifest.rs` + `ArtefactSchemaMismatch` in `src/family.rs`.
2. Update `src/lib.rs` with `mod manifest`.
3. `cargo test --test manifest` green → PR.

No feature flags needed. No config changes. The `manifest.json` file format is defined by the schema independently of this code; schemas already exist.

## 7. Open questions

- **Q1: Orphan sweep logging.** The issue says "or the run's logger if trace isn't wired yet." v0 does not have a structured logger abstraction, so the sweep will `eprintln!` to stderr. Once T-029 (trace writer) lands, this should be replaced by an injected log sink. **Resolution**: use `eprintln!` for now, leave a `// TODO(T-029):` comment.
- **Q2: Marker file shape.** `docs/run-state.md` defines the marker JSON body, but `Manifest::load` needs to know where `markers/` lives relative to `manifest.json`. The current convention is `markers/` as a sibling in the run directory.  The `load` function takes a full path to `manifest.json` and resolves the markers directory as `manifest_path.parent()/markers`. Is this coupling acceptable? **Resolution**: yes — the run directory layout is stable and documented in D3. If it changes, both T-025 and T-031 would also change.