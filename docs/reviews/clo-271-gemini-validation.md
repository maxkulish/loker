# Gemini design / implementation review - CLO-271

## Context
- Branch: feat/clo-271-run-command
- Design: docs/designs/clo-270-hook.md
- Plan / Spec: docs/status/clo-271-workflow.yaml

## Findings
### F1 [blocker] Deadlock in stdout/stderr stream reading
**Where:** `src/strategy/run_command.rs` (around line 243)
**What:** `read_stream` is awaited sequentially for stdout and then stderr. If the spawned child process writes a large amount of data to stderr (exceeding the OS pipe buffer size) before closing stdout, it will block on `write(2)`. The parent will concurrently be stuck awaiting `stdout`.
**Why it matters:** This will cause a deadlock until the wall-clock timeout expires, leading to flaky failures or broken verifications whenever hooks emit significant stderr output.
**Suggested fix:** Use `tokio::join!` to consume stdout and stderr concurrently:
```rust
let ((stdout_bytes, stdout_capped), (stderr_bytes, stderr_capped)) = tokio::join!(
    read_stream(child.stdout.take(), stdout_cap),
    read_stream(child.stderr.take(), stderr_cap)
);
```

### F2 [major] Sandbox bypass for inherited working directory
**Where:** `src/strategy/run_command.rs` (around line 202)
**What:** When `cwd` is `None`, the code skips calling `command.current_dir()`. By default, `tokio::process::Command` implicitly inherits the working directory of the parent orchestrator process.
**Why it matters:** The sandboxing constraint explicitly mandates: "Must be set explicitly; never inherits the orchestrator's cwd implicitly."
**Suggested fix:** Explicitly set the directory to a safe default (like `"/"`) when `cwd` is `None`:
```rust
if let Some(dir) = cwd {
    command.current_dir(dir);
} else {
    command.current_dir("/");
}
```

### F3 [major] Cancellation drop safety leaks process groups
**Where:** `src/strategy/run_command.rs` (around line 20 and line 242)
**What:** The module doc claims "If the future is dropped mid-execution ... the spawned child process is reaped". However, if the `verify()` future is dropped externally (e.g., the phase runner cancels the step), the local `child` drops without reaping. Standard `Child` drop does not kill the process, nor does it send signals to the PGID.
**Why it matters:** Cancelling a workflow step will leak orphaned child processes and process groups on the host, consuming resources indefinitely.
**Suggested fix:** Introduce a custom RAII Drop guard to ensure `kill_process_group` is reliably called when the future is dropped:
```rust
struct ChildGuard(tokio::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        kill_process_group(&self.0);
    }
}
// Wrap the spawn result and use the guard.
```

### F4 [minor] Sub-optimal ergonomics for builder methods
**Where:** `src/strategy/run_command.rs` (lines 107, 115)
**What:** `with_args` and `with_env_allowlist` accept `Vec<String>`, forcing heap allocations at the call site.
**Why it matters:** Inconvenient to use in the broader codebase compared to standard Rust builder patterns.
**Suggested fix:** Accept `impl IntoIterator<Item = impl Into<String>>`:
```rust
pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
    self.args = args.into_iter().map(Into::into).collect();
    self
}
```

### F5 [nit] Unreachable code for None exit status
**Where:** `src/strategy/run_command.rs` (around line 340)
**What:** The match arm `None => { ... }` for `result.exit_status` is unreachable because `execute_command` only ever returns `exit_status: None` when `timed_out` is `true`, and the timeout case is handled and returned immediately prior to this match.
**Why it matters:** Unnecessary defensive code that complicates the match block.
**Suggested fix:** Remove the `None` branch or refactor `RunResult` so `exit_status` is an unwrapped value if the command didn't time out.

## Strengths
- Detailed module documentation that explicitly defines the sandboxing threat model.
- Solid test coverage for output truncation, environment variable whitelisting, and secret redaction.
- Clean separation of the core `execute_command` function from the `VerifyHook` implementation, which improves testability.

## Verdict
rework

The implementation has solid foundations, but the sequential stdout/stderr reading deadlock and the process group leak on future cancellation violate critical correctness and sandboxing invariants. Addressing these concurrent process management flaws is required before merging.
