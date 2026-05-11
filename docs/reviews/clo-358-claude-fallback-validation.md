# Pre-PR validation: clo-358

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-11
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [Major] Schema not relaxed per plan ST4 — closed enum still rejects `OtherMd` kinds
**Where:** `docs/schemas/manifest.schema.json:40-51`
**What:** Plan ST4 required replacing the closed `enum` with `oneOf: [ { enum: [...] }, { type: "string", pattern: "^.*\\.md$" } ]` so any markdown kind validates, plus positive fixtures for `plan.md` *and* `analysis.md`. Only `"plan.md"` was added to the enum; no `OtherMd` fixture exists. Runtime can now emit `kind: "synthesis.md"` / `"analysis.md"` (the new integration test itself produces `synthesis.md`), but external consumers validating against this schema will reject those manifests. `tests/schema_validation.rs` only validates fixtures, so the divergence is silent in CI.
**Suggested fix:** Replace `"kind"` definition with `oneOf` of the existing enum and a `{ "type": "string", "pattern": "^.+\\.md$" }` branch; add `tests/fixtures/schemas/manifest/positive/other_md.json` with `kind: "analysis.md"`; consider a negative fixture for non-`.md` unknown kinds.

### F2 [Major] `kind_from_filename` uses full output string instead of basename — deviates from design
**Where:** `src/workflow/phase_bridge.rs:37-47`
**What:** Design "Architecture" section specifies `let name = basename(path).to_ascii_lowercase()`. The implementation lowercases the whole `output` string. Today every workflow uses flat names (`design.md`, `review.md`, etc.) so the bug is latent, but any future workflow whose `output` is `phaseX/plan.md` will get `Kind::OtherMd("phasex/plan.md")` instead of `Kind::PlanMd`, and the lowercased path will be stored as the kind string — a leaky, fragile contract.
**Suggested fix:** `let name = Path::new(output).file_name().and_then(|s| s.to_str()).unwrap_or(output).to_ascii_lowercase();` then match on `name.as_str()`. Add a unit test exercising a nested-path output.

### F3 [Minor] `OtherMd` lowercase normalization causes `entry.name` vs `entry.kind` case divergence
**Where:** `src/workflow/phase_bridge.rs:37-47` (vs `src/manifest.rs` Deserialize impl)
**What:** Design Open Question #2 (case sensitivity of `OtherMd`) was resolved silently by always lowercasing. An output named `Synthesis.MD` is stored on disk as `Synthesis.MD` (preserved as `entry.name`) but emitted as `kind: "synthesis.md"`, while the deserializer round-trips whatever string it reads. The asymmetry is surprising and not documented.
**Suggested fix:** Either (a) preserve case in `OtherMd` and add `eq_ignore_ascii_case` for the known-variant matching, or (b) document the lowercase-on-construct policy in `manifest.rs` Kind doc-comment and in the design doc's Open Question resolution.

### F4 [Info] `tests/schema_validation.rs` does not exercise runtime-produced manifests
**Where:** `tests/schema_validation.rs` (existing) and `tests/manifest_kind_per_phase.rs:41`
**What:** The new integration test loads the manifest via `Manifest::load` but never schema-validates it, so the F1 divergence is invisible. This is the structural reason the missed plan item slipped past `make check`.
**Suggested fix:** After F1's fix, extend `manifest_kind_per_phase` (or a new test) to load `manifest.json` and validate it against `docs/schemas/manifest.schema.json` using the same harness as `tests/schema_validation.rs`.

## Verdict
**approve_with_changes**

Core fix is correct: the `Kind` enum extension with manual serde impls is sound, downstream match sites (`run_state/load.rs`, `human_verifier.rs`) are updated consistently, the integration test pins the bug against regression, and `make check` passes. However two plan/design items were not completed: the schema was not relaxed per ST4, so runtime can produce manifests (including the one the new test itself creates) that fail external schema validation, and `kind_from_filename` operates on the full output string rather than the basename specified in the design — fine today, fragile tomorrow. Both are quick fixes; address F1 and F2 before merge, F3/F4 can land in a follow-up.
