# PRD: CLO-283 — Append-only manifest writer with crash-safe rewrite

| Field | Value |
|-------|-------|
| Author | pi (discovery phase) |
| Status | Draft |
| Created | 2026-05-02 |
| Task | CLO-283 |
| Depends on | CLO-244 (D2 schemas — done), CLO-245 (D3 atomic write protocol — done) |
| Blocks | T-026, T-029, T-031 |

## 1. Goal

Implement the canonical artefact registry: a `Manifest` struct persisted at `runs/<id>/manifest.json` that is append-only in semantics (entries added, never mutated/deleted) and physically crash-safe via atomic full-file rewrite (`NamedTempFile → fsync → rename → parent-dir fsync`) per `docs/run-state.md`.

## 2. Scope

### In scope
- `Manifest` struct: envelope (`loker.run_id`, `schema_version: 1`, `entries: Vec<...>`).
- `ManifestEntry { name, kind, schema_version, sha256, producer, phase?, attempt?, created_at? }` with strict `additionalProperties: false`.
- `kind` enum: `design.md`, `review.md`, `verify.json`, `phase_result.json`, `pending.json`, `response.json`, `summary.json`, `changes/`, `trace.jsonl`.
- `producer` enum: `single`, `parallel`, `escalating`, `verify`, `hitl`.
- `Manifest::append(entry) -> Result<()>`: in-memory append + full file rewrite via the atomic-commit primitive.
- Sha256 helper for byte payloads + deterministic directory digest for `kind: changes/` (sorted relative paths + per-file sha256, newline-separated).
- Orphan-entry sweep on `Manifest::load`: drop entries whose sha256 is not referenced by any `markers/<phase>.completed` marker. Logs each drop.
- Schema-version enforcement (`schema_version != 1` → `PhaseError::ArtefactSchemaMismatch`). Same error for sha256 mismatch.
- CI schema validation: tests assert produced manifests validate against `docs/schemas/manifest.schema.json` using existing T-002 fixture pipeline.
- `PhaseError::ArtefactSchemaMismatch` added to `src/family.rs`.

### Out of scope (deferred)
- Manifest compaction (entries accumulate for the life of the run).
- HITL `pending.json` / `response.json` writers (T-048).
- Streaming reader API; full-file load is fine at v0 sizes.
- Phase marker writers themselves (T-025).
- Resumability walk (T-031).

## 3. Acceptance Criteria

1. `cargo test --test manifest` is green.
2. No clippy warnings on the new module.
3. PRD FR-23b satisfied: append-only `manifest.json` with name, kind, schema version, sha256, producer; crash-safe rewrite.
4. PRD FR-23d enforced for sha256 + schema_version mismatch.

## 4. Design

### 4.1 Module layout

```
src/manifest.rs         # Manifest, ManifestEntry, Kind, Producer, sha256 helpers
src/family.rs           # PhaseError gains ArtefactSchemaMismatch
src/lib.rs              # mod manifest; re-export per existing pattern
tests/manifest.rs       # TDD test contract from issue body
```

### 4.2 Types

```rust
// src/manifest.rs

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    DesignMd,
    ReviewMd,
    VerifyJson,
    PhaseResultJson,
    PendingJson,
    ResponseJson,
    SummaryJson,
    ChangesDir,   // serializes as "changes/"
    TraceJsonl,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Producer {
    Single,
    Parallel,
    Escalating,
    Verify,
    Hitl,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestEntry {
    pub name: String,
    pub kind: Kind,
    pub schema_version: u32,
    pub sha256: String,
    pub producer: Producer,
    pub phase: Option<String>,
    pub attempt: Option<u32>,
    #[serde(with = "chrono::serde::iso8601")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    #[serde(rename = "loker.run_id")]
    pub run_id: String,
    pub schema_version: u32,
    pub entries: Vec<ManifestEntry>,
}
```

### 4.3 Atomic commit helper

```rust
fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut tmp = NamedTempFile::new_in(parent)?;
    tmp.write_all(contents)?;
    tmp.as_file().sync_all()?;
    let final_path = tmp.persist(path)?;
    let mut parent_file = File::open(parent)?;
    parent_file.sync_all()?;
    drop(final_path);
    Ok(())
}
```

### 4.4 Manifest operations

- `Manifest::new(run_id: impl Into<String>) -> Self` — empty entries, `schema_version: 1`.
- `Manifest::load(path: &Path) -> Result<Self, ManifestError>` — read, JSON parse, schema_version != 1 rejected with `ArtefactSchemaMismatch`, run orphan sweep.
- `Manifest::append(&mut self, entry: ManifestEntry) -> Result<(), ManifestError>` — push in-memory if version/sha256 OK, then call `atomic_write(self.path, new_json)`.
- `Manifest::sha256_for(&self, name: &str) -> Option<&str>` — O(N) lookup sufficient at v0.
- Orphan sweep reads `markers/` dir for `*.completed` files, extracts their `manifest_entry_sha256` fields, drops entries whose sha256 is not present.

### 4.5 Error variants

```rust
// src/family.rs PhaseError
#[error("artefact schema/version/hash mismatch: {detail}")]
ArtefactSchemaMismatch { detail: String },
```

### 4.6 Test contract (`tests/manifest.rs`)

Matches the issue-body TDD spec:

1. Empty manifest round-trips (write → load → equals).
2. Append + reload produces identical entries.
3. Atomic crash: simulate abort between tmp write and rename; reload yields old state with `*.tmp` left.
4. Atomic crash post-rename, pre-parent-fsync: rename target exists, tmp is gone.
5. Sha256 mismatch on a referenced artefact returns `ArtefactSchemaMismatch`.
6. Schema_version mismatch on entry envelope returns `ArtefactSchemaMismatch`.
7. Orphan sweep: entry with sha256 X but no `markers/<phase>.completed` referencing X → load drops it.
8. `kind: changes/` directory digest: deterministic across two calls; differs when one file content changes.
9. Generated manifest validates against `docs/schemas/manifest.schema.json`.

All tests use `tempfile::TempDir` for run-dir scaffolding.

## 5. Risks

| Risk | Mitigation |
|------|-----------|
| `NamedTempFile::persist` fails across filesystem boundaries on Linux. | Ensure tmp is created in same directory as final (`new_in(parent)`). | |
| Orphan sweep requires regexing JSON marker files. | Use serde for marker parse; `tempfile::TempDir` in tests. No real filesystem touched outside test root. | |
| `PhaseError` addition breaks downstream callers expecting exhaustiveness. | `#[non_exhaustive]` already present; new variant is additive only. | |

## 6. References

- PRD FR-23b, FR-23d
- `docs/schemas/manifest.schema.json`
- `docs/run-state.md` §"Atomic file commit primitive", §"Manifest rewrite", kill-matrix rows 8-9
- `tests/schema_validation.rs` (T-002 harness)
- CLO-244 (D2 schemas), CLO-245 (D3 atomic run-state)
