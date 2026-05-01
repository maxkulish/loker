# Gemini design review - CLO-271

## Context
- Branch: feat/clo-271-run-command-01
- Design: docs/designs/clo-271-run-command.md
- PRD: docs/prds/clo-271-run-command.md
- Discovery: docs/discovery/clo-271.md

## Findings

### F1 [minor] `cpu_timeout` via `setrlimit(RLIMIT_CPU)` needs `unsafe` `pre_exec`
**Where:** design doc §Public API, `cpu_timeout` field
**What:** The `RLIMIT_CPU` rlimit must be set in a `pre_exec` hook on `std::process::Command`, which requires `unsafe` (inherently unsafe per Rust docs). The design doc mentions `libc::setrlimit` but doesn't show *where* in the spawn lifecycle this happens.
**Why it matters:** Missing this detail could lead to an implementer placing the rlimit call after spawn (where it affects the parent process instead of the child) or avoiding `unsafe` and shipping a broken feature.
**Suggested fix:** Add an explicit note in the architecture data-flow step (c): "CPU limit is applied via `unsafe { cmd.pre_exec(|| { libc::setrlimit(...); Ok(()) }) }` before spawn. This is inherently unsafe and requires `#[allow(unsafe_code)]` scoped to the run_command module."

### F2 [minor] Process group setup differs Unix vs Windows
**Where:** design doc §Architecture, data flow step (c)
**What:** The design says "set process group (Unix: setpgid for signal routing)" but doesn't specify the Windows path. `tokio::process::Command` has `process_group(0)` on Unix but not on Windows.
**Why it matters:** The code will compile on Windows but the process-group-kill guarantee won't hold. This is fine for v0 (documented non-goal) but should be explicit.
**Suggested fix:** Add to non-goals: "Process group cleanup on Windows is best-effort; full guarantee requires `CREATE_NEW_PROCESS_GROUP` which is deferred."

### F3 [major] Missing `which` / PATH resolution detail
**Where:** design doc §Architecture, data flow step (a)
**What:** The design says "Resolve cmd from PATH" but doesn't specify *how*. The `which` crate is already a dependency in `Cargo.toml`.
**Why it matters:** If `cmd` is a relative path (e.g., `cargo`), we need `which::which(&self.cmd)` before spawning. If it's absolute (e.g., `/usr/bin/cargo`), we skip resolution. Without this detail, the implementer may forget path resolution, producing a `VerifyError::NotFound` that's misleading when the command actually exists on PATH.
**Suggested fix:** Add to data flow step (a): "Use `which::which(&self.cmd)` for bare command names; skip resolution for absolute paths. If resolution fails, return `VerifyError::CommandNotFound` (new variant)."

### F4 [minor] No `VerifyError` variant for "command not found"
**Where:** design doc §Public API, `VerifyHook` impl
**What:** The existing `VerifyError` from CLO-270 likely doesn't have a "command not found" variant. The design doc should call out whether we add one or reuse an existing variant.
**Why it matters:** Without a specific variant, the error will be a generic `VerifyError` with a string, losing structured diagnostics.
**Suggested fix:** Propose adding `VerifyError::CommandNotFound { cmd: String }` to the `VerifyError` enum. Since this touches CLO-270's type, document it as a small additive change with no breaking impact (new enum variant on `#[non_exhaustive]` type).

### F5 [nit] `SecretRedactor` naming vs `redact_secrets` in `escalating_retry.rs`
**Where:** design doc §Public API, `SecretRedactor` helper
**What:** `escalating_retry.rs` already has a `redact_secrets()` function. The design doc introduces `SecretRedactor` as a new helper. Two secret-redaction implementations is a recipe for drift.
**Why it matters:** If the regexes diverge, secrets might leak in one path but not the other.
**Suggested fix:** Either (a) extract the existing `redact_secrets()` into a shared `crate::utils::redact_secrets()` and have `RunCommand` call it, or (b) explicitly state that `RunCommand` uses a *different* redaction scope (env values only, not arbitrary text) so duplication is intentional.

### F6 [minor] `SandboxViolation` enum — should it be on `FailureReason`?
**Where:** design doc §Architecture, type taxonomy
**What:** The design proposes `SandboxViolation` as a separate enum. But `FailureReason` (from CLO-270) already has `exit_code: Option<i32>` and `truncated: bool`. Adding a new enum requires extending `FailureReason`.
**Why it matters:** This is a small but real change to CLO-270's type. Need to verify `FailureReason` is `#[non_exhaustive]` or that adding a field won't break consumers.
**Suggested fix:** Check `src/strategy/verify.rs` for `#[non_exhaustive]` on `FailureReason`. If present, add `sandbox_violation: Option<SandboxViolation>` field. If not, make it `#[non_exhaustive]` first (non-breaking for downstream since the enum is not publicly matched in v0).

### F7 [minor] Integration test #8 (CPU-limited infinite loop) is platform-dependent
**Where:** design doc §Test plan, integration test #8
**What:** Test #8 requires `RLIMIT_CPU` which is Unix-only. The test will fail on macOS CI if the platform doesn't support it, or will be skipped.
**Why it matters:** A skipped test in CI is fine but should be documented as `#[cfg(unix)]` or `#[cfg(target_os = "linux")]`.
**Suggested fix:** Mark test #8 with `#[cfg(unix)]` and add a comment: "Skipped on Windows; CPU limiting is Unix-only in v0."

## Strengths
- Clean module boundary: `run_command.rs` lives alongside `llm_verifier.rs` under `src/strategy/verify/`, mirroring the existing pattern.
- Default-deny env with allowlist is the right security posture for a shell-out primitive.
- Builder pattern (`with_args`, `with_env_allowlist`, etc.) is ergonomic and idiomatic.
- Process group cleanup on timeout prevents orphaned processes — a common bug in naive async process wrappers.
- Bounded reads with drain on truncation prevent writer-side blocking — learned directly from `verification.rs` and correctly carried forward.
- No breaking changes to existing code; additive-only public surface.

## Verdict
approve_with_suggestions

The design is solid, well-scoped, and architecturally sound. The 7 findings are all minor or nit-level; none are blockers. The most important fix is F3 (PATH resolution with `which` crate) — without it, the hook will be unreliable for common commands like `cargo test`. F1 and F6 should be addressed during implementation to avoid type-level surprises. F5 (redaction duplication) is worth a follow-up issue but not a blocker for this task.

## Actionable Feedback (prioritized)

1. **Add PATH resolution detail** — use `which` crate for bare command names, skip for absolute paths. Add `VerifyError::CommandNotFound` variant.
2. **Document `unsafe` `pre_exec` for `RLIMIT_CPU`** — add explicit note in architecture data flow.
3. **Verify `FailureReason` is `#[non_exhaustive]`** — add `sandbox_violation` field if so; otherwise make it `#[non_exhaustive]` first.
4. **Clarify redaction scope** — state whether `RunCommand` reuses `escalating_retry.rs`'s `redact_secrets()` or has its own env-specific redactor.
5. **Mark CPU-limit test as Unix-only** — `#[cfg(unix)]` on integration test #8.
6. **Document Windows process-group limitation** — add one sentence to non-goals.
