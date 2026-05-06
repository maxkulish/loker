# Design: CLO-318 — Severity ladder for HumanVerifier gates

## 1. Problem

Discovery for CLO-318 found that the CLO-317 `HumanVerifier` scaffold already writes `severity` and a default `timeout_at` into `pending/<phase>.json`, but the verifier never evaluates deadlines and the rest of the runner treats all missing responses as the same generic verify failure. Workflow authors and operators need severity to be executable policy, not only metadata: low gates should default to auto-approve after 1 hour, medium gates should default to auto-fail after 24 hours, and high gates should default to blocking indefinitely. Downstream T-051/T-053 tooling also needs severity to be visible in traces and status markers so HITL gates can be prioritized without parsing pending files directly.

## 2. Goals / Non-goals

### Goals

- Add explicit `low | medium | high` timeout/escalation policy to `HumanVerifier` while preserving the existing response-file flow.
- Provide default policy: low = 1h + auto-approve, medium = 24h + auto-fail, high = no timeout + block.
- Allow callers/workflow-spec plumbing to override per-severity timeout and timeout action.
- Make timeout evaluation deterministic in tests via a fake clock seam; no sleeps in unit or integration tests.
- Surface severity and timeout outcome in trace spans and phase status markers.
- Document severity defaults and override syntax in the workflow/spec reference.

### Non-goals

- UI rendering, notification, or paging integrations.
- Per-phase advisory locks or first-writer response locking.
- New `VerifyResult` variants; v0 remains binary pass/fail.
- Changing explicit human response semantics: `approve` passes, `reject` fails, `comment_only` fails.

## 3. Architecture

### Modules touched

```text
src/strategy/verify/human_verifier.rs  # policy types, fake clock seam, timeout evaluation
src/phase_runner.rs                    # extracts HITL metadata for trace/status marker writes
src/trace.rs                           # VerifySpanResult grows optional HITL metadata
src/trace/writer.rs                    # emits loker.hitl.* fields
src/trace/memory.rs                    # captures loker.hitl.* fields in tests
src/run_state/markers.rs               # optional HITL marker payloads on completed/failed markers
docs/schemas/pending.schema.json       # timeout_at validation/doc wording for configurable policies
docs/schemas/trace_event.schema.json   # document loker.hitl.* fields if strict schema needs it
docs/reference/workflow-spec.md        # default ladder and override examples (create if absent)
tests/phase_runner_human_verifier.rs   # runner-level trace/marker coverage
```

### Data flow

1. Workflow/spec plumbing constructs `VerifyHookName::HumanVerifier(HumanVerifierConfig)` with a `severity` and optional `timeout_policy` overrides. Until TOML parsing is expanded, tests and callers can construct the config directly.
2. `dispatch::resolve_verify_hook` builds `HumanVerifier::new(config)`, which attaches a system clock by default.
3. `HumanVerifier::verify` reads any response file first. Valid explicit responses continue to take precedence over timeout policy.
4. If there is no valid response, `HumanVerifier` ensures a pending request exists, derives the effective deadline/action for the configured severity, and compares `clock.now()` with the persisted/opened deadline.
5. If not timed out, high/default-blocking gates return `VerifyResult::Fail { waiting... }` as today.
6. If timed out:
   - `AutoApprove` returns `VerifyResult::Pass` with timeout outcome `auto_approved`.
   - `AutoFail` returns `VerifyResult::Fail` with a timeout-specific reason and outcome `auto_failed`.
   - `Block` returns the same waiting failure with outcome `blocking`, even if a timeout value was supplied.
7. `PhaseRunner` special-cases `VerifyHookName::HumanVerifier`, calls `HumanVerifier::verify_with_report`, emits `VerifySpanResult` with the returned HITL metadata, and writes completed/failed markers with optional HITL metadata. The normal `VerifyHook::verify` trait implementation delegates to the same helper and discards the report for non-PhaseRunner callers.

### Pending timestamp source of truth

`PendingRequest.opened_at` and `PendingRequest.timeout_at` must be stable once written. Re-running a blocked phase must not extend a deadline just because the verifier is invoked again. Therefore:

- If `pending/<phase>.json` already exists and parses, `HumanVerifier` uses that payload's `opened_at`/`timeout_at` for timeout evaluation.
- If it does not exist or is malformed/unreadable, `HumanVerifier` creates a new payload using `clock.now()` and the effective policy.
- If policy overrides change after a pending file exists, the existing pending file wins for that gate instance; changing active deadlines is a follow-up concern for UI/admin tooling.

### Configurable policy and schema impact

CLO-317's pending schema currently encodes the default ladder by requiring `high => timeout_at: null` and `low|medium => string`. CLO-318 intentionally relaxes this because the Linear scope says per-severity timeout defaults and timeout behavior are configurable. The schema should require `severity` and `timeout_at`, but allow `timeout_at` to be either a date-time string or `null` for every severity. The workflow/spec reference, not the pending JSON schema, defines the default ladder.

## 4. Public API surface

### `human_verifier.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanTimeoutAction {
    AutoApprove,
    AutoFail,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanTimeoutRule {
    pub timeout: Option<chrono::Duration>,
    pub on_timeout: HumanTimeoutAction,
}

impl HumanTimeoutRule {
    pub fn low_default() -> Self;    // Some(1h), AutoApprove
    pub fn medium_default() -> Self; // Some(24h), AutoFail
    pub fn high_default() -> Self;   // None, Block
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanTimeoutPolicy {
    pub low: HumanTimeoutRule,
    pub medium: HumanTimeoutRule,
    pub high: HumanTimeoutRule,
}

impl Default for HumanTimeoutPolicy;

impl HumanTimeoutPolicy {
    pub fn rule_for(&self, severity: HumanSeverity) -> &HumanTimeoutRule;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanTimeoutOutcome {
    NotTimedOut,
    AutoApproved,
    AutoFailed,
    Blocking,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanVerifyReport {
    pub severity: HumanSeverity,
    pub timeout_at: Option<String>,
    pub timeout_action: HumanTimeoutAction,
    pub timeout_outcome: HumanTimeoutOutcome,
}

pub struct HumanVerifierConfig {
    pub run_dir: PathBuf,
    pub run_id: String,
    pub workflow: String,
    pub phase: String,
    pub artefact_name: String,
    pub artefact_kind: Kind,
    pub severity: HumanSeverity,
    pub decision_options: Vec<HumanDecision>,
    pub timeout_policy: HumanTimeoutPolicy,
}

impl HumanVerifierConfig {
    pub fn with_default_timeout_policy(self) -> Self;
}

pub trait HumanClock: Send + Sync {
    fn now(&self) -> chrono::DateTime<chrono::Utc>;
}

pub struct SystemHumanClock;
impl HumanClock for SystemHumanClock;

pub struct HumanVerifier {
    pub config: HumanVerifierConfig,
    clock: Arc<dyn HumanClock>,
}

impl HumanVerifier {
    pub fn new(config: HumanVerifierConfig) -> Self;
    pub fn new_with_clock(config: HumanVerifierConfig, clock: Arc<dyn HumanClock>) -> Self;
    pub fn pending_path(&self) -> PathBuf;
    pub fn response_path(&self) -> PathBuf;
    pub async fn verify_with_report(&self, ctx: &VerifyContext) -> Result<(VerifyResult, HumanVerifyReport), VerifyError>;
}
```

Notes:

- The clock lives on `HumanVerifier`, not inside `HumanVerifierConfig`, so `VerifyHookName::HumanVerifier(HumanVerifierConfig)` can keep its existing `Debug + Clone + PartialEq + Eq` ergonomics.
- `HumanVerifierConfig` construction sites must add `timeout_policy: HumanTimeoutPolicy::default()`.
- `VerifyHook for HumanVerifier` delegates to `verify_with_report` and returns only the `VerifyResult`; `PhaseRunner` uses `verify_with_report` directly so no mutable `last_timeout_outcome` state or extra synchronization is needed.

### Trace API

```rust
pub struct HitlTraceMetadata {
    pub severity: String,
    pub timeout_at: Option<String>,
    pub timeout_action: Option<String>, // auto_approve | auto_fail | block
    pub timeout_outcome: Option<String>, // not_timed_out | auto_approved | auto_failed | blocking
}

pub struct VerifySpanResult {
    pub passed: bool,
    pub message: Option<String>,
    pub duration_ms: u64,
    pub hitl: Option<HitlTraceMetadata>,
}
```

`TraceWriter` and `InMemorySink` emit these as `loker.hitl.severity`, `loker.hitl.timeout_at`, `loker.hitl.timeout_action`, and `loker.hitl.timeout_outcome`.

### Marker API

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct HitlMarkerContext {
    pub severity: String,
    pub timeout_at: Option<String>,
    pub timeout_action: String,
    pub timeout_outcome: String,
}

pub struct CompletedMarker {
    // existing fields...
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hitl: Option<HitlMarkerContext>,
}

pub struct FailedMarker {
    // existing fields...
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hitl: Option<HitlMarkerContext>,
}

impl MarkerWriter {
    pub fn write_completed_with_hitl(..., hitl: Option<HitlMarkerContext>) -> Result<CompletedMarker, MarkerError>;
    pub fn write_failed_with_hitl(..., hitl: Option<HitlMarkerContext>) -> Result<FailedMarker, MarkerError>;
}
```

Existing `write_completed` and `write_failed` remain and delegate with `None`, preserving current marker output for non-HITL phases.

## 5. Test plan

### Unit tests in `human_verifier.rs`

- `default_policy_matches_severity_ladder`: low = 1h/auto-approve, medium = 24h/auto-fail, high = no-timeout/block.
- `missing_low_response_before_timeout_stays_pending`: fake clock before deadline returns fail/waiting and writes pending severity `low`.
- `missing_low_response_after_timeout_auto_approves`: fake clock after persisted deadline returns pass plus report outcome `AutoApproved`.
- `missing_medium_response_after_timeout_auto_fails`: fake clock after persisted deadline returns fail plus report outcome `AutoFailed`.
- `high_response_missing_blocks_indefinitely`: fake clock far in the future still returns fail/waiting and `timeout_at: null`.
- `explicit_response_before_or_after_timeout_wins`: approve/reject response is honored before evaluating timeout.
- `existing_pending_deadline_is_stable`: first call writes pending at T0, second call at T0+30m uses the original deadline, not a regenerated one.
- `custom_policy_allows_high_timeout`: high with `Some(10m)+AutoFail` writes a non-null deadline and fails after timeout.

### Integration tests in `tests/phase_runner_human_verifier.rs`

- `phase_human_verifier_low_timeout_completes_with_hitl_marker`: PhaseRunner succeeds after low timeout and completed marker contains `hitl.severity=low` and `timeout_outcome=auto_approved`.
- `phase_human_verifier_medium_timeout_fails_with_hitl_marker`: PhaseRunner fails after medium timeout and failed marker contains `hitl.severity=medium` and `timeout_outcome=auto_failed`.
- `phase_human_verifier_trace_includes_severity`: `InMemorySink` captures `loker.hitl.severity` on verify span.
- Existing response consumption tests continue to pass with `HumanTimeoutPolicy::default()`.

### Schema/docs tests

- Update pending schema fixtures so `high` with configured timeout is valid when intended, and keep a negative fixture for malformed `timeout_at`.
- Add/adjust trace schema fixture for a verify result containing `loker.hitl.severity`.
- If a workflow-spec parser test exists by implementation time, add a fixture for `severity = "medium"` and timeout/action overrides; otherwise document direct config support and leave parser wiring for the first task that owns TOML grammar expansion.

### Manual verification

- `cargo test human_verifier`
- `cargo test --test phase_runner_human_verifier`
- `make check`

## 6. Migration / rollout

- Runtime behavior changes only for `HumanVerifier` gates that have no response and whose persisted deadline has passed.
- Existing pending files with `high` and `timeout_at: null` remain valid.
- Existing marker files remain readable because new `hitl` fields are optional and default to `None` when absent.
- Existing trace consumers should tolerate `loker.*` additions; update `trace_event.schema.json` to document the new keys for test fixtures.
- Documentation rollout: create or update `docs/reference/workflow-spec.md` with default severity ladder and override examples:

```toml
[phases.review.verify.human]
severity = "medium"

[phases.review.verify.human.timeout.medium]
duration = "24h"
on_timeout = "auto_fail"
```

The exact TOML parser wiring may be implemented only if the current workflow grammar already has a `HumanVerifier` stanza. If not, the design still requires Rust-level config support now and a documented spec contract for parser work to consume.

## 7. Open questions

1. **Should an auto-approved low timeout consume/archive the pending request?** Resolution: no. Leave `pending/<phase>.json` in place for auditability; completion marker and trace indicate auto-approval.
2. **Should an auto-failed medium timeout trigger `EscalatingRetry` retries?** Resolution: yes, it is a normal `VerifyResult::Fail`. The timeout reason must be explicit so retry logs make the source clear. EscalatingRetry retry policy remains unchanged.
3. **Should configurable high timeouts be allowed even though CLO-317 schema disallowed them?** Resolution: yes. CLO-318 owns configurability, so relax pending schema and document defaults in the workflow/spec reference.
4. **Should malformed existing pending files be repaired or treated as failure?** Resolution: preserve current behavior pattern: write/ensure a pending file when possible and return a verify failure with a clear malformed-pending/response reason. Do not silently auto-approve on malformed audit state.
