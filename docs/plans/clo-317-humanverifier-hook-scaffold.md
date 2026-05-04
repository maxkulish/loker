# Plan: CLO-317 HumanVerifier hook scaffold

## Context
- Design: docs/designs/clo-317-humanverifier-hook-scaffold.md
- Discovery: docs/discovery/clo-317.md
- Linear: https://linear.app/cloud-ai/issue/CLO-317/t-048-humanverifier-hook-scaffold-phase-11-hitl-kickoff

## Sub-tasks

### ST1 Add HumanVerifier schema models and module skeleton
**Files:** `src/strategy/verify/human_verifier.rs`, `src/strategy/verify/mod.rs`
**Acceptance:** `cargo test human_verifier::tests::types_roundtrip_and_defaults`
**Estimate:** M

### ST2 Implement pending-response state machine in `HumanVerifier`
**Files:** `src/strategy/verify/human_verifier.rs`
**Acceptance:** `cargo test human_verifier::tests::returns_fail_when_response_missing` \
`cargo test human_verifier::tests::maps_approve_reject_comment_only`
**Estimate:** M

### ST3 Add response consumption + malformed-response handling
**Files:** `src/strategy/verify/human_verifier.rs`
**Acceptance:** `cargo test human_verifier::tests::consumes_response_after_successful_parse` \
`cargo test human_verifier::tests::keeps_pending_on_malformed_response`
**Estimate:** M

### ST4 Wire HumanVerifier into phase-runner dispatch and trace naming
**Files:** `src/phase_runner.rs`, `src/phase_runner/dispatch.rs`, `src/strategy/verify/mod.rs`
**Acceptance:** `cargo test phase_runner::dispatch::test::resolve_verify_hook_returns_human_verifier`
**Estimate:** S

### ST5 Add HumanVerifier phase integration and resume safety tests
**Files:** `tests/phase_runner_human_verifier.rs`
**Acceptance:** `cargo test phase_runner_human_verifier::phase_human_verifier_blocks_until_response` \
`cargo test phase_runner_human_verifier::phase_human_verifier_ignores_stale_response_after_consume`
**Estimate:** L

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Risks
- `HumanVerifier` needs a deterministic `VerifyHook` config path through `PhaseConfig`/workflow plumbing; this may require additional follow-on cleanup if additional fields are needed later.
- Severity/timeouts are scoped out for `T-049`; default flow must preserve stable behavior without timeout ladder.
- Comment-only decisions must stay blocked (no silent pass) to avoid changing existing acceptance semantics.