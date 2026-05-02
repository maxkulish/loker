# Design: CLO-273 — Implement TestRunner verify hook parsing cargo + pytest JSON

| Field | Value |
|-------|-------|
| Task | CLO-273 |
| Date | 2026-05-01 |
| Phase | design |
| Discovery | docs/discovery/clo-273.md |
| PRD | docs/prds/clo-273-test-runner.md |

## 1. Problem

`VerifyHook` is now available (`CLO-270`) and `RunCommand` already exists (`CLO-271`) as a reusable process-based verifier, but there is still no dedicated test-hook implementation. Without a test runner verifier, strategy retries are gated only by LLM-style checks and cannot reliably depend on deterministic pass/fail counts from project tests. This leaves the phase runner path (`T-029`) without a full production-quality test verification primitive.

CLO-273 requires a hook that can execute project test commands and convert structured test output into a binary gate signal (`Pass`/`Fail`) with enough context for downstream feedback. The issue explicitly requires cargo and pytest support with fixture-based parser contracts, so the implementation should be mostly deterministic and parser-first.

## 2. Goals & Non-goals

### Goals

1. **Implement `TestRunner` verify hook** in `src/strategy/verify/test_runner.rs` with `TestRunnerKind` enum (`Cargo`, `Pytest`) and builder-style configuration (`runner`, `cwd`, `extra_args`, `sandbox`).
2. **Execute tests via `RunCommand` internals**, reusing existing `cwd`, env allowlist, output caps, timeouts, signal handling, and `FailureReason` mapping.
3. **Parse structured output** into `TestResult` for:
   - cargo JSON-lines (`type: "test"`, `event: "ok"/"failed"`, etc.),
   - pytest JSON-report summary (`summary.passed`, `summary.failed`).
4. **Map outcomes to verify results**:
   - `failed > 0` ⇒ `Fail` with first-failure name/excerpt,
   - `failed == 0 && passed > 0` ⇒ `Pass`,
   - `passed == 0 && failed == 0` ⇒ `Fail { summary: "no tests ran" }`.
5. **Keep parsing robust** by skipping malformed cargo lines and handling pytest missing/unparseable output without crashing the hook.
6. **Add/retain fixture-driven parser tests** in `tests/verify_test_runner.rs` for the contract in ACs.

### Non-goals

- No new backend/protocol support beyond `cargo` and `pytest` in v0.
- No historical trend tracking; gate only current run.
- No attempt to implement external language-specific reporters outside pytest JSON output.
- No phase-runner wiring changes in this task (that is handled in `T-029`).

## 3. Architecture

### 3.1 Module layout

```
src/strategy/verify/
  mod.rs            # new re-export: TestRunner, TestRunnerKind, SandboxOpts, TestResult
  verify.rs         # VerifyHook trait + FailureReason + shared hook types
  run_command.rs    # reusable process verifier, used by both RunCommand and TestRunner
  test_runner.rs    # NEW: cargo + pytest parser and TestRunner::verify impl

tests/
  verify_test_runner.rs # fixture-driven unit/integration parser and mapping tests
  fixtures/test_runner/* # deterministic parser fixtures
```

### 3.2 Data flow

```
TestRunner::verify(ctx) [ctx unused for now]
  ├─ build RunCommand based on TestRunnerKind
  │   ├─ cargo => cargo test --message-format=json --no-fail-fast
  │   └─ pytest => pytest --json-report --json-report-file=-
  ├─ apply sandbox opts (cwd, allowlist, timeouts, caps)
  ├─ run command (inherited from RunCommand)
  ├─ parse stdout into TestResult
  │   ├─ cargo parser: line-by-line JSON events
  │   └─ pytest parser: full-json parse, fallback to line scan
  ├─ translate CommandRun + TestResult into VerifyResult
  └─ return Pass/Fail with structured FailureReason
```

### 3.3 Types

- `TestRunnerKind`: `Cargo | Pytest`.
- `SandboxOpts`: execution policy fields currently needed by tests (`env_allowlist`, `wall_timeout`, `stdout_cap`, `stderr_cap`).
- `TestRunner`: `{ runner, cwd, extra_args, sandbox }`.
- `TestResult`: `{ passed, failed, first_failure_name, first_failure_excerpt }`.
- `CommandRun` and `VerifyResult` are reused from `run_command` / verify module.

## 4. Public API and behaviors

### 4.1 Core structs

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestRunnerKind {
    Cargo,
    Pytest,
}

#[derive(Debug, Clone)]
pub struct SandboxOpts {
    pub env_allowlist: Vec<String>,
    pub wall_timeout: Duration,
    pub stdout_cap: usize,
    pub stderr_cap: usize,
}

#[derive(Debug, Clone)]
pub struct TestRunner {
    pub runner: TestRunnerKind,
    pub cwd: PathBuf,
    pub extra_args: Vec<String>,
    pub sandbox: SandboxOpts,
}
```

### 4.2 Hook contract

```rust
#[async_trait]
impl VerifyHook for TestRunner {
    fn name(&self) -> &str;
    async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError>;
}
```

`verify()` builds a `RunCommand`, executes tests, parses output, and maps to:
- `VerifyResult::Pass` (no failures, at least one pass),
- `VerifyResult::Fail` otherwise.

### 4.3 Parsing rules

#### Cargo (`parse_cargo_output`)

- Consume JSON-lines.
- Count only objects where `type == "test"`.
- `event == "ok"` increments `passed`.
- `event == "failed"` increments `failed` and captures first failure name + excerpt.
- Ignore other events (`ignored`, benchmarks, etc.).
- Malformed lines are ignored (not fatal).
- Ignore lines where test event is `ok` and `ignored == true`.

#### Pytest (`parse_pytest_output`)

- Parse full stdout as JSON; fallback to line-by-line JSON candidate parsing.
- Read `summary.passed` and `summary.failed`.
- Locate first failed test in `tests[]` (`outcome == "failed"`) for name and `longrepr` excerpt.
- On parse or missing-summary failure, return zero counts with descriptive non-empty excerpt in result.

### 4.4 Result translation

`to_verify_result(run, parsed)`:
1. If timed out/signaled, produce fail with sandbox violation metadata.
2. If parsed yields zero/zero → fail "no tests ran" + captured output.
3. If failed > 0 → include first-failure details in summary and exit code.
4. Else pass.

## 5. Implementation plan (v0)

1. **Parser-only stability:** keep `parse_cargo_output` / `parse_pytest_output` deterministic and independent of actual command execution.
2. **Map runner assembly:** ensure `build_run_command()` maps both kinds and forwards `extra_args`.
3. **Failure mapping:** ensure `to_verify_result()` handles timeout/signal/non-zero/empty output consistently with existing `FailureReason` patterns.
4. **Fixtures and tests:** add fixtures for pass/fail/malformed/no-output fixtures in `tests/fixtures/test_runner` and assertions in `tests/verify_test_runner.rs`.
5. **Run full checks:** `cargo test --test verify_test_runner`, then at least `cargo test --test verify_run_command` and full `cargo test`.

## 6. Validation plan (from discovery/PRD)

- `cargo test --test verify_test_runner` must pass.
- Full test suite remains green.
- `cargo clippy --all-targets` must pass with no warnings.
- Result mapping should preserve deterministic, parseable outputs for retry gating.

## 7. Risks and open questions

### Risks

- Pytest output schema can vary by plugin/version; we intentionally scope to json-report schema in issue contract.
- Some command failures may produce noisy non-JSON output; parser should fail-soft with safe `no tests ran`/partial context rather than panic.

### Open questions

- Should we add a future fallback parser (`pytest -q` style output) under a feature flag in a follow-up task once schema mismatches are observed in real projects?

## 8. Security considerations

- Execution remains within `RunCommand` sandbox boundaries (env allowlist, wall timeout, byte caps, process cleanup).
- `VerifyResult::FailureReason` keeps raw stdout/stderr for downstream redaction to remain explicit about trust boundaries.
- No credentials are passed through fixtures or test harnesses.
