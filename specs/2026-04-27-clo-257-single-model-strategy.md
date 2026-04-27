# Spec: CLO-257 Implement Strategy::SingleModel

**Created**: 2026-04-27
**Estimated scope**: S (3 production files + 1 integration test, ~4 sub-tasks)
**Linear**: [CLO-257](https://linear.app/cloud-ai/issue/CLO-257/implement-strategysinglemodel)
**PRD**: FR-5 (`docs/prd/2026-04-25-loker.md`)
**Roadmap**: T-011 (`docs/plans/001-implementation-roadmap.md`)
**Design doc**: `/Users/mk/Work/investigations/sakana-fugu/loker-design.md` §4.2

## 1. Problem Statement

The loker engine has no `Strategy` primitive yet. Today the workflow runner at
`src/workflow.rs:~1758-2200` interleaves backend dispatch, consensus aggregation,
apply_edits, and verify-hook plumbing inside one large match. That works for the
M0/M1 fork-and-tensorzero milestones but does not give T-029 (the phase runner)
or CLO-259 (`ParallelFanOut`) the trait-shaped seam they were promised by the
PRD and design doc §4.2:

```rust
#[async_trait]
trait Strategy {
    async fn execute(
        &self,
        backends: &[Box<dyn Backend>],
        prompt: Prompt,
        ctx: &PhaseContext,
    ) -> Result<StrategyOutput, StrategyError>;
}
```

Three primitives are scoped (design doc §4.2): `SingleModel`, `ParallelFanOut`,
`EscalatingRetry`. CLO-257 lands the first one - the simplest variant - and
defines the trait shape that the other two will reuse. Concretely the work is:

1. Add a new `src/strategy/` module with the `Strategy` trait and the value
   types (`Prompt`, `PhaseContext`, `StrategyOutput`, `StrategyError`,
   `Attempt`, `FinishReason`, `VerifyStatus`) used by its signature and return.
2. Implement `SingleModel { backend: String, prompt_template: String }`: render
   the template, call the chosen backend's `query()` once, wrap the
   `QueryOutput` into a one-element `attempts` vector, return.
3. Cover happy path and error pass-through with unit tests against a
   `MockBackend` (M2 test contract from design doc §8 says "each strategy
   unit-tested with a `MockBackend` returning canned responses").
4. Cover the D2 contract: serialising `StrategyOutput` for a SingleModel run
   produces JSON that validates against
   `docs/schemas/phase_result_single.schema.json`.

What is **out of scope** (deferred to later tasks named in the design doc):
- Wiring `SingleModel` into `workflow.rs::run_workflow`. The phase runner
  T-029 (CLO-261) is what consumes strategy outputs and writes
  `phase_result_*.json` to disk. CLO-257 only proves the trait shape and the
  serialisation; no production call site flips today.
- Verify-hook integration. `VerifyHook` lands in M4 / T-028. `Attempt.verify`
  in v0 is hard-coded to `VerifyStatus::Skipped { hook: None }`.
- `ParallelFanOut` / `EscalatingRetry`. CLO-259 / CLO-258. They share the
  trait but their bodies are separate variants in separate files.
- `Aggregator`. M3. SingleModel never aggregates - one backend, one response.
- Streaming, tool-use. Those are FR-4 capabilities (CLO-251, just landed) and
  remain unwired until later milestones.

The acceptance test from the Linear ticket - "snapshot of produced phase
result file matches D2 schema" - is satisfied by serialising `StrategyOutput`
to JSON and validating against `phase_result_single.schema.json` using the
same `jsonschema` crate already in use under `tests/schema_validation.rs`.

## 2. Acceptance Criteria

- [ ] **AC1**: New module `src/strategy/` exists with `mod.rs` exporting the
      public surface (`Strategy`, `Prompt`, `PhaseContext`, `StrategyOutput`,
      `StrategyError`, `Attempt`, `FinishReason`, `VerifyStatus`,
      `TokenUsageReport`) and a child module `single_model` exporting
      `SingleModel`. Wired into `src/lib.rs`.
- [ ] **AC2**: `Strategy` trait shape matches design doc §4.2 exactly:
      ```rust
      #[async_trait]
      pub trait Strategy: Send + Sync {
          async fn execute(
              &self,
              backends: &[Arc<dyn Backend>],
              prompt: &Prompt,
              ctx: &PhaseContext,
          ) -> Result<StrategyOutput, StrategyError>;
      }
      ```
      Deviation note: the design doc shows `&[Box<dyn Backend>]` but the
      existing engine uses `Arc<dyn Backend>` everywhere
      (`src/backend/mod.rs:346-374` `create_backend`, `src/workflow.rs`
      step-execution path). We follow the in-tree convention; this is the
      documented deviation. `prompt: &Prompt` (borrowed) keeps the trait
      object-safe and lets the same prompt be replayed by future strategies
      without cloning.
- [ ] **AC3**: `Prompt` is a struct with two public fields: `template:
      String` (mini-jinja source) and `model: Option<String>` (passes through
      to `Backend::query(.., model)`). Constructed via `Prompt::new(template)`
      and `Prompt::with_model(self, model)`. Marked `#[non_exhaustive]` so
      future fields (system prompt, tool definitions) land additively.
- [ ] **AC4**: `PhaseContext` is a struct carrying the metadata the
      strategy needs to render templates and stamp result rows: `phase_name:
      String`, `run_id: uuid::Uuid`, `template_engine:
      Arc<crate::template::TemplateEngine>`, `template_context:
      crate::template::context::TemplateContext`. Marked `#[non_exhaustive]`.
      `PhaseContext::new_for_test(...)` is provided behind `#[cfg(test)]` for
      tests; production callers (CLO-261 / T-029) build it themselves.
- [ ] **AC5**: `StrategyOutput` mirrors the D2 schema fields exactly so that
      `serde_json::to_value(&output)` validates against
      `phase_result_single.schema.json`:
      ```rust
      #[derive(Debug, Clone, serde::Serialize)]
      pub struct StrategyOutput {
          pub schema_version: u32,                 // const 1, hard-coded in constructor
          #[serde(rename = "loker.strategy")]
          pub strategy: StrategyKind,              // serializes as "single"
          #[serde(rename = "loker.phase")]
          pub phase: String,                       // from PhaseContext.phase_name
          #[serde(rename = "loker.run_id")]
          pub run_id: uuid::Uuid,                  // from PhaseContext.run_id
          pub attempts: Vec<Attempt>,              // exactly 1 entry for SingleModel
      }
      ```
      `StrategyKind::Single` serialises to the literal string `"single"`.
- [ ] **AC6**: `Attempt` carries the per-call record:
      ```rust
      pub struct Attempt {
          pub backend: String,                     // from QueryOutput.backend
          pub model: String,                       // from QueryOutput.model OR Prompt.model OR "default"
          pub finish_reasons: Vec<FinishReason>,   // ["stop"] on success in v0
          pub usage: TokenUsageReport,             // {input_tokens, output_tokens}
          pub output_path: String,                 // synthetic for v0: "attempts/0.txt"
          pub verify: VerifyOutcome,               // {status: skipped, hook: None} in v0
      }
      ```
      `FinishReason` is a serde enum mapping to the schema's allowed strings:
      `stop`, `length`, `tool_calls`, `content_filter`, `error`. v0 maps
      every successful backend call to `FinishReason::Stop`; the value
      becomes data-driven later when a backend exposes a real finish reason.
- [ ] **AC7**: `TokenUsageReport` flattens to `{input_tokens, output_tokens}`
      (the schema names) - **not** the existing `TokenUsage`'s `prompt_tokens`
      / `completion_tokens` (which match an OpenAI shape). Conversion lives
      on a `From<&TokenUsage> for TokenUsageReport` impl in
      `src/strategy/mod.rs`. Backends without usage reporting yield
      `TokenUsageReport { input_tokens: 0, output_tokens: 0 }`.
- [ ] **AC8**: `VerifyOutcome` is a struct with `status: VerifyStatus` and
      `hook: Option<String>`. `VerifyStatus` is a serde enum: `Pass`, `Fail`,
      `Skipped`. v0 always emits `VerifyOutcome { status: Skipped, hook: None
      }` - no verify hook is invoked. The struct is in place so T-028 can
      flip the behaviour without breaking SingleModel call sites.
- [ ] **AC9**: `StrategyError` is a thiserror enum with at minimum:
      ```rust
      pub enum StrategyError {
          BackendNotFound { name: String },
          NoBackends,
          PromptRender(crate::template::TemplateError),
          Backend(BackendError),
      }
      ```
      `From<BackendError>` is implemented so the `?` operator inside
      `SingleModel::execute` propagates the backend's typed error unchanged
      (acceptance criterion: "error surfaces unchanged"). The `Backend`
      variant `Display`s by delegating to the inner error.
- [ ] **AC10**: `SingleModel { backend: String, prompt_template: String }`
      lives in `src/strategy/single_model.rs`. Its constructor is `pub fn
      new(backend: impl Into<String>, prompt_template: impl Into<String>) ->
      Self`. Its `execute()` body:
      1. resolves `self.backend` against `backends: &[Arc<dyn Backend>]` by
         matching `backend.name() == self.backend` -> `StrategyError::
         BackendNotFound` if no match, `StrategyError::NoBackends` if the
         slice is empty;
      2. renders `self.prompt_template` through `ctx.template_engine.render(
         &self.prompt_template, &ctx.template_context)` -> wraps any error
         in `StrategyError::PromptRender`;
      3. calls `chosen_backend.query(&rendered_prompt, &ctx.cwd_for_test(),
         prompt.model.as_deref()).await?` (`?` propagates `BackendError` via
         `StrategyError::Backend`);
      4. wraps the resulting `QueryOutput` into one `Attempt`, builds the
         `StrategyOutput` with `schema_version: 1`, `strategy: Single`, `phase`
         and `run_id` from `ctx`, returns `Ok(output)`.
- [ ] **AC11**: New unit test file `tests/strategy_single_model.rs` exercises:
      - **happy path** - mock backend returns a fixed `QueryOutput`,
        `SingleModel::execute` returns `StrategyOutput` with exactly one
        attempt, the attempt's `backend`, `model`, `finish_reasons`, and
        `usage` fields match the canned response, the call counter on the
        mock equals 1 (one call out, one response in);
      - **error pass-through** - mock backend returns `BackendError::Timeout
        { elapsed_ms: 500 }`, `SingleModel::execute` returns
        `Err(StrategyError::Backend(BackendError::Timeout { elapsed_ms: 500,
        .. }))` with the elapsed_ms preserved; the error surfaces unchanged;
      - **no retry** - the call counter on the mock equals 1 even on the
        error path (SingleModel never retries; that's `EscalatingRetry`'s
        job);
      - **no aggregation** - the returned `attempts` vector has length 1;
      - **D2 schema snapshot** - `serde_json::to_value(&output)` validates
        against `docs/schemas/phase_result_single.schema.json` via the
        `jsonschema` crate (same approach as `tests/schema_validation.rs`).
        The validation is a hard assertion, not a manual eyeball check.
      - **prompt-render failure** - if `prompt_template` references an
        undefined variable, `execute()` returns
        `Err(StrategyError::PromptRender(_))` and the mock backend's call
        counter equals 0 (we fail before going out to the backend);
      - **backend not found** - constructing `SingleModel::new("missing",
        ...)` with a `backends` slice that does not contain a backend named
        `"missing"` returns `Err(StrategyError::BackendNotFound { name:
        "missing" })`;
      - **empty backends slice** - returns `Err(StrategyError::NoBackends)`.
- [ ] **AC12**: Trait extensibility check. The trait signature uses `&[Arc<dyn
      Backend>]` and `&Prompt`, both borrowed. Adding `ParallelFanOut`
      (CLO-259) and `EscalatingRetry` (CLO-258) implementations does not
      require modifying `SingleModel` or its call sites. Verified by writing
      a stub `struct StubFanOut; impl Strategy for StubFanOut { ... }` in a
      `#[cfg(test)] mod future_variant_compiles` block under
      `src/strategy/mod.rs` that returns `unimplemented!()` from `execute()`
      - it is a compile-time check, never run. (This is the "trait shape
      allows future variants without breaking SingleModel call sites" AC
      from the Linear ticket.)
- [ ] **AC13**: `make check` exits 0 (fmt + clippy + lib + integration tests
      green).

**Verification method**:
- AC1, AC2, AC3, AC4, AC5, AC6, AC7, AC8, AC9, AC10: `cargo build` + diff
  inspection.
- AC11: `cargo test --test strategy_single_model -- --nocapture`.
- AC12: `cargo build` (the stub impl is `#[cfg(test)]`, surfaces in `cargo
  test` build).
- AC13: `make check`.

## 3. Constraints

**Must**:
- Match the `Strategy` trait signature documented in design doc §4.2
  modulo the `Box<dyn Backend>` -> `Arc<dyn Backend>` deviation (justified
  in AC2). Do not invent new parameters; all four (`self`, `backends`,
  `prompt`, `ctx`) are required so that `ParallelFanOut` and
  `EscalatingRetry` can implement the same trait without divergence.
- Make `StrategyOutput` serialise to JSON that validates against
  `docs/schemas/phase_result_single.schema.json` *as written today*
  (`schema_version: 1`, `loker.strategy: "single"`, `attempts.len() == 1`).
  If the schema and the struct disagree, the schema wins - we change the
  struct, not the schema.
- Use `Arc<dyn Backend>` (not `Box`) to match the rest of the codebase
  (`src/backend/mod.rs:346`, `src/workflow.rs` step-execution).
- Pass `BackendError` through unchanged via `From<BackendError> for
  StrategyError`. The Linear AC says "error surfaces unchanged"; that means
  no remapping, no re-wrapping into a generic message. The unit test
  pattern-matches on the inner variant (`Timeout { elapsed_ms: 500, .. }`)
  to enforce this.
- Render templates via the existing `crate::template::TemplateEngine`. Do
  not introduce a second template engine. The engine is already covered by
  unit tests in `src/template/mod.rs:130-200`.
- Use `async_trait` for `Strategy`, matching the existing pattern on
  `Backend` (`src/backend/mod.rs:307`).

**Must-not**:
- Wire `SingleModel` into `src/workflow.rs` in this PR. That's CLO-261 /
  T-029. Touching `workflow.rs` invites scope creep and conflicts with
  in-flight work on the runner. Spec explicitly carves this out.
- Implement any verify hook. v0 hard-codes
  `VerifyOutcome::skipped()`. Touching verify is T-028.
- Implement `ParallelFanOut` or `EscalatingRetry`. They are separate
  Linear tickets (CLO-259, CLO-258); each must be designed independently.
  Do *not* anticipate their fields by adding `min_responses` or
  `pass_failure_context` to the trait or to `Prompt` / `PhaseContext`.
- Add a retry policy inside `SingleModel`. Backend-level retries are
  already handled by `RetryExecutor` (`src/backend/retry.rs`); strategy-
  level retries are `EscalatingRetry`'s job by design.
- Edit `Cargo.toml` other than to add the `uuid` and `jsonschema` deps if
  they aren't present already. Both are likely already in `[dev-
  dependencies]` for tests. Check before adding.
- Touch `src/backend/`, `src/workflow.rs`, `src/consensus.rs`. The strategy
  module is *additive*. SingleModel does not modify existing structs.
- Edit example workflows under `examples/workflows/` or
  `tests/workflows/`. SingleModel is not yet wired to the loader.

**Prefer**:
- One file per primitive: `src/strategy/single_model.rs` for SingleModel.
  When CLO-258 / CLO-259 land they get their own files
  (`escalating_retry.rs`, `parallel_fan_out.rs`); `mod.rs` only carries
  the trait + value types.
- A `MockBackend` defined inline in `tests/strategy_single_model.rs` (not
  exported from the crate). It implements `Backend`, holds an
  `Arc<AtomicUsize>` call counter, an `Arc<Mutex<Option<Result<QueryOutput,
  BackendError>>>>` canned response, and exposes the counter for assertion.
  Keep the mock under 50 lines.
- `#[non_exhaustive]` on `Prompt`, `PhaseContext`, `StrategyError`,
  `StrategyKind`, `FinishReason`, `VerifyStatus` - so future variants and
  fields are additive. This mirrors the FR-4 `BackendCapabilities` choice.
- A `pub const SCHEMA_VERSION: u32 = 1` constant on `StrategyOutput` so
  callers don't sprinkle magic numbers.
- Test helper `PhaseContext::new_for_test(phase_name, run_id)` behind
  `#[cfg(any(test, feature = "test-utils"))]` - tests build a context
  without dragging in a full TemplateEngine when the template under test
  is the literal string (no variables). The helper supplies a fresh
  `TemplateEngine::new()` and a `TemplateContext::default()`.

**Escalate when**:
- The D2 schema turns out to require a field we don't have a clean source
  for (e.g. `output_path` requires the runner to have already written the
  artefact to disk). v0 plan: synthesise the path as `"attempts/0.txt"`
  string-only, document in rustdoc that the runner overwrites it. If
  reviewers push back, surface for guidance before implementing differently.
- `genai`'s `QueryOutput` lacks the model name on a successful tensorzero
  call (the tensorzero backend currently returns `model: Some("...")`
  via `with_model()` - verify before implementation, fall back to
  `prompt.model` if not).
- A test reveals that `BackendError`'s `From<anyhow::Error>` impl is
  swallowing the original variant in some path. Stop and re-design - the
  whole point of AC9 is unchanged pass-through.

## 4. Decomposition

Four sub-tasks, each independently testable. Order matters.

1. **ST1: Define module skeleton + value types.**
   Create `src/strategy/mod.rs` with the `Strategy` trait, `Prompt`,
   `PhaseContext`, `StrategyOutput`, `Attempt`, `StrategyKind`,
   `FinishReason`, `TokenUsageReport`, `VerifyOutcome`, `VerifyStatus`,
   `StrategyError`. Wire `pub mod strategy;` in `src/lib.rs`. No `execute`
   body yet - the trait has only the signature; SingleModel does not yet
   exist. Done when `cargo build` is green and `cargo test --no-run`
   compiles.
   Files: `src/strategy/mod.rs`, `src/lib.rs`.

2. **ST2: Implement `SingleModel`.**
   Create `src/strategy/single_model.rs`. Define `SingleModel { backend:
   String, prompt_template: String }` and `impl Strategy for SingleModel`
   with the body sketched in AC10. Add `pub mod single_model;` and `pub
   use single_model::SingleModel;` to `src/strategy/mod.rs`. Done when
   `cargo build` is green.
   Files: `src/strategy/single_model.rs`, `src/strategy/mod.rs`.

3. **ST3: Mock-backend tests.**
   Create `tests/strategy_single_model.rs`. Define inline `MockBackend`.
   Write the seven test cases enumerated in AC11. Use `#[tokio::test]`.
   Done when `cargo test --test strategy_single_model` produces seven
   green tests.
   Files: `tests/strategy_single_model.rs`.

4. **ST4: D2 schema snapshot test.**
   Inside `tests/strategy_single_model.rs`, add the JSON-schema validation
   test. Load `docs/schemas/phase_result_single.schema.json` via
   `include_str!`. Compile via `jsonschema::JSONSchema::compile`. Run
   `serde_json::to_value(&output)` and assert `schema.validate(&value)`
   returns `Ok(())`. If the `jsonschema` crate is not yet a dev-dependency,
   add it to `Cargo.toml`. Done when the snapshot test is green and `make
   check` exits 0.
   Files: `tests/strategy_single_model.rs`, possibly `Cargo.toml`.

**Dependency order**: ST1 -> ST2 -> ST3 -> ST4. ST2 depends on ST1's
trait being defined; ST3 depends on ST2's `SingleModel`; ST4 depends on
ST3's `StrategyOutput` instance. ST3's seven cases can be written in any
order once ST2 is in place.

## 5. Evaluation

| # | Test | Expected Result | How to Run |
|---|------|-----------------|------------|
| 1 | Build clean | 0 errors, 0 new warnings | `cargo build` |
| 2 | Trait + types compile | `Strategy` trait usable from outside the crate | `cargo build --tests` |
| 3 | Future-variant stub compiles | A `StubFanOut: Strategy` impl in a `#[cfg(test)]` module compiles | `cargo test --no-run` |
| 4 | Happy path | `StrategyOutput { attempts: [Attempt { backend: "mock", finish_reasons: [Stop], .. }], .. }` and mock call count == 1 | `cargo test --test strategy_single_model single_model_returns_one_attempt_on_success` |
| 5 | Error pass-through | `Err(StrategyError::Backend(BackendError::Timeout { elapsed_ms: 500, .. }))` | `cargo test --test strategy_single_model single_model_propagates_backend_error_unchanged` |
| 6 | No retry on error | mock call count == 1 even when first call returns `BackendError` | (covered in test 5's mock counter assertion) |
| 7 | Single attempt | `output.attempts.len() == 1` | (covered in test 4) |
| 8 | D2 schema validation | `jsonschema::JSONSchema::compile(...).validate(&serde_json::to_value(&output))` returns `Ok(())` | `cargo test --test strategy_single_model output_validates_against_d2_schema` |
| 9 | Prompt render failure | `Err(StrategyError::PromptRender(_))` and mock call count == 0 | `cargo test --test strategy_single_model template_render_failure_short_circuits_before_backend` |
| 10 | Backend not found | `Err(StrategyError::BackendNotFound { name: "missing" })` | `cargo test --test strategy_single_model backend_not_found_returns_typed_error` |
| 11 | Empty backends slice | `Err(StrategyError::NoBackends)` | `cargo test --test strategy_single_model empty_backends_slice_returns_no_backends` |
| 12 | schema_version constant | `output.schema_version == 1` | `cargo test --test strategy_single_model output_uses_schema_version_one` |
| 13 | strategy field renders as "single" | `serde_json::to_value(&output)["loker.strategy"] == "single"` | (covered in test 8) |
| 14 | Existing tests still pass | green | `cargo test` |
| 15 | Pre-merge gate | exit 0 | `make check` |

**Edge cases to verify**:

- **Backend returns `QueryOutput` with `model: None`.** The `model` field on
  `Attempt` is required (the schema has `"model": { "minLength": 1 }`). v0
  rule: prefer `QueryOutput.model` -> fall back to `Prompt.model` -> fall
  back to the literal string `"default"`. Test pins the priority order
  with one mock returning `model: None` and a `Prompt` with
  `model: Some("haiku")` -> attempt records `"haiku"`. A second mock with
  `model: None` and a `Prompt` with `model: None` -> attempt records
  `"default"`.
- **Backend returns `QueryOutput` with `usage: None`.** v0 rule: zero out.
  `TokenUsageReport { input_tokens: 0, output_tokens: 0 }`. Test asserts
  this against the schema (the schema requires the keys, not non-zero
  values).
- **Backend returns `BackendError::Timeout` with `elapsed_ms` populated.**
  The error must surface with `elapsed_ms` preserved, not zeroed. Pattern-
  match the inner variant in the test.
- **Prompt template has no variables.** Must still go through
  `TemplateEngine::render` (zero-substitution case), not bypass it. This
  guards against a "fast path" that would diverge from the
  `ParallelFanOut`/`EscalatingRetry` rendering path later.
- **Strategy receives a backend slice with multiple entries, only one of
  which matches `self.backend`.** SingleModel calls only the matched
  backend. The other entries are ignored. Test pins this with a slice of
  three mocks - only the named one's counter increments.
- **`run_id` is a real UUID.** The `loker.run_id` field has
  `format: "uuid"` in the schema; `uuid::Uuid::new_v4()` satisfies it.
  Test uses a fixed UUID literal so the snapshot is stable.
- **`schema_version` is exactly 1 (not "1", not 1.0).** The schema has
  `"const": 1`, integer. `serde` serialises `u32` correctly; the test
  asserts the JSON value type via `value["schema_version"].as_u64() ==
  Some(1)`.
- **A future capability addition (CLO-251 added `BackendCapabilities`).**
  Strategy code does *not* check capabilities at execute time. Capability
  validation runs at workflow load (CLO-251's `validate_with_capabilities`).
  By the time `SingleModel::execute` runs, the caller has already proven
  the backend can do what's needed. Document this invariant in the
  rustdoc on `Strategy` and on `SingleModel::execute`.
