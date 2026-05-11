# Design: CLO-358 - Phase bridge mislabels artefact kind in manifest (all .md → design.md)

## 1. Problem

Per the discovery report, users running phase-based workflows (e.g. `loker run task-kickoff`) see every Markdown artefact labelled `kind: "design.md"` in `manifest.json`, even when the file on disk is `review.md` or `plan.md`. The root cause is a temporary simplification introduced by CLO-327: `src/workflow/phase_bridge.rs:129` maps any `.md` output to `Kind::DesignMd`, and the `Kind` enum lacks a `PlanMd` variant or any representation for arbitrary markdown filenames. This now breaks the manifest as a reliable index of run artefacts, because phase-based workflows have shipped and produce semantically distinct markdown outputs (`design.md`, `review.md`, `plan.md`, plus future variants like `analysis.md`, `judge.md`, `synthesis.md`) that downstream tooling and humans need to tell apart.

## 2. Goals / Non-goals

**Goals**

- Add `Kind::PlanMd` and `Kind::OtherMd(String)` to represent `plan.md` and arbitrary `.md` filenames respectively.
- Make `phase_bridge.rs` map artefact paths to `Kind` by filename: `design.md` → `DesignMd`, `review.md` → `ReviewMd`, `plan.md` → `PlanMd`, any other `*.md` → `OtherMd(filename)`.
- Update every `match Kind` site (`src/run_state/load.rs`, `src/strategy/verify/human_verifier.rs`, plus serialization round-trip) to handle the new variants without panics or fallthrough errors.
- Update `docs/schemas/manifest.schema.json` to accept the new known values and forward-compatible unknown `*.md` strings.
- Cover with a unit test that asserts manifest `kind` matches artefact filename for each phase output and an integration test on a multi-phase workflow.
- `make check` (fmt + clippy + test) passes.

**Non-goals**

- Changing artefact filenames or on-disk layout.
- Generalising non-markdown kinds (`verify.json`, `phase_result.json`, `response.json`) - they already map correctly.
- Reworking step-based workflows beyond the additive match arms needed to compile (`src/workflow/mod.rs:328` and `src/phase_runner.rs:87` keep their `DesignMd` defaults).
- Runtime JSON-schema validation of the manifest - existing tests already exercise the schema.
- Replacing `Kind` with a string newtype (Approach A in discovery).

## 3. Architecture

The change is type-local: `Kind` gains two variants, `phase_bridge` learns a filename mapper, and downstream match sites add arms. No new modules, no new traits, no new dependencies.

```
phase_bridge::record_phase_artefact()
        │
        │  output_path: &Path (e.g. "<run-dir>/p2/plan.md")
        ▼
+----------------------+
| kind_from_filename() |  new helper, private to phase_bridge
+----------------------+
        │
        │  Kind
        ▼
manifest::Manifest::record_artefact(kind, path, …)
        │
        ▼
manifest.json on disk
  (serde renders DesignMd→"design.md", ReviewMd→"review.md",
   PlanMd→"plan.md", OtherMd(s)→s)

Downstream consumers:
  run_state::load::manifest_kind_to_string()       adds PlanMd / OtherMd arms
  strategy::verify::human_verifier::mime_for()     adds PlanMd / OtherMd arms (text/markdown)
  docs/schemas/manifest.schema.json                gains "plan.md" + forward-compatible *.md
```

Data flow for a phase-based run:

1. `PhaseRunner` produces an artefact path under the run directory.
2. `phase_bridge` resolves that path, calls `kind_from_filename(path)`, and asks the `Manifest` to record the artefact.
3. `Manifest::write()` serialises via serde; `Kind::OtherMd(s)` flattens to the literal string `s`.
4. `loker run-state load` reads the manifest back; `manifest_kind_to_string` round-trips the variant.

### `Kind` enum shape

`Kind::OtherMd(String)` must serialise as a bare string (not as `{"OtherMd": "analysis.md"}`) so the manifest stays a flat list of filename strings. This is incompatible with a plain derive on a mixed `serde(rename = …)` enum, so the implementation uses a manual `Serialize` / `Deserialize` impl that:

- Serialises known variants to their literal strings (`"design.md"`, `"review.md"`, `"plan.md"`, `"verify.json"`, …).
- Serialises `OtherMd(s)` to `s`.
- Deserialises any known string to its variant; otherwise, if the string ends with `.md`, falls back to `OtherMd(s)`; otherwise returns a serde error.

This keeps closed-enum behaviour for non-markdown kinds (so a typo like `"verfy.json"` still fails to deserialise) while making the markdown side open-ended.

### Filename mapping helper

```text
kind_from_filename(path):
  let name = basename(path).to_ascii_lowercase()
  match name.as_str() {
    "design.md" => Kind::DesignMd,
    "review.md" => Kind::ReviewMd,
    "plan.md"   => Kind::PlanMd,
    s if s.ends_with(".md")   => Kind::OtherMd(s.to_string()),
    s if s.ends_with(".json") => existing JSON branch (VerifyJson / PhaseResultJson / ResponseJson),
    _ => Kind::PhaseResultJson,   // current fallback, unchanged
  }
```

The lowercase normalisation matches the existing `output_lower` logic at `src/workflow/phase_bridge.rs:129`.

## 4. Public API surface

`Kind` lives in `src/manifest.rs`. The rest of the surface stays the same.

```rust
// src/manifest.rs
pub enum Kind {
    DesignMd,
    ReviewMd,
    PlanMd,
    OtherMd(String),
    VerifyJson,
    PhaseResultJson,
    ResponseJson,
    // ... other existing non-markdown variants unchanged
}

impl Kind {
    /// Filename that this kind serialises to in the manifest.
    pub fn as_filename(&self) -> &str;

    /// True for DesignMd, ReviewMd, PlanMd, OtherMd.
    pub fn is_markdown(&self) -> bool;
}

impl serde::Serialize for Kind { /* see Architecture */ }
impl<'de> serde::Deserialize<'de> for Kind { /* see Architecture */ }
```

`phase_bridge` exposes a single helper (private to the module):

```rust
// src/workflow/phase_bridge.rs
fn kind_from_filename(path: &std::path::Path) -> Kind;
```

`run_state::load` and `human_verifier` add match arms but no new public functions:

```rust
// src/run_state/load.rs - within the existing kind-to-string function
match kind {
    Kind::DesignMd       => "design.md".to_string(),
    Kind::ReviewMd       => "review.md".to_string(),
    Kind::PlanMd         => "plan.md".to_string(),
    Kind::OtherMd(s)     => s.clone(),
    // existing non-markdown arms unchanged
}

// src/strategy/verify/human_verifier.rs
match kind {
    Kind::DesignMd
    | Kind::ReviewMd
    | Kind::PlanMd
    | Kind::OtherMd(_) => "text/markdown",
    // existing non-markdown arms unchanged
}
```

## 5. Test plan

**Unit tests (new)**

- `manifest::tests::kind_plan_md_round_trips` — serialize `Kind::PlanMd` → `"plan.md"`, deserialize back.
- `manifest::tests::kind_other_md_round_trips` — serialize `Kind::OtherMd("analysis.md".into())` → `"analysis.md"`, deserialize back.
- `manifest::tests::kind_other_md_rejects_non_md_strings` — deserializing `"random.txt"` into `Kind` returns an error (no silent fallback into `OtherMd`).
- `manifest::tests::kind_known_variants_unchanged` — `"design.md"`, `"review.md"`, `"verify.json"`, `"phase_result.json"` still round-trip exactly as before.
- `workflow::phase_bridge::tests::kind_from_filename_maps_known_names` — parameterised over `design.md`, `review.md`, `plan.md`, plus mixed-case variants (`Plan.MD`).
- `workflow::phase_bridge::tests::kind_from_filename_maps_unknown_md_to_other` — `analysis.md` → `Kind::OtherMd("analysis.md".into())`.
- `run_state::load::tests::manifest_kind_to_string_handles_new_variants` — covers `PlanMd` and `OtherMd("judge.md".into())`.
- `strategy::verify::human_verifier::tests::mime_for_new_markdown_variants` — `PlanMd` and `OtherMd("anything.md".into())` both return `text/markdown`.

**Integration test (new)**

- `tests/manifest_kind_per_phase.rs::manifest_kind_matches_filename_for_each_phase` — drives a phase-based workflow that emits at least one `design.md`, one `review.md`, one `plan.md`, and one custom name (e.g. `synthesis.md`). After the run, loads the manifest and asserts each entry's `kind` string equals the artefact filename. Backend mocked with `wiremock` per `docs/handoff.md` (no live TensorZero).

**Schema coverage**

- Extend the existing manifest-schema fixture set to include a manifest with `"plan.md"` and one `OtherMd` value (e.g. `"analysis.md"`). The current schema check harness should validate it without modification once the schema is updated.

**Regression coverage**

- All existing step-based tests in `src/workflow/mod.rs` and `src/phase_runner.rs` must continue to pass with only the additive match arms required to compile.

**Manual verification**

1. `cargo run --bin loker -- run task-kickoff` (or any phase-based workflow that produces `plan.md`).
2. Inspect `<run-dir>/manifest.json` and confirm each artefact's `kind` matches its filename.
3. `cargo run --bin loker -- run-state list` and confirm the human-readable listing reflects the corrected kinds.
4. `make check` — fmt + clippy + test, the project's pre-merge gate.

## 6. Migration / rollout

No data migration. Old manifests written before this change only contain the previous closed enum values (`design.md`, `review.md`, …); those continue to deserialise unchanged because the known variants keep their existing string mapping.

No backward-compatibility shim is needed for in-flight runs: every `Kind` write path goes through the updated `phase_bridge` or the unchanged step-based defaults. Consumers (`run_state load`, `human_verifier`) gain match arms but their string outputs for `DesignMd` / `ReviewMd` are byte-for-byte identical.

The manifest JSON schema (`docs/schemas/manifest.schema.json`) is the only artefact that needs an explicit relaxation:

- Add `"plan.md"` to the documented enum of `kind` values.
- Either replace the closed enum with `oneOf: [enum, pattern "^.*\\.md$"]`, or relax the field to `type: string` with a `description` listing the recommended values. The exact JSON-Schema shape is deferred to implementation (see open question 1).

No feature flag is required — the bug is purely a metadata defect, the fix is additive, and no callers branch on `Kind` in a way that changes user-visible behaviour beyond the manifest string. Roll out by landing the PR on `main`; `make release` cuts the next version normally.

## 7. Open questions

1. **Schema strictness vs. forward compatibility.** Should `manifest.schema.json`'s `kind` field stay a closed enum (now including `plan.md`), or be relaxed to allow any `*.md` string? The PRD acceptance criterion accepts either; the discovery report leans toward relaxation. Closed enum gives stronger validation at schema-check time but forces a schema PR for every new workflow output filename. An open string with a `\.md$` pattern keeps the schema in lockstep with the runtime. Decision deferred to implementation.

2. **Case sensitivity of unknown markdown names.** The current `phase_bridge` lowercases the path before matching. Should `Kind::OtherMd` preserve the original filename case (e.g. `Analysis.MD`) or the normalised lowercase form? Lowercase is simpler and matches existing behaviour; preserving case is more faithful to what is on disk and to what readers see in `ls`. The discovery report does not resolve this.

3. **Should `phase_runner.rs:87` and `src/workflow/mod.rs:328` also adopt `kind_from_filename`?** The PRD calls out the step-based default as a separate, smaller issue and explicitly out of scope. The helper added by this design could be reused there in a follow-up; tracked here so it is not lost. Not changed in this design.
