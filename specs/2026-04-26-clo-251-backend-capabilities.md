# Spec: CLO-251 Add BackendCapabilities struct (FR-4)

**Created**: 2026-04-26
**Estimated scope**: S (3 files, ~4 sub-tasks)
**Linear**: [CLO-251](https://linear.app/cloud-ai/issue/CLO-251/add-backendcapabilities-struct-fr-4)
**PRD**: FR-4 (`docs/prd/2026-04-25-loker.md:105`)
**Roadmap**: T-010 (`docs/plans/001-implementation-roadmap.md:52`)

## 1. Problem Statement

The `Backend` trait at `src/backend/mod.rs:252-262` exposes `name()`, `query()`, and
`is_available()` - but says nothing about *what* a backend can do. Two consequences
flow from this gap:

1. **Workflows can demand features the chosen backend cannot deliver.** A step
   today can set `apply_edits = true` (workflow.rs:230) on any backend, including
   `ollama` running a 4B-parameter local model that hallucinates JSON edit blocks.
   Nothing at load time refuses the misconfiguration; the failure surfaces only as
   a parse error on the first run.

2. **Forthcoming strategies will multiply the gap.** T-013 (`Strategy::EscalatingRetry`
   in CLO-258) walks an ordered backend list and re-issues the prompt to the next
   backend on verify failure - some of those backends may not support tool-calling.
   T-014/T-024 (verify hooks) will introduce `LLMVerifier` and other hooks that
   require streaming or tool_use. Without a capability declaration the workflow
   author has no way to express "this phase needs streaming" and the engine has
   no way to refuse a backend that lacks it.

PRD FR-4 names three capabilities to start with: `tool_use`, `streaming`,
`file_edit`. The acceptance criterion is "Strategy and phase config validation
rejects backends that lack a required capability" - that is, the validation
must run at workflow load, before any backend call goes out, so misconfiguration
surfaces as a typed error rather than as a runtime failure.

This is a structural addition, not a redesign. The trait stays shaped the same;
we add one method, one struct, one validation pass, and per-backend honest
defaults. All architecture decisions are settled by the PRD.

## 2. Acceptance Criteria

- [ ] **AC1**: `BackendCapabilities` is defined in `src/backend/mod.rs` with exactly
      three boolean fields (`tool_use`, `streaming`, `file_edit`), `Debug`, `Clone`,
      `PartialEq`, `Eq`, and a `const fn` constructor `none()` returning all-false.
- [ ] **AC2**: `Backend` trait gains a `capabilities(&self) -> BackendCapabilities`
      method with a default impl returning `BackendCapabilities::none()` (so
      out-of-tree backends compile without modification).
- [ ] **AC3**: Each in-tree backend overrides `capabilities()` with honest values
      reflecting *current* wiring, not upstream API potential. Concrete table:

      | Backend | `tool_use` | `streaming` | `file_edit` | Justification |
      |---------|------------|-------------|-------------|---------------|
      | `tensorzero` | false | false | true | `genai` exec_chat used (not exec_chat_stream); no tool param wired; gateway forwards to capable models |
      | `claude` | false | false | true | CLI/API both used in single-shot mode; no MCP wiring; Sonnet/Opus reliably emit JSON edit blocks |
      | `codex` | false | false | true | CLI subprocess, single-shot; codex models reliably emit JSON edit blocks |
      | `gemini` | false | false | true | CLI subprocess, single-shot; Gemini reliably emits JSON edit blocks |
      | `ollama` | false | false | false | local small models hallucinate edit JSON; streaming flag in request body but query() collects full response |
      | `bedrock` (feature-gated) | false | false | true | Claude-on-Bedrock; same reasoning as claude |

- [ ] **AC4**: Unit tests in each backend's `tests` module pin the expected
      `BackendCapabilities` value with one assertion per backend (six tests behind
      the appropriate `#[cfg]` gates).
- [ ] **AC5**: `WorkflowError` gains one new variant, `MissingCapability { workflow,
      step, backend, capability, reason }`, where `capability` is a `&'static str`
      naming the field (`"file_edit"`) and `reason` is a `&'static str` naming the
      step feature that demanded it (`"apply_edits = true"`).
- [ ] **AC6**: `Workflow::validate()` (workflow.rs:119) iterates each `Step`; for
      each `step.get_backends()` it resolves the `Arc<dyn Backend>` and checks the
      capability demand. v0 demand rule: `step.apply_edits == true` requires
      `file_edit == true` on every backend in `step.get_backends()`. Validation
      runs *after* dependency / timeout checks, so existing errors still surface
      first.
- [ ] **AC7**: A workflow with `apply_edits = true` on a step whose backend is
      `ollama` (file_edit=false) fails `Workflow::validate()` with
      `WorkflowError::MissingCapability { capability: "file_edit", reason:
      "apply_edits = true", backend: "ollama", .. }`. Verified by a unit test in
      `workflow.rs::tests`.
- [ ] **AC8**: A workflow with `apply_edits = true` on a step whose backend is
      `claude` passes validation. Verified by a unit test.
- [ ] **AC9**: `Workflow::validate()` accepts a context handle through which it
      can resolve a backend name to capabilities without instantiating a real
      `Arc<dyn Backend>` (which would require API keys / running daemons in the
      test). The chosen shape is `validate_with_capabilities<F>(&self, lookup: F)
      -> Result<(), WorkflowError> where F: Fn(&str) -> Option<BackendCapabilities>`.
      The existing parameter-less `validate(&self)` continues to compile and to
      run dependency/timeout checks; capability checks are layered on the new
      method. Production callers use a closure that consults the existing
      `create_backend` factory; tests use a `HashMap` literal.
- [ ] **AC10**: No runtime call site in `src/backend/` or `src/workflow.rs`
      attempts a feature reported as unsupported. Concretely: there is no code
      path that calls a streaming or tool-use API on a backend whose
      `capabilities()` reports the corresponding field false. Verified by `rg`
      for `exec_chat_stream`, `tools(`, and `with_tools` returning zero matches
      outside of doc strings.
- [ ] **AC11**: `make check` (fmt + clippy + test) exits 0.

**Verification method**:
- AC1, AC2, AC5, AC9: `cargo build` + inspection of the diff.
- AC3, AC4: `cargo test --lib backend::` (six new tests).
- AC6, AC7, AC8: `cargo test --lib workflow::tests::validate_capability_*`.
- AC10: `rg "exec_chat_stream|with_tools|\\.tools\\(" src/`.
- AC11: `make check`.

## 3. Constraints

**Must**:
- Preserve the `Backend` trait shape: only *add* `capabilities()`, do not remove
  or rename existing methods. Out-of-tree implementors must continue to compile
  without source changes (the default impl achieves this).
- Make the validation pass purely additive on `Workflow::validate()`. The current
  parameter-less signature must continue to work for callers that only need
  dependency/timeout checks (notably the loader path that doesn't yet have a
  backend factory in scope).
- Be honest about *current* wiring, not upstream API potential. If we ever wire
  streaming through `genai::exec_chat_stream`, the tensorzero backend's
  `capabilities()` flips at that PR's boundary - and never before.
- Validate at workflow load time, before any backend call goes out. The check
  must not require a live HTTP gateway, API key, or running CLI binary - the
  capability lookup is a pure function of the backend name + config.

**Must-not**:
- Add tool-use or streaming code paths in this PR. The struct declares the
  capabilities; the wiring is later tasks (T-013/T-024 onward). Any attempt to
  invoke a tool-use API in this PR is out-of-scope and would invalidate AC10.
- Introduce new public surface beyond `BackendCapabilities`, `Backend::
  capabilities()`, `Workflow::validate_with_capabilities()`, and
  `WorkflowError::MissingCapability`. No new traits, no new modules.
- Edit the consensus / strategy / verify-hook enums - they don't exist yet
  (T-011..T-019) and their capability demands belong in those tasks.
- Edit `Cargo.toml`, `examples/workflows/*.toml`, or any fixture files. The
  validation must pass on the existing example workflows (none of which set
  `apply_edits = true` on `ollama`).
- Touch `src/backend/retry.rs` or `src/backend/genai_error.rs`. Capabilities are
  orthogonal to retry/error mapping.

**Prefer**:
- `&'static str` over `String` for capability names and demand reasons in
  `WorkflowError::MissingCapability` - the values are compile-time constants and
  this saves an allocation per error.
- A helper free function `required_capabilities(step: &Step) -> Vec<(&'static
  str, &'static str)>` (returning `(capability, reason)` pairs) so the demand
  logic is unit-testable in isolation and trivially extensible when T-013 lands.
- One unit test per backend asserting the full struct via `assert_eq!`, not
  three field-by-field assertions. Pinning the whole struct catches accidental
  flips when fields are added.
- Move the existing `tests` module additions to the bottom of each backend
  file, grouped under a `#[cfg(test)] mod capability_tests`. Keeps the diff
  reviewable.

**Escalate when**:
- The honesty matrix in AC3 turns out to be wrong for any backend (e.g. ollama
  *does* reliably emit edit JSON for some model class). Stop and confirm before
  flipping a flag.
- A current example workflow under `examples/workflows/` *does* set
  `apply_edits = true` on a backend whose capability we'd mark false. Surface
  the conflict before downgrading the capability or special-casing the
  workflow.
- The default `capabilities()` impl on `Backend` causes a clippy warning under
  `-D warnings` (e.g. `clippy::default_trait_access`) - decide between an
  `#[allow]` and removing the default impl in favor of explicit per-backend
  impls.

## 4. Decomposition

Four sub-tasks, each independently testable. Order matters where indicated.

1. **ST1: Define `BackendCapabilities` + extend `Backend` trait.**
   Add the struct in `src/backend/mod.rs` with `Debug, Clone, PartialEq, Eq` and
   the `none()` constructor. Add `fn capabilities(&self) -> BackendCapabilities {
   BackendCapabilities::none() }` to the `Backend` trait. Files:
   `src/backend/mod.rs`. Done when `cargo build` is green.

2. **ST2: Implement `capabilities()` per backend with honest values + pinning
   tests.** One impl block per backend file (`tensorzero.rs`, `claude.rs`,
   `codex.rs`, `gemini.rs`, `ollama.rs`, `bedrock.rs`). One pinning test per
   file. Files: six backend files. Done when `cargo test --lib backend::`
   produces six new green tests.

3. **ST3: Add `WorkflowError::MissingCapability` + `validate_with_capabilities`.**
   Define the new variant. Define the `required_capabilities(step)` helper.
   Wire `Workflow::validate_with_capabilities(lookup)` to iterate steps, call
   the helper, and fail on the first missing capability. The legacy
   `Workflow::validate(&self)` continues to do dependency / timeout checks
   only. Files: `src/workflow.rs`. Done when both functions compile and
   existing tests still pass.

4. **ST4: Validation tests + integration with workflow loading.** Two unit
   tests in `workflow.rs::tests`: one fail (ollama + apply_edits=true), one
   pass (claude + apply_edits=true). Wire the production loader (currently any
   call site that invokes `Workflow::validate(&self)` after parsing TOML) to
   call `validate_with_capabilities` with a closure that resolves names via
   the existing `BackendConfig` map. Files: `src/workflow.rs`, plus the loader
   call site (located via `rg "\\.validate\\(\\)" src/`). Done when both new
   unit tests pass and `make check` exits 0.

**Dependency order**: ST1 -> ST2 (in parallel, six files independent) -> ST3 ->
ST4. ST3 cannot land before ST1 because the new error variant references the
struct field name. ST4 cannot land before ST3 (uses the new method).

## 5. Evaluation

| # | Test | Expected Result | How to Run |
|---|------|-----------------|------------|
| 1 | Build clean | 0 errors, 0 new warnings | `cargo build` |
| 2 | Trait default | Backend with no override returns `none()` | `cargo test --lib backend::tests::default_capabilities_are_none` |
| 3 | tensorzero pin | `BackendCapabilities { tool_use: false, streaming: false, file_edit: true }` | `cargo test --lib backend::tensorzero::tests::capabilities_match_current_wiring` |
| 4 | claude pin | `BackendCapabilities { tool_use: false, streaming: false, file_edit: true }` | `cargo test --lib backend::claude::tests::capabilities_match_current_wiring` |
| 5 | codex pin | `BackendCapabilities { tool_use: false, streaming: false, file_edit: true }` | `cargo test --lib backend::codex::tests::capabilities_match_current_wiring` |
| 6 | gemini pin | `BackendCapabilities { tool_use: false, streaming: false, file_edit: true }` | `cargo test --lib backend::gemini::tests::capabilities_match_current_wiring` |
| 7 | ollama pin | `BackendCapabilities { tool_use: false, streaming: false, file_edit: false }` | `cargo test --lib backend::ollama::tests::capabilities_match_current_wiring` |
| 8 | bedrock pin | `BackendCapabilities { tool_use: false, streaming: false, file_edit: true }` | `cargo test --lib backend::bedrock::tests::capabilities_match_current_wiring --features bedrock` |
| 9 | required_capabilities (no demand) | `[]` | `cargo test --lib workflow::tests::required_capabilities_returns_empty_for_plain_step` |
| 10 | required_capabilities (apply_edits) | `[("file_edit", "apply_edits = true")]` | `cargo test --lib workflow::tests::required_capabilities_returns_file_edit_for_apply_edits` |
| 11 | validate fail: ollama + apply_edits | `Err(MissingCapability { capability: "file_edit", reason: "apply_edits = true", backend: "ollama", .. })` | `cargo test --lib workflow::tests::validate_rejects_apply_edits_on_ollama` |
| 12 | validate pass: claude + apply_edits | `Ok(())` | `cargo test --lib workflow::tests::validate_accepts_apply_edits_on_claude` |
| 13 | validate pass: empty step list | `Ok(())` | `cargo test --lib workflow::tests::validate_with_capabilities_handles_empty_steps` |
| 14 | validate pass: shell-only step | `Ok(())` (shell steps have no backend, no capability demand) | `cargo test --lib workflow::tests::validate_skips_shell_only_steps` |
| 15 | No runtime tool/streaming calls | zero matches | `rg "exec_chat_stream\\|with_tools\\|\\.tools\\(" src/` |
| 16 | Existing workflow validation still works | dep/timeout errors fire as before | `cargo test --lib workflow::tests::validate_` (all existing tests) |
| 17 | Pre-merge gate | exit 0 | `make check` |

**Edge cases to verify**:
- A step with multi-backend `["claude", "ollama"]` and `apply_edits = true`
  fails on `ollama` (the *first* missing capability), not on `claude`. Test
  pins the failing backend name in the error.
- A step with `apply_edits = true` but an unknown backend name (e.g.
  `"deepseek"`) - the lookup closure returns `None`. Acceptable behavior:
  treat unknown as `none()` (all-false) so validation rejects, OR raise a
  separate `WorkflowError::UnknownBackend`. v0 picks: treat as `none()`,
  rejection surfaces via `MissingCapability` with the unknown name. Documented
  in the rustdoc on `validate_with_capabilities`.
- A step with `step.apply_edits == false` and no other demand passes
  validation regardless of the chosen backend's capabilities (the most common
  case in the existing example workflows).
- Out-of-tree `Backend` impls compile without changes - the trait's default
  `capabilities()` impl returning `none()` ensures that.
- Adding a fourth capability field later (e.g. `parallel_tools`) does not
  break source compatibility: `BackendCapabilities` is non-exhaustive in
  spirit; consumers should construct via the named constructors, not struct
  literals. Mark the struct `#[non_exhaustive]` to enforce this and document
  it in the rustdoc.
