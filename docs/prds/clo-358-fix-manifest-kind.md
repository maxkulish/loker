# PRD: Fix manifest kind mislabelling in phase-based workflows

## Context

After CLO-327 wired the phase-based runner into `loker run`, the manifest `kind` field for every Markdown artefact produced by a phase-based workflow is hardcoded to `"design.md"`, regardless of the actual filename. The filenames on disk are correct; only the manifest metadata is wrong.

## Problem

- `src/workflow/phase_bridge.rs:129` maps any output ending in `.md` to `Kind::DesignMd`.
- The `Kind` enum (`src/manifest.rs`) lacks a `PlanMd` variant, so even with a smarter mapper `plan.md` cannot be represented correctly.
- Step-based workflows (`src/workflow/mod.rs:328`, `src/phase_runner.rs:87`) also default unknown outputs to `Kind::DesignMd`, but this is a separate, smaller issue.

## Goals

1. Manifest entries for phase-based workflows report a `kind` that matches the artefact filename.
2. The `Kind` enum can represent arbitrary `.md` filenames without requiring a code change for every new workflow output name.
3. No regression in existing step-based runs or manifest consumers.
4. `make check` passes.

## Non-goals

- Changing how artefacts are written to disk (filenames stay the same).
- Generalising non-markdown kinds (`verify.json`, `phase_result.json`, etc.) — they already map correctly.
- Runtime JSON-schema validation of the manifest (the test harness already covers this).

## Acceptance criteria

- [ ] `Kind` enum supports `PlanMd` and an `OtherMd(String)` catch-all for unknown markdown filenames.
- [ ] `phase_bridge.rs` maps `design.md` → `DesignMd`, `review.md` → `ReviewMd`, `plan.md` → `PlanMd`, and any other `.md` → `OtherMd(filename)`.
- [ ] `run_state/load.rs` string conversion and `human_verifier.rs` MIME-type mapping handle `OtherMd` correctly.
- [ ] `docs/schemas/manifest.schema.json` updated to allow the new `plan.md` enum value (and any string for forward compatibility, or at minimum the new known values).
- [ ] Unit / integration test asserts that manifest `kind` matches the artefact filename per phase.
- [ ] `make check` passes (fmt + clippy + test).
- [ ] No regression in existing step-based runs.

## References

- `src/workflow/phase_bridge.rs:129`
- `src/manifest.rs`
- `src/run_state/load.rs:254-261, 389-397`
- `src/strategy/verify/human_verifier.rs:644`
- `docs/schemas/manifest.schema.json`
