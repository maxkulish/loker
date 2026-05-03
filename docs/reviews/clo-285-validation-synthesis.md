# Validation Synthesis - CLO-285

## Context
- Branch: feat/clo-285-manifest-load
- Design: docs/designs/clo-285-manifest-load.md
- Plan: docs/plans/clo-285-manifest-load.md
- Codex report: docs/reviews/clo-285-codex-validation.md
- Gemini report: docs/reviews/clo-285-gemini-validation.md

## Raw Verdicts
- **Codex:** rework
- **Gemini:** approve_with_changes

## Must Fix Before PR

### S1 Malformed `.completed` markers silently orphan valid entries (both reviewers)
**Severity:** blocker / major
**Where:** `src/run_state/load.rs`
**What:** `has_completed_markers` is set to `true` whenever any `.completed` file exists, even if parsing fails. When parsing fails, `completed_hashes` remains empty and `orphan_sweep` drops every manifest entry. This is a correctness bug for resume paths.
**Fix:** Only set `has_completed_markers = true` after successfully parsing a `.completed` marker that yields a `manifest_entry_sha256`. If a `.completed` file is unreadable or unparseable, propagate the error or at minimum do not enable orphan sweep.

### S2 Heartbeat API inconsistency between design doc and implementation (Codex F2, Gemini F3/F4)
**Severity:** major
**What:** The design doc lists both `RunState.heartbeat: Option<HeartbeatStatus>` AND `LoadError::StaleWriter` / `LoadError::LiveWriter`. The implementation correctly chose the struct-field approach but left unused error variants and a `Missing` variant that is never constructed (`None` is used instead).
**Fix:** Update the design doc to ratify the `RunState.heartbeat` API, remove dead `LoadError` variants (`StaleWriter`, `LiveWriter`), and reconcile `HeartbeatStatus::Missing` with the `Option` wrapper (remove the `Missing` variant since `None` already covers it).

### S3 `Heartbeat` struct not publicly re-exported (Gemini F2)
**Severity:** minor
**Fix:** Add `Heartbeat` to `pub use` in `src/run_state/mod.rs`.

### S4 Phase status precedence under-tested; missing rustdoc (Codex F3)
**Severity:** major
**Fix:** Add tests for `Failed` over `Started` precedence and for conflicting marker combos. Add rustdoc to the `RunState` struct.

### S5 Small code-quality nits (Gemini F5-F8)
**Severity:** nit
**Fix:** Use `status_from_heartbeat` helper in `load()`, remove unnecessary `.clone()`, make `ArtefactCorrupt` path formatting consistent, and add `WARN:` prefix + TODO comment to orphan log.

## Out of Scope / Deferred
- None of the findings are fundamentally out of scope; all identified issues are localized to the `run_state` module.

## False Positives / Tooling Artifacts
- Codex claimed "16 failed tests" in its sandbox. Local `make check` passes with 0 failures across all test suites. This was a tooling/environment artifact.

## Re-validation (post-fix iteration)

Fix iteration applied on commit `ee46fcf`. Re-running `make check`:
- `cargo fmt` clean
- `cargo clippy -D warnings` clean
- `cargo test` all suites pass (0 failures)

All "Must Fix Before PR" items from the synthesis have been addressed:
- S1: `.completed` marker parsing now propagates IO/JSON errors; orphan sweep only enabled on successful parse.
- S2: Dead `LoadError` variants (`StaleWriter`, `LiveWriter`) removed; `HeartbeatStatus::Missing` removed; design doc updated.
- S3: `Heartbeat` re-exported from `src/run_state/mod.rs`.
- S4: Phase status precedence fully tested (`Failed` over `Started`, `Completed` over `Failed`); `RunState` has rustdoc.
- S5: `status_from_heartbeat` helper used inline; orphan log has `WARN:` prefix and TODO; path formatting consistent; unnecessary clone removed.

No new issues introduced.

## Final Verdict
approve_with_changes

One fix iteration addresses all identified issues. No design pivot required.
