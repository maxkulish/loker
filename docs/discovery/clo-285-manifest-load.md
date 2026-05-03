# Discovery Report — CLO-285: Manifest-driven artefact load with orphan-entry sweep

## Step 1 — Problem Framing

### Who is affected
The phase runner and resume logic in M5+ cannot trust manifest output yet because the loader path currently has no typed per-entry validation or run-level state summary. Callers need a trustworthy, deterministic `RunState` before deciding which phases can be skipped after a crash.

### Current behaviour vs desired behaviour
Current code in `src/manifest.rs` can parse and write a manifest, and performs orphan filtering, but it does not verify referenced artefacts against their manifest digests, does not report stale/live heartbeat state, and does not expose per-phase completion/started/failed status. We need a loader that returns `RunState` with surviving entries, dropped orphans, and typed status while distinguishing missing/corrupt artefacts.

### Why now
`CLO-283` is already merged and the next critical-path dependency (`T-031` resumability) needs a canonical load contract to safely restart phases. The issue `CLO-284` is in progress and provides marker/heartbeat producers, so implementing the downstream reader contract now reduces integration risk for resume path tests.

## Step 2 — Existing code

### What exists
- `src/manifest.rs` contains schema types (`Manifest`, `ManifestEntry`, `Kind`, `Producer`), atomic rewrite append, sha helpers, and marker-based orphan filtering.
- `src/family.rs` already has `PhaseError::ArtefactSchemaMismatch`.
- `tests/manifest.rs` covers loader/orphan/schemas and atomic rewrite behavior.
- `docs/schemas/manifest.schema.json` defines the on-disk manifest contract.
- `docs/run-state.md` defines marker names, heartbeat behavior, and stale-run semantics.

### What is missing for this task
- Typed `LoadError` variants for file load failures (`ArtefactSchemaMismatch`, `ArtefactCorrupt`, `ArtefactMissing`, `StaleWriter`, `LiveWriter`).
- Per-entry hash verification against actual artefacts (or directory digest for `changes/`).
- `RunState` return type with phase status derivation.
- Heartbeat freshness check and stale/live classification.

### Baseline score
**4 / 10.** Core manifest types and persistence are present, but the read path is insufficient for resumability safety and typed API needs a new surface.

## Step 3 — Approaches

### Approach A — Extend `src/manifest.rs` loader into a new typed load API
- **Summary**: Keep existing `Manifest` types and add `RunState`, `LoadError`, heartbeat + phase-status helpers in `manifest.rs` with a new `Manifest::load_state(run_dir)` API.
- **Pros**: Minimal churn; reuses existing `Kind`, `Producer`, `dir_digest`, and atomic helpers; fewer new module boundaries.
- **Cons**: Ties load responsibilities to manifest module (already has manifest-specific naming). 
- **Effort**: M
- **Risk**: Low

### Approach B — Add `src/run_state/load.rs` as a separate loader module
- **Summary**: Keep `manifest.rs` focused on write/index semantics and implement a separate `run_state` module that owns `RunState`, `LoadError`, marker scan, heartbeat check, and artefact verification.
- **Pros**: Clear separation between manifest persistence and resume-oriented read semantics; easier future extension to resume walk.
- **Cons**: More file/module wiring and duplicated imports.
- **Effort**: M
- **Risk**: Low-medium

## Step 4 — Choice

**Chosen: Approach B — add `src/run_state/load.rs` with a focused read API.**

It keeps `manifest.rs` aligned with its existing manifest-domain responsibilities while introducing a dedicated loader surface for resume consumers. This makes the eventual `T-031` integration cleaner and keeps responsibilities separated.

## Step 5 — Discovery debt

- `CLO-284` is in progress and may slightly alter marker JSON shape. The design above assumes the final marker format in `docs/run-state.md`.

## References

- `docs/run-state.md`
- `docs/schemas/manifest.schema.json`
- `docs/designs/CLO-283-manifest.md`
- `docs/plans/001-implementation-roadmap.md` (T-026)
