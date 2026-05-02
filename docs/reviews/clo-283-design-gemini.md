# Gemini design review — CLO-283

## Context
- Branch: feat/clo-283-manifest
- Design: docs/designs/clo-283-manifest.md
- PRD: docs/prds/clo-283-manifest.md
- Discovery: docs/discovery/clo-283.md

## Findings

### F1 [major] `chrono::serde::iso8601` may not exist in chrono 0.4.43
**Where:** design doc §4.1, `ManifestEntry.created_at`
**What:** The design specifies `#[serde(with = "chrono::serde::iso8601", skip_serializing_if = "Option::is_none")]`. In chrono 0.4.x with the `serde` feature, there is no `chrono::serde::iso8601` module — ISO 8601 / RFC 3339 is the **default** serialization format for `DateTime<Utc>`. Adding the `with` attribute will produce a compile error.
**Why it matters:** This will break the build before any test runs.
**Suggested fix:** Remove the `with` attribute; `#[serde(skip_serializing_if = "Option::is_none")]` alone is sufficient. If a custom format is needed, use `chrono::serde::ts_seconds` or a custom module.

### F2 [major] Parent-directory fsync is not portable to Windows
**Where:** design doc §4.5, `atomic_write` helper
**What:** `File::open(parent_dir)` followed by `sync_all()` works on POSIX (opening a directory fd and calling fsync) but fails on Windows where directories cannot be opened as regular files.
**Why it matters:** The PRD states portability as a goal ("No clippy warnings" and the project targets macOS + Linux + Windows best-effort).
**Suggested fix:** On non-Unix systems, skip the parent-dir fsync step and document the reduced durability guarantee. Use `#[cfg(unix)]` to gate the parent sync, or accept the trade-off and document it.

### F3 [major] Orphan sweep criteria is imprecise in §4.6
**Where:** design doc §4.6
**What:** The text says "entries whose `sha256` is present in that set, **or whose phase/attempt marker values are otherwise referenced**." The marker files contain `manifest_entry_sha256` — they do NOT reference entries by phase/attempt. The "or" clause introduces incorrect filtering logic.
**Why it matters:** Incorrect sweep logic could retain orphan entries or incorrectly drop valid ones, violating kill-matrix row 9.
**Suggested fix:** Remove the "or whose phase/attempt" clause. The sweep should ONLY use sha256 for matching.

### F4 [minor] `#[non_exhaustive]` missing from new `ManifestError`
**Where:** design doc §4.3, `ManifestError` enum
**What:** `ManifestError` is a new public error type but lacks `#[non_exhaustive]`. The existing `PhaseError` in the same codebase uses `#[non_exhaustive]`.
**Why it matters:** Without `#[non_exhaustive]`, downstream crates or test code that match exhaustively on `ManifestError` would break when new variants are added.
**Suggested fix:** Add `#[non_exhaustive]` to `ManifestError`.

### F5 [minor] `NamedTempFile::persist` return type is ambiguous
**Where:** design doc §4.5, `atomic_write` helper
**What:** The design shows `let final_path = tmp.persist(path)?;` followed by `drop(final_path);` but does not state the type of `final_path`. In `tempfile` 3.x, `persist` returns a `Result<PathBuf, PersistError>` in some versions and `Result<File, PersistError>` in others.
**Why it matters:** Compilation ambiguity or API mismatch across tempfile minor versions.
**Suggested fix:** Update the design to specify the exact tempfile API or use a pattern that works regardless (e.g., ignore the return and just handle the `Result`).

### F6 [minor] Missing `entries` capacity hint
**Where:** design doc §4.2, `Manifest::new`
**What:** `Manifest` is created with an empty `Vec`. For large runs, this means repeated reallocation on append.
**Why it matters:** Not a correctness issue, but a performance papercut for long runs.
**Suggested fix:** Optional: add a `with_capacity` parameter to `Manifest::new` or use `entries: Vec::with_capacity(16)` as a small default.

### F7 [nit] Stray text at end of design document
**Where:** end of design doc §7
**What:** The line "Append-only manifest writer with crash-safe rewrite" appears after the open questions section.
**Why it matters:** Cosmetically messy; does not affect correctness.
**Suggested fix:** Remove the stray line.

## Strengths
- Clean separation of concerns: manifest module owns serialisation + atomicity + validation.
- Reuses existing infrastructure (T-002 schema harness, Cargo deps, tempfile dev-dep).
- 9 concrete tests matching the issue TDD contract.
- `PhaseError::ArtefactSchemaMismatch` follows the existing error taxonomy pattern.
- The D3 protocol compliance is explicit and well-documented.

## Verdict
approve_with_changes

The design is sound and will satisfy FR-23b/FR-23d. Two issues (F1, F2) are `major` but mechanical to fix — they will surface at compile time, not runtime. The orphan sweep logic (F3) must be tightened before implementation starts. Once F1–F3 are corrected (a five-minute delta in the design doc), the design is ready for plan phase.
