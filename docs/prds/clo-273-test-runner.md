# PRD: CLO-273 — Implement TestRunner verify hook parsing cargo + pytest JSON

| Field | Value |
|-------|-------|
| Source | Linear issue [CLO-273](https://linear.app/cloud-ai/issue/CLO-273/implement-testrunner-verify-hook-parsing-cargo-pytest-json) |
| PRD Reference | FR-16 — test-runner verify hook gates retries |
| Design doc | See `docs/plans/001-implementation-roadmap.md` and `docs/designs/clo-270-hook.md` for verify-hook architecture |
| Security | Follow existing FR-14 sandboxing expectations in `docs/prd/2026-04-25-loker.md` when invoking test commands (`cwd`, env allowlist, output caps, timeouts, process cleanup). |

## Scope

- Implement `TestRunner` verify hook under `src/strategy/verify/` with enum `TestRunnerKind` for supported runners.
- Support at least:
  - `Cargo` -> `cargo test --message-format=json --no-fail-fast`
  - `Pytest` -> `pytest --json-report --json-report-file=-`
- Reuse `RunCommand` internals for execution and sandboxing instead of inventing new process code.
- Parse structured output to produce `{failed, passed, first_failure_name, first_failure_excerpt}` metrics.
- Map verdicts:
  - `failed > 0` → `Fail` with structured `FailureReason`
  - `failed == 0 && passed > 0` → `Pass`
  - `passed == 0 && failed == 0` → `Fail` with reason `no tests ran`
- Tests-first implementation:
  - cargo: 3 pass / 0 fail → `Pass`
  - cargo: 2 pass / 1 fail → `Fail` with first failure details
  - cargo empty (0/0) → `Fail { "no tests ran" }`
  - cargo malformed JSON line in stream is skipped
  - pytest summary path with `summary.failed` / `summary.passed` -> expected pass/fail
  - pytest non-zero exit without JSON still fails with raw stderr summary.

## Acceptance criteria

- `cargo test --test verify_test_runner` is green.
- `cargo test` overall remains green after integration.
- `cargo clippy --all-targets` reports no warnings for the new module.
- PRD FR-16 requirement is satisfied: test-runner verify hook gates retries from pass/fail counts.

## Dependencies / blocks

- Blocked by: CLO-270 (VerifyHook trait), CLO-271 (RunCommand hook + sandboxing internals) — both required.
- Blocks: T-029 (phase runner)
