# Design: CLO-266 — Aggregator::Concat with per-source headings

## Problem

Discovery (`docs/discovery/clo-266.md`) found that `ParallelFanOut` already emits schema-shaped branch metadata and an `aggregator: concat` label, but there is no implementation that folds successful branch text into a deterministic phase artefact. The canonical loker design (§4.3 Aggregator, M3 Aggregator vocabulary) defines aggregators as the seam that turns N backend outputs into one artefact; CLO-266 implements the simplest member of that vocabulary, `Concat`, while preserving current parallel phase-result schema compatibility.

## Goals

- Add a real, pure concat aggregation API that returns one markdown/string artefact.
- Preserve successful target order exactly as provided by the caller; for `ParallelFanOut`, this is arrival order.
- Render one heading per successful target using a configurable template.
- Support and document exactly these heading placeholders: `{backend_id}`, `{family}`, `{index}`.
- Include failed targets in a deterministic `## Errors` footer with structured per-target fields.
- Return a documented empty-input sentinel instead of panicking.
- Keep existing `phase_result_parallel.schema.json` serialization unchanged.

## Non-goals

- Do not implement `LLMJudge`, `AnyFail`, or `Vote` behavior.
- Do not write the aggregate artefact to disk or make it the canonical phase output; the phase runner work is T-028/T-029.
- Do not make `ParallelFanOut::execute` responsible for aggregation in this task.
- Do not introduce cross-family enforcement for concat. `family_of()` already exists and judge/vote tasks will consume it.

## Architecture

### Module layout

```text
src/
  aggregator.rs          # new: pure aggregation types + concat implementation
  lib.rs                 # add `pub mod aggregator;`
  strategy/mod.rs        # keep schema-facing Aggregator label stable
```

`src/strategy/mod.rs::Aggregator` stays the schema-facing label used by `StrategyOutput` serialization. The new module owns behavior and config. To reduce import ambiguity, implementation code should prefer explicit imports (`use loker::aggregator::Aggregator as AggregatorConfig;`) when both behavioral config and schema labels are in scope. This avoids changing how existing parallel results serialize (`"concat"`, `"llm_judge"`, etc.) while giving the phase runner a concrete aggregation seam.

### Data flow

```text
ParallelFanOut / future PhaseRunner
  └─ produces ordered branch outcomes: success text or failure reason
       └─ aggregator::Aggregator::Concat { heading_template }.aggregate(input)
            ├─ successful branches -> markdown sections with rendered headings
            ├─ failed branches -> `## Errors` footer
            └─ returns AggregatedArtifact { text, successful, failed }
```

For CLO-266 tests, inputs are hand-built fixtures. Later, T-028/T-029 adapts real `ParallelFanOut` outputs plus per-branch artefact text/errors into these same input structs.

### Concrete types

```rust
pub mod aggregator {
    /// Sentinel emitted for an aggregate call with no branch outcomes.
    pub const EMPTY_CONCAT_SENTINEL: &str = "<!-- loker: concat aggregator received no target outputs -->";

    /// Behavioral aggregator config. This is distinct from the schema label in
    /// `strategy::Aggregator`.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Aggregator {
        Concat { heading_template: String },
    }

    impl Aggregator {
        pub fn concat(heading_template: impl Into<String>) -> Self;
        pub fn kind(&self) -> crate::strategy::Aggregator;
        pub fn aggregate(&self, input: AggregateInput) -> Result<AggregatedArtifact, AggregatorError>;
    }

    #[derive(Debug, Clone, Default, PartialEq, Eq)]
    pub struct AggregateInput {
        pub branches: Vec<BranchOutcome>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum BranchOutcome {
        Success(BranchSuccess),
        Failure(BranchFailure),
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BranchSuccess {
        pub backend_id: String,
        pub family: String,
        /// 1-based caller-visible index. For ParallelFanOut this should be the
        /// arrival-order position supplied by the phase runner.
        pub index: usize,
        pub output: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct BranchFailure {
        pub backend_id: String,
        pub family: String,
        /// 1-based caller-visible index.
        pub index: usize,
        pub reason: String,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct AggregatedArtifact {
        pub text: String,
        pub successful: usize,
        pub failed: usize,
    }

    #[derive(Debug, thiserror::Error, PartialEq, Eq)]
    pub enum AggregatorError {
        #[error("unsupported aggregator operation: {0}")]
        Unsupported(String),
    }
}
```

Only concat is implemented in CLO-266. The enum shape allows future aggregator behavior variants to land without changing the phase-runner seam.

### Rendering rules

1. Empty `branches` returns `AggregatedArtifact { text: EMPTY_CONCAT_SENTINEL.into(), successful: 0, failed: 0 }`.
2. Successful branches are rendered in input order.
3. A heading is rendered by literal placeholder replacement only:
   - `{backend_id}` → `BranchSuccess.backend_id`
   - `{family}` → `BranchSuccess.family`
   - `{index}` → decimal `BranchSuccess.index`
4. Unknown placeholders are left unchanged. This keeps the renderer simple and avoids a template-engine dependency in the aggregator layer.
5. Each success section shape is:

   ```markdown
   <rendered heading>

   <output with surrounding whitespace trimmed>
   ```

   Successful sections are separated from each other by exactly two newline characters (`\n\n`) for valid Markdown block separation and stable snapshots.

6. If any failures exist, append:

   ```markdown
   ## Errors

   - backend_id: <backend_id>
     family: <family>
     index: <index>
     reason: <reason with embedded newlines escaped as \\n>
   ```

7. Failure entries are ordered by input order, not by backend name, preserving the same arrival-order semantics as successes.
8. The returned string ends with exactly one trailing newline for stable snapshots.

### Schema compatibility

`AggregatedArtifact.text` is not part of `StrategyOutput` yet. CLO-266 still proves D2 compatibility by retaining the existing `StrategyOutput` serialization path and adding/keeping a schema validation test that uses `strategy::Aggregator::Concat`, a non-empty `aggregate_output_path`, and branch metadata compatible with `docs/schemas/phase_result_parallel.schema.json`.

## Public API surface

Callers can either use the config enum directly:

```rust
use loker::aggregator::{AggregateInput, Aggregator, BranchOutcome, BranchSuccess};

let artifact = Aggregator::concat("## {index}. {backend_id} ({family})")
    .aggregate(AggregateInput {
        branches: vec![BranchOutcome::Success(BranchSuccess {
            backend_id: "claude".into(),
            family: "anthropic".into(),
            index: 1,
            output: "review text".into(),
        })],
    })?;
```

Or map behavior to the schema label:

```rust
let label: loker::strategy::Aggregator = Aggregator::concat("## {backend_id}").kind();
assert_eq!(label.as_str(), "concat");
```

The crate root exports the module with `pub mod aggregator;`. No existing imports of `loker::strategy::Aggregator` need to change.

## Test plan

### Unit tests

- `concat_renders_success_sections_in_input_order`: two successes, custom heading template, assert exact text.
- `concat_preserves_unknown_placeholders`: heading contains `{unknown}` and output leaves it unchanged.
- `concat_empty_input_returns_sentinel`: no branches returns `EMPTY_CONCAT_SENTINEL` and zero counts.
- `concat_counts_success_and_failure`: mixed inputs produce expected `successful` / `failed` counts.
- `concat_kind_maps_to_strategy_label`: behavioral concat maps to `strategy::Aggregator::Concat`.

### Snapshot tests

Add `insta` as a dev-dependency. Snapshot a 3-target merge with mixed success/failure:

1. success: `claude`, family `anthropic`, index `1`
2. failure: `codex`, family `openai`, index `2`, reason `network: timeout`
3. success: `gemini`, family `google`, index `3`

The snapshot captures heading rendering and the full `## Errors` footer shape.

### Integration/schema tests

Keep or add a test in `tests/strategy_parallel_fanout.rs` (or a focused schema test) that serializes a parallel `StrategyOutput` using `strategy::Aggregator::Concat` and validates it against `docs/schemas/phase_result_parallel.schema.json`. This proves the new behavioral module did not regress D2 phase-result schema compatibility.

### Manual checks

- `cargo fmt`
- `cargo test aggregator`
- `cargo test --test strategy_parallel_fanout phase_result_validates_against_parallel_schema`
- `make check` before PR

## Migration / rollout

- This is additive. Existing `strategy::Aggregator` users continue to compile and serialize as before.
- New code should import behavioral aggregation from `loker::aggregator` and schema labels from `loker::strategy` only when constructing `StrategyOutput`.
- T-028/T-029 will wire real branch stdout and error values into `AggregateInput` and write `AggregatedArtifact.text` to `aggregate_output_path`.
- Documentation should call out the supported concat heading placeholders in rustdoc and, if a workflow grammar doc exists by then, in the aggregator grammar section.

## Open questions

None for CLO-266. The phase-runner wiring and on-disk aggregate path semantics remain intentionally tracked by T-028/T-029.
