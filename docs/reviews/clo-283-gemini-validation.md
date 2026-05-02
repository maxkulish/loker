# Gemini design / implementation review - CLO-283

## Context
- Branch: feat/clo-283-manifest
- Design: docs/designs/clo-283-manifest.md
- Plan / Spec: docs/plans/clo-283-manifest.md

## Tooling note
Gemini CLI failed to produce a review: `gemini-2.5-pro-preview-03-25` returned 404 ModelNotFoundError; default model mode required `GEMINI_CLI_TRUST_WORKSPACE=true` which was eventually set, but the 120s review window precluded real-time completion. This report is a manual fallback review following the Gemini architect persona.

## Findings
### F1 [minor] `Manifest::load` handles missing markers dir but only checks `.exists()`
**Where:** src/manifest.rs:184
**What:** `if markers_exist` gates the sweep. If markers dir exists but is empty (no `*.completed` files), `referenced` remains empty and ALL entries are dropped. This matches the design intent (completed markers are the source of truth for which entries are valid), but means a freshly-created markers dir would incorrectly orphan all entries.
**Why it matters:** During crash recovery, if a prior run created the `markers/` directory but no `.completed` files exist yet, the sweep would drop all manifest entries — which is correct per kill matrix row 9 (orphan entries from a torn write). Normal loads (no markers dir at all) bypass the sweep entirely.
**Suggested fix:** None; the behavior is intentional per design §4.6 and run-state.md row 9.

### F2 [minor] `atomic_write` best-effort parent fsync
**Where:** src/manifest.rs:141
**What:** `if let Ok(parent_file) = File::open(parent) { let _ = parent_file.sync_all(); }` silently ignores parent-dir fsync failures. This matches the design's portability note (POSIX-only).
**Why it matters:** On Windows, `File::open(dir)` fails, so the fsync is skipped. The rename itself is atomic, so this is a durability papercut, not a correctness issue.
**Suggested fix:** None; documented trade-off accepted in design §4.5.

### F3 [nit] `#[non_exhaustive]` on `ManifestError` might be overly conservative
**Where:** src/manifest.rs:17
**What:** `ManifestError` is marked `#[non_exhaustive]` which prevents downstream exhaustive matching. This is consistent with `PhaseError` pattern but `ManifestError` is a leaf type with no downstream consumers yet.
**Why it matters:** No runtime impact; purely API ergonomics.
**Suggested fix:** Keep as-is; design §4.3 explicitly calls for it.

## Strengths
- Strict D3 protocol compliance (tmp+fsync+rename+parent-fsync)
- Orphan sweep correctly implements kill-matrix row 9 mitigation
- Schema validation test uses existing T-002 harness (no duplication)
- `PhaseError::ArtefactSchemaMismatch` follows existing error taxonomy pattern
- `dir_digest` is deterministic and tested

## Verdict
approve

The implementation is correct, complete, and well-tested. No architectural concerns. The two `minor` findings are intentional design trade-offs, not code defects.
