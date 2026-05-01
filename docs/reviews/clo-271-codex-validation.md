# Codex pre-PR validation - CLO-271

## Context
- Branch: `feat/clo-271-run-command-01`
- Plan / Spec: `docs/plans/clo-271-run-command.md`
- Design: `docs/designs/clo-271-run-command.md`

## Checklist
- [x] `cargo fmt --check`
- [x] `cargo clippy --all-targets --all-features -- -D warnings`
- [x] `cargo clippy --tests -- -D warnings`
- [x] `cargo test` via `make check` (`568` total tests passed)
- [x] `make check` green
- [ ] All ACs covered
- [x] No unintended public surface
- [x] Error handling
- [ ] Tests
- [x] Schema / docs

## Findings

### F1 [major] Secret-like allowlisted env values can leak into `FailureReason`
**Where:** [run_command.rs](/Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01/src/strategy/verify/run_command.rs:137)

**What:** `build_environment()` forwards allowlisted env vars unchanged, and the secret-key detector at [run_command.rs](/Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01/src/strategy/verify/run_command.rs:329) is unused. Output redaction only applies generic token regexes, so a secret-like env key such as `MY_SECRET=plain-internal-value` will leak if the command prints only the value. The current test uses an AWS-shaped value, so it passes through value-pattern redaction and does not cover the AC requiring known-secret-shaped env names to be redacted.

**Suggested fix:** Track values for allowlisted keys where `is_secret_like_env_key(key)` is true and redact those exact values from stdout/stderr before constructing `FailureReason`; add a test with a non-regex-shaped value like `CLO271_SECRET_TOKEN=plain-secret-value`.

## Verdict
rework

The build gate is green, but the secret-redaction acceptance criterion is not satisfied or adequately tested. Since `RunCommand` is explicitly a sandboxing/security hook and `FailureReason` is meant to be safe after redaction, this needs a targeted fix before the branch is PR-ready.
