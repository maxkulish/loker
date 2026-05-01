# PRD: CLO-271 — Implement RunCommand verify hook with sandboxing

| Field | Value |
|-------|-------|
| Source | Linear issue [CLO-271](https://linear.app/cloud-ai/issue/CLO-271/implement-runcommand-verify-hook-with-sandboxing) |
| PRD Reference | FR-14 (shell command verify hook gates retries with structured failure reasons) |
| Design doc | See `docs/designs/clo-270-hook.md` for trait architecture |
| Security | PRD §5 — sandboxing NFRs (cwd, env, wall/cpu timeout, cap, signal cleanup, redaction) |

## Scope

- `RunCommand` struct: cmd, args, env_allowlist, cwd, wall_timeout, cpu_timeout, stdout_cap, stderr_cap
- `impl VerifyHook for RunCommand` — maps exit-0 → `Pass`, non-zero → `Fail { reason: FailureReason }`
- Structured `FailureReason` with exit code, signal, truncated stderr tail, sandbox-violation discriminant
- Secret redaction for allowlisted vars with known-secret-shaped names
- Process group cleanup on timeout/cancel
- Tests in `tests/verify_run_command.rs`

## Acceptance criteria

- [ ] `cargo test --test verify_run_command` is green
- [ ] No clippy warnings on the new module
- [ ] PRD FR-14 satisfied: shell command verify hook gates retries with structured failure reasons

## Dependencies

- Blocked by: CLO-270 (VerifyHook trait + VerifyResult enum) — **done**
- Blocks: T-023 (TestRunner reuses RunCommand internals), T-029 (phase runner)
