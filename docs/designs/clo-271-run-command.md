# Design: CLO-271 — Implement RunCommand verify hook with sandboxing

**Task:** T-023 (Roadmap Phase 4 - Verify hooks)  
**PRD:** FR-14 · Canonical design: §5  
**Status:** Draft  

---

## Problem

The `VerifyHook` trait and `VerifyResult` enum (CLO-270) are now defined in
`src/strategy/verify.rs`, but no concrete hook implementation exists for
shell-command verification. `EscalatingRetry` (CLO-258) accepts an
`Arc<dyn VerifyHook>` but today can only be wired to an `LLMVerifier`
(CLO-272). The PRD (FR-14) requires a shell-command verify hook that gates
retries with structured `FailureReason` values — exit code, signal, timeout,
truncated stderr, and a sandbox-violation discriminant. Without `RunCommand`,
the escalating-retry ladder cannot run compile/test gates, and T-029 (phase
runner) has no way to shell-out.

The existing `apply_verify::Verification` struct has shell-execution logic
but uses a private `VerifyResult` incompatible with the trait, and it lacks
sandboxing features (env allowlist, cpu timeout, secret redaction) required
by PRD §5.

---

## Goals / Non-goals

### Goals

1. **Implement `RunCommand` struct** with fields: `cmd`, `args`, `env_allowlist`,
   `cwd`, `wall_timeout`, `cpu_timeout`, `stdout_cap`, `stderr_cap`.
2. **Implement `VerifyHook` for `RunCommand`** — returning `Pass` on exit-0,
   `Fail { reason: FailureReason }` on non-zero, with structured reason.
3. **Structured `FailureReason`** carrying exit code, signal, truncated stderr
   tail, and a sandbox-violation discriminant (timeout, signal, non-zero exit).
4. **Stdout/stderr capture with byte caps** and a truncation marker
   (`…[truncated, N bytes elided]`).
5. **Secret redaction** for allowlisted environment variables with
   known-secret-shaped names before they flow into `FailureReason`.
6. **Process group cleanup** on wall timeout (SIGKILL to process group).
7. **Optional CPU timeout** via `libc::setrlimit(RLIMIT_CPU)` on Unix.
8. **Integration tests** in `tests/verify_run_command.rs` covering 8 scenarios
   (pass, non-zero exit, timeout, signal, output truncation, missing command,
   env filtering, secret redaction).

### Non-goals

- **Do not** refactor `apply_verify::verification.rs` — it is legacy code
  (`#[allow(dead_code)]`) and will remain untouched.
- **Do not** implement `Repair` or `Score` variants of `VerifyResult` — those
  are reserved for future work (CLO-272 LLMVerifier, CLO-274 scoring hook).
- **Do not** wire `RunCommand` into the workflow loader (T-029) or config
  deserialization — that's T-029's scope, not CLO-271.
- **Do not** implement process namespace isolation (cgroups, chroot, seccomp) —
  those are beyond v0 scope per PRD §5.
- **Do not** support Windows-specific CPU limiting — `RLIMIT_CPU` is Unix-only.
- **Do not** guarantee process group cleanup on Windows — best-effort only; full
  guarantee deferred to post-v0.

---

## Architecture

### Module layout

```
src/strategy/verify/
  ├── mod.rs          ← re-exports (existing: add RunCommand)
  ├── verify.rs       ← trait + types (existing: CLO-270)
  ├── llm_verifier.rs ← existing: CLO-272
  └── run_command.rs  ← NEW (this task)

tests/verify_run_command.rs  ← NEW integration tests
```

### Data flow

```
EscalatingRetry::execute()
   │
   ├── backend.query() → QueryOutput
   │
   ├── VerifyContext::from_query_output(&query)
   │
   ▼
RunCommand::verify(&ctx)
   │
   ├── (a) Resolve cmd from PATH
   │   ├── bare command name → `which::which(&self.cmd)` (skip for absolute paths)
   │   ├── if not found → Err(VerifyError::CommandNotFound { cmd: String })
   │
   ├── (b) Filter env: start from empty, then add only allowlisted vars
   │   ├── if no allowlist → empty env (default-deny)
   │   ├── apply secret redaction to known-secret-shaped values
   │
   ├── (c) Spawn process: cmd + args + filtered env + cwd
   │   ├── set process group (Unix: setpgid for signal routing)
   │   ├── optionally set cpu limit via `unsafe { cmd.pre_exec(|| libc::setrlimit(RLIMIT_CPU, ...)) }`
   │   │   (inherently unsafe; scoped `#[allow(unsafe_code)]` in run_command module)
   │
   ├── (d) Capture stdout/stderr with bounded reads + drain on truncation
   │   ├── bounded by stdout_cap / stderr_cap (bytes)
   │   ├── if truncated: append `…[truncated, N bytes elided]`
   │
   ├── (e) Wait with wall_timeout
   │   ├── if timeout → SIGKILL to process group
   │   ├── → Fail { reason: sandbox_violation("timeout") }
   │
   ├── (f) Map exit status
   │   ├── exit 0 → Pass
   │   ├── non-zero exit → Fail { reason: non_zero(exit_code, stderr_tail) }
   │   ├── signal → Fail { reason: sandbox_violation("signal") }
   │
   └── (g) Build FailureReason
       ├── summary: "cmd exited {code}" or "cmd timed out" or "cmd killed by signal"
       ├── stdout, stderr (truncated + redacted)
       ├── truncated: bool
       ├── exit_code: Option<i32>
       ├── signal: Option<i32>
       ├── sandbox_violation: Option<SandboxViolation>
```

### Type taxonomy

| Type | Purpose | v0 concrete? |
|------|---------|-------------|
| `RunCommand` | VerifyHook impl that shells out | ✅ |
| `FailureReason` | Carries structured failure context (existing from CLO-270) | ✅ |
| `VerifyResult::Pass` | Exit 0 → Pass | ✅ |
| `VerifyResult::Fail` | Exit non-zero / timeout / signal | ✅ |
| `SandboxViolation` | Discriminant for timeout vs signal vs non-zero | ✅ |
| `SecretRedactor` | Reuses `crate::utils::redact_secrets()` (shared with `escalating_retry.rs`) | ✅ |

---

## Public API surface

### `RunCommand` struct

```rust
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RunCommand {
    pub cmd: String,
    pub args: Vec<String>,
    pub env_allowlist: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub wall_timeout: Duration,
    pub cpu_timeout: Option<Duration>,
    pub stdout_cap: usize,
    pub stderr_cap: usize,
}

impl Default for RunCommand {
    fn default() -> Self { … }
}

impl RunCommand {
    pub fn new(cmd: impl Into<String>) -> Self { … }
    pub fn with_args(mut self, args: impl Into<Vec<String>>) -> Self { … }
    pub fn with_env_allowlist(mut self, vars: &[&str]) -> Self { … }
    pub fn with_cwd(mut self, path: impl Into<PathBuf>) -> Self { … }
    pub fn with_wall_timeout(mut self, timeout: Duration) -> Self { … }
    pub fn with_cpu_timeout(mut self, timeout: Option<Duration>) -> Self { … }
    pub fn with_stdout_cap(mut self, cap: usize) -> Self { … }
    pub fn with_stderr_cap(mut self, cap: usize) -> Self { … }
}
```

### `VerifyHook` trait impl

```rust
use async_trait::async_trait;
use crate::strategy::verify::{FailureReason, VerifyContext, VerifyError, VerifyHook, VerifyResult};

#[async_trait]
impl VerifyHook for RunCommand {
    fn name(&self) -> String {
        format!("run_command:{}", self.cmd)
    }

    async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> { … }
}
```

### `SandboxViolation` enum

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum SandboxViolation {
    Timeout,
    Signal { signal: i32 },
    NonZeroExit { code: i32 },
}
```

### `SecretRedactor` helper

```rust
/// Reuses the shared `crate::utils::redact_secrets()` helper (originally from
/// `escalating_retry.rs`) to avoid duplicate regex implementations.
///
/// Applied to both stdout and stderr before they flow into `FailureReason`.
pub(crate) fn redact_output(output: &str) -> String {
    crate::utils::redact_secrets(output)
}
```

---

## Test plan

### Unit tests (in `src/strategy/verify/run_command.rs` under `#[cfg(test)]`)

| # | Scenario | What it asserts |
|---|----------|-----------------|
| 1 | Exit 0 → Pass | `verify()` returns `Ok(VerifyResult::Pass)` |
| 2 | Exit non-zero → Fail | Returns `Fail` with exit code and stderr in `FailureReason` |
| 3 | Missing command → VerifyError | Returns `Err(VerifyError)` — command not found |
| 4 | Env allowlist filters correctly | Only allowlisted vars are present in child env |
| 5 | Secret redaction in stderr | Known-secret shapes are redacted before `FailureReason` |
| 6 | Output truncation marker | stdout > cap gets `…[truncated, N bytes elided]` appended |

### Integration tests (`tests/verify_run_command.rs`)

| # | Scenario | What it asserts |
|---|----------|-----------------|
| 1 | `echo hello` → Pass | End-to-end: builds `RunCommand`, calls `verify()`, asserts Pass |
| 2 | `false` → Fail | Asserts exit code 1 in `FailureReason` |
| 3 | `sleep 60` with 100ms wall_timeout | Asserts `SandboxViolation::Timeout` |
| 4 | Process group killed on timeout | Asserts no orphaned `sleep` processes after timeout |
| 5 | `sh -c "echo aaaaaaaa; echo bbbbbbbb >&2"` with 4B caps | Asserts both stdout and stderr are truncated with markers |
| 6 | `printenv` with allowlist `["USER", "HOME"]` | Asserts only `USER` and `HOME` appear in output |
| 7 | `sh -c "echo $SECRET_TOKEN"` with redaction | Asserts `[REDACTED]` appears in `FailureReason.stderr` |
| 8 | Cpu-limited infinite loop | `#[cfg(unix)]` — assert `SandboxViolation::Signal` |

---

## Migration / Rollout

1. **No breaking changes** — `RunCommand` is a new type; existing code is untouched.
2. **Export in `src/strategy/mod.rs`** — add `pub use verify::RunCommand;` alongside `LLMVerifier`.
3. **No config wiring** — deserialization from TOML is T-029's scope; this task
   provides only the programmatic API.
4. **No workflow changes** — the reference `design-doc-tdd` workflow can use
   `RunCommand` once T-029 lands the loader.

---

## Open questions

1. **Signal mapping on non-Unix**: On Windows, `setpgid` and `RLIMIT_CPU` are
   unavailable. The implementation should still work (process group and cpu limit
   are no-ops), but the sandbox guarantees are weaker. Document this in rustdoc.

2. **Verify `FailureReason` is `#[non_exhaustive]`**: Before adding the
   `sandbox_violation: Option<SandboxViolation>` field, confirm the struct is
   `#[non_exhaustive]`. If not, add the attribute first (non-breaking for v0
   consumers).

3. **Secret redaction scope**: Should the redactor also scrub values from
   `stdout`, or only `stderr`? For now, redact both — an errant `printenv`
   or debug dump could leak secrets to stdout too. Revisit if performance
   becomes an issue.

4. **Default wall_timeout**: What is a reasonable default? `30s` for compile/test
   gates, `5s` for lightweight checks. Use `Duration::from_secs(30)` as default
   in `RunCommand::default()`; callers can override.

5. **Cancellation token integration**: Should `RunCommand::verify()` accept a
   `tokio_util::sync::CancellationToken` for cooperative cancellation? The trait
   signature does not currently support this. For v0, rely on wall_timeout
   SIGKILL only. A future trait revision could add cancellation support.

6. **Redaction duplication**: `escalating_retry.rs` already has a `redact_secrets()`
   helper. To avoid drift, extract it to `crate::utils::redact_secrets()` and have
   both `RunCommand` and `EscalatingRetry` call it. This is a small refactoring
   that can be done in the same PR or a follow-up issue.
