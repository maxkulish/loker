# Gemini design / implementation review - CLO-271

## Context
- Branch: feat/clo-271-run-command-01
- Design: docs/designs/clo-271-run-command.md
- Plan / Spec: docs/plans/clo-271-run-command.md

## Findings
### F1 [major] Missing explicit redaction for known-secret-shaped environment variables
**Where:** `src/strategy/verify/run_command.rs:125` (and `is_secret_like_env_key` at 329)
**What:** The function `is_secret_like_env_key` is defined but entirely unused. Environment variables with secret-shaped names are passed to the child process correctly, but their specific values are not collected for explicit redaction in the captured output. The hook currently relies exclusively on the generic `utils::redact_secrets()` regex heuristics.
**Why it matters:** The plan and PRD mandate redaction for allowlisted secrets. If a user allowlists a key like `MY_API_KEY=foobar`, the literal `foobar` will leak into `FailureReason` unredacted because it does not match the AWS or bearer token regexes. The integration test only passes because it coincidentally uses an AWS key literal (`AKIA...`) that `redact_secrets()` catches via regex.
**Suggested fix:** When building the environment, collect the literal string values of any keys matching `is_secret_like_env_key()`. Ensure these specific literal strings are explicitly replaced with `[REDACTED]` in the captured stdout/stderr before flowing into `FailureReason`.

### F2 [major] Process timeout handler will hang indefinitely on Windows
**Where:** `src/strategy/verify/run_command.rs:242`
**What:** On timeout, the `Err(_)` arm calls `kill_process_group` and then immediately awaits `child.wait()`. On non-Unix platforms, `kill_process_group` is a no-op, but `child.kill().await` is never explicitly called.
**Why it matters:** Because the direct child is never signaled to die on Windows, `child.wait().await` will block indefinitely if the timed-out process is an infinite loop or hung network call. This completely defeats the wall-clock timeout safety net on non-Unix platforms.
**Suggested fix:** Add `let _ = child.kill().await;` immediately after `kill_process_group(child.id());` to ensure the direct child process is terminated universally before awaiting its exit status.

### F3 [nit] Redundant double-redaction of captured output
**Where:** `src/strategy/verify/run_command.rs:375-376`
**What:** `CapturedOutput::to_reason_text()` internally calls `redact_secrets(&text)`, but the caller in `verify()` wraps the return value in another `redact_secrets()` call.
**Why it matters:** This causes double-redaction of the stdout and stderr strings. While harmless and idempotent, it is redundant and wastes regex evaluation CPU cycles.
**Suggested fix:** Remove the outer `redact_secrets()` wrappers in `verify()` (i.e., change `let stdout = redact_secrets(&run.stdout.to_reason_text());` to `let stdout = run.stdout.to_reason_text();`).

## Strengths
- Clean module refactoring of `verify.rs` into a directory module with zero disruption.
- Bounded stdout/stderr capture is well-implemented using async I/O and proper drain logic to prevent pipe deadlocks.
- Exhaustive and high-quality integration tests covering process groups, truncation markers, and signals.
- Safely implemented Unix `RLIMIT_CPU` using `pre_exec` with appropriate scoping and safety comments.

## Verdict
approve_with_changes

The implementation closely adheres to the design document and handles async subprocess execution securely. However, two critical omissions require fixing before merge: the hook fails to explicitly redact arbitrary secret values from `env_allowlist` (relying instead on fragile regex heuristics that let non-standard secrets leak), and the timeout handler fails to invoke `child.kill()` on non-Unix platforms, leading to an indefinite hang. Correct these two oversights, clean up the redundant double-redaction, and the task will be complete.
