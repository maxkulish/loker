# Plan: CLO-273 Implement TestRunner verify hook parsing cargo + pytest JSON

## Context
- Design: `docs/designs/clo-273-test-runner.md`
- Discovery: `docs/discovery/clo-273.md`
- PRD: `docs/prds/clo-273-test-runner.md`
- Linear: https://linear.app/cloud-ai/issue/CLO-273/implement-testrunner-verify-hook-parsing-cargo-pytest-json
- Dependency: CLO-271 (RunCommand verify hook), CLO-270 (VerifyHook/FailureReason)

## Sub-tasks

### ST1 Consolidate TestRunner construction and public API
**Files:** `src/strategy/verify/test_runner.rs`
**Acceptance:** `cargo test --test verify_test_runner cargo_3_pass_0_fail -- --exact`
**Estimate:** S

Make `TestRunner`, `TestRunnerKind`, `SandboxOpts`, and `TestResult` APIs stable and explicit for execution wiring:
- confirm default behavior for `cwd`, `extra_args`, and `SandboxOpts` values,
- ensure command assembly for cargo/pytest is deterministic and unit-testable,
- keep parser entry points (`parse_cargo_output`, `parse_pytest_output`) publicly testable and documented.

### ST2 Lock parser behavior to contract fixtures
**Files:** `src/strategy/verify/test_runner.rs`, `tests/fixtures/test_runner/*.json`, `tests/verify_test_runner.rs`
**Acceptance:** `cargo test --test verify_test_runner cargo_2_pass_1_fail cargo_empty_no_tests pytest_4_pass_2_fail -- --exact`
**Estimate:** M

Harden and finalize parsing semantics:
- cargo JSON-lines: count `type == "test"`, handle `ignored == true` as not passed, capture first failure name/excerpt, skip malformed lines,
- pytest JSON-report: extract `summary.passed/failed`, first failed `nodeid` + `longrepr`, parse/line-fallback on malformed stream,
- keep all parser failures soft (return `0/0` with diagnostics).

### ST3 Complete `to_verify_result()` mapping and sandbox violation paths
**Files:** `src/strategy/verify/test_runner.rs`
**Acceptance:** `cargo test --test verify_test_runner verify_result_no_tests_ran -- --exact`
**Estimate:** M

Map command execution outcomes + parsed counts into `VerifyResult`:
- timeout → `Fail` with sandbox violation timeout,
- signal/non-zero status → `Fail` with structured reason and signal/non-zero context,
- `failed > 0` → first-failure summary + stdout/stderr passthrough,
- `passed > 0 && failed == 0` → `Pass`,
- `passed == 0 && failed == 0` → `Fail { reason: "no tests ran" }`.

### ST4 Finish `VerifyHook` implementation and verify integration path
**Files:** `src/strategy/verify/test_runner.rs`, `src/strategy/verify/mod.rs`
**Acceptance:** `cargo test --test verify_test_runner pytest_non_json_exit -- --exact`
**Estimate:** M

Implement/retain `VerifyHook` execution path using `RunCommand::run()`:
- execute via resolved cargo/pytest command,
- pass parsed output through conversion path,
- verify `mod.rs` exports include `TestRunner`, `TestRunnerKind`, `SandboxOpts`, `TestResult`.

### ST5 Sanity gate for changed modules
**Files:** `src/strategy/verify/test_runner.rs`, `src/strategy/verify/mod.rs`, `tests/verify_test_runner.rs`
**Acceptance:** `cargo test --test verify_test_runner`
**Estimate:** S

Run the focused test module in CI-like mode, update any remaining fixture/parser gaps, and prepare for full-suite pre-merge.

### ST6 Pre-merge gate
**Files:** Entire workspace
**Acceptance:** `make check`
**Estimate:** S

Run project-wide pre-merge checks.

## Pre-merge gate
- `make check`

## Risks
- Pytest schema drift across plugin versions can change `summary` shape; parser is bounded to contract and guarded by parse-fail fallback.
- Non-zero command exits with malformed output require conservative failure semantics (`no tests ran`) to avoid false pass.
- Fixture-only tests can miss shell/runtime behavior; pre-merge full test gate remains required.
