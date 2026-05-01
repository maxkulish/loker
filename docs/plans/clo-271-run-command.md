# Plan: CLO-271 — Implement RunCommand verify hook with sandboxing

## Context
- Design: `docs/designs/clo-271-run-command.md`
- Discovery: `docs/discovery/clo-271.md`
- PRD: `docs/prds/clo-271-run-command.md`
- Linear: https://linear.app/cloud-ai/issue/clo-271/implement-runcommand-verify-hook-with-sandboxing
- Dependency: CLO-270 (VerifyHook trait + VerifyResult enum) — **done**
- Blocks: T-023 (TestRunner reuses RunCommand internals), T-029 (phase runner)

## Sub-tasks

### ST1 Convert `verify.rs` to directory module
**Files:** `src/strategy/verify.rs` → deleted; `src/strategy/verify/mod.rs`, `src/strategy/verify/verify.rs`, `src/strategy/verify/llm_verifier.rs` — created by moving existing content.
**What:** Mechanical refactoring. Split the 575-line `verify.rs` into a directory module: `mod.rs` re-exports, `verify.rs` holds trait + types, `llm_verifier.rs` holds `LLMVerifier` impl. No logic changes.
**Acceptance:** `cargo check` passes with zero new warnings.
**Estimate:** S

### ST2 Scaffold `RunCommand` struct + builder
**Files:** `src/strategy/verify/run_command.rs` (new), `src/strategy/verify/mod.rs` (add re-export).
**What:** Create `RunCommand` with all 8 fields (cmd, args, env_allowlist, cwd, wall_timeout, cpu_timeout, stdout_cap, stderr_cap). Add `Default` impl with `wall_timeout = 30s`, `stdout_cap = stderr_cap = 4096`. Add builder methods (`with_args`, `with_env_allowlist`, etc.). Add empty `impl VerifyHook for RunCommand` skeleton that returns `unimplemented!()`.
**Acceptance:** `cargo check` passes; `RunCommand` is visible in `strategy::verify` module.
**Estimate:** S

### ST3 Implement core shell-out execution
**Files:** `src/strategy/verify/run_command.rs`.
**What:** Implement the core async execution:
- Resolve `cmd` via `which::which` for bare names; skip for absolute paths
- Spawn with `tokio::process::Command`, set process group (Unix `setpgid`)
- Bounded async reads on stdout/stderr with drain-to-null on truncation
- Wait with `tokio::time::timeout(wall_timeout, child.wait())`
- On timeout: `libc::kill(-pgid, SIGKILL)` to process group, wait for exit
- Collect output as strings (before truncation, not yet with redaction)
- Return raw exit status

**Acceptance:** `cargo test --lib` — unit tests for these scenarios pass:
1. `echo hello` → exit 0, stdout captured
2. `false` → exit 1, no stdout
3. `sleep 60` with 100ms timeout → SIGKILL, no orphan processes
4. Missing command → `VerifyError::CommandNotFound`
**Estimate:** M

### ST4 Add sandboxing (env allowlist + secret redaction + cpu timeout)
**Files:** `src/strategy/verify/run_command.rs`, `src/utils.rs` (if extracting shared redact_secrets).
**What:**
- **Env filtering:** Start from empty env; add only keys in `env_allowlist` (default-deny). Apply secret redaction to values of known-secret-shaped keys.
- **Secret redaction:** Extract `redact_secrets()` from `escalating_retry.rs` into `src/utils.rs` (shared helper), update `EscalatingRetry` to call it, have `RunCommand` reuse it.
- **CPU timeout:** On Unix, `unsafe { cmd.pre_exec(|| { libc::setrlimit(RLIMIT_CPU, ...) }) }` before spawn. No-op on non-Unix with rustdoc note.

**Acceptance:** `cargo test --lib` — unit tests for these scenarios pass:
5. `printenv` with allowlist `["USER"]` → only `USER` appears in stdout
6. `sh -c "echo $SECRET_TOKEN"` with redaction → `[REDACTED]` in output
7. CPU-limited infinite loop (Unix-only `#[cfg(unix)]`) → exits by signal
**Estimate:** M

### ST5 Implement `VerifyHook` trait mapping + extend `FailureReason`
**Files:** `src/strategy/verify/run_command.rs`, `src/strategy/verify/verify.rs`.
**What:**
- Add `SandboxViolation` enum (`Timeout`, `Signal { signal: i32 }`, `NonZeroExit { code: i32 }`).
- Verify `FailureReason` is `#[non_exhaustive]`; if not, add the attribute. Then add `sandbox_violation: Option<SandboxViolation>` field.
- Implement full `VerifyHook::verify()`:
  - exit 0 → `Ok(VerifyResult::Pass)`
  - non-zero → `Ok(VerifyResult::Fail { reason: FailureReason { ..., sandbox_violation: Some(NonZeroExit) } })`
  - signal → `Fail { ..., sandbox_violation: Some(Signal) }`
  - timeout → `Fail { ..., sandbox_violation: Some(Timeout) }`
  - Apply redaction to stdout/stderr before building `FailureReason`

**Acceptance:** `cargo test --lib` — all 6 unit test scenarios from design doc pass (Pass, non-zero, missing command, env filtering, secret redaction, output truncation).
**Estimate:** M

### ST6 Write integration tests
**Files:** `tests/verify_run_command.rs` (new).
**What:** Write 8 integration test scenarios per design doc §Test plan:
1. `echo hello` → Pass
2. `false` → Fail (exit code 1)
3. `sleep 60` with 100ms timeout → `SandboxViolation::Timeout`
4. Process group killed → no orphaned `sleep` processes
5. Output truncation with 4B caps → truncation markers present
6. `printenv` with allowlist → only allowlisted vars visible
7. Secret redaction → `[REDACTED]` in `FailureReason.stderr`
8. CPU-limited infinite loop → `#[cfg(unix)]` `SandboxViolation::Signal`

**Acceptance:** `cargo test --test verify_run_command` passes.
**Estimate:** M

### ST7 Pre-merge gate
**What:** Run full pre-merge gate: `make check` (fmt + clippy -D warnings + test).
**Acceptance:** `make check` exits 0 with no warnings.
**Estimate:** S

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Risks

| Risk | Mitigation |
|------|-----------|
| **Directory module conversion breaks existing imports** | ST1 is purely mechanical; no logic changes. Run `cargo check` after every file move. |
| **`which` crate returns different results on CI vs local** | Unit tests use absolute paths or mock the `which` call. Integration tests rely on POSIX utilities (`echo`, `sleep`, `false`) which are available on macOS/Linux CI. |
| **`libc::setrlimit` pre_exec `unsafe` block flagged by clippy** | Scope `#[allow(unsafe_code)]` to the `run_command` module only; add safety comment explaining child-only scope. |
| **Process group kill leaves orphans on macOS** | Test #4 (no orphaned processes) validates this empirically. Use `pgrep` in test to confirm no `sleep` processes remain. |
| **`FailureReason` is not `#[non_exhaustive]`** | Check before adding field; if missing, add the attribute (non-breaking for v0). |
