# PRD: CLO-285 — Manifest-driven artefact load with orphan-entry sweep

| Field | Value |
|-------|-------|
| Author | pi |
| Status | Draft |
| Created | 2026-05-02 |
| Task | CLO-285 |
| Depends on | CLO-283 (manifest writer), CLO-284 (phase markers, heartbeat)
| Blocks | T-031 (resumability)

## 1. Goal

Implement a typed resume-ready loader for `runs/<id>/manifest.json` that validates artefacts, drops orphan manifest entries, derives phase status from marker files, and reports writer heartbeat state so restart logic can decide whether rerun is safe.

## 2. Scope

### In scope
- Parse `runs/<id>/manifest.json` and enforce manifest schema version compatibility.
- Return a public `RunState` object containing:
  - `run_id`
  - `entries: Vec<ManifestEntry>` (post-orphan sweep)
  - `dropped_orphans: Vec<ManifestEntry>`
  - `phase_status: HashMap<String, PhaseStatus>`
- Detect orphan entries by comparing manifest hashes to `runs/<id>/markers/*.completed` and split entries into surviving vs dropped.
- Verify each surviving entry path against stored `sha256`:
  - For file kinds: hash file bytes.
  - For `kind: changes/`: hash directory digest via deterministic `dir_digest` behavior.
- Return typed load errors for schema mismatch, missing files, and corrupt files.
- Add stale/writer heartbeat detection from `runs/<id>/heartbeat.json` with TTL check.
- Derive phase status (`Started | Completed | Failed | None`) from marker files (`<phase>.started`, `<phase>.completed`, `<phase>.failed`).
- Document how this loader is used by resume path in rustdoc.

### Out of scope
- Full resume orchestration and phase rerun logic.
- Mutating `manifest.json` to remove orphans on disk.
- Attempt-directory walk/cleanup.

## 3. Acceptance Criteria

1. `LoadError` enum exists with typed variants covering at least schema mismatch, corrupted artefact bytes, and missing artefact. Writer heartbeat state (stale/live) is surfaced via `RunState.heartbeat: Option<HeartbeatStatus>` rather than error variants, so resume orchestration can decide how to act on a live writer.
2. `RunState` is returned from a typed loader method and includes both surviving entries and dropped-orphan list.
3. Loader verifies entries against actual artefacts and fails with typed error variants for missing/corrupt values.
4. Orphan sweep is deterministic and logs each dropped entry with phase/kind/sha256.
5. `cargo test` + `make check` pass on `tests/run_state_load.rs` and existing manifest tests.
6. Loader phase-status derives deterministically from marker files.
