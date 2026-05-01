OpenAI Codex v0.128.0 (research preview)
--------
workdir: /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
model: gpt-5.5
provider: openai
approval: never
sandbox: workspace-write [workdir, /tmp, $TMPDIR, /Users/mk/.codex/memories]
reasoning effort: high
reasoning summaries: none
session id: 019de2ef-3465-7fd2-b19c-38f8a3ec4f83
--------
user
changes against 'main'
exec
/bin/zsh -lc 'git diff 30fe7a742785a98cb2b1894b6a4dbc6c616d6c49 --stat && git diff 30fe7a742785a98cb2b1894b6a4dbc6c616d6c49' in /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
 succeeded in 0ms:
 docs/designs/clo-271-run-command.md      | 282 ++++++++++++++++
 docs/discovery/clo-271.md                | 124 ++++++++
 docs/plans/clo-271-run-command.md        | 102 ++++++
 docs/prds/clo-271-run-command.md         |  28 ++
 docs/reviews/clo-271-design-gemini.md    |  73 +++++
 docs/reviews/clo-271-design-synthesis.md |  34 ++
 docs/status/clo-271-workflow.yaml        | 156 +++++++++
 src/main.rs                              |   2 +
 src/strategy/escalating_retry.rs         |  32 +-
 src/strategy/mod.rs                      |   3 +-
 src/strategy/verify/llm_verifier.rs      | 142 +++++++++
 src/strategy/verify/mod.rs               |  22 ++
 src/strategy/verify/run_command.rs       | 529 +++++++++++++++++++++++++++++++
 src/strategy/{ => verify}/verify.rs      | 165 ++--------
 src/utils.rs                             |  57 ++++
 tests/verify_run_command.rs              | 176 ++++++++++
 16 files changed, 1762 insertions(+), 165 deletions(-)
diff --git a/docs/designs/clo-271-run-command.md b/docs/designs/clo-271-run-command.md
new file mode 100644
index 0000000..f7cf75a
--- /dev/null
+++ b/docs/designs/clo-271-run-command.md
@@ -0,0 +1,282 @@
+# Design: CLO-271 — Implement RunCommand verify hook with sandboxing
+
+**Task:** T-023 (Roadmap Phase 4 - Verify hooks)  
+**PRD:** FR-14 · Canonical design: §5  
+**Status:** Draft  
+
+---
+
+## Problem
+
+The `VerifyHook` trait and `VerifyResult` enum (CLO-270) are now defined in
+`src/strategy/verify.rs`, but no concrete hook implementation exists for
+shell-command verification. `EscalatingRetry` (CLO-258) accepts an
+`Arc<dyn VerifyHook>` but today can only be wired to an `LLMVerifier`
+(CLO-272). The PRD (FR-14) requires a shell-command verify hook that gates
+retries with structured `FailureReason` values — exit code, signal, timeout,
+truncated stderr, and a sandbox-violation discriminant. Without `RunCommand`,
+the escalating-retry ladder cannot run compile/test gates, and T-029 (phase
+runner) has no way to shell-out.
+
+The existing `apply_verify::Verification` struct has shell-execution logic
+but uses a private `VerifyResult` incompatible with the trait, and it lacks
+sandboxing features (env allowlist, cpu timeout, secret redaction) required
+by PRD §5.
+
+---
+
+## Goals / Non-goals
+
+### Goals
+
+1. **Implement `RunCommand` struct** with fields: `cmd`, `args`, `env_allowlist`,
+   `cwd`, `wall_timeout`, `cpu_timeout`, `stdout_cap`, `stderr_cap`.
+2. **Implement `VerifyHook` for `RunCommand`** — returning `Pass` on exit-0,
+   `Fail { reason: FailureReason }` on non-zero, with structured reason.
+3. **Structured `FailureReason`** carrying exit code, signal, truncated stderr
+   tail, and a sandbox-violation discriminant (timeout, signal, non-zero exit).
+4. **Stdout/stderr capture with byte caps** and a truncation marker
+   (`…[truncated, N bytes elided]`).
+5. **Secret redaction** for allowlisted environment variables with
+   known-secret-shaped names before they flow into `FailureReason`.
+6. **Process group cleanup** on wall timeout (SIGKILL to process group).
+7. **Optional CPU timeout** via `libc::setrlimit(RLIMIT_CPU)` on Unix.
+8. **Integration tests** in `tests/verify_run_command.rs` covering 8 scenarios
+   (pass, non-zero exit, timeout, signal, output truncation, missing command,
+   env filtering, secret redaction).
+
+### Non-goals
+
+- **Do not** refactor `apply_verify::verification.rs` — it is legacy code
+  (`#[allow(dead_code)]`) and will remain untouched.
+- **Do not** implement `Repair` or `Score` variants of `VerifyResult` — those
+  are reserved for future work (CLO-272 LLMVerifier, CLO-274 scoring hook).
+- **Do not** wire `RunCommand` into the workflow loader (T-029) or config
+  deserialization — that's T-029's scope, not CLO-271.
+- **Do not** implement process namespace isolation (cgroups, chroot, seccomp) —
+  those are beyond v0 scope per PRD §5.
+- **Do not** support Windows-specific CPU limiting — `RLIMIT_CPU` is Unix-only.
+- **Do not** guarantee process group cleanup on Windows — best-effort only; full
+  guarantee deferred to post-v0.
+
+---
+
+## Architecture
+
+### Module layout
+
+```
+src/strategy/verify/
+  ├── mod.rs          ← re-exports (existing: add RunCommand)
+  ├── verify.rs       ← trait + types (existing: CLO-270)
+  ├── llm_verifier.rs ← existing: CLO-272
+  └── run_command.rs  ← NEW (this task)
+
+tests/verify_run_command.rs  ← NEW integration tests
+```
+
+### Data flow
+
+```
+EscalatingRetry::execute()
+   │
+   ├── backend.query() → QueryOutput
+   │
+   ├── VerifyContext::from_query_output(&query)
+   │
+   ▼
+RunCommand::verify(&ctx)
+   │
+   ├── (a) Resolve cmd from PATH
+   │   ├── bare command name → `which::which(&self.cmd)` (skip for absolute paths)
+   │   ├── if not found → Err(VerifyError::CommandNotFound { cmd: String })
+   │
+   ├── (b) Filter env: start from empty, then add only allowlisted vars
+   │   ├── if no allowlist → empty env (default-deny)
+   │   ├── apply secret redaction to known-secret-shaped values
+   │
+   ├── (c) Spawn process: cmd + args + filtered env + cwd
+   │   ├── set process group (Unix: setpgid for signal routing)
+   │   ├── optionally set cpu limit via `unsafe { cmd.pre_exec(|| libc::setrlimit(RLIMIT_CPU, ...)) }`
+   │   │   (inherently unsafe; scoped `#[allow(unsafe_code)]` in run_command module)
+   │
+   ├── (d) Capture stdout/stderr with bounded reads + drain on truncation
+   │   ├── bounded by stdout_cap / stderr_cap (bytes)
+   │   ├── if truncated: append `…[truncated, N bytes elided]`
+   │
+   ├── (e) Wait with wall_timeout
+   │   ├── if timeout → SIGKILL to process group
+   │   ├── → Fail { reason: sandbox_violation("timeout") }
+   │
+   ├── (f) Map exit status
+   │   ├── exit 0 → Pass
+   │   ├── non-zero exit → Fail { reason: non_zero(exit_code, stderr_tail) }
+   │   ├── signal → Fail { reason: sandbox_violation("signal") }
+   │
+   └── (g) Build FailureReason
+       ├── summary: "cmd exited {code}" or "cmd timed out" or "cmd killed by signal"
+       ├── stdout, stderr (truncated + redacted)
+       ├── truncated: bool
+       ├── exit_code: Option<i32>
+       ├── signal: Option<i32>
+       ├── sandbox_violation: Option<SandboxViolation>
+```
+
+### Type taxonomy
+
+| Type | Purpose | v0 concrete? |
+|------|---------|-------------|
+| `RunCommand` | VerifyHook impl that shells out | ✅ |
+| `FailureReason` | Carries structured failure context (existing from CLO-270) | ✅ |
+| `VerifyResult::Pass` | Exit 0 → Pass | ✅ |
+| `VerifyResult::Fail` | Exit non-zero / timeout / signal | ✅ |
+| `SandboxViolation` | Discriminant for timeout vs signal vs non-zero | ✅ |
+| `SecretRedactor` | Reuses `crate::utils::redact_secrets()` (shared with `escalating_retry.rs`) | ✅ |
+
+---
+
+## Public API surface
+
+### `RunCommand` struct
+
+```rust
+use std::path::PathBuf;
+use std::time::Duration;
+
+#[derive(Debug, Clone)]
+pub struct RunCommand {
+    pub cmd: String,
+    pub args: Vec<String>,
+    pub env_allowlist: Vec<String>,
+    pub cwd: Option<PathBuf>,
+    pub wall_timeout: Duration,
+    pub cpu_timeout: Option<Duration>,
+    pub stdout_cap: usize,
+    pub stderr_cap: usize,
+}
+
+impl Default for RunCommand {
+    fn default() -> Self { … }
+}
+
+impl RunCommand {
+    pub fn new(cmd: impl Into<String>) -> Self { … }
+    pub fn with_args(mut self, args: impl Into<Vec<String>>) -> Self { … }
+    pub fn with_env_allowlist(mut self, vars: &[&str]) -> Self { … }
+    pub fn with_cwd(mut self, path: impl Into<PathBuf>) -> Self { … }
+    pub fn with_wall_timeout(mut self, timeout: Duration) -> Self { … }
+    pub fn with_cpu_timeout(mut self, timeout: Option<Duration>) -> Self { … }
+    pub fn with_stdout_cap(mut self, cap: usize) -> Self { … }
+    pub fn with_stderr_cap(mut self, cap: usize) -> Self { … }
+}
+```
+
+### `VerifyHook` trait impl
+
+```rust
+use async_trait::async_trait;
+use crate::strategy::verify::{FailureReason, VerifyContext, VerifyError, VerifyHook, VerifyResult};
+
+#[async_trait]
+impl VerifyHook for RunCommand {
+    fn name(&self) -> String {
+        format!("run_command:{}", self.cmd)
+    }
+
+    async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> { … }
+}
+```
+
+### `SandboxViolation` enum
+
+```rust
+#[derive(Debug, Clone, PartialEq)]
+pub enum SandboxViolation {
+    Timeout,
+    Signal { signal: i32 },
+    NonZeroExit { code: i32 },
+}
+```
+
+### `SecretRedactor` helper
+
+```rust
+/// Reuses the shared `crate::utils::redact_secrets()` helper (originally from
+/// `escalating_retry.rs`) to avoid duplicate regex implementations.
+///
+/// Applied to both stdout and stderr before they flow into `FailureReason`.
+pub(crate) fn redact_output(output: &str) -> String {
+    crate::utils::redact_secrets(output)
+}
+```
+
+---
+
+## Test plan
+
+### Unit tests (in `src/strategy/verify/run_command.rs` under `#[cfg(test)]`)
+
+| # | Scenario | What it asserts |
+|---|----------|-----------------|
+| 1 | Exit 0 → Pass | `verify()` returns `Ok(VerifyResult::Pass)` |
+| 2 | Exit non-zero → Fail | Returns `Fail` with exit code and stderr in `FailureReason` |
+| 3 | Missing command → VerifyError | Returns `Err(VerifyError)` — command not found |
+| 4 | Env allowlist filters correctly | Only allowlisted vars are present in child env |
+| 5 | Secret redaction in stderr | Known-secret shapes are redacted before `FailureReason` |
+| 6 | Output truncation marker | stdout > cap gets `…[truncated, N bytes elided]` appended |
+
+### Integration tests (`tests/verify_run_command.rs`)
+
+| # | Scenario | What it asserts |
+|---|----------|-----------------|
+| 1 | `echo hello` → Pass | End-to-end: builds `RunCommand`, calls `verify()`, asserts Pass |
+| 2 | `false` → Fail | Asserts exit code 1 in `FailureReason` |
+| 3 | `sleep 60` with 100ms wall_timeout | Asserts `SandboxViolation::Timeout` |
+| 4 | Process group killed on timeout | Asserts no orphaned `sleep` processes after timeout |
+| 5 | `sh -c "echo aaaaaaaa; echo bbbbbbbb >&2"` with 4B caps | Asserts both stdout and stderr are truncated with markers |
+| 6 | `printenv` with allowlist `["USER", "HOME"]` | Asserts only `USER` and `HOME` appear in output |
+| 7 | `sh -c "echo $SECRET_TOKEN"` with redaction | Asserts `[REDACTED]` appears in `FailureReason.stderr` |
+| 8 | Cpu-limited infinite loop | `#[cfg(unix)]` — assert `SandboxViolation::Signal` |
+
+---
+
+## Migration / Rollout
+
+1. **No breaking changes** — `RunCommand` is a new type; existing code is untouched.
+2. **Export in `src/strategy/mod.rs`** — add `pub use verify::RunCommand;` alongside `LLMVerifier`.
+3. **No config wiring** — deserialization from TOML is T-029's scope; this task
+   provides only the programmatic API.
+4. **No workflow changes** — the reference `design-doc-tdd` workflow can use
+   `RunCommand` once T-029 lands the loader.
+
+---
+
+## Open questions
+
+1. **Signal mapping on non-Unix**: On Windows, `setpgid` and `RLIMIT_CPU` are
+   unavailable. The implementation should still work (process group and cpu limit
+   are no-ops), but the sandbox guarantees are weaker. Document this in rustdoc.
+
+2. **Verify `FailureReason` is `#[non_exhaustive]`**: Before adding the
+   `sandbox_violation: Option<SandboxViolation>` field, confirm the struct is
+   `#[non_exhaustive]`. If not, add the attribute first (non-breaking for v0
+   consumers).
+
+3. **Secret redaction scope**: Should the redactor also scrub values from
+   `stdout`, or only `stderr`? For now, redact both — an errant `printenv`
+   or debug dump could leak secrets to stdout too. Revisit if performance
+   becomes an issue.
+
+4. **Default wall_timeout**: What is a reasonable default? `30s` for compile/test
+   gates, `5s` for lightweight checks. Use `Duration::from_secs(30)` as default
+   in `RunCommand::default()`; callers can override.
+
+5. **Cancellation token integration**: Should `RunCommand::verify()` accept a
+   `tokio_util::sync::CancellationToken` for cooperative cancellation? The trait
+   signature does not currently support this. For v0, rely on wall_timeout
+   SIGKILL only. A future trait revision could add cancellation support.
+
+6. **Redaction duplication**: `escalating_retry.rs` already has a `redact_secrets()`
+   helper. To avoid drift, extract it to `crate::utils::redact_secrets()` and have
+   both `RunCommand` and `EscalatingRetry` call it. This is a small refactoring
+   that can be done in the same PR or a follow-up issue.
diff --git a/docs/discovery/clo-271.md b/docs/discovery/clo-271.md
new file mode 100644
index 0000000..f6d153b
--- /dev/null
+++ b/docs/discovery/clo-271.md
@@ -0,0 +1,124 @@
+# Discovery Report: CLO-271 — Implement RunCommand verify hook with sandboxing
+
+**Date:** 2026-05-01
+**Author:** Codex (orchestrator)
+
+---
+
+## Problem framing
+
+The `VerifyHook` trait and `VerifyResult` enum (CLO-270) are now defined in
+`src/strategy/verify.rs`, but no concrete hook implementations exist for
+shell-command verification. `EscalatingRetry` (CLO-258) accepts an
+`Arc<dyn VerifyHook>` but today can only be wired to an `LLMVerifier`
+(CLO-272). The PRD (FR-14) requires a shell-command verify hook that gates
+retries with structured `FailureReason` values — exit code, signal, timeout,
+truncated stderr, and a sandbox-violation discriminant. Without `RunCommand`,
+the escalating-retry ladder cannot run compile/test gates, and T-029 (phase
+runner) has no way to shell-out. The `apply_verify::Verification` struct
+exists but uses a private `VerifyResult` that is incompatible with the
+`VerifyHook` trait, and it lacks sandboxing features (env allowlist, cpu
+timeout, secret redaction) specified in PRD §5.
+
+## Existing code
+
+### `src/strategy/verify.rs` (CLO-270)
+- `VerifyHook` trait with `name()` and `verify()` methods
+- `VerifyResult` enum: `Pass`, `Fail { reason: FailureReason }`, `Repair`, `Score`
+- `FailureReason` struct: summary, stdout, stderr, truncated, exit_code
+- `VerifyContext` with `from_query_output()` factory
+- `VerifyError` for hook failures
+- `LLMVerifier` — first concrete `VerifyHook` impl (LLM-based judge)
+
+### `src/apply_verify/verification.rs`
+- `Verification` struct: shell command with timeout + output caps
+- Own `VerifyResult` struct (not the trait enum): success, stdout, stderr, exit_code, elapsed_ms, timed_out, truncated
+- Process group kill via `libc::kill(-pid, SIGKILL)` on timeout
+- Bounded read with drain to prevent writer block
+- Note: module is `#[allow(dead_code)]` — legacy code path
+
+### `src/strategy/escalating_retry.rs`
+- Holds `verify: Arc<dyn VerifyHook>` — injected at construction
+- Creates `VerifyContext::from_query_output(&query)` and calls `self.verify.verify(&verify_ctx)`
+- Maps `result.is_pass()` to ladder continuation/termination
+- Has `redact_secrets()` helper and `truncate_excerpt()` — reusable
+
+### `tests/verify_llm_verifier.rs`
+- Integration test pattern: `MockBackend` + `LLMVerifier` + assertions
+- Template for `tests/verify_run_command.rs`
+
+## Baseline score
+
+**6/10** — The trait foundation is solid (CLO-270), and the shell-execution
+pattern exists in `verification.rs`, but no RunCommand hook or tests exist.
+The sandboxing requirements (env allowlist, cpu timeout via rlimit, secret
+redaction in failure reasons) are entirely new code. The bridge between
+shell execution and the `VerifyHook` trait API has not been designed.
+
+## Approaches identified
+
+### Approach A — New `RunCommand` verify hook in `src/strategy/verify/run_command.rs`
+
+Implement `RunCommand` as a standalone module under `src/strategy/verify/`,
+directly implementing `impl VerifyHook for RunCommand`. Reuse the execution
+patterns from `verification.rs` (async tokio::process::Command, process group
+kill, bounded reads with drain) but adapt for the trait API. Add env-allowlist
+filtering, cpu_timeout (rlimit on unix), and secret redaction for
+known-secret-shaped env vars in failure reasons.
+
+- **Pros:** Clean module boundary; no legacy debt; evolves independently;
+  directly uses the `VerifyHook` trait API; sandboxing features are first-class.
+- **Cons:** Some duplicated execution logic from `verification.rs`.
+- **Effort:** M (3-5 files: mod.rs re-export + run_command.rs + test file)
+- **Risk:** Low — pattern is well-understood
+
+### Approach B — Refactor `Verification` into a `VerifyHook` impl
+
+Modify `src/apply_verify/verification.rs` to implement `VerifyHook` directly,
+renaming its `VerifyResult` to avoid collision with the trait enum, and adding
+sandboxing features to the struct.
+
+- **Pros:** Reuses tested execution logic; fewer total files
+- **Cons:** The module is `#[allow(dead_code)]` for a reason — it's legacy;
+  would blur module boundaries; env allowlist and cpu_timeout are structural
+  changes; secret redaction doesn't belong in `apply_verify`.
+- **Effort:** M (same scope but messier)
+- **Risk:** Medium — touching legacy module increases regression surface
+
+### Approach C — Adapter pattern: `RunCommand` wrapping `Verification`
+
+Implement `RunCommand` as a thin adapter that delegates execution to
+`Verification` but converts its result to the trait's `VerifyResult`.
+
+- **Pros:** Zero duplication of execution logic; `Verification` remains untouched
+- **Cons:** Env allowlist filtering and cpu timeout aren't supported by
+  `Verification`'s API; adapter would need those features before delegation;
+  `Verification::run()` is designed for `sh -c` not `cmd + args`; the resulting
+  type would be a leaky abstraction.
+- **Effort:** L (delegation plus pre-processing)
+- **Risk:** Medium-high — the gap between `Verification`'s API and the issue's
+  requirements is wider than it first appears; adapter becomes complex.
+
+## Approach chosen
+
+**Approach A** — New `RunCommand` verify hook in `src/strategy/verify/run_command.rs`.
+
+**Reason:** The issue demands a clean `VerifyHook` impl with specific
+sandboxing features (env allowlist, cpu_timeout, secret redaction) that the
+existing `Verification` struct was never designed to support. A fresh module
+under `src/strategy/verify/` is the right architectural home — it mirrors
+`LLMVerifier`'s placement and keeps the trait + all impls co-located. The
+execution patterns from `verification.rs` (process group kill, bounded reads
+with drain) can be directly re-imported or adapted without coupling to the
+legacy module.
+
+## Discovery debt
+
+None — the issue body provides a complete specification including TDD test
+contract (8 test scenarios) and sandboxing NFRs (9 items).
+
+## PRD
+
+The issue body (`CLO-271`) contains a fully specified scope, acceptance
+criteria, and test contract. No separate PRD file is needed — the discovery
+report references `docs/prds/clo-271-run-command.md` as the canonical spec.
diff --git a/docs/plans/clo-271-run-command.md b/docs/plans/clo-271-run-command.md
new file mode 100644
index 0000000..7a478cf
--- /dev/null
+++ b/docs/plans/clo-271-run-command.md
@@ -0,0 +1,102 @@
+# Plan: CLO-271 — Implement RunCommand verify hook with sandboxing
+
+## Context
+- Design: `docs/designs/clo-271-run-command.md`
+- Discovery: `docs/discovery/clo-271.md`
+- PRD: `docs/prds/clo-271-run-command.md`
+- Linear: https://linear.app/cloud-ai/issue/clo-271/implement-runcommand-verify-hook-with-sandboxing
+- Dependency: CLO-270 (VerifyHook trait + VerifyResult enum) — **done**
+- Blocks: T-023 (TestRunner reuses RunCommand internals), T-029 (phase runner)
+
+## Sub-tasks
+
+### ST1 Convert `verify.rs` to directory module
+**Files:** `src/strategy/verify.rs` → deleted; `src/strategy/verify/mod.rs`, `src/strategy/verify/verify.rs`, `src/strategy/verify/llm_verifier.rs` — created by moving existing content.
+**What:** Mechanical refactoring. Split the 575-line `verify.rs` into a directory module: `mod.rs` re-exports, `verify.rs` holds trait + types, `llm_verifier.rs` holds `LLMVerifier` impl. No logic changes.
+**Acceptance:** `cargo check` passes with zero new warnings.
+**Estimate:** S
+
+### ST2 Scaffold `RunCommand` struct + builder
+**Files:** `src/strategy/verify/run_command.rs` (new), `src/strategy/verify/mod.rs` (add re-export).
+**What:** Create `RunCommand` with all 8 fields (cmd, args, env_allowlist, cwd, wall_timeout, cpu_timeout, stdout_cap, stderr_cap). Add `Default` impl with `wall_timeout = 30s`, `stdout_cap = stderr_cap = 4096`. Add builder methods (`with_args`, `with_env_allowlist`, etc.). Add empty `impl VerifyHook for RunCommand` skeleton that returns `unimplemented!()`.
+**Acceptance:** `cargo check` passes; `RunCommand` is visible in `strategy::verify` module.
+**Estimate:** S
+
+### ST3 Implement core shell-out execution
+**Files:** `src/strategy/verify/run_command.rs`.
+**What:** Implement the core async execution:
+- Resolve `cmd` via `which::which` for bare names; skip for absolute paths
+- Spawn with `tokio::process::Command`, set process group (Unix `setpgid`)
+- Bounded async reads on stdout/stderr with drain-to-null on truncation
+- Wait with `tokio::time::timeout(wall_timeout, child.wait())`
+- On timeout: `libc::kill(-pgid, SIGKILL)` to process group, wait for exit
+- Collect output as strings (before truncation, not yet with redaction)
+- Return raw exit status
+
+**Acceptance:** `cargo test --lib` — unit tests for these scenarios pass:
+1. `echo hello` → exit 0, stdout captured
+2. `false` → exit 1, no stdout
+3. `sleep 60` with 100ms timeout → SIGKILL, no orphan processes
+4. Missing command → `VerifyError::CommandNotFound`
+**Estimate:** M
+
+### ST4 Add sandboxing (env allowlist + secret redaction + cpu timeout)
+**Files:** `src/strategy/verify/run_command.rs`, `src/utils.rs` (if extracting shared redact_secrets).
+**What:**
+- **Env filtering:** Start from empty env; add only keys in `env_allowlist` (default-deny). Apply secret redaction to values of known-secret-shaped keys.
+- **Secret redaction:** Extract `redact_secrets()` from `escalating_retry.rs` into `src/utils.rs` (shared helper), update `EscalatingRetry` to call it, have `RunCommand` reuse it.
+- **CPU timeout:** On Unix, `unsafe { cmd.pre_exec(|| { libc::setrlimit(RLIMIT_CPU, ...) }) }` before spawn. No-op on non-Unix with rustdoc note.
+
+**Acceptance:** `cargo test --lib` — unit tests for these scenarios pass:
+5. `printenv` with allowlist `["USER"]` → only `USER` appears in stdout
+6. `sh -c "echo $SECRET_TOKEN"` with redaction → `[REDACTED]` in output
+7. CPU-limited infinite loop (Unix-only `#[cfg(unix)]`) → exits by signal
+**Estimate:** M
+
+### ST5 Implement `VerifyHook` trait mapping + extend `FailureReason`
+**Files:** `src/strategy/verify/run_command.rs`, `src/strategy/verify/verify.rs`.
+**What:**
+- Add `SandboxViolation` enum (`Timeout`, `Signal { signal: i32 }`, `NonZeroExit { code: i32 }`).
+- Verify `FailureReason` is `#[non_exhaustive]`; if not, add the attribute. Then add `sandbox_violation: Option<SandboxViolation>` field.
+- Implement full `VerifyHook::verify()`:
+  - exit 0 → `Ok(VerifyResult::Pass)`
+  - non-zero → `Ok(VerifyResult::Fail { reason: FailureReason { ..., sandbox_violation: Some(NonZeroExit) } })`
+  - signal → `Fail { ..., sandbox_violation: Some(Signal) }`
+  - timeout → `Fail { ..., sandbox_violation: Some(Timeout) }`
+  - Apply redaction to stdout/stderr before building `FailureReason`
+
+**Acceptance:** `cargo test --lib` — all 6 unit test scenarios from design doc pass (Pass, non-zero, missing command, env filtering, secret redaction, output truncation).
+**Estimate:** M
+
+### ST6 Write integration tests
+**Files:** `tests/verify_run_command.rs` (new).
+**What:** Write 8 integration test scenarios per design doc §Test plan:
+1. `echo hello` → Pass
+2. `false` → Fail (exit code 1)
+3. `sleep 60` with 100ms timeout → `SandboxViolation::Timeout`
+4. Process group killed → no orphaned `sleep` processes
+5. Output truncation with 4B caps → truncation markers present
+6. `printenv` with allowlist → only allowlisted vars visible
+7. Secret redaction → `[REDACTED]` in `FailureReason.stderr`
+8. CPU-limited infinite loop → `#[cfg(unix)]` `SandboxViolation::Signal`
+
+**Acceptance:** `cargo test --test verify_run_command` passes.
+**Estimate:** M
+
+### ST7 Pre-merge gate
+**What:** Run full pre-merge gate: `make check` (fmt + clippy -D warnings + test).
+**Acceptance:** `make check` exits 0 with no warnings.
+**Estimate:** S
+
+## Pre-merge gate
+- `make check` (fmt + clippy + test)
+
+## Risks
+
+| Risk | Mitigation |
+|------|-----------|
+| **Directory module conversion breaks existing imports** | ST1 is purely mechanical; no logic changes. Run `cargo check` after every file move. |
+| **`which` crate returns different results on CI vs local** | Unit tests use absolute paths or mock the `which` call. Integration tests rely on POSIX utilities (`echo`, `sleep`, `false`) which are available on macOS/Linux CI. |
+| **`libc::setrlimit` pre_exec `unsafe` block flagged by clippy** | Scope `#[allow(unsafe_code)]` to the `run_command` module only; add safety comment explaining child-only scope. |
+| **Process group kill leaves orphans on macOS** | Test #4 (no orphaned processes) validates this empirically. Use `pgrep` in test to confirm no `sleep` processes remain. |
+| **`FailureReason` is not `#[non_exhaustive]`** | Check before adding field; if missing, add the attribute (non-breaking for v0). |
diff --git a/docs/prds/clo-271-run-command.md b/docs/prds/clo-271-run-command.md
new file mode 100644
index 0000000..a65d7b8
--- /dev/null
+++ b/docs/prds/clo-271-run-command.md
@@ -0,0 +1,28 @@
+# PRD: CLO-271 — Implement RunCommand verify hook with sandboxing
+
+| Field | Value |
+|-------|-------|
+| Source | Linear issue [CLO-271](https://linear.app/cloud-ai/issue/CLO-271/implement-runcommand-verify-hook-with-sandboxing) |
+| PRD Reference | FR-14 (shell command verify hook gates retries with structured failure reasons) |
+| Design doc | See `docs/designs/clo-270-hook.md` for trait architecture |
+| Security | PRD §5 — sandboxing NFRs (cwd, env, wall/cpu timeout, cap, signal cleanup, redaction) |
+
+## Scope
+
+- `RunCommand` struct: cmd, args, env_allowlist, cwd, wall_timeout, cpu_timeout, stdout_cap, stderr_cap
+- `impl VerifyHook for RunCommand` — maps exit-0 → `Pass`, non-zero → `Fail { reason: FailureReason }`
+- Structured `FailureReason` with exit code, signal, truncated stderr tail, sandbox-violation discriminant
+- Secret redaction for allowlisted vars with known-secret-shaped names
+- Process group cleanup on timeout/cancel
+- Tests in `tests/verify_run_command.rs`
+
+## Acceptance criteria
+
+- [ ] `cargo test --test verify_run_command` is green
+- [ ] No clippy warnings on the new module
+- [ ] PRD FR-14 satisfied: shell command verify hook gates retries with structured failure reasons
+
+## Dependencies
+
+- Blocked by: CLO-270 (VerifyHook trait + VerifyResult enum) — **done**
+- Blocks: T-023 (TestRunner reuses RunCommand internals), T-029 (phase runner)
diff --git a/docs/reviews/clo-271-design-gemini.md b/docs/reviews/clo-271-design-gemini.md
new file mode 100644
index 0000000..b3f4f9a
--- /dev/null
+++ b/docs/reviews/clo-271-design-gemini.md
@@ -0,0 +1,73 @@
+# Gemini design review - CLO-271
+
+## Context
+- Branch: feat/clo-271-run-command-01
+- Design: docs/designs/clo-271-run-command.md
+- PRD: docs/prds/clo-271-run-command.md
+- Discovery: docs/discovery/clo-271.md
+
+## Findings
+
+### F1 [minor] `cpu_timeout` via `setrlimit(RLIMIT_CPU)` needs `unsafe` `pre_exec`
+**Where:** design doc §Public API, `cpu_timeout` field
+**What:** The `RLIMIT_CPU` rlimit must be set in a `pre_exec` hook on `std::process::Command`, which requires `unsafe` (inherently unsafe per Rust docs). The design doc mentions `libc::setrlimit` but doesn't show *where* in the spawn lifecycle this happens.
+**Why it matters:** Missing this detail could lead to an implementer placing the rlimit call after spawn (where it affects the parent process instead of the child) or avoiding `unsafe` and shipping a broken feature.
+**Suggested fix:** Add an explicit note in the architecture data-flow step (c): "CPU limit is applied via `unsafe { cmd.pre_exec(|| { libc::setrlimit(...); Ok(()) }) }` before spawn. This is inherently unsafe and requires `#[allow(unsafe_code)]` scoped to the run_command module."
+
+### F2 [minor] Process group setup differs Unix vs Windows
+**Where:** design doc §Architecture, data flow step (c)
+**What:** The design says "set process group (Unix: setpgid for signal routing)" but doesn't specify the Windows path. `tokio::process::Command` has `process_group(0)` on Unix but not on Windows.
+**Why it matters:** The code will compile on Windows but the process-group-kill guarantee won't hold. This is fine for v0 (documented non-goal) but should be explicit.
+**Suggested fix:** Add to non-goals: "Process group cleanup on Windows is best-effort; full guarantee requires `CREATE_NEW_PROCESS_GROUP` which is deferred."
+
+### F3 [major] Missing `which` / PATH resolution detail
+**Where:** design doc §Architecture, data flow step (a)
+**What:** The design says "Resolve cmd from PATH" but doesn't specify *how*. The `which` crate is already a dependency in `Cargo.toml`.
+**Why it matters:** If `cmd` is a relative path (e.g., `cargo`), we need `which::which(&self.cmd)` before spawning. If it's absolute (e.g., `/usr/bin/cargo`), we skip resolution. Without this detail, the implementer may forget path resolution, producing a `VerifyError::NotFound` that's misleading when the command actually exists on PATH.
+**Suggested fix:** Add to data flow step (a): "Use `which::which(&self.cmd)` for bare command names; skip resolution for absolute paths. If resolution fails, return `VerifyError::CommandNotFound` (new variant)."
+
+### F4 [minor] No `VerifyError` variant for "command not found"
+**Where:** design doc §Public API, `VerifyHook` impl
+**What:** The existing `VerifyError` from CLO-270 likely doesn't have a "command not found" variant. The design doc should call out whether we add one or reuse an existing variant.
+**Why it matters:** Without a specific variant, the error will be a generic `VerifyError` with a string, losing structured diagnostics.
+**Suggested fix:** Propose adding `VerifyError::CommandNotFound { cmd: String }` to the `VerifyError` enum. Since this touches CLO-270's type, document it as a small additive change with no breaking impact (new enum variant on `#[non_exhaustive]` type).
+
+### F5 [nit] `SecretRedactor` naming vs `redact_secrets` in `escalating_retry.rs`
+**Where:** design doc §Public API, `SecretRedactor` helper
+**What:** `escalating_retry.rs` already has a `redact_secrets()` function. The design doc introduces `SecretRedactor` as a new helper. Two secret-redaction implementations is a recipe for drift.
+**Why it matters:** If the regexes diverge, secrets might leak in one path but not the other.
+**Suggested fix:** Either (a) extract the existing `redact_secrets()` into a shared `crate::utils::redact_secrets()` and have `RunCommand` call it, or (b) explicitly state that `RunCommand` uses a *different* redaction scope (env values only, not arbitrary text) so duplication is intentional.
+
+### F6 [minor] `SandboxViolation` enum — should it be on `FailureReason`?
+**Where:** design doc §Architecture, type taxonomy
+**What:** The design proposes `SandboxViolation` as a separate enum. But `FailureReason` (from CLO-270) already has `exit_code: Option<i32>` and `truncated: bool`. Adding a new enum requires extending `FailureReason`.
+**Why it matters:** This is a small but real change to CLO-270's type. Need to verify `FailureReason` is `#[non_exhaustive]` or that adding a field won't break consumers.
+**Suggested fix:** Check `src/strategy/verify.rs` for `#[non_exhaustive]` on `FailureReason`. If present, add `sandbox_violation: Option<SandboxViolation>` field. If not, make it `#[non_exhaustive]` first (non-breaking for downstream since the enum is not publicly matched in v0).
+
+### F7 [minor] Integration test #8 (CPU-limited infinite loop) is platform-dependent
+**Where:** design doc §Test plan, integration test #8
+**What:** Test #8 requires `RLIMIT_CPU` which is Unix-only. The test will fail on macOS CI if the platform doesn't support it, or will be skipped.
+**Why it matters:** A skipped test in CI is fine but should be documented as `#[cfg(unix)]` or `#[cfg(target_os = "linux")]`.
+**Suggested fix:** Mark test #8 with `#[cfg(unix)]` and add a comment: "Skipped on Windows; CPU limiting is Unix-only in v0."
+
+## Strengths
+- Clean module boundary: `run_command.rs` lives alongside `llm_verifier.rs` under `src/strategy/verify/`, mirroring the existing pattern.
+- Default-deny env with allowlist is the right security posture for a shell-out primitive.
+- Builder pattern (`with_args`, `with_env_allowlist`, etc.) is ergonomic and idiomatic.
+- Process group cleanup on timeout prevents orphaned processes — a common bug in naive async process wrappers.
+- Bounded reads with drain on truncation prevent writer-side blocking — learned directly from `verification.rs` and correctly carried forward.
+- No breaking changes to existing code; additive-only public surface.
+
+## Verdict
+approve_with_suggestions
+
+The design is solid, well-scoped, and architecturally sound. The 7 findings are all minor or nit-level; none are blockers. The most important fix is F3 (PATH resolution with `which` crate) — without it, the hook will be unreliable for common commands like `cargo test`. F1 and F6 should be addressed during implementation to avoid type-level surprises. F5 (redaction duplication) is worth a follow-up issue but not a blocker for this task.
+
+## Actionable Feedback (prioritized)
+
+1. **Add PATH resolution detail** — use `which` crate for bare command names, skip for absolute paths. Add `VerifyError::CommandNotFound` variant.
+2. **Document `unsafe` `pre_exec` for `RLIMIT_CPU`** — add explicit note in architecture data flow.
+3. **Verify `FailureReason` is `#[non_exhaustive]`** — add `sandbox_violation` field if so; otherwise make it `#[non_exhaustive]` first.
+4. **Clarify redaction scope** — state whether `RunCommand` reuses `escalating_retry.rs`'s `redact_secrets()` or has its own env-specific redactor.
+5. **Mark CPU-limit test as Unix-only** — `#[cfg(unix)]` on integration test #8.
+6. **Document Windows process-group limitation** — add one sentence to non-goals.
diff --git a/docs/reviews/clo-271-design-synthesis.md b/docs/reviews/clo-271-design-synthesis.md
new file mode 100644
index 0000000..fdbe9bc
--- /dev/null
+++ b/docs/reviews/clo-271-design-synthesis.md
@@ -0,0 +1,34 @@
+# Design review synthesis - CLO-271
+
+## Reviews consulted
+- Gemini architect review (self-review via persona): `docs/reviews/clo-271-design-gemini.md`
+
+## Verdict
+**approve_with_suggestions** — 7 findings total: 0 blockers, 1 major, 4 minor, 2 nits.
+
+## Applied suggestions
+
+### A1. PATH resolution (F3 — major)
+Added explicit data-flow step: "Use `which::which(&self.cmd)` for bare command names; skip resolution for absolute paths."
+Added `VerifyError::CommandNotFound { cmd: String }` variant to the type taxonomy.
+
+### A2. `unsafe` `pre_exec` for RLIMIT_CPU (F1 — minor)
+Added note to architecture data-flow step (c): "CPU limit applied via `unsafe { cmd.pre_exec(|| libc::setrlimit(...)) }` before spawn."
+
+### A3. `FailureReason` `#[non_exhaustive]` (F6 — minor)
+Added explicit call-out: "Verify `FailureReason` is `#[non_exhaustive]`; if not, make it so before adding `sandbox_violation` field."
+
+### A4. Redaction scope (F5 — minor)
+Added to open questions: "Reuse `escalating_retry.rs`'s `redact_secrets()` via a shared `crate::utils` helper to avoid duplication."
+
+### A5. CPU-limit test platform (F7 — minor)
+Marked integration test #8 as `#[cfg(unix)]` in test plan.
+
+### A6. Windows process-group (F2 — minor)
+Added sentence to non-goals: "Process group cleanup on Windows is best-effort; full guarantee deferred."
+
+## Flagged suggestions (none)
+No suggestions contradicted the chosen approach. All 7 findings were additive or refinement.
+
+## Remaining open questions
+All open questions from the draft remain valid. No new questions introduced by review.
diff --git a/docs/status/clo-271-workflow.yaml b/docs/status/clo-271-workflow.yaml
new file mode 100644
index 0000000..856d659
--- /dev/null
+++ b/docs/status/clo-271-workflow.yaml
@@ -0,0 +1,156 @@
+task_id: clo-271
+task_type: development
+classification_reason: ''
+task_profile:
+  has_backend: false
+  has_frontend: false
+  has_data_model: false
+  has_external_deps: false
+  skip_probe: false
+pending_human_action: null
+linear:
+  team: Cloud-ai
+  project: Loker
+  status_at_start: Backlog
+  blocks: []
+  blocked_by: []
+  branch_actual: feat/clo-271-run-command-01
+workflow:
+  current_phase: implement
+  status: active
+  created_at: '2026-05-01T08:41:34.520Z'
+  updated_at: '2026-05-01T09:45:57.044Z'
+phases:
+  discovery:
+    status: complete
+    approach_chosen: New RunCommand VerifyHook in src/strategy/verify/run_command.rs
+    approaches_identified: 3
+    approved: true
+    baseline_score: 6
+    discovery_debt: []
+    discovery_report: docs/discovery/clo-271.md
+    prd_created: true
+    prd_exists: true
+    prd_file: docs/prds/clo-271-run-command.md
+    problem_framed: true
+  design:
+    status: complete
+    design_doc: docs/designs/clo-271-run-command.md
+    discovery_context_used: true
+    draft_ready: true
+    draft_source: manual
+    applied_suggestions:
+      - A1 PATH resolution with which crate
+      - A2 unsafe pre_exec for RLIMIT_CPU
+      - A3 FailureReason non_exhaustive check
+      - A4 redaction scope shared helper
+      - A5 CPU test Unix-only
+      - A6 Windows process-group best-effort
+    flagged_suggestions: []
+    review_completed: true
+    review_gemini: docs/reviews/clo-271-design-gemini.md
+    review_synthesis: docs/reviews/clo-271-design-synthesis.md
+    review_verdict: approve_with_changes
+    human_review_completed: true
+    plannotator_annotations: ''
+    finalized: true
+  plan:
+    status: complete
+    plan_file: docs/plans/clo-271-run-command.md
+    approved: true
+  implement:
+    status: complete
+    commits:
+      - 97c6a08
+      - f2a38e7
+      - 3444c4f
+  pr:
+    status: pending
+  complete:
+    status: pending
+history:
+  - timestamp: '2026-05-01T08:41:34.520Z'
+    action: workflow_started
+    phase: init
+    details: Workflow initialized for clo-271 as development
+  - timestamp: '2026-05-01T08:42:04.484Z'
+    action: project_sync_skipped
+    phase: init
+    details: No PROJECT.md/ROADMAP.md/DEPENDENCIES.md exist in this repo.
+  - timestamp: '2026-05-01T08:44:19.264Z'
+    action: branch_created
+    phase: discovery
+    details: Already on branch feat/clo-271-run-command-01 (matches Linear suggestion)
+  - timestamp: '2026-05-01T08:46:15.022Z'
+    action: discovery_approved
+    phase: discovery
+    details: >-
+      Approach: New RunCommand VerifyHook in src/strategy/verify/run_command.rs. Baseline 6/10. 3 approaches considered
+      (new module, refactor verification.rs, adapter pattern).
+  - timestamp: '2026-05-01T08:46:17.366Z'
+    action: phase_transition
+    phase: discovery
+    details: Transitioned from discovery to design
+  - timestamp: '2026-05-01T08:49:46.676Z'
+    action: design_draft_ready
+    phase: design
+    details: >-
+      Draft generated manually (lok workflow timed out). 186 lines at docs/designs/clo-271-run-command.md. 7 sections
+      verified: Problem, Goals/Non-goals, Architecture, Public API, Test plan, Migration, Open questions.
+  - timestamp: '2026-05-01T08:57:19.711Z'
+    action: design_review_complete
+    phase: design
+    details: >-
+      Review verdict: approve_with_suggestions. 6 applied (PATH resolution, unsafe pre_exec, FailureReason
+      non_exhaustive, redaction scope, CPU test platform, Windows process-group), 0 flagged.
+  - timestamp: '2026-05-01T09:03:29.659Z'
+    action: design_human_review_complete
+    phase: design
+    details: Plannotator review approved. No annotations.
+  - timestamp: '2026-05-01T09:04:01.380Z'
+    action: design_finalized
+    phase: design
+    details: >-
+      Design ready for plan phase. 282 lines, 7 sections, 6 open questions documented. Public API signatures: RunCommand
+      struct + VerifyHook impl + SandboxViolation enum. Test plan: 6 unit tests + 8 integration tests.
+  - timestamp: '2026-05-01T09:04:04.471Z'
+    action: phase_transition
+    phase: design
+    details: Transitioned from design to plan
+  - timestamp: '2026-05-01T09:06:13.811Z'
+    action: plan_created
+    phase: plan
+    details: >-
+      Plan with 7 sub-tasks at docs/plans/clo-271-run-command.md. Ordered: ST1 structural (S), ST2 scaffold (S), ST3
+      core execution (M), ST4 sandboxing (M), ST5 trait mapping (M), ST6 integration tests (M), ST7 pre-merge gate (S).
+  - timestamp: '2026-05-01T09:06:17.590Z'
+    action: plan_approved
+    phase: plan
+    details: >-
+      Plan approved. 7 sub-tasks queued: ST1 directory conversion (S), ST2 scaffold (S), ST3 core execution (M), ST4
+      sandboxing (M), ST5 trait mapping (M), ST6 integration tests (M), ST7 pre-merge gate (S).
+  - timestamp: '2026-05-01T09:06:20.192Z'
+    action: phase_transition
+    phase: plan
+    details: Transitioned from plan to implement
+  - timestamp: '2026-05-01T09:09:43.856Z'
+    action: subtask_complete
+    phase: implement
+    details: >-
+      ST1 landed: Converted verify.rs to directory module (verify/mod.rs + verify/verify.rs + verify/llm_verifier.rs).
+      Commit 97c6a08.
+  - timestamp: '2026-05-01T09:10:49.015Z'
+    action: subtask_complete
+    phase: implement
+    details: 'ST2 landed: RunCommand struct with 8 fields, Default, builder pattern, skeleton VerifyHook impl. Commit f2a38e7.'
+  - timestamp: '2026-05-01T09:45:32.721Z'
+    action: subtask_complete
+    phase: implement
+    details: >-
+      ST3-ST7 landed: core execution, sandboxing (env allowlist, secret redaction, cpu timeout), VerifyHook trait
+      mapping + SandboxViolation enum, integration tests (8 scenarios), and pre-merge gate (make check green). Commit
+      3444c4f.
+  - timestamp: '2026-05-01T09:45:57.044Z'
+    action: implementation_complete
+    phase: implement
+    details: All 7 sub-tasks landed. make check green (636 lib + 532 bin + all integration tests pass). 3 commits.
diff --git a/src/main.rs b/src/main.rs
index 54b5151..b15a609 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,5 @@
+#![allow(dead_code)]
+
 mod apply_verify;
 mod backend;
 mod cache;
diff --git a/src/strategy/escalating_retry.rs b/src/strategy/escalating_retry.rs
index 13b1995..d6e29b1 100644
--- a/src/strategy/escalating_retry.rs
+++ b/src/strategy/escalating_retry.rs
@@ -19,9 +19,8 @@ use crate::strategy::{
     VerifyOutcome, VerifyResult, SCHEMA_VERSION,
 };
 use async_trait::async_trait;
-use regex::Regex;
 use serde::{Deserialize, Serialize};
-use std::sync::{Arc, LazyLock};
+use std::sync::Arc;
 
 /// 4 KiB excerpt fits inside an 8 KiB envelope while leaving headroom for
 /// verifier reason and backend error class.
@@ -133,25 +132,6 @@ fn backend_error_class(err: &BackendError) -> String {
     .to_string()
 }
 
-/// AWS access keys.
-static AWS_KEY_RE: LazyLock<Regex> =
-    LazyLock::new(|| Regex::new(r"AKIA[0-9A-Z]{16}").expect("valid regex"));
-
-/// Generic `key=value` shapes (case-insensitive; redacts only the value side).
-static KEY_VALUE_RE: LazyLock<Regex> = LazyLock::new(|| {
-    Regex::new(r#"(?i)((?:api[_-]?key|secret|token|password)\s*[=:]\s*)[^\s'\"]+"#)
-        .expect("valid regex")
-});
-
-/// Bearer tokens in `Authorization` headers.
-static BEARER_RE: LazyLock<Regex> =
-    LazyLock::new(|| Regex::new(r"(?i)Bearer\s+[A-Za-z0-9._\-~+/=]+").expect("valid regex"));
-
-/// Heuristic: long base64-ish blob preceded by key/secret/token.
-static SECRET_HEURISTIC_RE: LazyLock<Regex> = LazyLock::new(|| {
-    Regex::new(r"(?i)\b(key|secret|token)[\s:=]+([A-Za-z0-9+/=_\-]{32,})").expect("valid regex")
-});
-
 /// Redact common secret shapes from text before they reach the next rung's
 /// prompt envelope. Applied to *every* byte of `FailureContext` text
 /// (verify_reason, response_excerpt, and the final assembled header).
@@ -160,15 +140,7 @@ static SECRET_HEURISTIC_RE: LazyLock<Regex> = LazyLock::new(|| {
 /// future centralised secret-scrubbing service should absorb this function
 /// rather than invent a second one.
 pub(crate) fn redact_secrets(input: &str) -> String {
-    let mut result = AWS_KEY_RE.replace_all(input, "[REDACTED]").into_owned();
-    result = KEY_VALUE_RE
-        .replace_all(&result, "${1}[REDACTED]")
-        .into_owned();
-    result = BEARER_RE.replace_all(&result, "[REDACTED]").into_owned();
-    result = SECRET_HEURISTIC_RE
-        .replace_all(&result, "$1 [REDACTED]")
-        .into_owned();
-    result
+    crate::utils::redact_secrets(input)
 }
 
 /// Truncate `s` to at most `max_bytes` total (including the suffix),
diff --git a/src/strategy/mod.rs b/src/strategy/mod.rs
index 7ceac7d..0319576 100644
--- a/src/strategy/mod.rs
+++ b/src/strategy/mod.rs
@@ -31,7 +31,8 @@ pub use escalating_retry::EscalatingRetry;
 pub use parallel_fanout::{ParallelFanOut, TargetSpec};
 pub use single_model::SingleModel;
 pub use verify::{
-    FailureReason, LLMVerifier, VerifyContext, VerifyError, VerifyHook, VerifyResult,
+    FailureReason, LLMVerifier, RunCommand, SandboxViolation, VerifyContext, VerifyError,
+    VerifyHook, VerifyResult,
 };
 
 /// `schema_version` value emitted by every `StrategyOutput`. Pinned to the
diff --git a/src/strategy/verify/llm_verifier.rs b/src/strategy/verify/llm_verifier.rs
new file mode 100644
index 0000000..33cb908
--- /dev/null
+++ b/src/strategy/verify/llm_verifier.rs
@@ -0,0 +1,142 @@
+//! LLM-based verify hook.
+//!
+//! Delegates the verification decision to a backend (LLM) and parses a
+//! deterministic yes/no verdict from the response.
+
+use std::collections::HashMap;
+use std::path::Path;
+use std::sync::Arc;
+
+use async_trait::async_trait;
+
+use crate::backend::Backend;
+use crate::strategy::verify::{
+    FailureReason, VerifyContext, VerifyError, VerifyHook, VerifyResult,
+};
+
+/// Concrete verify hook that delegates to a backend and parses a
+/// deterministic yes/no verdict from the backend response.
+pub struct LLMVerifier {
+    /// Identifier used for observability/debugging.
+    pub backend: String,
+    backend_client: Arc<dyn Backend>,
+    /// Optional model override passed to the backend.
+    pub model: Option<String>,
+    /// Prompt template used for verification. `{candidate}` is replaced with
+    /// the candidate text under test; any `{key}` present in `params`
+    /// is also substituted.
+    pub prompt_template: String,
+    /// Optional system-level context prepended to the candidate prompt.
+    pub system_prompt: Option<String>,
+    /// Temperature hint used when available. `0.0` is deterministic default.
+    pub temperature: f32,
+    params: HashMap<String, String>,
+}
+
+impl LLMVerifier {
+    pub const DEFAULT_TEMPERATURE: f32 = 0.0;
+
+    /// Construct a verifier bound to a backend object.
+    pub fn new(
+        backend: impl Into<String>,
+        backend_client: Arc<dyn Backend>,
+        prompt_template: impl Into<String>,
+    ) -> Self {
+        Self {
+            backend: backend.into(),
+            backend_client,
+            model: None,
+            prompt_template: prompt_template.into(),
+            system_prompt: None,
+            temperature: Self::DEFAULT_TEMPERATURE,
+            params: HashMap::new(),
+        }
+    }
+
+    /// Set deterministic temperature hint (used where backend support exists).
+    pub fn with_temperature(mut self, temperature: f32) -> Self {
+        self.temperature = temperature;
+        self
+    }
+
+    /// Set a model override.
+    pub fn with_model(mut self, model: impl Into<String>) -> Self {
+        self.model = Some(model.into());
+        self
+    }
+
+    /// Set system prompt prepended to candidate prompt.
+    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
+        self.system_prompt = Some(prompt.into());
+        self
+    }
+
+    /// Add one user-supplied template parameter.
+    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
+        self.params.insert(key.into(), value.into());
+        self
+    }
+
+    fn rendered_prompt(&self, candidate: &str) -> String {
+        let mut prompt = self.prompt_template.clone();
+
+        // Sort params by key length descending so that longer, more specific
+        // keys (e.g. {env_name}) are replaced before shorter substrings (e.g. {env}).
+        // HashMap iteration order is non-deterministic, so we must sort explicitly
+        // for reproducible prompt rendering.
+        let mut sorted_params: Vec<(&String, &String)> = self.params.iter().collect();
+        sorted_params.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
+        for (key, value) in sorted_params {
+            let needle = format!("{{{key}}}");
+            prompt = prompt.replace(&needle, value);
+        }
+
+        let prompt = prompt.replace("{candidate}", candidate);
+
+        match &self.system_prompt {
+            Some(system) => format!("{system}\n\n{prompt}"),
+            None => prompt,
+        }
+    }
+
+    fn parse_response(raw: &str) -> VerifyResult {
+        let first = raw
+            .split_whitespace()
+            .next()
+            .map(|token| token.trim().trim_matches(|c: char| !c.is_alphanumeric()))
+            .map(|token| token.to_ascii_lowercase())
+            .unwrap_or_default();
+
+        if first == "yes" {
+            return VerifyResult::pass();
+        }
+
+        if first == "no" {
+            return VerifyResult::fail_with(FailureReason::new("no").with_stdout(raw.to_string()));
+        }
+
+        VerifyResult::fail_with(
+            FailureReason::new("unparseable verifier response").with_stdout(raw.to_string()),
+        )
+    }
+}
+
+#[async_trait]
+impl VerifyHook for LLMVerifier {
+    fn name(&self) -> &str {
+        "LLMVerifier"
+    }
+
+    async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
+        let prompt = self.rendered_prompt(&ctx.stdout);
+
+        match self
+            .backend_client
+            .query(&prompt, Path::new("."), self.model.as_deref())
+            .await
+        {
+            Ok(query) => Ok(Self::parse_response(&query.stdout)),
+            Err(err) => Err(VerifyError::new(format!("backend error: {err}"))),
+        }
+    }
+}
diff --git a/src/strategy/verify/mod.rs b/src/strategy/verify/mod.rs
new file mode 100644
index 0000000..5ba44a5
--- /dev/null
+++ b/src/strategy/verify/mod.rs
@@ -0,0 +1,22 @@
+#![allow(clippy::module_inception)]
+
+//! Verification hooks for strategy gating.
+//!
+//! Each hook implements [`VerifyHook`] and returns a [`VerifyResult`]
+//! (Pass / Fail / reserved variants). Concrete implementations:
+//!
+//! - [`LLMVerifier`] — delegates to an LLM backend and parses yes/no.
+//! - [`RunCommand`] — shells out and maps exit status (CLO-271).
+
+pub mod llm_verifier;
+pub mod run_command;
+pub mod verify;
+
+// Re-export the core types so `strategy::verify::FailureReason` etc. work.
+pub use verify::{
+    FailureReason, SandboxViolation, VerifyContext, VerifyError, VerifyHook, VerifyResult,
+};
+
+// Re-export concrete implementations.
+pub use llm_verifier::LLMVerifier;
+pub use run_command::RunCommand;
diff --git a/src/strategy/verify/run_command.rs b/src/strategy/verify/run_command.rs
new file mode 100644
index 0000000..d7edc4c
--- /dev/null
+++ b/src/strategy/verify/run_command.rs
@@ -0,0 +1,529 @@
+#![allow(unsafe_code)]
+
+//! Shell-command verify hook with sandboxing.
+//!
+//! Spawns a subprocess, captures stdout/stderr with byte caps, and maps
+//! exit status to [`VerifyResult`].
+
+use std::path::{Path, PathBuf};
+use std::process::Stdio;
+use std::time::{Duration, Instant};
+
+use async_trait::async_trait;
+use tokio::io::{AsyncRead, AsyncReadExt};
+use tokio::process::Command;
+use tokio::time::timeout;
+use which::which;
+
+use crate::strategy::verify::{
+    FailureReason, SandboxViolation, VerifyContext, VerifyError, VerifyHook, VerifyResult,
+};
+use crate::utils::redact_secrets;
+
+/// Default wall-clock timeout for compile/test gates.
+const DEFAULT_WALL_TIMEOUT: Duration = Duration::from_secs(30);
+
+/// Default byte cap for stdout and stderr capture.
+const DEFAULT_OUTPUT_CAP: usize = 4096;
+
+/// Shell-command verify hook.
+#[derive(Debug, Clone)]
+pub struct RunCommand {
+    /// Command to execute (bare name resolved via `which`, absolute path used as-is).
+    pub cmd: String,
+    /// Arguments passed to the command.
+    pub args: Vec<String>,
+    /// Environment variable names allowed in child process.
+    /// Default: empty (default-deny).
+    pub env_allowlist: Vec<String>,
+    /// Working directory for the command.
+    pub cwd: Option<PathBuf>,
+    /// Wall-clock timeout before process-group SIGKILL.
+    pub wall_timeout: Duration,
+    /// Optional CPU-time timeout (Unix only).
+    pub cpu_timeout: Option<Duration>,
+    /// Max bytes captured from stdout.
+    pub stdout_cap: usize,
+    /// Max bytes captured from stderr.
+    pub stderr_cap: usize,
+}
+
+impl Default for RunCommand {
+    fn default() -> Self {
+        Self {
+            cmd: String::new(),
+            args: Vec::new(),
+            env_allowlist: Vec::new(),
+            cwd: None,
+            wall_timeout: DEFAULT_WALL_TIMEOUT,
+            cpu_timeout: None,
+            stdout_cap: DEFAULT_OUTPUT_CAP,
+            stderr_cap: DEFAULT_OUTPUT_CAP,
+        }
+    }
+}
+
+impl RunCommand {
+    /// Construct with a command name/path.
+    pub fn new(cmd: impl Into<String>) -> Self {
+        Self {
+            cmd: cmd.into(),
+            ..Self::default()
+        }
+    }
+
+    /// Replace command-line arguments.
+    pub fn with_args(mut self, args: impl Into<Vec<String>>) -> Self {
+        self.args = args.into();
+        self
+    }
+
+    /// Replace env allow-list.
+    pub fn with_env_allowlist(mut self, vars: &[&str]) -> Self {
+        self.env_allowlist = vars.iter().map(|s| s.to_string()).collect();
+        self
+    }
+
+    /// Set working directory.
+    pub fn with_cwd(mut self, path: impl Into<PathBuf>) -> Self {
+        self.cwd = Some(path.into());
+        self
+    }
+
+    /// Override wall-clock timeout.
+    pub fn with_wall_timeout(mut self, timeout: Duration) -> Self {
+        self.wall_timeout = timeout;
+        self
+    }
+
+    /// Override CPU timeout.
+    pub fn with_cpu_timeout(mut self, timeout: Option<Duration>) -> Self {
+        self.cpu_timeout = timeout;
+        self
+    }
+
+    /// Override stdout capture cap.
+    pub fn with_stdout_cap(mut self, cap: usize) -> Self {
+        self.stdout_cap = cap;
+        self
+    }
+
+    /// Override stderr capture cap.
+    pub fn with_stderr_cap(mut self, cap: usize) -> Self {
+        self.stderr_cap = cap;
+        self
+    }
+
+    fn resolve_command(&self) -> Result<PathBuf, VerifyError> {
+        if self.cmd.is_empty() {
+            return Err(VerifyError::new("no command configured"));
+        }
+
+        let path = Path::new(&self.cmd);
+        if path.is_absolute() {
+            Ok(PathBuf::from(path))
+        } else {
+            which(&self.cmd)
+                .map_err(|_| VerifyError::new(format!("command not found: {}", self.cmd)))
+        }
+    }
+
+    fn build_environment(&self) -> Vec<(String, String)> {
+        self.env_allowlist
+            .iter()
+            .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
+            .map(|(name, value)| {
+                let redacted = if is_secret_like_env_key(&name) {
+                    redact_secrets(&value)
+                } else {
+                    value
+                };
+                (name, redacted)
+            })
+            .collect()
+    }
+
+    async fn run(&self) -> Result<CommandRun, VerifyError> {
+        let command_path = self.resolve_command()?;
+
+        let mut command = Command::new(&command_path);
+        command
+            .args(&self.args)
+            .env_clear()
+            .stdin(Stdio::null())
+            .stdout(Stdio::piped())
+            .stderr(Stdio::piped());
+
+        if let Some(cwd) = &self.cwd {
+            command.current_dir(cwd);
+        }
+
+        for (key, value) in self.build_environment() {
+            command.env(key, value);
+        }
+
+        // Put each child in its own process group, so SIGKILL on timeout
+        // reaps descendants as well.
+        #[cfg(unix)]
+        command.process_group(0);
+
+        #[cfg(unix)]
+        if let Some(cpu_timeout) = self.cpu_timeout {
+            let secs = cpu_timeout.as_secs().max(1);
+            // SAFETY: this closure runs in the child before exec and only
+            // mutates process-local resource limits.
+            unsafe {
+                command.pre_exec(move || {
+                    let limit = libc::rlimit {
+                        rlim_cur: secs as libc::rlim_t,
+                        rlim_max: secs as libc::rlim_t,
+                    };
+                    let rc = libc::setrlimit(libc::RLIMIT_CPU, &limit);
+                    if rc != 0 {
+                        return Err(std::io::Error::last_os_error());
+                    }
+                    Ok(())
+                });
+            }
+        }
+
+        let start = Instant::now();
+        let mut child = command.spawn().map_err(|err| {
+            VerifyError::new(format!("failed to spawn command '{}': {err}", self.cmd))
+        })?;
+
+        let stdout_stream = child
+            .stdout
+            .take()
+            .ok_or_else(|| VerifyError::new("failed to capture child stdout pipe".to_string()))?;
+        let stderr_stream = child
+            .stderr
+            .take()
+            .ok_or_else(|| VerifyError::new("failed to capture child stderr pipe".to_string()))?;
+
+        let stdout_handle = tokio::spawn(read_stream_bounded(stdout_stream, self.stdout_cap));
+        let stderr_handle = tokio::spawn(read_stream_bounded(stderr_stream, self.stderr_cap));
+
+        let (status, timed_out) = match timeout(self.wall_timeout, child.wait()).await {
+            Ok(result) => {
+                let status = result.map_err(|err| {
+                    VerifyError::new(format!("failed to wait for command '{}': {err}", self.cmd))
+                })?;
+                (status, false)
+            }
+            Err(_) => {
+                kill_process_group(child.id());
+                let status = child.wait().await.map_err(|err| {
+                    VerifyError::new(format!(
+                        "failed to reap timed-out command '{}': {err}",
+                        self.cmd
+                    ))
+                })?;
+                (status, true)
+            }
+        };
+
+        let stdout = stdout_handle
+            .await
+            .map_err(|_| VerifyError::new("stdout reader task failed"))?;
+        let stderr = stderr_handle
+            .await
+            .map_err(|_| VerifyError::new("stderr reader task failed"))?;
+
+        Ok(CommandRun {
+            status,
+            timed_out,
+            stdout,
+            stderr,
+            elapsed_ms: start.elapsed().as_millis() as u64,
+        })
+    }
+}
+
+#[derive(Debug)]
+struct CommandRun {
+    status: std::process::ExitStatus,
+    timed_out: bool,
+    stdout: CapturedOutput,
+    stderr: CapturedOutput,
+    elapsed_ms: u64,
+}
+
+#[derive(Debug)]
+struct CapturedOutput {
+    data: Vec<u8>,
+    truncated: bool,
+    elided_bytes: usize,
+}
+
+impl CapturedOutput {
+    fn to_reason_text(&self) -> String {
+        let mut text = String::from_utf8_lossy(&self.data).into_owned();
+        if self.truncated {
+            text.push_str(&format!(
+                " …[truncated, {} bytes elided]",
+                self.elided_bytes
+            ));
+        }
+        redact_secrets(&text)
+    }
+}
+
+async fn read_stream_bounded<R: AsyncRead + Unpin + Send + 'static>(
+    mut reader: R,
+    max_bytes: usize,
+) -> CapturedOutput {
+    let mut buf = [0u8; 8192];
+    let mut data = Vec::new();
+    let mut truncated = false;
+    let mut elided_bytes = 0usize;
+
+    loop {
+        match reader.read(&mut buf).await {
+            Ok(0) => break,
+            Ok(n) => {
+                let remaining = max_bytes.saturating_sub(data.len());
+                if remaining == 0 {
+                    truncated = true;
+                    elided_bytes += n;
+                    elided_bytes += drain_stream(&mut reader).await;
+                    break;
+                }
+
+                let take = n.min(remaining);
+                data.extend_from_slice(&buf[..take]);
+
+                if take < n {
+                    truncated = true;
+                    elided_bytes += n - take;
+                    elided_bytes += drain_stream(&mut reader).await;
+                    break;
+                }
+            }
+            Err(_) => break,
+        }
+    }
+
+    CapturedOutput {
+        data,
+        truncated,
+        elided_bytes,
+    }
+}
+
+async fn drain_stream<R: AsyncRead + Unpin + Send>(reader: &mut R) -> usize {
+    let mut buf = [0u8; 8192];
+    let mut total = 0usize;
+
+    loop {
+        match reader.read(&mut buf).await {
+            Ok(0) | Err(_) => break,
+            Ok(n) => {
+                total += n;
+            }
+        }
+    }
+
+    total
+}
+
+fn is_secret_like_env_key(key: &str) -> bool {
+    let key = key.to_ascii_uppercase();
+    key.contains("SECRET")
+        || key.contains("TOKEN")
+        || key.contains("PASSWORD")
+        || key.contains("API_KEY")
+        || key.contains("AUTH")
+}
+
+#[cfg(unix)]
+fn status_signal(status: &std::process::ExitStatus) -> Option<i32> {
+    use std::os::unix::process::ExitStatusExt;
+    status.signal()
+}
+
+#[cfg(not(unix))]
+fn status_signal(_status: &std::process::ExitStatus) -> Option<i32> {
+    None
+}
+
+#[cfg(unix)]
+fn kill_process_group(pid: Option<u32>) {
+    if let Some(pid) = pid {
+        // SAFETY: this sends SIGKILL to a process group spawned by us and avoids
+        // orphaned children. If the pid is stale or already dead, this is a
+        // best-effort cleanup attempt.
+        unsafe {
+            libc::kill(-(pid as i32), libc::SIGKILL);
+        }
+    }
+}
+
+#[cfg(not(unix))]
+fn kill_process_group(pid: Option<u32>) {
+    let _ = pid;
+}
+
+#[async_trait]
+impl VerifyHook for RunCommand {
+    fn name(&self) -> &str {
+        "RunCommand"
+    }
+
+    async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
+        let run = self.run().await?;
+
+        let stdout = redact_secrets(&run.stdout.to_reason_text());
+        let stderr = redact_secrets(&run.stderr.to_reason_text());
+        let truncated = run.stdout.truncated || run.stderr.truncated;
+        let signal = status_signal(&run.status);
+
+        if run.timed_out {
+            return Ok(VerifyResult::Fail {
+                reason: FailureReason::new(format!("command timed out: {}", self.cmd))
+                    .with_stdout(stdout)
+                    .with_stderr(stderr)
+                    .with_truncated(truncated)
+                    .with_sandbox_violation(SandboxViolation::Timeout),
+            });
+        }
+
+        if let Some(code) = run.status.code() {
+            if code == 0 {
+                return Ok(VerifyResult::Pass);
+            }
+
+            return Ok(VerifyResult::Fail {
+                reason: FailureReason::new(format!("command exited with status {code}"))
+                    .with_exit_code(code)
+                    .with_stdout(stdout)
+                    .with_stderr(stderr)
+                    .with_truncated(truncated)
+                    .with_sandbox_violation(SandboxViolation::NonZeroExit { code }),
+            });
+        }
+
+        if let Some(sig) = signal {
+            return Ok(VerifyResult::Fail {
+                reason: FailureReason::new(format!("command killed by signal {sig}"))
+                    .with_stdout(stdout)
+                    .with_stderr(stderr)
+                    .with_truncated(truncated)
+                    .with_sandbox_violation(SandboxViolation::Signal { signal: sig }),
+            });
+        }
+
+        Ok(VerifyResult::Fail {
+            reason: FailureReason::new(format!("command terminated unexpectedly: {}", self.cmd))
+                .with_stdout(stdout)
+                .with_stderr(stderr)
+                .with_truncated(truncated),
+        })
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    fn context() -> VerifyContext {
+        VerifyContext {
+            stdout: String::new(),
+            stderr: None,
+            exit_code: None,
+            backend_name: "test".to_string(),
+            model: None,
+            structured: None,
+            duration: Duration::ZERO,
+        }
+    }
+
+    #[test]
+    fn run_command_builder_api() {
+        let rc = RunCommand::new("cargo")
+            .with_args(vec!["test".to_string(), "--quiet".to_string()])
+            .with_env_allowlist(&["PATH", "HOME"])
+            .with_cwd("/tmp")
+            .with_wall_timeout(Duration::from_secs(10))
+            .with_cpu_timeout(Some(Duration::from_secs(5)))
+            .with_stdout_cap(1024)
+            .with_stderr_cap(2048);
+
+        assert_eq!(rc.cmd, "cargo");
+        assert_eq!(rc.args, vec!["test", "--quiet"]);
+        assert_eq!(rc.env_allowlist, vec!["PATH", "HOME"]);
+        assert_eq!(rc.cwd, Some(PathBuf::from("/tmp")));
+        assert_eq!(rc.wall_timeout, Duration::from_secs(10));
+        assert_eq!(rc.cpu_timeout, Some(Duration::from_secs(5)));
+        assert_eq!(rc.stdout_cap, 1024);
+        assert_eq!(rc.stderr_cap, 2048);
+    }
+
+    #[test]
+    fn run_command_default_values() {
+        let rc = RunCommand::new("echo");
+        assert_eq!(rc.cmd, "echo");
+        assert!(rc.args.is_empty());
+        assert!(rc.env_allowlist.is_empty());
+        assert!(rc.cwd.is_none());
+        assert_eq!(rc.wall_timeout, DEFAULT_WALL_TIMEOUT);
+        assert_eq!(rc.cpu_timeout, None);
+        assert_eq!(rc.stdout_cap, DEFAULT_OUTPUT_CAP);
+        assert_eq!(rc.stderr_cap, DEFAULT_OUTPUT_CAP);
+    }
+
+    #[cfg(unix)]
+    #[tokio::test]
+    async fn verify_echo_passes() {
+        let rc = RunCommand::new("sh").with_args(vec!["-c".to_string(), "echo hello".to_string()]);
+        let result = rc.verify(&context()).await.unwrap();
+        assert!(matches!(result, VerifyResult::Pass));
+    }
+
+    #[cfg(unix)]
+    #[tokio::test]
+    async fn verify_false_fails_with_code() {
+        let rc = RunCommand::new("sh")
+            .with_args(vec!["-c".to_string(), "echo err >&2; exit 1".to_string()]);
+        let result = rc.verify(&context()).await.unwrap();
+        match result {
+            VerifyResult::Fail { reason } => {
+                assert_eq!(reason.exit_code, Some(1));
+                assert!(reason.stderr.contains("err"));
+                assert!(matches!(
+                    reason.sandbox_violation,
+                    Some(SandboxViolation::NonZeroExit { code: 1 })
+                ));
+            }
+            other => panic!("expected fail, got {other:?}"),
+        }
+    }
+
+    #[tokio::test]
+    async fn verify_missing_command_fails() {
+        let rc = RunCommand::new("__no_such_command__");
+        let err = rc
+            .verify(&context())
+            .await
+            .expect_err("expected verify error");
+        assert!(err.message.contains("command not found"));
+    }
+
+    #[cfg(unix)]
+    #[tokio::test]
+    async fn verify_sleeps_timeout() {
+        let rc = RunCommand::new("sleep")
+            .with_args(vec!["60".to_string()])
+            .with_wall_timeout(Duration::from_millis(100));
+        let result = rc.verify(&context()).await.unwrap();
+        match result {
+            VerifyResult::Fail { reason } => {
+                assert!(matches!(
+                    reason.sandbox_violation,
+                    Some(SandboxViolation::Timeout)
+                ));
+            }
+            other => panic!("expected fail, got {other:?}"),
+        }
+    }
+}
diff --git a/src/strategy/verify.rs b/src/strategy/verify/verify.rs
similarity index 77%
rename from src/strategy/verify.rs
rename to src/strategy/verify/verify.rs
index 6295abf..e32992d 100644
--- a/src/strategy/verify.rs
+++ b/src/strategy/verify/verify.rs
@@ -1,4 +1,9 @@
-//! Verification hook trait used by ladder strategies (`EscalatingRetry`).
+//! Core verification types and trait.
+//!
+//! This module defines the `VerifyHook` trait and supporting types
+//! (`VerifyResult`, `FailureReason`, `VerifyError`, `VerifyContext`).
+//! Concrete implementations live in sibling modules (`llm_verifier`,
+//! `run_command`).
 //!
 //! v0 hooks only need `Pass` and `Fail`. `Repair` and `Score` are reserved so
 //! later hook implementations can evolve without changing the public enum.
@@ -11,14 +16,25 @@
 //! reason into the next prompt. Hook implementations that log or persist
 //! `FailureReason` fields directly must apply their own redaction.
 
-use std::collections::HashMap;
-use std::path::Path;
-use std::sync::Arc;
+/// Sandbox-level signal captured from command execution failures.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub enum SandboxViolation {
+    /// Wall-clock timeout expired while the command was running.
+    Timeout,
+    /// Process exited due to an OS signal (Unix-only).
+    Signal { signal: i32 },
+    /// Non-zero process exit code.
+    NonZeroExit { code: i32 },
+}
+
+// ── FailureReason ────────────────────────────────────────────
+
 use std::time::Duration;
 
 use async_trait::async_trait;
 
-use crate::backend::{Backend, QueryOutput};
+use crate::backend::QueryOutput;
+
 // ── FailureReason ────────────────────────────────────────────
 
 /// Structured reason a verification hook returned `Fail`.
@@ -35,6 +51,7 @@ use crate::backend::{Backend, QueryOutput};
 /// path runs `redact_secrets()` on the reason before flowing it into
 /// the next prompt. Hook implementations that log or persist
 /// `FailureReason` fields directly must apply their own redaction.
+#[non_exhaustive]
 #[derive(Debug, Clone, PartialEq)]
 pub struct FailureReason {
     /// Human-readable summary (e.g. "test `it_adds` failed").
@@ -50,6 +67,8 @@ pub struct FailureReason {
     /// Exit code if the verifier ran as a process. `None` for in‑process
     /// verifiers (e.g. `LLMVerifier`).
     pub exit_code: Option<i32>,
+    /// Optional sandbox signal that further explains process termination.
+    pub sandbox_violation: Option<SandboxViolation>,
 }
 
 impl FailureReason {
@@ -62,6 +81,7 @@ impl FailureReason {
             stderr: String::new(),
             truncated: false,
             exit_code: None,
+            sandbox_violation: None,
         }
     }
 
@@ -88,6 +108,12 @@ impl FailureReason {
         self.exit_code = Some(exit_code);
         self
     }
+
+    /// Attach sandbox metadata (builder-pattern).
+    pub fn with_sandbox_violation(mut self, sandbox_violation: SandboxViolation) -> Self {
+        self.sandbox_violation = Some(sandbox_violation);
+        self
+    }
 }
 
 impl std::fmt::Display for FailureReason {
@@ -271,135 +297,6 @@ pub trait VerifyHook: Send + Sync {
     async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError>;
 }
 
-// ── LLMVerifier ────────────────────────────────────────────
-
-/// Concrete verify hook that delegates to a backend and parses a
-/// deterministic yes/no verdict from the backend response.
-pub struct LLMVerifier {
-    /// Identifier used for observability/debugging.
-    pub backend: String,
-    backend_client: Arc<dyn Backend>,
-    /// Optional model override passed to the backend.
-    pub model: Option<String>,
-    /// Prompt template used for verification. `{candidate}` is replaced with
-    /// the candidate text under test; any `{key}` present in `params`
-    /// is also substituted.
-    pub prompt_template: String,
-    /// Optional system-level context prepended to the candidate prompt.
-    pub system_prompt: Option<String>,
-    /// Temperature hint used when available. `0.0` is deterministic default.
-    pub temperature: f32,
-    params: HashMap<String, String>,
-}
-
-impl LLMVerifier {
-    pub const DEFAULT_TEMPERATURE: f32 = 0.0;
-
-    /// Construct a verifier bound to a backend object.
-    pub fn new(
-        backend: impl Into<String>,
-        backend_client: Arc<dyn Backend>,
-        prompt_template: impl Into<String>,
-    ) -> Self {
-        Self {
-            backend: backend.into(),
-            backend_client,
-            model: None,
-            prompt_template: prompt_template.into(),
-            system_prompt: None,
-            temperature: Self::DEFAULT_TEMPERATURE,
-            params: HashMap::new(),
-        }
-    }
-
-    /// Set deterministic temperature hint (used where backend support exists).
-    pub fn with_temperature(mut self, temperature: f32) -> Self {
-        self.temperature = temperature;
-        self
-    }
-
-    /// Set a model override.
-    pub fn with_model(mut self, model: impl Into<String>) -> Self {
-        self.model = Some(model.into());
-        self
-    }
-
-    /// Set system prompt prepended to candidate prompt.
-    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
-        self.system_prompt = Some(prompt.into());
-        self
-    }
-
-    /// Add one user-supplied template parameter.
-    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
-        self.params.insert(key.into(), value.into());
-        self
-    }
-
-    fn rendered_prompt(&self, candidate: &str) -> String {
-        let mut prompt = self.prompt_template.clone();
-
-        // Sort params by key length descending so that longer, more specific
-        // keys (e.g. {env_name}) are replaced before shorter substrings (e.g. {env}).
-        // HashMap iteration order is non-deterministic, so we must sort explicitly
-        // for reproducible prompt rendering.
-        let mut sorted_params: Vec<(&String, &String)> = self.params.iter().collect();
-        sorted_params.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
-        for (key, value) in sorted_params {
-            let needle = format!("{{{key}}}");
-            prompt = prompt.replace(&needle, value);
-        }
-
-        let prompt = prompt.replace("{candidate}", candidate);
-
-        match &self.system_prompt {
-            Some(system) => format!("{system}\n\n{prompt}"),
-            None => prompt,
-        }
-    }
-
-    fn parse_response(raw: &str) -> VerifyResult {
-        let first = raw
-            .split_whitespace()
-            .next()
-            .map(|token| token.trim().trim_matches(|c: char| !c.is_alphanumeric()))
-            .map(|token| token.to_ascii_lowercase())
-            .unwrap_or_default();
-
-        if first == "yes" {
-            return VerifyResult::pass();
-        }
-
-        if first == "no" {
-            return VerifyResult::fail_with(FailureReason::new("no").with_stdout(raw.to_string()));
-        }
-
-        VerifyResult::fail_with(
-            FailureReason::new("unparseable verifier response").with_stdout(raw.to_string()),
-        )
-    }
-}
-
-#[async_trait]
-impl VerifyHook for LLMVerifier {
-    fn name(&self) -> &str {
-        "LLMVerifier"
-    }
-
-    async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
-        let prompt = self.rendered_prompt(&ctx.stdout);
-
-        match self
-            .backend_client
-            .query(&prompt, Path::new("."), self.model.as_deref())
-            .await
-        {
-            Ok(query) => Ok(Self::parse_response(&query.stdout)),
-            Err(err) => Err(VerifyError::new(format!("backend error: {err}"))),
-        }
-    }
-}
-
 // ── Tests ────────────────────────────────────────────────────
 
 #[cfg(test)]
diff --git a/src/utils.rs b/src/utils.rs
index b196636..afd5e59 100644
--- a/src/utils.rs
+++ b/src/utils.rs
@@ -2,6 +2,9 @@
 
 use colored::Colorize;
 use std::path::{Path, PathBuf};
+use std::sync::LazyLock;
+
+use regex::Regex;
 
 use crate::backend::BackendError;
 
@@ -195,10 +198,64 @@ pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
         .unwrap_or("")
 }
 
+// ── Secret redaction ─────────────────────────────────────
+
+static AWS_KEY_RE: LazyLock<Regex> =
+    LazyLock::new(|| Regex::new(r"AKIA[0-9A-Z]{16}").expect("valid regex"));
+
+static KEY_VALUE_RE: LazyLock<Regex> = LazyLock::new(|| {
+    Regex::new(r#"(?i)((?:api[_-]?key|secret|token|password)\s*[=:]\s*)[^\s'\"]+"#)
+        .expect("valid regex")
+});
+
+static BEARER_RE: LazyLock<Regex> =
+    LazyLock::new(|| Regex::new(r"(?i)Bearer\s+[A-Za-z0-9._\-~+/=]+").expect("valid regex"));
+
+static SECRET_HEURISTIC_RE: LazyLock<Regex> = LazyLock::new(|| {
+    Regex::new(r"(?i)\b(key|secret|token)[\s:=]+([A-Za-z0-9+/=_\-]{32,})").expect("valid regex")
+});
+
+/// Redact common secret-like tokens from free text.
+pub fn redact_secrets(input: &str) -> String {
+    let mut result = AWS_KEY_RE.replace_all(input, "[REDACTED]").into_owned();
+    result = KEY_VALUE_RE
+        .replace_all(&result, "${1}[REDACTED]")
+        .into_owned();
+    result = BEARER_RE.replace_all(&result, "[REDACTED]").into_owned();
+    result = SECRET_HEURISTIC_RE
+        .replace_all(&result, "$1 [REDACTED]")
+        .into_owned();
+    result
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;
 
+    #[test]
+    fn test_redact_secrets_aws_key() {
+        assert_eq!(
+            redact_secrets("key: AKIA0123456789ABCDEF rest"),
+            "key: [REDACTED] rest"
+        );
+    }
+
+    #[test]
+    fn test_redact_secrets_api_key_value() {
+        assert_eq!(
+            redact_secrets("api_key=AKIA0123456789ABCDEF other"),
+            "api_key=[REDACTED] other"
+        );
+    }
+
+    #[test]
+    fn test_redact_secrets_bearer_token() {
+        assert_eq!(
+            redact_secrets("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9"),
+            "Authorization: [REDACTED]"
+        );
+    }
+
     #[test]
     fn test_truncate_short_string() {
         assert_eq!(truncate("hello", 10), "hello");
diff --git a/tests/verify_run_command.rs b/tests/verify_run_command.rs
new file mode 100644
index 0000000..6b47303
--- /dev/null
+++ b/tests/verify_run_command.rs
@@ -0,0 +1,176 @@
+#![cfg(unix)]
+
+use std::env;
+use std::process::Command;
+use std::time::Duration;
+
+use loker::backend::QueryOutput;
+use loker::strategy::verify::{RunCommand, SandboxViolation};
+use loker::strategy::VerifyContext;
+use loker::strategy::{VerifyHook, VerifyResult};
+
+fn ctx() -> VerifyContext {
+    VerifyContext::from_query_output(&QueryOutput::from_text(
+        String::new(),
+        "run_command_integration".to_string(),
+        Duration::ZERO,
+    ))
+}
+
+fn has_process_containing(fragment: &str) -> bool {
+    let output = Command::new("ps")
+        .args(["-axo", "command="])
+        .output()
+        .expect("ps should run")
+        .stdout;
+    let ps = String::from_utf8_lossy(&output);
+    ps.lines().any(|line| line.contains(fragment))
+}
+
+#[tokio::test]
+async fn runcommand_echo_passes() {
+    let rc = RunCommand::new("sh").with_args(vec!["-c".to_string(), "echo hello".to_string()]);
+
+    let result = rc.verify(&ctx()).await.unwrap();
+    assert!(matches!(result, VerifyResult::Pass));
+}
+
+#[tokio::test]
+async fn runcommand_false_fails_with_exit_code() {
+    let rc = RunCommand::new("sh")
+        .with_args(vec!["-c".to_string(), "echo boom >&2; exit 1".to_string()]);
+
+    let result = rc.verify(&ctx()).await.unwrap();
+    match result {
+        VerifyResult::Fail { reason } => {
+            assert_eq!(reason.exit_code, Some(1));
+            assert!(matches!(
+                reason.sandbox_violation,
+                Some(SandboxViolation::NonZeroExit { code: 1 })
+            ));
+        }
+        other => panic!("expected fail, got {other:?}"),
+    }
+}
+
+#[tokio::test]
+async fn runcommand_sleeps_timeout() {
+    let rc = RunCommand::new("sleep")
+        .with_args(vec!["60".to_string()])
+        .with_wall_timeout(Duration::from_millis(120));
+
+    let result = rc.verify(&ctx()).await.unwrap();
+    match result {
+        VerifyResult::Fail { reason } => {
+            assert!(matches!(
+                reason.sandbox_violation,
+                Some(SandboxViolation::Timeout)
+            ));
+        }
+        other => panic!("expected timeout fail, got {other:?}"),
+    }
+}
+
+#[tokio::test]
+async fn runcommand_process_group_killed_on_timeout() {
+    let marker = "sleep 777";
+
+    let rc = RunCommand::new("sh")
+        .with_args(vec!["-c".to_string(), "sleep 777".to_string()])
+        .with_wall_timeout(Duration::from_millis(120));
+
+    let result = rc.verify(&ctx()).await;
+    assert!(matches!(result, Ok(VerifyResult::Fail { .. })));
+
+    let has_orphan = has_process_containing(marker);
+    assert!(
+        !has_orphan,
+        "orphaned timeout process still running: {marker}"
+    );
+}
+
+#[tokio::test]
+async fn runcommand_output_truncation_markers() {
+    let rc = RunCommand::new("sh")
+        .with_args(vec![
+            "-c".to_string(),
+            "printf 'abcdefgh'; printf 'ijklmnop' 1>&2; exit 1".to_string(),
+        ])
+        .with_stdout_cap(4)
+        .with_stderr_cap(4);
+
+    let result = rc.verify(&ctx()).await.unwrap();
+    match result {
+        VerifyResult::Fail { reason } => {
+            assert!(reason.truncated);
+            assert!(reason.stdout.contains("…[truncated"));
+            assert!(reason.stderr.contains("…[truncated"));
+        }
+        other => panic!("expected fail, got {other:?}"),
+    }
+}
+
+#[tokio::test]
+async fn runcommand_env_allowlist() {
+    let rc = RunCommand::new("sh")
+        .with_args(vec!["-c".to_string(), "printenv; exit 1".to_string()])
+        .with_env_allowlist(&["USER", "HOME"]);
+
+    let result = rc.verify(&ctx()).await.unwrap();
+    match result {
+        VerifyResult::Fail { reason } => {
+            assert!(reason.stdout.contains("USER="));
+            assert!(reason.stdout.contains("HOME="));
+            assert!(!reason.stdout.contains("PATH="));
+        }
+        other => panic!("expected fail, got {other:?}"),
+    }
+}
+
+#[tokio::test]
+async fn runcommand_secret_redaction_in_output() {
+    let key = "CLO271_SECRET_TOKEN";
+    let value = "AKIAABCDEFGHIJKLMN1234";
+    let original = env::var_os(key);
+    env::set_var(key, value);
+
+    let rc = RunCommand::new("sh")
+        .with_args(vec![
+            "-c".to_string(),
+            "echo ${CLO271_SECRET_TOKEN}; exit 1".to_string(),
+        ])
+        .with_env_allowlist(&[key]);
+
+    let result = rc.verify(&ctx()).await.unwrap();
+    match result {
+        VerifyResult::Fail { reason } => {
+            assert!(reason.stdout.contains("[REDACTED]"));
+            assert!(!reason.stdout.contains(value));
+        }
+        other => panic!("expected fail, got {other:?}"),
+    }
+
+    match original {
+        Some(v) => env::set_var(key, v),
+        None => env::remove_var(key),
+    }
+}
+
+#[tokio::test]
+async fn runcommand_cpu_limit_forced_signal() {
+    let rc = RunCommand::new("sh")
+        .with_args(vec!["-c".to_string(), "while :; do :; done".to_string()])
+        .with_cpu_timeout(Some(Duration::from_secs(1)))
+        .with_wall_timeout(Duration::from_secs(4));
+
+    let result = rc.verify(&ctx()).await.unwrap();
+    match result {
+        VerifyResult::Fail { reason } => {
+            assert!(matches!(
+                reason.sandbox_violation,
+                Some(SandboxViolation::Signal { .. })
+            ));
+        }
+        other => panic!("expected fail, got {other:?}"),
+    }
+}

exec
/bin/zsh -lc 'git diff 30fe7a742785a98cb2b1894b6a4dbc6c616d6c49 -- src/strategy/verify/run_command.rs src/strategy/verify/mod.rs src/strategy/verify/llm_verifier.rs src/strategy/escalating_retry.rs src/strategy/mod.rs src/main.rs Cargo.toml src/utils.rs' in /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
 succeeded in 0ms:
diff --git a/src/main.rs b/src/main.rs
index 54b5151..b15a609 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,5 @@
+#![allow(dead_code)]
+
 mod apply_verify;
 mod backend;
 mod cache;
diff --git a/src/strategy/escalating_retry.rs b/src/strategy/escalating_retry.rs
index 13b1995..d6e29b1 100644
--- a/src/strategy/escalating_retry.rs
+++ b/src/strategy/escalating_retry.rs
@@ -19,9 +19,8 @@ use crate::strategy::{
     VerifyOutcome, VerifyResult, SCHEMA_VERSION,
 };
 use async_trait::async_trait;
-use regex::Regex;
 use serde::{Deserialize, Serialize};
-use std::sync::{Arc, LazyLock};
+use std::sync::Arc;
 
 /// 4 KiB excerpt fits inside an 8 KiB envelope while leaving headroom for
 /// verifier reason and backend error class.
@@ -133,25 +132,6 @@ fn backend_error_class(err: &BackendError) -> String {
     .to_string()
 }
 
-/// AWS access keys.
-static AWS_KEY_RE: LazyLock<Regex> =
-    LazyLock::new(|| Regex::new(r"AKIA[0-9A-Z]{16}").expect("valid regex"));
-
-/// Generic `key=value` shapes (case-insensitive; redacts only the value side).
-static KEY_VALUE_RE: LazyLock<Regex> = LazyLock::new(|| {
-    Regex::new(r#"(?i)((?:api[_-]?key|secret|token|password)\s*[=:]\s*)[^\s'\"]+"#)
-        .expect("valid regex")
-});
-
-/// Bearer tokens in `Authorization` headers.
-static BEARER_RE: LazyLock<Regex> =
-    LazyLock::new(|| Regex::new(r"(?i)Bearer\s+[A-Za-z0-9._\-~+/=]+").expect("valid regex"));
-
-/// Heuristic: long base64-ish blob preceded by key/secret/token.
-static SECRET_HEURISTIC_RE: LazyLock<Regex> = LazyLock::new(|| {
-    Regex::new(r"(?i)\b(key|secret|token)[\s:=]+([A-Za-z0-9+/=_\-]{32,})").expect("valid regex")
-});
-
 /// Redact common secret shapes from text before they reach the next rung's
 /// prompt envelope. Applied to *every* byte of `FailureContext` text
 /// (verify_reason, response_excerpt, and the final assembled header).
@@ -160,15 +140,7 @@ static SECRET_HEURISTIC_RE: LazyLock<Regex> = LazyLock::new(|| {
 /// future centralised secret-scrubbing service should absorb this function
 /// rather than invent a second one.
 pub(crate) fn redact_secrets(input: &str) -> String {
-    let mut result = AWS_KEY_RE.replace_all(input, "[REDACTED]").into_owned();
-    result = KEY_VALUE_RE
-        .replace_all(&result, "${1}[REDACTED]")
-        .into_owned();
-    result = BEARER_RE.replace_all(&result, "[REDACTED]").into_owned();
-    result = SECRET_HEURISTIC_RE
-        .replace_all(&result, "$1 [REDACTED]")
-        .into_owned();
-    result
+    crate::utils::redact_secrets(input)
 }
 
 /// Truncate `s` to at most `max_bytes` total (including the suffix),
diff --git a/src/strategy/mod.rs b/src/strategy/mod.rs
index 7ceac7d..0319576 100644
--- a/src/strategy/mod.rs
+++ b/src/strategy/mod.rs
@@ -31,7 +31,8 @@ pub use escalating_retry::EscalatingRetry;
 pub use parallel_fanout::{ParallelFanOut, TargetSpec};
 pub use single_model::SingleModel;
 pub use verify::{
-    FailureReason, LLMVerifier, VerifyContext, VerifyError, VerifyHook, VerifyResult,
+    FailureReason, LLMVerifier, RunCommand, SandboxViolation, VerifyContext, VerifyError,
+    VerifyHook, VerifyResult,
 };
 
 /// `schema_version` value emitted by every `StrategyOutput`. Pinned to the
diff --git a/src/strategy/verify/llm_verifier.rs b/src/strategy/verify/llm_verifier.rs
new file mode 100644
index 0000000..33cb908
--- /dev/null
+++ b/src/strategy/verify/llm_verifier.rs
@@ -0,0 +1,142 @@
+//! LLM-based verify hook.
+//!
+//! Delegates the verification decision to a backend (LLM) and parses a
+//! deterministic yes/no verdict from the response.
+
+use std::collections::HashMap;
+use std::path::Path;
+use std::sync::Arc;
+
+use async_trait::async_trait;
+
+use crate::backend::Backend;
+use crate::strategy::verify::{
+    FailureReason, VerifyContext, VerifyError, VerifyHook, VerifyResult,
+};
+
+/// Concrete verify hook that delegates to a backend and parses a
+/// deterministic yes/no verdict from the backend response.
+pub struct LLMVerifier {
+    /// Identifier used for observability/debugging.
+    pub backend: String,
+    backend_client: Arc<dyn Backend>,
+    /// Optional model override passed to the backend.
+    pub model: Option<String>,
+    /// Prompt template used for verification. `{candidate}` is replaced with
+    /// the candidate text under test; any `{key}` present in `params`
+    /// is also substituted.
+    pub prompt_template: String,
+    /// Optional system-level context prepended to the candidate prompt.
+    pub system_prompt: Option<String>,
+    /// Temperature hint used when available. `0.0` is deterministic default.
+    pub temperature: f32,
+    params: HashMap<String, String>,
+}
+
+impl LLMVerifier {
+    pub const DEFAULT_TEMPERATURE: f32 = 0.0;
+
+    /// Construct a verifier bound to a backend object.
+    pub fn new(
+        backend: impl Into<String>,
+        backend_client: Arc<dyn Backend>,
+        prompt_template: impl Into<String>,
+    ) -> Self {
+        Self {
+            backend: backend.into(),
+            backend_client,
+            model: None,
+            prompt_template: prompt_template.into(),
+            system_prompt: None,
+            temperature: Self::DEFAULT_TEMPERATURE,
+            params: HashMap::new(),
+        }
+    }
+
+    /// Set deterministic temperature hint (used where backend support exists).
+    pub fn with_temperature(mut self, temperature: f32) -> Self {
+        self.temperature = temperature;
+        self
+    }
+
+    /// Set a model override.
+    pub fn with_model(mut self, model: impl Into<String>) -> Self {
+        self.model = Some(model.into());
+        self
+    }
+
+    /// Set system prompt prepended to candidate prompt.
+    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
+        self.system_prompt = Some(prompt.into());
+        self
+    }
+
+    /// Add one user-supplied template parameter.
+    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
+        self.params.insert(key.into(), value.into());
+        self
+    }
+
+    fn rendered_prompt(&self, candidate: &str) -> String {
+        let mut prompt = self.prompt_template.clone();
+
+        // Sort params by key length descending so that longer, more specific
+        // keys (e.g. {env_name}) are replaced before shorter substrings (e.g. {env}).
+        // HashMap iteration order is non-deterministic, so we must sort explicitly
+        // for reproducible prompt rendering.
+        let mut sorted_params: Vec<(&String, &String)> = self.params.iter().collect();
+        sorted_params.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
+        for (key, value) in sorted_params {
+            let needle = format!("{{{key}}}");
+            prompt = prompt.replace(&needle, value);
+        }
+
+        let prompt = prompt.replace("{candidate}", candidate);
+
+        match &self.system_prompt {
+            Some(system) => format!("{system}\n\n{prompt}"),
+            None => prompt,
+        }
+    }
+
+    fn parse_response(raw: &str) -> VerifyResult {
+        let first = raw
+            .split_whitespace()
+            .next()
+            .map(|token| token.trim().trim_matches(|c: char| !c.is_alphanumeric()))
+            .map(|token| token.to_ascii_lowercase())
+            .unwrap_or_default();
+
+        if first == "yes" {
+            return VerifyResult::pass();
+        }
+
+        if first == "no" {
+            return VerifyResult::fail_with(FailureReason::new("no").with_stdout(raw.to_string()));
+        }
+
+        VerifyResult::fail_with(
+            FailureReason::new("unparseable verifier response").with_stdout(raw.to_string()),
+        )
+    }
+}
+
+#[async_trait]
+impl VerifyHook for LLMVerifier {
+    fn name(&self) -> &str {
+        "LLMVerifier"
+    }
+
+    async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
+        let prompt = self.rendered_prompt(&ctx.stdout);
+
+        match self
+            .backend_client
+            .query(&prompt, Path::new("."), self.model.as_deref())
+            .await
+        {
+            Ok(query) => Ok(Self::parse_response(&query.stdout)),
+            Err(err) => Err(VerifyError::new(format!("backend error: {err}"))),
+        }
+    }
+}
diff --git a/src/strategy/verify/mod.rs b/src/strategy/verify/mod.rs
new file mode 100644
index 0000000..5ba44a5
--- /dev/null
+++ b/src/strategy/verify/mod.rs
@@ -0,0 +1,22 @@
+#![allow(clippy::module_inception)]
+
+//! Verification hooks for strategy gating.
+//!
+//! Each hook implements [`VerifyHook`] and returns a [`VerifyResult`]
+//! (Pass / Fail / reserved variants). Concrete implementations:
+//!
+//! - [`LLMVerifier`] — delegates to an LLM backend and parses yes/no.
+//! - [`RunCommand`] — shells out and maps exit status (CLO-271).
+
+pub mod llm_verifier;
+pub mod run_command;
+pub mod verify;
+
+// Re-export the core types so `strategy::verify::FailureReason` etc. work.
+pub use verify::{
+    FailureReason, SandboxViolation, VerifyContext, VerifyError, VerifyHook, VerifyResult,
+};
+
+// Re-export concrete implementations.
+pub use llm_verifier::LLMVerifier;
+pub use run_command::RunCommand;
diff --git a/src/strategy/verify/run_command.rs b/src/strategy/verify/run_command.rs
new file mode 100644
index 0000000..d7edc4c
--- /dev/null
+++ b/src/strategy/verify/run_command.rs
@@ -0,0 +1,529 @@
+#![allow(unsafe_code)]
+
+//! Shell-command verify hook with sandboxing.
+//!
+//! Spawns a subprocess, captures stdout/stderr with byte caps, and maps
+//! exit status to [`VerifyResult`].
+
+use std::path::{Path, PathBuf};
+use std::process::Stdio;
+use std::time::{Duration, Instant};
+
+use async_trait::async_trait;
+use tokio::io::{AsyncRead, AsyncReadExt};
+use tokio::process::Command;
+use tokio::time::timeout;
+use which::which;
+
+use crate::strategy::verify::{
+    FailureReason, SandboxViolation, VerifyContext, VerifyError, VerifyHook, VerifyResult,
+};
+use crate::utils::redact_secrets;
+
+/// Default wall-clock timeout for compile/test gates.
+const DEFAULT_WALL_TIMEOUT: Duration = Duration::from_secs(30);
+
+/// Default byte cap for stdout and stderr capture.
+const DEFAULT_OUTPUT_CAP: usize = 4096;
+
+/// Shell-command verify hook.
+#[derive(Debug, Clone)]
+pub struct RunCommand {
+    /// Command to execute (bare name resolved via `which`, absolute path used as-is).
+    pub cmd: String,
+    /// Arguments passed to the command.
+    pub args: Vec<String>,
+    /// Environment variable names allowed in child process.
+    /// Default: empty (default-deny).
+    pub env_allowlist: Vec<String>,
+    /// Working directory for the command.
+    pub cwd: Option<PathBuf>,
+    /// Wall-clock timeout before process-group SIGKILL.
+    pub wall_timeout: Duration,
+    /// Optional CPU-time timeout (Unix only).
+    pub cpu_timeout: Option<Duration>,
+    /// Max bytes captured from stdout.
+    pub stdout_cap: usize,
+    /// Max bytes captured from stderr.
+    pub stderr_cap: usize,
+}
+
+impl Default for RunCommand {
+    fn default() -> Self {
+        Self {
+            cmd: String::new(),
+            args: Vec::new(),
+            env_allowlist: Vec::new(),
+            cwd: None,
+            wall_timeout: DEFAULT_WALL_TIMEOUT,
+            cpu_timeout: None,
+            stdout_cap: DEFAULT_OUTPUT_CAP,
+            stderr_cap: DEFAULT_OUTPUT_CAP,
+        }
+    }
+}
+
+impl RunCommand {
+    /// Construct with a command name/path.
+    pub fn new(cmd: impl Into<String>) -> Self {
+        Self {
+            cmd: cmd.into(),
+            ..Self::default()
+        }
+    }
+
+    /// Replace command-line arguments.
+    pub fn with_args(mut self, args: impl Into<Vec<String>>) -> Self {
+        self.args = args.into();
+        self
+    }
+
+    /// Replace env allow-list.
+    pub fn with_env_allowlist(mut self, vars: &[&str]) -> Self {
+        self.env_allowlist = vars.iter().map(|s| s.to_string()).collect();
+        self
+    }
+
+    /// Set working directory.
+    pub fn with_cwd(mut self, path: impl Into<PathBuf>) -> Self {
+        self.cwd = Some(path.into());
+        self
+    }
+
+    /// Override wall-clock timeout.
+    pub fn with_wall_timeout(mut self, timeout: Duration) -> Self {
+        self.wall_timeout = timeout;
+        self
+    }
+
+    /// Override CPU timeout.
+    pub fn with_cpu_timeout(mut self, timeout: Option<Duration>) -> Self {
+        self.cpu_timeout = timeout;
+        self
+    }
+
+    /// Override stdout capture cap.
+    pub fn with_stdout_cap(mut self, cap: usize) -> Self {
+        self.stdout_cap = cap;
+        self
+    }
+
+    /// Override stderr capture cap.
+    pub fn with_stderr_cap(mut self, cap: usize) -> Self {
+        self.stderr_cap = cap;
+        self
+    }
+
+    fn resolve_command(&self) -> Result<PathBuf, VerifyError> {
+        if self.cmd.is_empty() {
+            return Err(VerifyError::new("no command configured"));
+        }
+
+        let path = Path::new(&self.cmd);
+        if path.is_absolute() {
+            Ok(PathBuf::from(path))
+        } else {
+            which(&self.cmd)
+                .map_err(|_| VerifyError::new(format!("command not found: {}", self.cmd)))
+        }
+    }
+
+    fn build_environment(&self) -> Vec<(String, String)> {
+        self.env_allowlist
+            .iter()
+            .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
+            .map(|(name, value)| {
+                let redacted = if is_secret_like_env_key(&name) {
+                    redact_secrets(&value)
+                } else {
+                    value
+                };
+                (name, redacted)
+            })
+            .collect()
+    }
+
+    async fn run(&self) -> Result<CommandRun, VerifyError> {
+        let command_path = self.resolve_command()?;
+
+        let mut command = Command::new(&command_path);
+        command
+            .args(&self.args)
+            .env_clear()
+            .stdin(Stdio::null())
+            .stdout(Stdio::piped())
+            .stderr(Stdio::piped());
+
+        if let Some(cwd) = &self.cwd {
+            command.current_dir(cwd);
+        }
+
+        for (key, value) in self.build_environment() {
+            command.env(key, value);
+        }
+
+        // Put each child in its own process group, so SIGKILL on timeout
+        // reaps descendants as well.
+        #[cfg(unix)]
+        command.process_group(0);
+
+        #[cfg(unix)]
+        if let Some(cpu_timeout) = self.cpu_timeout {
+            let secs = cpu_timeout.as_secs().max(1);
+            // SAFETY: this closure runs in the child before exec and only
+            // mutates process-local resource limits.
+            unsafe {
+                command.pre_exec(move || {
+                    let limit = libc::rlimit {
+                        rlim_cur: secs as libc::rlim_t,
+                        rlim_max: secs as libc::rlim_t,
+                    };
+                    let rc = libc::setrlimit(libc::RLIMIT_CPU, &limit);
+                    if rc != 0 {
+                        return Err(std::io::Error::last_os_error());
+                    }
+                    Ok(())
+                });
+            }
+        }
+
+        let start = Instant::now();
+        let mut child = command.spawn().map_err(|err| {
+            VerifyError::new(format!("failed to spawn command '{}': {err}", self.cmd))
+        })?;
+
+        let stdout_stream = child
+            .stdout
+            .take()
+            .ok_or_else(|| VerifyError::new("failed to capture child stdout pipe".to_string()))?;
+        let stderr_stream = child
+            .stderr
+            .take()
+            .ok_or_else(|| VerifyError::new("failed to capture child stderr pipe".to_string()))?;
+
+        let stdout_handle = tokio::spawn(read_stream_bounded(stdout_stream, self.stdout_cap));
+        let stderr_handle = tokio::spawn(read_stream_bounded(stderr_stream, self.stderr_cap));
+
+        let (status, timed_out) = match timeout(self.wall_timeout, child.wait()).await {
+            Ok(result) => {
+                let status = result.map_err(|err| {
+                    VerifyError::new(format!("failed to wait for command '{}': {err}", self.cmd))
+                })?;
+                (status, false)
+            }
+            Err(_) => {
+                kill_process_group(child.id());
+                let status = child.wait().await.map_err(|err| {
+                    VerifyError::new(format!(
+                        "failed to reap timed-out command '{}': {err}",
+                        self.cmd
+                    ))
+                })?;
+                (status, true)
+            }
+        };
+
+        let stdout = stdout_handle
+            .await
+            .map_err(|_| VerifyError::new("stdout reader task failed"))?;
+        let stderr = stderr_handle
+            .await
+            .map_err(|_| VerifyError::new("stderr reader task failed"))?;
+
+        Ok(CommandRun {
+            status,
+            timed_out,
+            stdout,
+            stderr,
+            elapsed_ms: start.elapsed().as_millis() as u64,
+        })
+    }
+}
+
+#[derive(Debug)]
+struct CommandRun {
+    status: std::process::ExitStatus,
+    timed_out: bool,
+    stdout: CapturedOutput,
+    stderr: CapturedOutput,
+    elapsed_ms: u64,
+}
+
+#[derive(Debug)]
+struct CapturedOutput {
+    data: Vec<u8>,
+    truncated: bool,
+    elided_bytes: usize,
+}
+
+impl CapturedOutput {
+    fn to_reason_text(&self) -> String {
+        let mut text = String::from_utf8_lossy(&self.data).into_owned();
+        if self.truncated {
+            text.push_str(&format!(
+                " …[truncated, {} bytes elided]",
+                self.elided_bytes
+            ));
+        }
+        redact_secrets(&text)
+    }
+}
+
+async fn read_stream_bounded<R: AsyncRead + Unpin + Send + 'static>(
+    mut reader: R,
+    max_bytes: usize,
+) -> CapturedOutput {
+    let mut buf = [0u8; 8192];
+    let mut data = Vec::new();
+    let mut truncated = false;
+    let mut elided_bytes = 0usize;
+
+    loop {
+        match reader.read(&mut buf).await {
+            Ok(0) => break,
+            Ok(n) => {
+                let remaining = max_bytes.saturating_sub(data.len());
+                if remaining == 0 {
+                    truncated = true;
+                    elided_bytes += n;
+                    elided_bytes += drain_stream(&mut reader).await;
+                    break;
+                }
+
+                let take = n.min(remaining);
+                data.extend_from_slice(&buf[..take]);
+
+                if take < n {
+                    truncated = true;
+                    elided_bytes += n - take;
+                    elided_bytes += drain_stream(&mut reader).await;
+                    break;
+                }
+            }
+            Err(_) => break,
+        }
+    }
+
+    CapturedOutput {
+        data,
+        truncated,
+        elided_bytes,
+    }
+}
+
+async fn drain_stream<R: AsyncRead + Unpin + Send>(reader: &mut R) -> usize {
+    let mut buf = [0u8; 8192];
+    let mut total = 0usize;
+
+    loop {
+        match reader.read(&mut buf).await {
+            Ok(0) | Err(_) => break,
+            Ok(n) => {
+                total += n;
+            }
+        }
+    }
+
+    total
+}
+
+fn is_secret_like_env_key(key: &str) -> bool {
+    let key = key.to_ascii_uppercase();
+    key.contains("SECRET")
+        || key.contains("TOKEN")
+        || key.contains("PASSWORD")
+        || key.contains("API_KEY")
+        || key.contains("AUTH")
+}
+
+#[cfg(unix)]
+fn status_signal(status: &std::process::ExitStatus) -> Option<i32> {
+    use std::os::unix::process::ExitStatusExt;
+    status.signal()
+}
+
+#[cfg(not(unix))]
+fn status_signal(_status: &std::process::ExitStatus) -> Option<i32> {
+    None
+}
+
+#[cfg(unix)]
+fn kill_process_group(pid: Option<u32>) {
+    if let Some(pid) = pid {
+        // SAFETY: this sends SIGKILL to a process group spawned by us and avoids
+        // orphaned children. If the pid is stale or already dead, this is a
+        // best-effort cleanup attempt.
+        unsafe {
+            libc::kill(-(pid as i32), libc::SIGKILL);
+        }
+    }
+}
+
+#[cfg(not(unix))]
+fn kill_process_group(pid: Option<u32>) {
+    let _ = pid;
+}
+
+#[async_trait]
+impl VerifyHook for RunCommand {
+    fn name(&self) -> &str {
+        "RunCommand"
+    }
+
+    async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
+        let run = self.run().await?;
+
+        let stdout = redact_secrets(&run.stdout.to_reason_text());
+        let stderr = redact_secrets(&run.stderr.to_reason_text());
+        let truncated = run.stdout.truncated || run.stderr.truncated;
+        let signal = status_signal(&run.status);
+
+        if run.timed_out {
+            return Ok(VerifyResult::Fail {
+                reason: FailureReason::new(format!("command timed out: {}", self.cmd))
+                    .with_stdout(stdout)
+                    .with_stderr(stderr)
+                    .with_truncated(truncated)
+                    .with_sandbox_violation(SandboxViolation::Timeout),
+            });
+        }
+
+        if let Some(code) = run.status.code() {
+            if code == 0 {
+                return Ok(VerifyResult::Pass);
+            }
+
+            return Ok(VerifyResult::Fail {
+                reason: FailureReason::new(format!("command exited with status {code}"))
+                    .with_exit_code(code)
+                    .with_stdout(stdout)
+                    .with_stderr(stderr)
+                    .with_truncated(truncated)
+                    .with_sandbox_violation(SandboxViolation::NonZeroExit { code }),
+            });
+        }
+
+        if let Some(sig) = signal {
+            return Ok(VerifyResult::Fail {
+                reason: FailureReason::new(format!("command killed by signal {sig}"))
+                    .with_stdout(stdout)
+                    .with_stderr(stderr)
+                    .with_truncated(truncated)
+                    .with_sandbox_violation(SandboxViolation::Signal { signal: sig }),
+            });
+        }
+
+        Ok(VerifyResult::Fail {
+            reason: FailureReason::new(format!("command terminated unexpectedly: {}", self.cmd))
+                .with_stdout(stdout)
+                .with_stderr(stderr)
+                .with_truncated(truncated),
+        })
+    }
+}
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    fn context() -> VerifyContext {
+        VerifyContext {
+            stdout: String::new(),
+            stderr: None,
+            exit_code: None,
+            backend_name: "test".to_string(),
+            model: None,
+            structured: None,
+            duration: Duration::ZERO,
+        }
+    }
+
+    #[test]
+    fn run_command_builder_api() {
+        let rc = RunCommand::new("cargo")
+            .with_args(vec!["test".to_string(), "--quiet".to_string()])
+            .with_env_allowlist(&["PATH", "HOME"])
+            .with_cwd("/tmp")
+            .with_wall_timeout(Duration::from_secs(10))
+            .with_cpu_timeout(Some(Duration::from_secs(5)))
+            .with_stdout_cap(1024)
+            .with_stderr_cap(2048);
+
+        assert_eq!(rc.cmd, "cargo");
+        assert_eq!(rc.args, vec!["test", "--quiet"]);
+        assert_eq!(rc.env_allowlist, vec!["PATH", "HOME"]);
+        assert_eq!(rc.cwd, Some(PathBuf::from("/tmp")));
+        assert_eq!(rc.wall_timeout, Duration::from_secs(10));
+        assert_eq!(rc.cpu_timeout, Some(Duration::from_secs(5)));
+        assert_eq!(rc.stdout_cap, 1024);
+        assert_eq!(rc.stderr_cap, 2048);
+    }
+
+    #[test]
+    fn run_command_default_values() {
+        let rc = RunCommand::new("echo");
+        assert_eq!(rc.cmd, "echo");
+        assert!(rc.args.is_empty());
+        assert!(rc.env_allowlist.is_empty());
+        assert!(rc.cwd.is_none());
+        assert_eq!(rc.wall_timeout, DEFAULT_WALL_TIMEOUT);
+        assert_eq!(rc.cpu_timeout, None);
+        assert_eq!(rc.stdout_cap, DEFAULT_OUTPUT_CAP);
+        assert_eq!(rc.stderr_cap, DEFAULT_OUTPUT_CAP);
+    }
+
+    #[cfg(unix)]
+    #[tokio::test]
+    async fn verify_echo_passes() {
+        let rc = RunCommand::new("sh").with_args(vec!["-c".to_string(), "echo hello".to_string()]);
+        let result = rc.verify(&context()).await.unwrap();
+        assert!(matches!(result, VerifyResult::Pass));
+    }
+
+    #[cfg(unix)]
+    #[tokio::test]
+    async fn verify_false_fails_with_code() {
+        let rc = RunCommand::new("sh")
+            .with_args(vec!["-c".to_string(), "echo err >&2; exit 1".to_string()]);
+        let result = rc.verify(&context()).await.unwrap();
+        match result {
+            VerifyResult::Fail { reason } => {
+                assert_eq!(reason.exit_code, Some(1));
+                assert!(reason.stderr.contains("err"));
+                assert!(matches!(
+                    reason.sandbox_violation,
+                    Some(SandboxViolation::NonZeroExit { code: 1 })
+                ));
+            }
+            other => panic!("expected fail, got {other:?}"),
+        }
+    }
+
+    #[tokio::test]
+    async fn verify_missing_command_fails() {
+        let rc = RunCommand::new("__no_such_command__");
+        let err = rc
+            .verify(&context())
+            .await
+            .expect_err("expected verify error");
+        assert!(err.message.contains("command not found"));
+    }
+
+    #[cfg(unix)]
+    #[tokio::test]
+    async fn verify_sleeps_timeout() {
+        let rc = RunCommand::new("sleep")
+            .with_args(vec!["60".to_string()])
+            .with_wall_timeout(Duration::from_millis(100));
+        let result = rc.verify(&context()).await.unwrap();
+        match result {
+            VerifyResult::Fail { reason } => {
+                assert!(matches!(
+                    reason.sandbox_violation,
+                    Some(SandboxViolation::Timeout)
+                ));
+            }
+            other => panic!("expected fail, got {other:?}"),
+        }
+    }
+}
diff --git a/src/utils.rs b/src/utils.rs
index b196636..afd5e59 100644
--- a/src/utils.rs
+++ b/src/utils.rs
@@ -2,6 +2,9 @@
 
 use colored::Colorize;
 use std::path::{Path, PathBuf};
+use std::sync::LazyLock;
+
+use regex::Regex;
 
 use crate::backend::BackendError;
 
@@ -195,10 +198,64 @@ pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
         .unwrap_or("")
 }
 
+// ── Secret redaction ─────────────────────────────────────
+
+static AWS_KEY_RE: LazyLock<Regex> =
+    LazyLock::new(|| Regex::new(r"AKIA[0-9A-Z]{16}").expect("valid regex"));
+
+static KEY_VALUE_RE: LazyLock<Regex> = LazyLock::new(|| {
+    Regex::new(r#"(?i)((?:api[_-]?key|secret|token|password)\s*[=:]\s*)[^\s'\"]+"#)
+        .expect("valid regex")
+});
+
+static BEARER_RE: LazyLock<Regex> =
+    LazyLock::new(|| Regex::new(r"(?i)Bearer\s+[A-Za-z0-9._\-~+/=]+").expect("valid regex"));
+
+static SECRET_HEURISTIC_RE: LazyLock<Regex> = LazyLock::new(|| {
+    Regex::new(r"(?i)\b(key|secret|token)[\s:=]+([A-Za-z0-9+/=_\-]{32,})").expect("valid regex")
+});
+
+/// Redact common secret-like tokens from free text.
+pub fn redact_secrets(input: &str) -> String {
+    let mut result = AWS_KEY_RE.replace_all(input, "[REDACTED]").into_owned();
+    result = KEY_VALUE_RE
+        .replace_all(&result, "${1}[REDACTED]")
+        .into_owned();
+    result = BEARER_RE.replace_all(&result, "[REDACTED]").into_owned();
+    result = SECRET_HEURISTIC_RE
+        .replace_all(&result, "$1 [REDACTED]")
+        .into_owned();
+    result
+}
+
 #[cfg(test)]
 mod tests {
     use super::*;
 
+    #[test]
+    fn test_redact_secrets_aws_key() {
+        assert_eq!(
+            redact_secrets("key: AKIA0123456789ABCDEF rest"),
+            "key: [REDACTED] rest"
+        );
+    }
+
+    #[test]
+    fn test_redact_secrets_api_key_value() {
+        assert_eq!(
+            redact_secrets("api_key=AKIA0123456789ABCDEF other"),
+            "api_key=[REDACTED] other"
+        );
+    }
+
+    #[test]
+    fn test_redact_secrets_bearer_token() {
+        assert_eq!(
+            redact_secrets("Authorization: Bearer eyJhbGciOiJIUzI1NiJ9"),
+            "Authorization: [REDACTED]"
+        );
+    }
+
     #[test]
     fn test_truncate_short_string() {
         assert_eq!(truncate("hello", 10), "hello");

exec
/bin/zsh -lc "grep -n \"which\\|libc\\|regex\\|tokio\" Cargo.toml && cargo test -q" in /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
 exited 101 in 372ms:

running 636 tests
....................................................................................... 87/636
................................................................... 154/636
backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime --- FAILED
backend::tensorzero::tests::maps_401_to_auth_not_retryable --- FAILED
. 157/636
backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable --- FAILED
backend::tensorzero::tests::maps_429_to_rate_limit_retryable --- FAILED
backend::tensorzero::tests::maps_500_to_retryable_error --- FAILED
backend::tensorzero::tests::maps_502_generic_to_network_retryable --- FAILED
backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable --- FAILED
backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable --- FAILED
backend::tensorzero::tests::maps_malformed_json_to_parse_error --- FAILED
backend::tensorzero::tests::maps_request_timeout_to_timeout_error --- FAILED
.. 167/636
backend::tensorzero::tests::returns_text_on_200_success --- FAILED
backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model --- FAILED
....... 176/636
backend::tests::tensorzero_create_backend_queries_wiremock_gateway --- FAILED
....................................................................................... 264/636
..................................................... 317/636
backend::ollama::tests::capabilities_match_current_wiring --- FAILED
backend::tensorzero::tests::name_is_tensorzero --- FAILED
backend::tests::tensorzero_create_backend_supported_when_capability_supported --- FAILED
....................................................................................... 407/636
....................................................................................... 494/636
....................................................................................... 581/636
.......................................................
failures:

---- backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime stdout ----

thread 'backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime' (44732446) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- backend::tensorzero::tests::maps_401_to_auth_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_401_to_auth_not_retryable' (44732493) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable' (44732506) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_429_to_rate_limit_retryable stdout ----

thread 'backend::tensorzero::tests::maps_429_to_rate_limit_retryable' (44732514) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_500_to_retryable_error stdout ----

thread 'backend::tensorzero::tests::maps_500_to_retryable_error' (44732518) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_generic_to_network_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_generic_to_network_retryable' (44732523) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable' (44732528) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable' (44732533) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_malformed_json_to_parse_error stdout ----

thread 'backend::tensorzero::tests::maps_malformed_json_to_parse_error' (44732538) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_request_timeout_to_timeout_error stdout ----

thread 'backend::tensorzero::tests::maps_request_timeout_to_timeout_error' (44732541) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::returns_text_on_200_success stdout ----

thread 'backend::tensorzero::tests::returns_text_on_200_success' (44732560) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model stdout ----

thread 'backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model' (44732572) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tests::tensorzero_create_backend_queries_wiremock_gateway stdout ----

thread 'backend::tests::tensorzero_create_backend_queries_wiremock_gateway' (44732608) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::ollama::tests::capabilities_match_current_wiring stdout ----

thread 'backend::ollama::tests::capabilities_match_current_wiring' (44732427) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
Attempted to create a NULL object.

---- backend::tensorzero::tests::name_is_tensorzero stdout ----

thread 'backend::tensorzero::tests::name_is_tensorzero' (44732550) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
Attempted to create a NULL object.

---- backend::tests::tensorzero_create_backend_supported_when_capability_supported stdout ----

thread 'backend::tests::tensorzero_create_backend_supported_when_capability_supported' (44732609) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
Attempted to create a NULL object.


failures:
    backend::ollama::tests::capabilities_match_current_wiring
    backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime
    backend::tensorzero::tests::maps_401_to_auth_not_retryable
    backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable
    backend::tensorzero::tests::maps_429_to_rate_limit_retryable
    backend::tensorzero::tests::maps_500_to_retryable_error
    backend::tensorzero::tests::maps_502_generic_to_network_retryable
    backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable
    backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable
    backend::tensorzero::tests::maps_malformed_json_to_parse_error
    backend::tensorzero::tests::maps_request_timeout_to_timeout_error
    backend::tensorzero::tests::name_is_tensorzero
    backend::tensorzero::tests::returns_text_on_200_success
    backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model
    backend::tests::tensorzero_create_backend_queries_wiremock_gateway
    backend::tests::tensorzero_create_backend_supported_when_capability_supported

test result: FAILED. 620 passed; 16 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s

error: test failed, to rerun pass `--lib`

exec
/bin/zsh -lc "sed -n '1,360p' src/strategy/verify/verify.rs && sed -n '1,120p' src/lib.rs && sed -n '1,120p' src/main.rs && git diff 30fe7a742785a98cb2b1894b6a4dbc6c616d6c49 -- Cargo.toml Cargo.lock" in /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
 succeeded in 0ms:
//! Core verification types and trait.
//!
//! This module defines the `VerifyHook` trait and supporting types
//! (`VerifyResult`, `FailureReason`, `VerifyError`, `VerifyContext`).
//! Concrete implementations live in sibling modules (`llm_verifier`,
//! `run_command`).
//!
//! v0 hooks only need `Pass` and `Fail`. `Repair` and `Score` are reserved so
//! later hook implementations can evolve without changing the public enum.
//!
//! ## Security: redaction
//!
//! `FailureReason.stdout` and `FailureReason.stderr` carry raw output that may
//! contain secrets. Redaction is deferred to the consumer — CLO-260's
//! `pass_failure_context` path runs `redact_secrets()` before flowing the
//! reason into the next prompt. Hook implementations that log or persist
//! `FailureReason` fields directly must apply their own redaction.

/// Sandbox-level signal captured from command execution failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxViolation {
    /// Wall-clock timeout expired while the command was running.
    Timeout,
    /// Process exited due to an OS signal (Unix-only).
    Signal { signal: i32 },
    /// Non-zero process exit code.
    NonZeroExit { code: i32 },
}

// ── FailureReason ────────────────────────────────────────────

use std::time::Duration;

use async_trait::async_trait;

use crate::backend::QueryOutput;

// ── FailureReason ────────────────────────────────────────────

/// Structured reason a verification hook returned `Fail`.
///
/// Carries enough detail to feed `pass_failure_context` (CLO-260).
/// Fields are `pub` so callers can extract individual signals without
/// parsing the combined `display()` string.
///
/// ## Security: Redaction
///
/// `stdout` and `stderr` carry raw output that may contain secrets
/// (API keys in LLM responses, stack traces with env vars). Redaction
/// is **deferred to the consumer** — CLO-260's `pass_failure_context`
/// path runs `redact_secrets()` on the reason before flowing it into
/// the next prompt. Hook implementations that log or persist
/// `FailureReason` fields directly must apply their own redaction.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct FailureReason {
    /// Human-readable summary (e.g. "test `it_adds` failed").
    pub summary: String,
    /// Captured stdout from the verification run (may be truncated).
    /// **Unredacted** — consumers must apply redaction before prompt injection.
    pub stdout: String,
    /// Captured stderr from the verification run (may be truncated).
    /// **Unredacted** — consumers must apply redaction before prompt injection.
    pub stderr: String,
    /// `true` iff stdout or stderr was truncated at `MAX_OUTPUT_BYTES`.
    pub truncated: bool,
    /// Exit code if the verifier ran as a process. `None` for in‑process
    /// verifiers (e.g. `LLMVerifier`).
    pub exit_code: Option<i32>,
    /// Optional sandbox signal that further explains process termination.
    pub sandbox_violation: Option<SandboxViolation>,
}

impl FailureReason {
    /// Create a new failure reason with a human-readable summary.
    /// All other fields default to empty / `None`.
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            stdout: String::new(),
            stderr: String::new(),
            truncated: false,
            exit_code: None,
            sandbox_violation: None,
        }
    }

    /// Attach captured stdout (builder-pattern).
    pub fn with_stdout(mut self, stdout: impl Into<String>) -> Self {
        self.stdout = stdout.into();
        self
    }

    /// Attach captured stderr (builder-pattern).
    pub fn with_stderr(mut self, stderr: impl Into<String>) -> Self {
        self.stderr = stderr.into();
        self
    }

    /// Mark the output as truncated (builder-pattern).
    pub fn with_truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }

    /// Attach an exit code (builder-pattern).
    pub fn with_exit_code(mut self, exit_code: i32) -> Self {
        self.exit_code = Some(exit_code);
        self
    }

    /// Attach sandbox metadata (builder-pattern).
    pub fn with_sandbox_violation(mut self, sandbox_violation: SandboxViolation) -> Self {
        self.sandbox_violation = Some(sandbox_violation);
        self
    }
}

impl std::fmt::Display for FailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.summary)?;
        if self.truncated {
            write!(f, " (truncated)")?;
        }
        Ok(())
    }
}

// ── VerifyResult ─────────────────────────────────────────────

/// Verdict returned by a `VerifyHook::verify()` call.
///
/// **Variant lifecycle:**
///
/// | Variant | v0 status | Notes |
/// |---------|-----------|-------|
/// | `Pass`  | **live** — emitted by v0 hooks | |
/// | `Fail { reason }` | **live** — `reason` is `FailureReason` | |
/// | `Repair { suggestion }` | **reserved** — compiles, no caller acts on it yet | M10 `HumanVerifier` will emit this |
/// | `Score(f32)` | **reserved** — compiles, no caller acts on it yet. Higher values = better quality. | Future cascadeflow‑style semantic gates |
///
/// Callers that pattern‑match MUST include arms for reserved variants;
/// the recommended pattern is a documented fallthrough (see
/// `escalating_retry.rs` for the reference consumer).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum VerifyResult {
    Pass,
    Fail { reason: FailureReason },
    Repair { suggestion: String },
    Score(f32),
}

impl VerifyResult {
    /// Convenience constructor for a `Pass` variant.
    pub fn pass() -> Self {
        Self::Pass
    }

    /// Convenience constructor for a `Fail` variant with a simple summary.
    /// Other `FailureReason` fields default to empty.
    pub fn fail(summary: impl Into<String>) -> Self {
        Self::Fail {
            reason: FailureReason::new(summary),
        }
    }

    /// Convenience constructor for a `Fail` variant with a fully populated reason.
    pub fn fail_with(reason: FailureReason) -> Self {
        Self::Fail { reason }
    }

    /// `true` iff this is a `Pass` variant.
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    /// `true` iff this is a `Fail` variant.
    pub fn is_fail(&self) -> bool {
        matches!(self, Self::Fail { .. })
    }
}

// ── VerifyError ──────────────────────────────────────────────

/// Error surfaced when a `VerifyHook` implementation itself fails.
///
/// Distinct from `VerifyResult::Fail`:
/// - `Fail` means the hook ran, decided "that output isn't good enough",
///   and produced a structured `FailureReason`.
/// - `VerifyError` means the hook could not run at all: sandbox crash,
///   backend unreachable, `make` missing from `$PATH`, etc.
///
/// ## Future: error source chain
/// For v0 the `message` string suffices. When CLO-271 (RunCommand)
/// introduces I/O errors and subprocess failures, `VerifyError` should
/// gain a `#[source]`-annotated field (e.g. `source: Option<Box<dyn std::error::Error + Send + Sync>>`)
/// to preserve the original error chain for debugging.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("verify hook failed: {message}")]
pub struct VerifyError {
    pub message: String,
}

impl VerifyError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

// ── VerifyContext ────────────────────────────────────────────

/// Input passed to every `VerifyHook::verify()` call.
///
/// Carries the output under verification plus metadata about the phase
/// and backend that produced it. Does **not** carry credentials
/// (API keys, tokens) — those live in `BackendConfig` and are never
/// exposed to verify hooks.
///
/// `#[non_exhaustive]` so the phase runner (T-029) can add fields
/// (manifest pointer, run‑dir paths) without breaking hook implementations.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct VerifyContext {
    /// Raw stdout from the backend call under verification.
    pub stdout: String,
    /// Raw stderr from the backend call, if any.
    pub stderr: Option<String>,
    /// Exit code if the backend ran as a subprocess.
    pub exit_code: Option<i32>,
    /// Name of the backend that produced this output (e.g. `"claude-3"`).
    pub backend_name: String,
    /// Model name reported by the backend, if known.
    pub model: Option<String>,
    /// Parsed JSON if the output was successfully deserialized.
    pub structured: Option<serde_json::Value>,
    /// Wall‑clock duration of the backend call, as measured by `Backend::query()`.
    pub duration: Duration,
}

impl VerifyContext {
    /// Build a `VerifyContext` from a `QueryOutput` plus a backend name.
    ///
    /// This is the EscalatingRetry call‑site constructor. When the phase
    /// runner (T-029) replaces EscalatingRetry as the direct caller, it
    /// can build `VerifyContext` from other sources (manifest, run dir)
    /// without touching hook implementations.
    pub fn from_query_output(query: &QueryOutput) -> Self {
        Self {
            stdout: query.stdout.clone(),
            stderr: query.stderr.clone(),
            exit_code: query.exit_code,
            backend_name: query.backend.clone(),
            model: query.model.clone(),
            structured: query.structured.clone(),
            duration: query.duration,
        }
    }
}

// ── VerifyHook trait ─────────────────────────────────────────

/// Verification hook that gates strategy progress.
///
/// Implementations are `Send + Sync` so they can be shared behind `Arc`
/// and driven across async tasks by the phase runner.
///
/// ## Method contract
///
/// - `name()` returns a stable, human‑readable label for trace output
///   (e.g. `"TestRunner"`, `"LLMVerifier"`).
/// - `verify(ctx)` inspects the backend output in `ctx` and returns a
///   verdict. `Err(VerifyError)` signals the hook itself failed;
///   `Ok(VerifyResult::Fail { .. })` signals the hook ran successfully
///   but judged the output insufficient.
///
/// ## Required context contract
///
/// As `VerifyContext` gains fields over time (via `#[non_exhaustive]`),
/// a hook may receive a context that lacks a field required for its
/// operation. In that case the hook MUST return `Err(VerifyError)` with
/// a descriptive message — it MUST NOT panic.
///
/// ## Cancellation safety
///
/// `verify()` implementors are responsible for cancellation safety.
/// If a `tokio::timeout` (or similar) drops the future mid-execution,
/// any spawned subprocesses or in-flight HTTP requests must be cleaned
/// up (e.g. via `tokio::spawn` with a cancellation token, or by
/// ensuring the future is abort-safe).
#[async_trait]
pub trait VerifyHook: Send + Sync {
    fn name(&self) -> &str;

    async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError>;
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A stub hook that returns a predetermined result.
    struct StubHook {
        name: &'static str,
        result: Result<VerifyResult, VerifyError>,
    }

    #[async_trait]
    impl VerifyHook for StubHook {
        fn name(&self) -> &str {
            self.name
        }

        async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
            match &self.result {
                Ok(r) => Ok(r.clone()),
                Err(e) => Err(e.clone()),
            }
        }
    }

    fn dummy_context() -> VerifyContext {
        VerifyContext {
            stdout: String::new(),
            stderr: None,
            exit_code: None,
            backend_name: "test".to_string(),
            model: None,
            structured: None,
            duration: Duration::ZERO,
        }
    }

    #[tokio::test]
    async fn stub_verify_hook_returns_pass() {
        let hook = Arc::new(StubHook {
            name: "pass_stub",
            result: Ok(VerifyResult::Pass),
        });
        let result = hook.verify(&dummy_context()).await.unwrap();
        assert!(result.is_pass());
        assert!(!result.is_fail());
    }

    #[tokio::test]
    async fn stub_verify_hook_returns_fail() {
        let reason = FailureReason::new("test failed");
        let hook = Arc::new(StubHook {
            name: "fail_stub",
            result: Ok(VerifyResult::Fail {
                reason: reason.clone(),
            }),
        });
        let result = hook.verify(&dummy_context()).await.unwrap();
        assert!(result.is_fail());
// Most modules are private to this library and are surfaced only because
// they sit in the dependency closure of the public ones (`strategy` →
// `template` → `workflow` → ...). The binary (src/main.rs) re-declares
// them via its own private `mod` tree, so this file's job is just to
// satisfy the lib-side compile and expose the integration-test surface.
#![allow(dead_code)]

pub mod aggregator;
pub mod backend;
pub mod family;
pub mod strategy;
pub mod template;

mod apply_verify;
mod cache;
mod config;
mod consensus;
mod context;
mod git_agent;
mod role;
mod utils;
mod workflow;
mod workflows;
#![allow(dead_code)]

mod apply_verify;
mod backend;
mod cache;
mod conductor;
mod config;
mod consensus;
mod context;
mod debate;
mod delegation;
mod git_agent;
mod output;
mod role;
mod spawn;
mod tasks;
mod team;
mod template;
mod utils;
mod workflow;
mod workflows;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use colored::Colorize;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "loker")]
#[command(about = "LLM orchestration: cross-family aggregation, escalating retry, verify hooks")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to config file
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Verbose output (show prompts, timing, debug info)
    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Ask LLM backends a question
    Ask {
        /// The prompt to send
        prompt: String,

        /// Specific backends to use (comma-separated)
        #[arg(short, long)]
        backend: Option<String>,

        /// Working directory for the query
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,

        /// Skip cache and force fresh query
        #[arg(long)]
        no_cache: bool,
    },

    /// Run a bug hunt on a codebase
    Hunt {
        /// Directory to analyze
        #[arg(default_value = ".")]
        dir: PathBuf,

        /// Create issues for each finding (auto-detects gh or glab)
        #[arg(long)]
        issues: bool,

        /// Issue backend: github, gitlab, or auto (default: auto)
        #[arg(long, default_value = "auto")]
        issue_backend: String,

        /// Skip confirmation prompt when creating issues
        #[arg(long, short = 'y')]
        yes: bool,
    },

    /// Fix a GitHub issue
    Fix {
        /// Issue number, #number, or full URL
        issue: String,

        /// Working directory
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,

        /// Specific backend to use
        #[arg(short, long)]
        backend: Option<String>,

        /// Dry run - analyze but don't suggest applying changes
        #[arg(long)]
        dry_run: bool,
    },

    /// Analyze CI failures for a PR
    Ci {
        /// PR number (e.g., "123")
        pr: String,

        /// Working directory
        #[arg(short, long, default_value = ".")]
        dir: PathBuf,

        /// Specific backends to use (comma-separated)
        #[arg(short, long)]
        backend: Option<String>,
    },

    /// Run a security audit on a codebase
    Audit {
        /// Directory to analyze
        #[arg(default_value = ".")]
        dir: PathBuf,

exec
/bin/zsh -lc 'cargo test -q --test verify_run_command' in /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
 exited 101 in 1127ms:

running 8 tests
...... 6/8
runcommand_process_group_killed_on_timeout --- FAILED
.
failures:

---- runcommand_process_group_killed_on_timeout stdout ----

thread 'runcommand_process_group_killed_on_timeout' (44735002) panicked at tests/verify_run_command.rs:24:10:
ps should run: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    runcommand_process_group_killed_on_timeout

test result: FAILED. 7 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.01s

error: test failed, to rerun pass `--test verify_run_command`

exec
/bin/zsh -lc "git diff 30fe7a742785a98cb2b1894b6a4dbc6c616d6c49 -- Cargo.toml Cargo.lock | head -200 && rg \"RunCommand|VerifyContext \\{\" -n src tests | head -50 && rg \"process_group|kill_on_drop|timeout\" src/apply_verify/verification.rs -n -C 2" in /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
 succeeded in 0ms:
tests/verify_run_command.rs:8:use loker::strategy::verify::{RunCommand, SandboxViolation};
tests/verify_run_command.rs:12:fn ctx() -> VerifyContext {
tests/verify_run_command.rs:32:    let rc = RunCommand::new("sh").with_args(vec!["-c".to_string(), "echo hello".to_string()]);
tests/verify_run_command.rs:40:    let rc = RunCommand::new("sh")
tests/verify_run_command.rs:58:    let rc = RunCommand::new("sleep")
tests/verify_run_command.rs:78:    let rc = RunCommand::new("sh")
tests/verify_run_command.rs:94:    let rc = RunCommand::new("sh")
tests/verify_run_command.rs:115:    let rc = RunCommand::new("sh")
tests/verify_run_command.rs:137:    let rc = RunCommand::new("sh")
tests/verify_run_command.rs:161:    let rc = RunCommand::new("sh")
src/strategy/mod.rs:34:    FailureReason, LLMVerifier, RunCommand, SandboxViolation, VerifyContext, VerifyError,
src/strategy/verify/verify.rs:195:/// For v0 the `message` string suffices. When CLO-271 (RunCommand)
src/strategy/verify/verify.rs:226:pub struct VerifyContext {
src/strategy/verify/verify.rs:243:impl VerifyContext {
src/strategy/verify/verify.rs:327:    fn dummy_context() -> VerifyContext {
src/strategy/verify/verify.rs:328:        VerifyContext {
src/strategy/verify/mod.rs:9://! - [`RunCommand`] — shells out and maps exit status (CLO-271).
src/strategy/verify/mod.rs:22:pub use run_command::RunCommand;
src/strategy/verify/run_command.rs:31:pub struct RunCommand {
src/strategy/verify/run_command.rs:51:impl Default for RunCommand {
src/strategy/verify/run_command.rs:66:impl RunCommand {
src/strategy/verify/run_command.rs:368:impl VerifyHook for RunCommand {
src/strategy/verify/run_command.rs:370:        "RunCommand"
src/strategy/verify/run_command.rs:429:    fn context() -> VerifyContext {
src/strategy/verify/run_command.rs:430:        VerifyContext {
src/strategy/verify/run_command.rs:443:        let rc = RunCommand::new("cargo")
src/strategy/verify/run_command.rs:464:        let rc = RunCommand::new("echo");
src/strategy/verify/run_command.rs:478:        let rc = RunCommand::new("sh").with_args(vec!["-c".to_string(), "echo hello".to_string()]);
src/strategy/verify/run_command.rs:486:        let rc = RunCommand::new("sh")
src/strategy/verify/run_command.rs:504:        let rc = RunCommand::new("__no_such_command__");
src/strategy/verify/run_command.rs:515:        let rc = RunCommand::new("sleep")
tests/verify_llm_verifier.rs:82:fn ctx_with_candidate(candidate: impl Into<String>) -> VerifyContext {
1://! Runs shell verification commands with bounded output and timeout.
2-//!
3-//! `Verification::run(cwd)` executes `sh -c <command>`, caps stdout+stderr
4-//! capture at `max_output_bytes`, measures real wall-clock elapsed time, and
5-//! on Unix places the child in a new process group so the entire process
6://! tree can be reaped on timeout.
7-
8-use std::path::Path;
--
21-    /// Shell command to run (will be wrapped with `sh -c`).
22-    pub command: String,
23:    /// Hard wall-clock timeout. On timeout the whole process group is killed.
24:    pub timeout: Duration,
25-    /// Maximum bytes to capture from each of stdout and stderr independently.
26-    /// Further output is dropped and `VerifyResult::truncated` is set to `true`.
--
33-/// Structured verification result.
34-///
35:/// Never wrapped in `Result`: verify failure (non-zero exit, timeout, spawn
36-/// error) is a normal outcome, not a Rust error. Callers inspect `success`.
37-#[derive(Debug, Clone)]
--
43-    /// Captured stderr (possibly truncated - see `truncated`).
44-    pub stderr: String,
45:    /// Exit code if the process exited normally. `None` on timeout, signal,
46-    /// or spawn failure.
47-    pub exit_code: Option<i32>,
48-    /// Actual wall-clock elapsed time from spawn to reap, in milliseconds.
49:    /// Measured via `std::time::Instant`, never derived from `timeout`.
50-    pub elapsed_ms: u64,
51:    /// `true` iff the process was killed because `timeout` was exceeded.
52-    pub timed_out: bool,
53-    /// `true` iff stdout or stderr capture hit `max_output_bytes` and further
--
71-            .stdout(Stdio::piped())
72-            .stderr(Stdio::piped())
73:            .kill_on_drop(true);
74-
75-        // On Unix, place the child in its own process group so we can reap
76:        // the whole tree (sh + its descendants) on timeout. Without this,
77:        // `kill_on_drop` only kills the direct `sh` child and orphans any
78-        // grandchildren it spawned.
79-        #[cfg(unix)]
80:        cmd.process_group(0);
81-
82-        let mut child = match cmd.spawn() {
--
115-        let stderr_handle = tokio::spawn(read_bounded(stderr, max_bytes));
116-
117:        match tokio::time::timeout(self.timeout, child.wait()).await {
118-            Ok(Ok(status)) => {
119-                let (stdout_bytes, stdout_trunc) = stdout_handle.await.unwrap_or_default();
--
142-                // descendants (e.g. `sleep` spawned by `sh -c "sleep 30 & wait"`)
143-                // die along with the direct child.
144:                kill_process_group(pid);
145:                // Await the child so we don't leave a zombie; kill_on_drop will
146-                // also cover this but explicit wait drains the state machine.
147-                let _ = child.wait().await;
--
164-/// Send `SIGKILL` to the process group led by `pid`. No-op on non-Unix or
165-/// if `pid` is `None` (child already reaped).
166:fn kill_process_group(pid: Option<u32>) {
167-    #[cfg(unix)]
168-    if let Some(pid) = pid {
--
231-    use tempfile::tempdir;
232-
233:    fn verification(command: &str, timeout_ms: u64) -> Verification {
234-        Verification {
235-            command: command.to_string(),
236:            timeout: Duration::from_millis(timeout_ms),
237-            max_output_bytes: 64 * 1024,
238-        }
--
301-
302-    #[tokio::test]
303:    async fn test_verify_timeout_real_elapsed() {
304-        let dir = tempdir().unwrap();
305-        let result = verification("sleep 10", 200).run(dir.path()).await;
--
307-        assert!(result.timed_out);
308-        assert_eq!(result.exit_code, None);
309:        // Real elapsed time should be in the vicinity of the timeout, not
310:        // hardcoded to `timeout.as_millis()`. We allow a generous upper bound
311-        // to accommodate slow CI runners.
312-        assert!(
--
347-    #[cfg(unix)]
348-    #[tokio::test]
349:    async fn test_verify_timeout_kills_process_group() {
350-        let dir = tempdir().unwrap();
351:        // Spawn a grandchild sleeper via sh -c. With process_group(0) + the
352-        // libc::kill(-pid, SIGKILL) path, the grandchild is reaped when the
353:        // timeout fires - so `run` returns quickly instead of waiting 30s.
354-        let start = Instant::now();
355-        let result = verification("sleep 30 & wait", 200).run(dir.path()).await;

exec
/bin/zsh -lc "nl -ba src/strategy/verify/run_command.rs | sed -n '130,250p;340,365p'" in /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
 succeeded in 0ms:
   130	
   131	    fn build_environment(&self) -> Vec<(String, String)> {
   132	        self.env_allowlist
   133	            .iter()
   134	            .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
   135	            .map(|(name, value)| {
   136	                let redacted = if is_secret_like_env_key(&name) {
   137	                    redact_secrets(&value)
   138	                } else {
   139	                    value
   140	                };
   141	                (name, redacted)
   142	            })
   143	            .collect()
   144	    }
   145	
   146	    async fn run(&self) -> Result<CommandRun, VerifyError> {
   147	        let command_path = self.resolve_command()?;
   148	
   149	        let mut command = Command::new(&command_path);
   150	        command
   151	            .args(&self.args)
   152	            .env_clear()
   153	            .stdin(Stdio::null())
   154	            .stdout(Stdio::piped())
   155	            .stderr(Stdio::piped());
   156	
   157	        if let Some(cwd) = &self.cwd {
   158	            command.current_dir(cwd);
   159	        }
   160	
   161	        for (key, value) in self.build_environment() {
   162	            command.env(key, value);
   163	        }
   164	
   165	        // Put each child in its own process group, so SIGKILL on timeout
   166	        // reaps descendants as well.
   167	        #[cfg(unix)]
   168	        command.process_group(0);
   169	
   170	        #[cfg(unix)]
   171	        if let Some(cpu_timeout) = self.cpu_timeout {
   172	            let secs = cpu_timeout.as_secs().max(1);
   173	            // SAFETY: this closure runs in the child before exec and only
   174	            // mutates process-local resource limits.
   175	            unsafe {
   176	                command.pre_exec(move || {
   177	                    let limit = libc::rlimit {
   178	                        rlim_cur: secs as libc::rlim_t,
   179	                        rlim_max: secs as libc::rlim_t,
   180	                    };
   181	                    let rc = libc::setrlimit(libc::RLIMIT_CPU, &limit);
   182	                    if rc != 0 {
   183	                        return Err(std::io::Error::last_os_error());
   184	                    }
   185	                    Ok(())
   186	                });
   187	            }
   188	        }
   189	
   190	        let start = Instant::now();
   191	        let mut child = command.spawn().map_err(|err| {
   192	            VerifyError::new(format!("failed to spawn command '{}': {err}", self.cmd))
   193	        })?;
   194	
   195	        let stdout_stream = child
   196	            .stdout
   197	            .take()
   198	            .ok_or_else(|| VerifyError::new("failed to capture child stdout pipe".to_string()))?;
   199	        let stderr_stream = child
   200	            .stderr
   201	            .take()
   202	            .ok_or_else(|| VerifyError::new("failed to capture child stderr pipe".to_string()))?;
   203	
   204	        let stdout_handle = tokio::spawn(read_stream_bounded(stdout_stream, self.stdout_cap));
   205	        let stderr_handle = tokio::spawn(read_stream_bounded(stderr_stream, self.stderr_cap));
   206	
   207	        let (status, timed_out) = match timeout(self.wall_timeout, child.wait()).await {
   208	            Ok(result) => {
   209	                let status = result.map_err(|err| {
   210	                    VerifyError::new(format!("failed to wait for command '{}': {err}", self.cmd))
   211	                })?;
   212	                (status, false)
   213	            }
   214	            Err(_) => {
   215	                kill_process_group(child.id());
   216	                let status = child.wait().await.map_err(|err| {
   217	                    VerifyError::new(format!(
   218	                        "failed to reap timed-out command '{}': {err}",
   219	                        self.cmd
   220	                    ))
   221	                })?;
   222	                (status, true)
   223	            }
   224	        };
   225	
   226	        let stdout = stdout_handle
   227	            .await
   228	            .map_err(|_| VerifyError::new("stdout reader task failed"))?;
   229	        let stderr = stderr_handle
   230	            .await
   231	            .map_err(|_| VerifyError::new("stderr reader task failed"))?;
   232	
   233	        Ok(CommandRun {
   234	            status,
   235	            timed_out,
   236	            stdout,
   237	            stderr,
   238	            elapsed_ms: start.elapsed().as_millis() as u64,
   239	        })
   240	    }
   241	}
   242	
   243	#[derive(Debug)]
   244	struct CommandRun {
   245	    status: std::process::ExitStatus,
   246	    timed_out: bool,
   247	    stdout: CapturedOutput,
   248	    stderr: CapturedOutput,
   249	    elapsed_ms: u64,
   250	}
   340	fn status_signal(status: &std::process::ExitStatus) -> Option<i32> {
   341	    use std::os::unix::process::ExitStatusExt;
   342	    status.signal()
   343	}
   344	
   345	#[cfg(not(unix))]
   346	fn status_signal(_status: &std::process::ExitStatus) -> Option<i32> {
   347	    None
   348	}
   349	
   350	#[cfg(unix)]
   351	fn kill_process_group(pid: Option<u32>) {
   352	    if let Some(pid) = pid {
   353	        // SAFETY: this sends SIGKILL to a process group spawned by us and avoids
   354	        // orphaned children. If the pid is stale or already dead, this is a
   355	        // best-effort cleanup attempt.
   356	        unsafe {
   357	            libc::kill(-(pid as i32), libc::SIGKILL);
   358	        }
   359	    }
   360	}
   361	
   362	#[cfg(not(unix))]
   363	fn kill_process_group(pid: Option<u32>) {
   364	    let _ = pid;
   365	}

exec
/bin/zsh -lc "sed -n '60,160p' src/apply_verify/verification.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
 succeeded in 0ms:
    ///
    /// The `cwd` parameter (rather than a struct field) ensures `RetryLoop`
    /// has a single source of truth for the working directory.
    pub async fn run(&self, cwd: &Path) -> VerifyResult {
        let start = Instant::now();

        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg(&self.command)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // On Unix, place the child in its own process group so we can reap
        // the whole tree (sh + its descendants) on timeout. Without this,
        // `kill_on_drop` only kills the direct `sh` child and orphans any
        // grandchildren it spawned.
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return VerifyResult {
                    success: false,
                    stdout: String::new(),
                    stderr: format!("spawn failed: {e}"),
                    exit_code: None,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    timed_out: false,
                    truncated: false,
                };
            }
        };

        let pid = child.id();
        let (stdout, stderr) = match (child.stdout.take(), child.stderr.take()) {
            (Some(out), Some(err)) => (out, err),
            _ => {
                return VerifyResult {
                    success: false,
                    stdout: String::new(),
                    stderr: "failed to capture stdout/stderr pipes".to_string(),
                    exit_code: None,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    timed_out: false,
                    truncated: false,
                };
            }
        };
        let max_bytes = self.max_output_bytes;

        let stdout_handle = tokio::spawn(read_bounded(stdout, max_bytes));
        let stderr_handle = tokio::spawn(read_bounded(stderr, max_bytes));

        match tokio::time::timeout(self.timeout, child.wait()).await {
            Ok(Ok(status)) => {
                let (stdout_bytes, stdout_trunc) = stdout_handle.await.unwrap_or_default();
                let (stderr_bytes, stderr_trunc) = stderr_handle.await.unwrap_or_default();
                VerifyResult {
                    success: status.success(),
                    stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
                    exit_code: status.code(),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    timed_out: false,
                    truncated: stdout_trunc || stderr_trunc,
                }
            }
            Ok(Err(e)) => VerifyResult {
                success: false,
                stdout: String::new(),
                stderr: format!("wait failed: {e}"),
                exit_code: None,
                elapsed_ms: start.elapsed().as_millis() as u64,
                timed_out: false,
                truncated: false,
            },
            Err(_) => {
                // Timeout fired. Reap the entire process group so that
                // descendants (e.g. `sleep` spawned by `sh -c "sleep 30 & wait"`)
                // die along with the direct child.
                kill_process_group(pid);
                // Await the child so we don't leave a zombie; kill_on_drop will
                // also cover this but explicit wait drains the state machine.
                let _ = child.wait().await;
                let (stdout_bytes, stdout_trunc) = stdout_handle.await.unwrap_or_default();
                let (stderr_bytes, stderr_trunc) = stderr_handle.await.unwrap_or_default();
                VerifyResult {
                    success: false,
                    stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
                    stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
                    exit_code: None,
                    elapsed_ms: start.elapsed().as_millis() as u64,
                    timed_out: true,
                    truncated: stdout_trunc || stderr_trunc,
                }
            }
        }

exec
/bin/zsh -lc "grep -n \"rust-version\\|edition\" Cargo.toml && rustc --version" in /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
 succeeded in 0ms:
4:edition = "2021"
5:rust-version = "1.80"
rustc 1.95.0 (59807616e 2026-04-14)

exec
/bin/zsh -lc "sed -n '1,140p' tests/verify_llm_verifier.rs && rg \"FailureReason \\{\" -n" in /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
 succeeded in 0ms:
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use loker::backend::{Backend, BackendError, QueryOutput};
use loker::strategy::verify::LLMVerifier;
use loker::strategy::{VerifyContext, VerifyHook, VerifyResult};

struct MockBackend {
    name: String,
    calls: AtomicUsize,
    prompts: Mutex<Vec<String>>, // record rendered prompts for assertions
    response: Box<dyn Fn() -> Result<QueryOutput, BackendError> + Send + Sync>,
}

impl MockBackend {
    fn ok(name: &str, text: &str) -> Arc<Self> {
        let name = name.to_string();
        let response_text = text.to_string();
        Arc::new(Self {
            name: name.clone(),
            calls: AtomicUsize::new(0),
            prompts: Mutex::new(Vec::new()),
            response: Box::new(move || {
                Ok(QueryOutput::from_text(
                    response_text.clone(),
                    name.clone(),
                    Duration::from_millis(5),
                ))
            }),
        })
    }

    fn fail(name: &str, err: BackendError) -> Arc<Self> {
        let name = name.to_string();
        Arc::new(Self {
            name: name.clone(),
            calls: AtomicUsize::new(0),
            prompts: Mutex::new(Vec::new()),
            response: Box::new(move || Err(err.clone())),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn last_prompt(&self) -> Option<String> {
        self.prompts.lock().unwrap().last().cloned()
    }

    fn all_prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

#[async_trait]
impl Backend for MockBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn query(
        &self,
        prompt: &str,
        _cwd: &Path,
        _model: Option<&str>,
    ) -> Result<QueryOutput, BackendError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.prompts.lock().unwrap().push(prompt.to_string());
        (self.response)()
    }

    fn is_available(&self) -> bool {
        true
    }
}

fn ctx_with_candidate(candidate: impl Into<String>) -> VerifyContext {
    let mut ctx = VerifyContext::from_query_output(&QueryOutput::from_text(
        candidate.into(),
        "source-backend",
        Duration::ZERO,
    ));
    ctx.structured = None;
    ctx
}

#[tokio::test]
async fn yes_is_pass() {
    let backend = MockBackend::ok("judge", "yes");
    let verifier = LLMVerifier::new("judge", backend.clone(), "{candidate}");

    let result = verifier
        .verify(&ctx_with_candidate("anything"))
        .await
        .unwrap();

    assert!(matches!(result, VerifyResult::Pass));
}

#[tokio::test]
async fn no_is_fail() {
    let backend = MockBackend::ok("judge", "no");
    let verifier = LLMVerifier::new("judge", backend.clone(), "{candidate}");

    let result = verifier
        .verify(&ctx_with_candidate("anything"))
        .await
        .unwrap();

    match result {
        VerifyResult::Fail { reason } => {
            assert_eq!(reason.summary, "no");
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[tokio::test]
async fn yes_variants_pass() {
    for response in ["Yes.", "YES\n", " yes - because..."] {
        let backend = MockBackend::ok("judge", response);
        let verifier = LLMVerifier::new("judge", backend.clone(), "{candidate}");
        let result = verifier
            .verify(&ctx_with_candidate("any output"))
            .await
            .unwrap();
        assert!(
            matches!(result, VerifyResult::Pass),
            "{response} should parse to pass"
        );
    }
}

#[tokio::test]
async fn unparseable_response_fails() {
src/strategy/verify/verify.rs:56:pub struct FailureReason {
src/strategy/verify/verify.rs:74:impl FailureReason {
src/strategy/verify/verify.rs:119:impl std::fmt::Display for FailureReason {
docs/plans/clo-271-run-command.md:63:  - non-zero → `Ok(VerifyResult::Fail { reason: FailureReason { ..., sandbox_violation: Some(NonZeroExit) } })`
docs/designs/clo-270-hook.md:125:pub struct FailureReason {
docs/designs/clo-270-hook.md:141:impl FailureReason {
docs/designs/clo-270-hook.md:173:impl std::fmt::Display for FailureReason {
docs/reviews/clo-270-design-gemini.md:100:impl std::fmt::Display for FailureReason {
docs/reviews/clo-270-design-synthesis.md:38:impl std::fmt::Display for FailureReason {
docs/reviews/clo-271-codex-validation.md:520:+  - non-zero → `Ok(VerifyResult::Fail { reason: FailureReason { ..., sandbox_violation: Some(NonZeroExit) } })`
docs/reviews/clo-271-codex-validation.md:1722: pub struct FailureReason {
docs/reviews/clo-271-codex-validation.md:1724:@@ -50,6 +67,8 @@ pub struct FailureReason {
docs/reviews/clo-271-codex-validation.md:1732: impl FailureReason {
docs/reviews/clo-271-codex-validation.md:1733:@@ -62,6 +81,7 @@ impl FailureReason {
docs/reviews/clo-271-codex-validation.md:1741:@@ -88,6 +108,12 @@ impl FailureReason {
docs/reviews/clo-271-codex-validation.md:1753: impl std::fmt::Display for FailureReason {
docs/reviews/clo-271-codex-validation.md:3224:pub struct FailureReason {
docs/reviews/clo-271-codex-validation.md:3242:impl FailureReason {
docs/reviews/clo-271-codex-validation.md:3287:impl std::fmt::Display for FailureReason {

exec
/bin/zsh -lc 'tmp=$(mktemp -d); echo '"'#"'!/bin/sh'"\\nexit 0' > "'$tmp/check.sh; chmod +x $tmp/check.sh; (cd /; /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01/target/debug/deps/verify_run_command-* 2>/dev/null || true); echo $tmp' in /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
 succeeded in 0ms:

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.00s

/var/folders/w4/flvhbr_n53g0jr8r22r0hwkh0000gn/T/tmp.SjNHsr1h2O

exec
/bin/zsh -lc "rg \"pub fn which\" ~/.cargo/registry/src/index.crates.io-*/which-7*/src -n && sed -n '1,220p' ~/.cargo/registry/src/index.crates.io-*/which-7*/src/lib.rs | sed -n '1,180p'" in /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
 succeeded in 0ms:
/Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/which-7.0.3/src/lib.rs:58:pub fn which<T: AsRef<OsStr>>(binary_name: T) -> Result<path::PathBuf> {
/Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/which-7.0.3/src/lib.rs:82:pub fn which_global<T: AsRef<OsStr>>(binary_name: T) -> Result<path::PathBuf> {
/Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/which-7.0.3/src/lib.rs:87:pub fn which_all<T: AsRef<OsStr>>(binary_name: T) -> Result<impl Iterator<Item = path::PathBuf>> {
/Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/which-7.0.3/src/lib.rs:100:pub fn which_all_global<T: AsRef<OsStr>>(
/Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/which-7.0.3/src/lib.rs:145:pub fn which_re(regex: impl Borrow<Regex>) -> Result<impl Iterator<Item = path::PathBuf>> {
/Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/which-7.0.3/src/lib.rs:150:pub fn which_in<T, U, V>(binary_name: T, paths: Option<U>, cwd: V) -> Result<path::PathBuf>
/Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/which-7.0.3/src/lib.rs:184:pub fn which_re_in<T>(
/Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/which-7.0.3/src/lib.rs:195:pub fn which_in_all<'a, T, U, V>(
/Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/which-7.0.3/src/lib.rs:209:pub fn which_in_global<T, U>(
//! which
//!
//! A Rust equivalent of Unix command `which(1)`.
//! # Example:
//!
//! To find which rustc executable binary is using:
//!
//! ```no_run
//! use which::which;
//! use std::path::PathBuf;
//!
//! let result = which("rustc").unwrap();
//! assert_eq!(result, PathBuf::from("/usr/bin/rustc"));
//!
//! ```

#![forbid(unsafe_code)]

mod checker;
mod error;
mod finder;
#[cfg(windows)]
mod helper;

#[cfg(feature = "regex")]
use std::borrow::Borrow;
use std::env;
use std::fmt;
use std::path;

use std::ffi::{OsStr, OsString};

use crate::checker::CompositeChecker;
pub use crate::error::*;
use crate::finder::Finder;

/// Find an executable binary's path by name.
///
/// If given an absolute path, returns it if the file exists and is executable.
///
/// If given a relative path, returns an absolute path to the file if
/// it exists and is executable.
///
/// If given a string without path separators, looks for a file named
/// `binary_name` at each directory in `$PATH` and if it finds an executable
/// file there, returns it.
///
/// # Example
///
/// ```no_run
/// use which::which;
/// use std::path::PathBuf;
///
/// let result = which::which("rustc").unwrap();
/// assert_eq!(result, PathBuf::from("/usr/bin/rustc"));
///
/// ```
pub fn which<T: AsRef<OsStr>>(binary_name: T) -> Result<path::PathBuf> {
    which_all(binary_name).and_then(|mut i| i.next().ok_or(Error::CannotFindBinaryPath))
}

/// Find an executable binary's path by name, ignoring `cwd`.
///
/// If given an absolute path, returns it if the file exists and is executable.
///
/// Does not resolve relative paths.
///
/// If given a string without path separators, looks for a file named
/// `binary_name` at each directory in `$PATH` and if it finds an executable
/// file there, returns it.
///
/// # Example
///
/// ```no_run
/// use which::which;
/// use std::path::PathBuf;
///
/// let result = which::which_global("rustc").unwrap();
/// assert_eq!(result, PathBuf::from("/usr/bin/rustc"));
///
/// ```
pub fn which_global<T: AsRef<OsStr>>(binary_name: T) -> Result<path::PathBuf> {
    which_all_global(binary_name).and_then(|mut i| i.next().ok_or(Error::CannotFindBinaryPath))
}

/// Find all binaries with `binary_name` using `cwd` to resolve relative paths.
pub fn which_all<T: AsRef<OsStr>>(binary_name: T) -> Result<impl Iterator<Item = path::PathBuf>> {
    let cwd = env::current_dir().ok();

    Finder::new().find(
        binary_name,
        env::var_os("PATH"),
        cwd,
        CompositeChecker::new(),
        Noop,
    )
}

/// Find all binaries with `binary_name` ignoring `cwd`.
pub fn which_all_global<T: AsRef<OsStr>>(
    binary_name: T,
) -> Result<impl Iterator<Item = path::PathBuf>> {
    Finder::new().find(
        binary_name,
        env::var_os("PATH"),
        Option::<&Path>::None,
        CompositeChecker::new(),
        Noop,
    )
}

/// Find all binaries matching a regular expression in a the system PATH.
///
/// Only available when feature `regex` is enabled.
///
/// # Arguments
///
/// * `regex` - A regular expression to match binaries with
///
/// # Examples
///
/// Find Python executables:
///
/// ```no_run
/// use regex::Regex;
/// use which::which;
/// use std::path::PathBuf;
///
/// let re = Regex::new(r"python\d$").unwrap();
/// let binaries: Vec<PathBuf> = which::which_re(re).unwrap().collect();
/// let python_paths = vec![PathBuf::from("/usr/bin/python2"), PathBuf::from("/usr/bin/python3")];
/// assert_eq!(binaries, python_paths);
/// ```
///
/// Find all cargo subcommand executables on the path:
///
/// ```
/// use which::which_re;
/// use regex::Regex;
///
/// which_re(Regex::new("^cargo-.*").unwrap()).unwrap()
///     .for_each(|pth| println!("{}", pth.to_string_lossy()));
/// ```
#[cfg(feature = "regex")]
pub fn which_re(regex: impl Borrow<Regex>) -> Result<impl Iterator<Item = path::PathBuf>> {
    which_re_in(regex, env::var_os("PATH"))
}

/// Find `binary_name` in the path list `paths`, using `cwd` to resolve relative paths.
pub fn which_in<T, U, V>(binary_name: T, paths: Option<U>, cwd: V) -> Result<path::PathBuf>
where
    T: AsRef<OsStr>,
    U: AsRef<OsStr>,
    V: AsRef<path::Path>,
{
    which_in_all(binary_name, paths, cwd)
        .and_then(|mut i| i.next().ok_or(Error::CannotFindBinaryPath))
}

/// Find all binaries matching a regular expression in a list of paths.
///
/// Only available when feature `regex` is enabled.
///
/// # Arguments
///
/// * `regex` - A regular expression to match binaries with
/// * `paths` - A string containing the paths to search
///   (separated in the same way as the PATH environment variable)
///
/// # Examples
///
/// ```no_run
/// use regex::Regex;
/// use which::which;
/// use std::path::PathBuf;
///
/// let re = Regex::new(r"python\d$").unwrap();
/// let paths = Some("/usr/bin:/usr/local/bin");
/// let binaries: Vec<PathBuf> = which::which_re_in(re, paths).unwrap().collect();
/// let python_paths = vec![PathBuf::from("/usr/bin/python2"), PathBuf::from("/usr/bin/python3")];

exec
/bin/zsh -lc "nl -ba src/strategy/verify/run_command.rs | sed -n '112,132p'" in /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01
 succeeded in 0ms:
   112	    pub fn with_stderr_cap(mut self, cap: usize) -> Self {
   113	        self.stderr_cap = cap;
   114	        self
   115	    }
   116	
   117	    fn resolve_command(&self) -> Result<PathBuf, VerifyError> {
   118	        if self.cmd.is_empty() {
   119	            return Err(VerifyError::new("no command configured"));
   120	        }
   121	
   122	        let path = Path::new(&self.cmd);
   123	        if path.is_absolute() {
   124	            Ok(PathBuf::from(path))
   125	        } else {
   126	            which(&self.cmd)
   127	                .map_err(|_| VerifyError::new(format!("command not found: {}", self.cmd)))
   128	        }
   129	    }
   130	
   131	    fn build_environment(&self) -> Vec<(String, String)> {
   132	        self.env_allowlist

codex
The new RunCommand hook has functional issues around command resolution, secret environment handling, and cancellation cleanup. These can cause valid verifier commands to fail or leave subprocesses running in common usage scenarios.

Full review comments:

- [P2] Resolve relative commands against cwd — /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01/src/strategy/verify/run_command.rs:125-127
  When `cmd` is a relative path and `cwd` is set, this resolves with `which(&self.cmd)` using the orchestrator process's current directory, not the configured command cwd. A verifier like `RunCommand::new("./scripts/check").with_cwd(repo_dir)` will fail with `command not found` or resolve a different file unless the parent process happens to already be in `repo_dir`; resolve path-like commands relative to `self.cwd` or use `which_in` with the configured cwd.

- [P2] Preserve allowlisted secret env values for the child — /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01/src/strategy/verify/run_command.rs:135-139
  For allowlisted variables whose names look secret-like, this redacts the value before passing it to the subprocess. That means any verifier command that legitimately needs `GITHUB_TOKEN`, `API_KEY`, etc. receives `[REDACTED]` instead of the credential and can fail even though the caller explicitly allowlisted it; redaction should happen when building `FailureReason`, not when populating the child's environment.

- [P2] Kill spawned commands if verification is cancelled — /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01/src/strategy/verify/run_command.rs:150-155
  `tokio::process::Command` defaults to leaving the child running when `Child` is dropped. If a caller aborts this verify future or wraps it in an outer timeout shorter than `wall_timeout`, the subprocess and detached pipe-reader tasks can continue after verification is gone, violating the hook's cancellation-safety contract and potentially leaving long-running commands behind; add `kill_on_drop(true)` or an equivalent process-group cleanup guard.
The new RunCommand hook has functional issues around command resolution, secret environment handling, and cancellation cleanup. These can cause valid verifier commands to fail or leave subprocesses running in common usage scenarios.

Full review comments:

- [P2] Resolve relative commands against cwd — /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01/src/strategy/verify/run_command.rs:125-127
  When `cmd` is a relative path and `cwd` is set, this resolves with `which(&self.cmd)` using the orchestrator process's current directory, not the configured command cwd. A verifier like `RunCommand::new("./scripts/check").with_cwd(repo_dir)` will fail with `command not found` or resolve a different file unless the parent process happens to already be in `repo_dir`; resolve path-like commands relative to `self.cwd` or use `which_in` with the configured cwd.

- [P2] Preserve allowlisted secret env values for the child — /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01/src/strategy/verify/run_command.rs:135-139
  For allowlisted variables whose names look secret-like, this redacts the value before passing it to the subprocess. That means any verifier command that legitimately needs `GITHUB_TOKEN`, `API_KEY`, etc. receives `[REDACTED]` instead of the credential and can fail even though the caller explicitly allowlisted it; redaction should happen when building `FailureReason`, not when populating the child's environment.

- [P2] Kill spawned commands if verification is cancelled — /Users/mk/Code/orchestrator/loker--feat-clo-271-run-command-01/src/strategy/verify/run_command.rs:150-155
  `tokio::process::Command` defaults to leaving the child running when `Child` is dropped. If a caller aborts this verify future or wraps it in an outer timeout shorter than `wall_timeout`, the subprocess and detached pipe-reader tasks can continue after verification is gone, violating the hook's cancellation-safety contract and potentially leaving long-running commands behind; add `kill_on_drop(true)` or an equivalent process-group cleanup guard.
