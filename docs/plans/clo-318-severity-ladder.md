# Plan: CLO-318 [T-049] Severity ladder for HITL gates

## Context

- Design: `docs/designs/clo-318-severity-ladder.md`
- Discovery: `docs/discovery/clo-318.md`
- PRD: `docs/prds/clo-318-severity-ladder.md`
- Linear: https://linear.app/cloud-ai/issue/CLO-318/t-049-severity-ladder-lowmediumhigh-for-hitl-gates
- Chosen approach: extend `HumanVerifierConfig` with timeout policy plus a clock seam, then surface HITL metadata through trace/status outputs.

## Sub-tasks

### ST1 Add timeout policy types and fake-clock seam to HumanVerifier

**Files:**
- `src/strategy/verify/human_verifier.rs`
- `src/phase_runner/dispatch.rs`
- existing `HumanVerifierConfig` construction sites found by `rg "HumanVerifierConfig" src tests`

**Work:**
- Add `HumanTimeoutAction`, `HumanTimeoutRule`, `HumanTimeoutPolicy`, `HumanTimeoutOutcome`, `HumanVerifyReport`, `HumanClock`, and `SystemHumanClock`.
- Add `timeout_policy: HumanTimeoutPolicy` to `HumanVerifierConfig` with a default-policy helper.
- Add `HumanVerifier::new_with_clock` for tests while keeping `HumanVerifier::new` on the system clock path.
- Update all config construction sites to use `HumanTimeoutPolicy::default()`.

**Acceptance:** `cargo test human_verifier::tests::default_policy_matches_severity_ladder`

**Estimate:** M

### ST2 Implement timeout decision behavior in HumanVerifier

**Files:**
- `src/strategy/verify/human_verifier.rs`

**Work:**
- Implement `verify_with_report(&self, ctx) -> Result<(VerifyResult, HumanVerifyReport), VerifyError>`.
- Preserve explicit response precedence for approve/reject/comment-only.
- Preserve persisted `pending/<phase>.json` `opened_at` and `timeout_at` as the source of truth across reruns.
- Apply defaults: low = 1h + auto-approve, medium = 24h + auto-fail, high = no timeout + block.
- Support custom policies, including configured high deadlines.
- Make the `VerifyHook` implementation delegate to `verify_with_report` and discard the report.

**Acceptance:** `cargo test human_verifier::tests::missing_low_response_after_timeout_auto_approves human_verifier::tests::missing_medium_response_after_timeout_auto_fails human_verifier::tests::high_response_missing_blocks_indefinitely human_verifier::tests::existing_pending_deadline_is_stable`

**Estimate:** M

### ST3 Surface HITL metadata in trace spans

**Files:**
- `src/trace.rs`
- `src/trace/writer.rs`
- `src/trace/memory.rs`
- `src/phase_runner.rs`
- `src/phase_runner/dispatch.rs`
- `tests/phase_runner_human_verifier.rs`

**Work:**
- Add optional `HitlTraceMetadata` to `VerifySpanResult`.
- Emit `loker.hitl.severity`, `loker.hitl.timeout_at`, `loker.hitl.timeout_action`, and `loker.hitl.timeout_outcome` in JSON trace writer output.
- Capture the same attributes in `InMemorySink` for tests.
- Make `PhaseRunner` detect `VerifyHookName::HumanVerifier`, call `verify_with_report`, and attach HITL metadata to the verify trace span.
- Keep non-HITL verify hooks producing unchanged trace metadata.

**Acceptance:** `cargo test --test phase_runner_human_verifier phase_human_verifier_trace_includes_severity`

**Estimate:** M

### ST4 Add HITL context to completed/failed status markers

**Files:**
- `src/run_state/markers.rs`
- `src/phase_runner.rs`
- `tests/phase_runner_human_verifier.rs`

**Work:**
- Add optional `HitlMarkerContext` to `CompletedMarker` and `FailedMarker` with `serde(default, skip_serializing_if = "Option::is_none")`.
- Add `MarkerWriter::write_completed_with_hitl` and `MarkerWriter::write_failed_with_hitl`, preserving existing `write_completed` and `write_failed` as delegating wrappers.
- Persist `hitl` marker context for HumanVerifier pass/fail outcomes, including auto-approved low and auto-failed medium gates.
- Keep existing marker JSON readable and keep non-HITL marker output unchanged.

**Acceptance:** `cargo test --test phase_runner_human_verifier phase_human_verifier_low_timeout_completes_with_hitl_marker phase_human_verifier_medium_timeout_fails_with_hitl_marker`

**Estimate:** M

### ST5 Update schemas and workflow documentation

**Files:**
- `docs/schemas/pending.schema.json`
- `docs/schemas/trace_event.schema.json`
- `docs/reference/workflow-spec.md` (create if absent)
- schema/docs fixtures or tests located by `rg "pending.schema|trace_event.schema|workflow-spec" tests docs`

**Work:**
- Relax the pending schema so every severity permits `timeout_at` as date-time string or `null`; document that the workflow/spec contract, not the pending file schema, owns default ladder semantics.
- Document new `loker.hitl.*` trace fields if the trace schema enumerates attributes.
- Add workflow-spec reference content for default severity ladder and override syntax.
- Update schema fixtures/tests so high with configured timeout can be valid and malformed `timeout_at` remains invalid.

**Acceptance:** `make check`

**Estimate:** S

### ST6 Run focused and full verification, then tidy implementation notes

**Files:**
- `src/strategy/verify/human_verifier.rs`
- `tests/phase_runner_human_verifier.rs`
- any docs changed in ST5

**Work:**
- Run focused HumanVerifier and phase-runner tests.
- Run full pre-merge gate.
- Fix format/clippy/test failures.
- Confirm the implementation still matches the finalized design and plan.

**Acceptance:** `make check`

**Estimate:** S

## Pre-merge gate

- `make check` (fmt + clippy + tests)

## Risks

- `PhaseRunner` may currently resolve verify hooks through trait objects, so calling `HumanVerifier::verify_with_report` may require a small HumanVerifier-specific branch before generic dispatch.
- Adding `timeout_policy` to `HumanVerifierConfig` can break many tests until all direct config construction sites are updated.
- Existing pending files should not have deadlines extended by reruns; tests must guard this because it is easy to regress when regenerating pending payloads.
- `CompletedMarker` and `FailedMarker` use strict serde settings; optional HITL fields must remain backward-compatible for old marker files and must not alter non-HITL marker output.
