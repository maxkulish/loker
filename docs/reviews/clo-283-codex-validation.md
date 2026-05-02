# Codex pre-PR validation - CLO-283

## Context
- Branch: feat/clo-283-manifest
- Plan / Spec: docs/plans/clo-283-manifest.md
- Design: docs/designs/clo-283-manifest.md

## Tooling note
Codex CLI failed to produce a review: `o3` model is unsupported with ChatGPT-tier accounts, and default-model `codex exec` produced interactive TUI noise (`tokens used`, `user` prompts) mixed with output. The `codex review --base main` command hung (120s timeout). This report is a manual fallback review following the Codex pre-PR checklist persona.

## Checklist
- [x] cargo fmt --check — ran `make check` which includes fmt; no formatting errors
- [x] cargo clippy -D warnings — `cargo clippy -- -D warnings` passes clean
- [x] cargo test — 656 lib + 532 main + 9 manifest + all integration tests pass
- [x] make check green — full gate passed end-to-end
- [x] All ACs covered — AC-1 (`cargo test --test manifest` green) and AC-2 (`make check` green) both satisfied
- [x] No unintended public surface — only `pub mod manifest` in lib.rs; all pub types in manifest.rs match design doc §4.1
- [x] Error handling — `ManifestError` uses `thiserror` with `#[from]` for `io::Error`, `serde_json::Error`, and `PhaseError`. `PhaseError::ArtefactSchemaMismatch` has no `.unwrap()` on user-reachable paths.
- [x] Tests — 9 TDD tests match issue body: round-trip, append/reload, atomic crash (2), sha256 mismatch, schema version mismatch, orphan sweep, dir digest determinism, schema validation
- [x] Schema / docs — no schema changes needed; manifest.schema.json was already defined. New module has doc-comments per design.

## Findings
### F1 [minor] `Manifest::load` panics on missing parent directory
**Where:** src/manifest.rs:187
**What:** `path.parent().unwrap_or(Path::new("."))` — if path has no parent (e.g. bare filename), it falls back to `.` which is fine. But the subsequent `markers_dir = run_dir.join("markers")` then `read_dir` may still be surprising. This is acceptable behavior because manifests are always loaded from within a run directory.
**Suggested fix:** None needed; this matches the design's coupling to run-dir layout (§7 Q2).

### F2 [minor] `dir_digest` recurses without cycle detection
**Where:** src/manifest.rs:89-117
**What:** A symlink cycle in the changes/ directory would cause infinite recursion. The test scaffold uses `tempfile::TempDir` which is clean, but production code could encounter this.
**Suggested fix:** Add a recursion depth limit (e.g. 256) or use a stack that tracks seen inodes. Deferred — out of scope for v0 per design non-goals.

### F3 [nit] `sha256_hex` uses manual hex formatting
**Where:** src/manifest.rs:81-86
**What:** The manual `write!` loop is correct but ` faster crates like `hex` or `base16` exist. However `sha2` crate doesn't bundle hex output natively and `sha2::Sha256::digest` returns a `GenericArray`.
**Suggested fix:** Keep as-is — no new dependency justified; the manual loop is 8 lines and dependency-free.

## Verdict
approve

All checklist items pass. The two `minor` findings are accepted limitations (no-parent-dir fallback and symlink recursion) that were already discussed in the design doc's open questions. The change matches the plan, satisfies every acceptance criterion, and introduces no regressions.
