# Design: CLO-317 - HumanVerifier hook scaffold

## 1. Problem

Workflow authors currently cannot pause execution for explicit human judgment within the existing verify path. `VerifyHook` in this codebase only includes machine-oriented implementations (`RunCommand`, `LLMVerifier`, `TestRunner`), and there is no runtime mechanism that writes a machine-readable pending request and later consumes a reviewer decision to decide pass/fail. Without this, milestone M10 (FR-17) cannot land because phase progress cannot be halted for human review in a resumable way. `docs/schemas/pending.schema.json` and `docs/schemas/response.schema.json` are already present, so this task is to wire them into `PhaseRunner` with a scoped hook implementation.

## 2. Goals / Non-goals

### Goals

- Add `VerifyHookName::HumanVerifier` and dispatch support in phase-runner modules.
- Implement `HumanVerifier` in `src/strategy/verify/human_verifier.rs`.
- Define filesystem schema types and helpers for:
  - pending request: `runs/<run_id>/pending/<phase>.json`
  - response: `runs/<run_id>/responses/<phase>.json`
- On each verify call:
  1. If response exists and is valid, map `approve`/`reject` (and optional `comment_only`) into `VerifyResult`.
  2. If no valid response exists, write/refresh a pending request file and return non-pass outcome.
  3. Consume the response once used to avoid stale replay on retry/resume.
- Add unit/integration tests for idempotent pending creation, malformed-response handling, and end-to-end phase resume behavior.
- Preserve existing `RunCommand` and `LLMVerifier` behavior.
- Keep changes additive to existing config/runner surfaces where possible.

### Non-goals

- Severity ladder (`low/medium/high` timing semantics) and auto-fail actions (handled in T-049).
- First-write-wins/heartbeat advisory locks (`T-050`).
- Per-gate fallback HTTP endpoint / local daemon (`T-051`).
- Polling or background daemons; hook evaluation is synchronous and resume-driven.
- Modifying schema files in this task (they already exist).

## 3. Architecture

### Module layout

```text
src/strategy/verify/
  verify.rs               (existing trait + FailureReason)
  run_command.rs          (existing)
  llm_verifier.rs         (existing)
  test_runner.rs          (existing)
  human_verifier.rs       (NEW)
  mod.rs                  (export HumanVerifier + model types)

src/phase_runner.rs       (extend VerifyHookName)
src/phase_runner/dispatch.rs (extend resolve_verify_hook)
```

### Data flow

```text
PhaseRunner::run
    -> strategy execution
    -> canonical_bytes (existing)
    -> resolve_verify_hook(cfg.verify, inputs.verify)
        -> HumanVerifier::verify(ctx)
            -> read runs/<run_id>/responses/<phase>.json
                -> if valid and decision exists:
                   approve   => VerifyResult::Pass
                   reject    => VerifyResult::Fail (reason carries reject info)
                   comment_only => VerifyResult::Fail (pending/blocked)
                -> if malformed/mismatched/empty => keep pending
            -> else:
                ensure pending/<phase>.json exists (idempotent)
                return VerifyResult::Fail (blocked for human review)
            -> when response is accepted, optionally move to responses/<phase>.json.handled

PhaseRunner::run
    -> non-pass results still surface as verify failure today
    -> `loker resume` reruns phase and HumanVerifier consumes any new response
```

### Key types (proposed)

- `HumanVerifierConfig` (runtime constructor config): run_dir, phase, workflow slug, run slug, severity, decision options.
- `PendingArtefactContext`: maps to `artefact` in pending schema (`path`, `kind`, `preview_lines`).
- `HumanDecision` (`approve`, `reject`, `comment_only`) and `HumanSeverity` (`low`, `medium`, `high`).
- `PendingRequest` / `HumanResponse` serde types for schema-compliant persistence and validation.
- `HumanVerifier` object implementing `VerifyHook`.

### Run-state interaction

- `runs/<run_id>/pending/<phase>.json` is a durable, restartable request.
- `runs/<run_id>/responses/<phase>.json` is written by operators/UI and read by the hook.
- Response files are consumed (rename or remove) after first successful parse and mapping to avoid replay across retries.
- No new marker family is required for v0; resume uses existing failed/started markers.

## 4. Public API surface

### `src/strategy/verify/human_verifier.rs`

```rust
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

use crate::strategy::verify::{VerifyContext, VerifyHook, VerifyResult};

#[derive(Debug, Clone)]
pub struct HumanVerifierConfig {
    pub run_dir: PathBuf,
    pub run_id: String,
    pub workflow: String,
    pub phase: String,
    pub severity: HumanSeverity,
    pub decision_options: Vec<HumanDecision>,
}

#[derive(Debug, Clone)]
pub struct HumanVerifier {
    pub config: HumanVerifierConfig,
}

impl HumanVerifier {
    pub fn new(config: HumanVerifierConfig) -> Self;
    pub fn pending_path(&self) -> PathBuf;
    pub fn response_path(&self) -> PathBuf;
    fn ensure_pending_payload(&self, artefact_path: &str, artefact_kind: &str) -> PendingRequest;
    fn parse_response(&self, response_path: &std::path::Path) -> Result<HumanResponse, std::io::Error>;
    fn load_decision(&self, response: &HumanResponse) -> VerifyResult;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HumanSeverity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum HumanDecision {
    Approve,
    Reject,
    #[serde(rename = "comment_only")]
    CommentOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingRequest {
    pub schema_version: u32,
    pub run_id: String,
    pub workflow: String,
    pub phase: String,
    pub severity: HumanSeverity,
    pub opened_at: String,
    pub timeout_at: Option<String>,
    pub artefact: PendingArtefact,
    pub context: PendingContext,
    pub decision_options: Vec<HumanDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingContext {
    pub preceded_by: Vec<String>,
    pub next_phase: Option<String>,
    pub prompt_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingArtefact {
    pub path: String,
    pub kind: String,
    pub preview_lines: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HumanResponse {
    pub schema_version: u32,
    pub phase: String,
    pub claimed_by: String,
    pub decided_at: String,
    pub decision: HumanDecision,
    pub global_comment: Option<String>,
    pub inline_comments_path: Option<String>,
}

#[async_trait::async_trait]
impl VerifyHook for HumanVerifier {
    fn name(&self) -> &str;
    async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, crate::strategy::verify::VerifyError>;
}
```

### `src/phase_runner.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyHookName {
    None,
    RunCommand,
    LlmVerifier,
    HumanVerifier(crate::strategy::verify::human_verifier::HumanVerifierConfig),
}
```

### `src/phase_runner/dispatch.rs`

```rust
pub fn resolve_verify_hook(
    cfg: &VerifyHookName,
    verify: Option<std::sync::Arc<dyn VerifyHook>>,
) -> Result<Option<std::sync::Arc<dyn VerifyHook>>, PhaseError>;

// New arm:
// VerifyHookName::HumanVerifier(cfg) => Ok(Some(Arc::new(HumanVerifier::new(cfg.clone()))))
```

### `src/strategy/verify/mod.rs` (exports)

```rust
pub use human_verifier::{
    HumanDecision, HumanResponse, HumanSeverity, HumanVerifier, HumanVerifierConfig,
};
```

## 5. Test plan

### Unit tests (`src/strategy/verify/human_verifier.rs`)

- `writes_pending_file_when_no_response`
- `keeps_existing_pending_file_without_mutating_if_response_absent`
- `reads_valid_approve_response_as_pass`
- `reads_valid_reject_response_as_fail_with_comment`
- `treats_comment_only_as_pending_gate`
- `rejects_malformed_response_and_maintains_pending`
- `consumes_response_file_after_first_use`
- `schema_roundtrip_pending_and_response_structs` (optional)

### Integration (`tests/phase_runner_human_verifier.rs`)

- `phase_blocks_until_response_present`: missing response → `PhaseError::VerifyFailed`; pending file emitted.
- `phase_resumes_after_approve_response`: second run with approved response passes and writes successful artefact.
- `phase_rejects_on_reject_response`: approved/deny path returns fail as expected.
- `retry_does_not_replay_old_response`: stale response file from prior attempt is not reused after consume.
- `malformed_response_keeps_pending`: parser/schema errors return pending-style failure, not terminal backend failure.

### Manual verification

1. Build a workflow that routes a phase through `VerifyHookName::HumanVerifier`.
2. Run once and confirm `runs/<id>/pending/<phase>.json` exists and phase is paused.
3. Add `runs/<id>/responses/<phase>.json` with `{"decision":"approve", ...}` and rerun resume.
4. Observe `PhaseRunner` advances and failed marker no longer blocks.
5. Rerun `cargo test`/`make check`.

## 6. Migration / rollout

- Additive implementation: no existing flow is deleted or behaviorally replaced.
- No feature flags and no user-visible CLI changes in this task.
- No manifest schema migration is needed.
- Rollout order:
  1. Add `human_verifier.rs` plus `VerifyHookName::HumanVerifier` + config plumbing.
  2. Add dispatch + resume-path tests.
  3. Add unit/integration tests for malformed input and response consumption.

## 7. Open questions

- Should `comment_only` remain in `decision_options` but map to blocked behavior (current recommendation) or be filtered out by design-time config? The pending schema allows it, while `response.schema.json` currently does not include it as a first-class decision.
- Where should response consumption happen (rename vs delete) to preserve operator audit while preventing replay? Recommendation: atomic rename to `.handled` to retain evidence.
- Should a future phase introduce a dedicated phase-state marker (separate from failed/started/completed) for `human_pending` observability, or is pending-file presence sufficient for v0?