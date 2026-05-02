# Plan: CLO-283 — Append-only manifest writer with crash-safe rewrite

## Context
- Design: `docs/designs/clo-283-manifest.md`
- PRD: `docs/prds/clo-283-manifest.md`
- Discovery: `docs/discovery/clo-283.md`
- Linear: https://linear.app/cloud-ai/issue/CLO-283/implement-append-only-manifest-writer-with-crash-safe-rewrite

## Sub-tasks

### ST1 — Wire module and add error variant
**Files:** `src/family.rs`, `src/lib.rs`, `Cargo.toml`
**What:**
- Add `PhaseError::ArtefactSchemaMismatch { detail: String }` to the existing `#[non_exhaustive]` enum in `src/family.rs`.
- Add `pub mod manifest;` to `src/lib.rs` (per existing pattern; public because integration tests need access).
- Promote `tempfile = "3"` from `[dev-dependencies]` to `[dependencies]` (required for `NamedTempFile` in production `src/manifest.rs`).
**Acceptance:** `cargo check` passes, existing tests still green.
**Estimate:** S

### ST2 — Core types and serde round-trip
**Files:** `src/manifest.rs`
**What:**
- Implement `Kind` and `Producer` enums with `#[serde(rename = "...")]` attributes matching `docs/schemas/manifest.schema.json`.
- Implement `ManifestEntry` and `Manifest` structs with correct serde attributes (`loker.run_id` rename, `skip_serializing_if` on `created_at`).
- Implement `Manifest::new(run_id)` returning empty manifest with `schema_version: 1`.
- Implement private `to_json` / `from_json` helpers or use `serde_json` directly.
**Acceptance:** A basic round-trip test passes: `Manifest::new` → `serde_json::to_string` → `serde_json::from_str` → `assert_eq!`.
**Estimate:** M

### ST3 — Atomic write, sha256 helpers, and append
**Files:** `src/manifest.rs`
**What:**
- Implement `sha256_hex(bytes: &[u8]) -> String` using `sha2::Sha256`.
- Implement `dir_digest(root: &Path) -> Result<String, io::Error>`: recursive walk, sort by relative path, `sha256_hex` of `"<path>\t<sha256>\n"` concatenation.
- Implement private `atomic_write(path: &Path, contents: &[u8]) -> io::Result<()>` using `NamedTempFile::new_in(parent)` → `write_all` → `sync_all(file)` → `persist(path)` → `sync_all(parent_dir)` (POSIX-only for parent; documented on Windows).
- Implement `Manifest::append(&mut self, entry, path)` pushing in-memory then calling `atomic_write`.
- Implement `Manifest::sha256_for(&self, name) -> Option<&str>`.
**Acceptance:** `cargo test --test manifest` atomic-write and sha256 tests green (tests 3, 4, 5, 8 from design §5).
**Estimate:** M

### ST4 — Load, orphan sweep, schema enforcement, and full test suite
**Files:** `src/manifest.rs`, `tests/manifest.rs`
**What:**
- Implement `Manifest::load(path: &Path) -> Result<Self, ManifestError>`: read file, JSON parse, reject `schema_version != 1` with `PhaseError::ArtefactSchemaMismatch`.
- Implement orphan sweep: resolve `path.parent()/markers`, read `*.completed` marker files, extract `manifest_entry_sha256` fields, drop unreferenced entries with `eprintln!` log + `// TODO(T-029):` comment.
- Implement `Manifest::verify(&self, name, payload) -> Result<(), PhaseError>` computing sha256 and returning `ArtefactSchemaMismatch` on mismatch.
- Write the full `tests/manifest.rs` TDD suite (9 tests from design §5):
  1. `empty_manifest_roundtrips`
  2. `append_and_reload_preserves_entries`
  3. `atomic_crash_before_rename_leaves_tmp`
  4. `atomic_crash_after_rename_before_parent_fsync`
  5. `sha256_mismatch_returns_schema_error`
  6. `schema_version_mismatch_rejected`
  7. `orphan_sweep_drops_unreferenced_entries`
  8. `changes_dir_digest_is_deterministic`
  9. `generated_manifest_validates_against_schema`
**Acceptance:** `cargo test --test manifest` is green.
**Estimate:** M

## Pre-merge gate
- `make check` (fmt + clippy + test)
- Schema validation: tests assert produced manifests validate against `docs/schemas/manifest.schema.json` via existing T-002 harness

## Risks
| Risk | Mitigation |
|------|-----------|
| `tempfile` promotion from dev-deps to deps increases compile closure | Unavoidable: `NamedTempFile` is needed for D3 atomic write in production. Dependency is already vetted in dev path. |
| Orphan sweep marker file format may drift from `docs/run-state.md` | Sweep code uses serde with a private struct; if marker format changes, compile will catch the drift. |
| `#[non_exhaustive]` `PhaseError` + new variant may break downstream pattern matches | This is additive-only; `#[non_exhaustive]` prevents exhaustive matching, so adding a variant is semver-compatible. |
| Parent-dir fsync is POSIX-only | Documented in design. Windows rename is atomic by OS guarantee; the extra fsync is a best-effort durability enhancement. |

## Notes
- Sub-tasks are ordered: ST1 → ST2 → ST3 → ST4. Each builds on the previous and is independently testable.
- No feature flags needed. The manifest module is entirely additive.
- The `tests/manifest.rs` file is the final acceptance for ST4; ST1–ST3 can be validated with inline `#[cfg(test)]` blocks or ad-hoc test commands.
