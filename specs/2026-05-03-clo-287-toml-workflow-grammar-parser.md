# CLO-287 Implement TOML workflow grammar parser

**Status:** draft
**Type:** specification
**Linear:** https://linear.app/cloud-ai/issue/CLO-287/implement-toml-workflow-grammar-parser
**PRD:** FR-29, FR-30, FR-31 (`docs/prd/2026-04-25-loker.md` lines 170-172)
**Roadmap:** T-033 (`docs/plans/001-implementation-roadmap.md` Phase 7)

## 1. Problem and goal

The loker CLI needs a phase-based workflow grammar parser that reads `.lok/workflows/*.toml` files into a typed `Workflow` AST. Unlike the existing step-based workflow system (`src/workflow.rs`), this phase-based grammar defines multiple phases per workflow, each with a `Strategy` (Single, ParallelFanOut, EscalatingRetry), backend references, prompt templates, input/output artefacts, and optional contract blocks. This is the entry point for every CLI command (`loker run`, `loker explain`, `loker resume`) and the load-bearing piece for the M6 reference workflow (T-033). The parser must produce typed errors (no `anyhow` at the boundary), validate the workflow graph (no forward input references), and resolve backend scheme strings into typed `BackendRef` variants.

## 2. Acceptance criteria

- [ ] **AC1**: New module `src/workflow/grammar.rs` exports `Workflow::from_str(&str) -> Result<Workflow, Vec<WorkflowError>>` as the public API. Wired into `src/workflow/mod.rs` or `src/lib.rs`.
      Verification: `cargo build`

- [ ] **AC2**: `Workflow` AST contains `name: String`, `description: Option<String>`, `phases: Vec<Phase>`, `defaults: Option<toml::Value>`.
      `Phase` contains `name: String`, `strategy: Strategy`, `backends: Vec<BackendRef>`, `prompt_template: String`, `inputs: Vec<InputRef>`, `output: String`, `contract: Option<toml::Value>`.
      Verification: `cargo build` + test fixture round-trips

- [ ] **AC3**: `Strategy` is an enum with variants `Single`, `ParallelFanOut { min_responses: usize }`, `EscalatingRetry { pass_failure_context: bool }`. Deserialised from TOML (e.g. `strategy = { parallel = { min_responses = 3 } }`).
      Verification: `cargo test --test workflow_grammar` (test 2: backend scheme matrix includes strategy variants)

- [ ] **AC4**: `BackendRef` is an enum with variants per scheme: `TensorZero(String)` (function name), `Claude`, `Codex`, `Gemini`, `Ollama(String)` (model name). Unknown schemes raise `WorkflowError::UnknownBackendScheme { scheme: String }`.
      Verification: `cargo test --test workflow_grammar` (test 3: unknown scheme)

- [ ] **AC5**: `InputRef` is an enum with variants `PhaseRef(String)` (references a phase by name), `VarRef(String)` (CLI `--var` value), `Spec` (the `--spec` file). Deserialised from TOML strings like `"phase:analysis"`, `"var:branch"`, `"spec"`.
      Verification: `cargo test --test workflow_grammar` (test 1: fixture round-trips all input types)

- [ ] **AC6**: Validation pass rejects forward input references: phase A referencing `phase:B` where B is declared after A produces `WorkflowError::ForwardInputRef { from: String, to: String }`. Returns `Vec<WorkflowError>` to surface all problems at once.
      Verification: `cargo test --test workflow_grammar` (test 5: forward ref, test 7: multi-error)

- [ ] **AC7**: Validation enforces strategy-specific constraints: `parallel.min_responses <= len(backends)` → `WorkflowError::MinResponsesExceedsBackends`. `escalating.backends.len() >= 2` → `WorkflowError::TooFewBackendsForEscalating`. Empty workflow (zero phases) → `WorkflowError::NoPhases`.
      Verification: `cargo test --test workflow_grammar` (test 6: strategy constraint, test 8: empty workflow)

- [ ] **AC8**: `phase.contract` is parsed and stored as `Option<toml::Value>`. A lint function (`lint_workflow`) emits `WARN: "phase.contract reserved for post-v0; ignored"` when contract is present but does not error.
      Verification: `cargo test --test workflow_grammar` (test 4: contract ignored with WARN)

- [ ] **AC9**: All error types are typed via `thiserror`; no `anyhow` at the parser boundary. `WorkflowError` enum covers: `UnknownBackendScheme`, `ForwardInputRef`, `MinResponsesExceedsBackends`, `TooFewBackendsForEscalating`, `NoPhases`, `TomlParse(toml::de::Error)`, `DuplicatePhaseName`, `InvalidInputRef`.
      Verification: `cargo build` (no `anyhow` in parser boundary) + `cargo test`

- [ ] **AC10**: `make check` is green. Rustdoc on public types (`Workflow`, `Phase`, `Strategy`, `BackendRef`, `InputRef`, `WorkflowError`) points at PRD requirement IDs (FR-29/30/31).
      Verification: `make check`

## 3. Sub-tasks

### ST1 Define module skeleton + AST types

**Files:** `src/workflow/grammar.rs`, `Cargo.toml` (if `serde` re-exports needed)
**Tests:** `tests/workflow_grammar.rs` (skeleton compiles)
**Estimate:** S

Create `src/workflow/grammar.rs` with the public AST types: `Workflow`, `Phase`, `Strategy`, `BackendRef`, `InputRef`, `WorkflowError`. Wire `pub mod grammar;` into `src/lib.rs` (or into `src/workflow/mod.rs` — currently `src/workflow.rs` is a flat file, so create `src/workflow/mod.rs` and shim the existing `workflow.rs` content into it, or export grammar from `src/lib.rs` directly). The `Strategy` enum must derive `serde::Deserialize` with the correct TOML key shape. `WorkflowError` derives `thiserror::Error` and `Display`.

Done when: `cargo build` is green and types are visible from `tests/workflow_grammar.rs`.

### ST2 Implement TOML deserialisation

**Files:** `src/workflow/grammar.rs`
**Tests:** `tests/workflow_grammar.rs` (fixture parsing test)
**Estimate:** M

Implement `Workflow::from_str(&str)` using `toml::from_str` with custom deserialisation for `Strategy` (tagged enum via `serde`'s adjacently-tagged or internally-tagged representation), `BackendRef` (string-to-enum parsing), and `InputRef` (string prefix parsing). Create a hand-rolled fixture at `tests/fixtures/workflows/design-doc-tdd.toml` with four phases covering all input types, strategy variants, and backends.

Done when: `cargo test --test workflow_grammar byt_for_byte_design_doc_tdd` passes — the fixture parses and all four phases round-trip into the AST.

### ST3 Implement backend name resolver

**Files:** `src/workflow/grammar.rs`
**Tests:** `tests/workflow_grammar.rs` (backend scheme tests)
**Estimate:** S

Implement `resolve_backend_scheme(s: &str) -> Result<BackendRef, WorkflowError>` that parses scheme prefixes: `tensorzero/<fn>`, `claude/`, `codex/`, `gemini/`, `ollama/<model>`. Unknown schemes return `Err(WorkflowError::UnknownBackendScheme)`. Each backend ref carries enough info to instantiate the right `Backend` impl at runtime (the actual instantiation lives in PhaseRunner / T-028).

Done when: `cargo test --test workflow_grammar backend_scheme_matrix` and `unknown_scheme` pass.

### ST4 Implement validation pass

**Files:** `src/workflow/grammar.rs`
**Tests:** `tests/workflow_grammar.rs` (validation tests)
**Estimate:** M

Implement `validate(&self) -> Vec<WorkflowError>` on `Workflow` that checks:
1. No forward input references (phase A references `phase:B` where B comes after A)
2. Each backend string parses via `resolve_backend_scheme`
3. Strategy constraints: `ParallelFanOut.min_responses <= backends.len()`, `EscalatingRetry.backends.len() >= 2`
4. No duplicate phase names
5. At least one phase exists
6. `InputRef` values parse correctly

Returns `Vec<WorkflowError>` so caller sees all problems at once.

Done when: `cargo test --test workflow_grammar forward_input_ref`, `strategy_constraint`, `multi_error`, `empty_workflow` all pass.

### ST5 Implement `phase.contract` reservation + lint

**Files:** `src/workflow/grammar.rs`
**Tests:** `tests/workflow_grammar.rs` (contract test)
**Estimate:** S

Parser accepts `contract = { ... }` on phases, stores as `Option<toml::Value>`. Add `lint_workflow(workflow: &Workflow) -> Vec<String>` that returns warning strings. When `contract` is `Some`, emit `"WARN: phase.contract reserved for post-v0; ignored"`.

Done when: `cargo test --test workflow_grammar contract_ignored_with_warn` passes, verifying the parser succeeds and lint output contains the reservation warning.

### ST6 Create test fixture + integration tests

**Files:** `tests/fixtures/workflows/design-doc-tdd.toml` (hand-rolled fixture), `tests/workflow_grammar.rs`
**Tests:** covered by ST2-ST5
**Estimate:** S

Create the TOML fixture with four phases covering: all `InputRef` types, all `Strategy` variants, every `BackendRef` scheme, and a phase with `contract`. The fixture is used by test 1 (byte-for-byte round-trip). Add `Cargo.toml` dev-dependency for `toml` if not already present (it is: `toml = "0.8"`).

Done when: `cargo test --test workflow_grammar` runs all 8 test cases green.

## 4. Evaluation table

| # | Scenario | Input | Expected | Verification |
|---|---|---|---|---|
| 1 | Design-doc-tdd fixture round-trips | `tests/fixtures/workflows/design-doc-tdd.toml` | All 4 phases parse; name, description, phases count match | `cargo test --test workflow_grammar byt_for_byte_design_doc_tdd` |
| 2 | Backend scheme matrix | TOML with 5 phases, one per scheme | Each resolves to correct `BackendRef` variant | `cargo test --test workflow_grammar backend_scheme_matrix` |
| 3 | Unknown backend scheme | `unknownscheme/foo` | `Err(WorkflowError::UnknownBackendScheme { scheme: "unknownscheme/foo" })` | `cargo test --test workflow_grammar unknown_scheme` |
| 4 | `phase.contract` ignored with WARN | Phase with `contract = { ... }` | Parses OK; lint output contains reservation warning | `cargo test --test workflow_grammar contract_ignored_with_warn` |
| 5 | Forward input ref | Phase A references `phase:B`, B declared after A | `WorkflowError::ForwardInputRef { from: "A", to: "B" }` | `cargo test --test workflow_grammar forward_input_ref` |
| 6 | Strategy constraint violation | `parallel { min_responses = 5, backends = [a, b] }` | `WorkflowError::MinResponsesExceedsBackends` | `cargo test --test workflow_grammar strategy_constraint` |
| 7 | Multi-error | Workflow with 2 distinct violations | Both errors returned in `Vec` | `cargo test --test workflow_grammar multi_error` |
| 8 | Empty workflow | TOML with zero phases | `WorkflowError::NoPhases` | `cargo test --test workflow_grammar empty_workflow` |
| 9 | Build clean | `cargo build` | 0 errors, 0 new warnings | `cargo build` |
| 10 | Pre-merge gate | `make check` | exit 0 | `make check` |

## 5. Edge cases

- **Edge 1: Phase name collision.** Two phases with the same name → `WorkflowError::DuplicatePhaseName { name }`. Handled by scanning phase names into a `HashSet` during validation.
- **Edge 2: Self-referencing input.** Phase A references `phase:A` (self-reference) → treated as forward ref since A cannot depend on itself; `WorkflowError::ForwardInputRef { from: "A", to: "A" }`.
- **Edge 3: Multiple unknown schemes.** Workflow with 3 phases, each using a different unknown scheme → validation returns 3 `UnknownBackendScheme` errors (multi-error test).
- **Edge 4: Empty backend list with parallel strategy.** `parallel { min_responses = 1, backends = [] }` → `MinResponsesExceedsBackends` (backends len is 0, min 1 > 0).
- **Edge 5: EscalatingRetry with single backend.** `escalating { pass_failure_context = false }` with only one backend → `WorkflowError::TooFewBackendsForEscalating` (need ≥2).
- **Edge 6: Invalid input ref format.** String that doesn't match `phase:`, `var:`, or `spec` → `WorkflowError::InvalidInputRef { raw: String }`.
- **Edge 7: TOML parse error at top level.** Malformed TOML (e.g. unclosed bracket) → `WorkflowError::TomlParse(toml::de::Error)`. Handled by wrapping `toml::from_str` error.
- **Edge 8: `contract` is arbitrary TOML value.** Not just a table — could be string, number, array. Stored as `Option<toml::Value>` which handles any TOML type. No validation on the contract content.
- **Edge 9: Phase with no `strategy` key.** Missing `strategy` in phase definition → TOML deserialisation error for missing field. The struct must not `#[serde(default)]` strategy — require it explicitly.
- **Edge 10: `BackendRef` variants carry different data.** `tensorzero/judge` has function name "judge", `ollama/glm-5.1` has model name "glm-5.1", `claude/` has no extra data. Each variant correctly stores or ignores the path segment after the scheme.
