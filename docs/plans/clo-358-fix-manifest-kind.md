# Plan: CLO-358 — Fix manifest kind mislabelling in phase-based workflows

## Context
- Design: `docs/designs/CLO-358-fix-manifest-kind.md`
- Discovery: `docs/discovery/clo-358.md`
- PRD: `docs/prds/clo-358-fix-manifest-kind.md`
- Linear: https://linear.app/cloud-ai/issue/CLO-358/phase-bridge-mislabels-artefact-kind-in-manifest-all-md-designmd

## Sub-tasks

### ST1 Extend `Kind` enum with `PlanMd` and `OtherMd(String)`
**Files:** `src/manifest.rs`
**Description:**
- Add `PlanMd` and `OtherMd(String)` variants to the `Kind` enum.
- Replace derived `Serialize`/`Deserialize` with manual impls so `OtherMd(s)` serialises as the bare string `s` (not `{"OtherMd":"s"}`).
- Add `is_markdown()` helper returning true for all markdown variants.
- Add unit tests: `kind_plan_md_round_trips`, `kind_other_md_round_trips`, `kind_other_md_rejects_non_md_strings`, `kind_known_variants_unchanged`.
**Acceptance:** `cargo test -q manifest::tests::kind` passes.
**Estimate:** S

### ST2 Add `kind_from_filename()` helper in `phase_bridge.rs`
**Files:** `src/workflow/phase_bridge.rs`
**Description:**
- Add `fn kind_from_filename(output: &str) -> Kind` that maps exact known names (`design.md`, `review.md`, `plan.md`) and any other `*.md` to `OtherMd`.
- Replace the existing `.ends_with(".md")` → `DesignMd` hardcode in `build_phase_config` with a call to `kind_from_filename`.
- Add unit tests: `kind_from_filename_maps_known_names`, `kind_from_filename_maps_unknown_md_to_other`.
**Acceptance:** `cargo test -q workflow::phase_bridge::tests::kind_from_filename` passes.
**Estimate:** S

### ST3 Update downstream match sites for new variants
**Files:** `src/run_state/load.rs`, `src/strategy/verify/human_verifier.rs`
**Description:**
- Add `PlanMd` and `OtherMd(s)` arms to the string-conversion match in `run_state/load.rs`.
- Add `PlanMd` and `OtherMd(_)` to the MIME-type match in `human_verifier.rs` (both map to `text/markdown`).
- Add unit tests covering the new arms.
**Acceptance:** `cargo test -q run_state::load::tests::manifest_kind` and `cargo test -q strategy::verify::human_verifier::tests::mime` pass.
**Estimate:** S

### ST4 Relax manifest JSON schema
**Files:** `docs/schemas/manifest.schema.json`
**Description:**
- Add `"plan.md"` to the `kind` enum.
- Replace the closed enum with `oneOf: [ { enum: [...] }, { type: "string", pattern: "^.*\\.md$" } ]` so any markdown kind validates.
- Add positive fixtures for `"plan.md"` and `"analysis.md"` under `tests/fixtures/schemas/manifest/positive/`.
**Acceptance:** `cargo test -q run_artefact_schemas_validate_their_fixtures` passes.
**Estimate:** S

### ST5 Integration test: manifest kind matches filename per phase
**Files:** `tests/manifest_kind_per_phase.rs` (new)
**Description:**
- Create an integration test that builds `PhaseConfig` values for `design.md`, `review.md`, `plan.md`, and `synthesis.md`, commits them via `commit_success`, loads the resulting manifest, and asserts each entry's `kind` equals the artefact filename.
**Acceptance:** `cargo test -q manifest_kind_per_phase` passes.
**Estimate:** S

### ST6 Pre-merge gate
**Files:** all of the above
**Description:**
- Run `make check` (fmt + clippy + test) to verify no regressions in step-based or phase-based runs.
**Acceptance:** `make check` exits 0.
**Estimate:** S

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Risks
- Manual serde impl for `Kind` could have an edge case in deserialization. Mitigated by comprehensive round-trip unit tests (ST1).
- Schema relaxation to allow any `*.md` might let invalid strings through during fixture validation. Mitigated by the `OtherMd` constructor only accepting `.md` inputs and the `pattern` regex in the schema.
- `PhaseConfig::single()` still defaults to `Kind::DesignMd` for step-based workflows; this is an accepted non-goal per the design doc and PRD.
