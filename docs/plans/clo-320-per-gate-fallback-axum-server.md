# Plan: CLO-320 — Per-gate fallback axum server for HITL approval

## Context
- **Design:** `docs/designs/clo-320-per-gate-fallback-axum-server.md`
- **Discovery:** `docs/discovery/clo-320.md`
- **PRD:** `docs/prds/clo-320-per-gate-fallback-axum-server.md`
- **Linear:** https://linear.app/cloud-ai/issue/CLO-320/t-051-per-gate-fallback-axum-server-for-hitl-approval
- **Branch:** `feat/clo-320-fallback`
- **Depends on:** CLO-317 (T-048 HumanVerifier scaffold), CLO-318 (T-049 severity ladder), CLO-319 (T-050 advisory lock)

## Sub-tasks

### ST1 Add axum + tower to Cargo.toml
**Files:** `Cargo.toml`
**What:** Add `axum = "0.7"` (or latest compatible) and `tower = "0.5"` to `[dependencies]`. Ensure `tokio` features are sufficient (`full` already covers `rt-multi-thread`, `net`, `sync`, `time`).
**Acceptance:** `cargo check` compiles with new deps, `clippy` green.
**Estimate:** S

### ST2 Scaffold `src/hitl_server/` module with types
**Files:** `src/hitl_server/mod.rs`, `src/hitl_server/mod.rs` (lib.rs re-export)
**What:** Create the module directory and root file. Define `GateConfig`, `ServerOutcome`, `ServerError` per the design §4. Add `mod hitl_server;` to `src/lib.rs` or re-export from `src/main.rs` as appropriate (check existing pattern).
**Acceptance:** `cargo check` compiles. Module-level unit tests for `GateConfig` construction and `ServerOutcome` round-trips pass.
**Estimate:** S

### ST3 Implement route handlers (HTML rendering + POST approve/reject)
**Files:** `src/hitl_server/routes.rs`
**What:** Implement `gate_context`, `approve`, `reject` handlers. `approve` and `reject` acquire `PhaseLock`, atomically write `responses/<phase>.json`, then send `ServerOutcome` through `tokio::sync::oneshot`. HTML form renders gate context from `pending/<phase>.json`. HTTP status codes: 200 (success), 423 (locked), 409 (already exists), 500 (unexpected).
**Acceptance:** `cargo test hitl_server::routes::tests` passes (mock `PhaseLock`, assert HTML contains phase name, assert handlers return correct status codes for locked/already-exists paths).
**Estimate:** M

### ST4 Implement one-shot server bootstrap
**Files:** `src/hitl_server/one_shot.rs`
**What:** `start(config: GateConfig) -> Result<(SocketAddr, ServerHandle), ServerError>`. Bind to `127.0.0.1:0`, build axum router from `routes::router`, spawn server task with graceful shutdown via `tokio::sync::oneshot`. `ServerHandle::outcome()` awaits the decision/timed-out/cancelled signal.
**Acceptance:** `cargo test hitl_server::one_shot::tests` passes (binds to free port, shuts down within 1s after decision signal, shuts down within 1s on handle drop).
**Estimate:** M

### ST5 Wire fallback path into HumanVerifier
**Files:** `src/strategy/verify/human_verifier.rs`
**What:** Add `fallback_server: bool` (default `false`) to `HumanVerifierConfig`. In `verify_with_report`, when `fallback_server = true` and no response exists: write pending file → build `GateConfig` → call `hitl_server::one_shot::start` → print URL → `tokio::select!` between server outcome and timeout → re-enter existing `verify_with_report` path (which will find the response file or apply timeout).
**Acceptance:** All existing `human_verifier.rs` unit tests pass unchanged. New test `verify_with_report_fallback_server_starts_and_awaits` passes.
**Estimate:** M

### ST6 Integration tests for HITL server
**Files:** `tests/hitl_server.rs`
**What:** Full end-to-end integration tests using `reqwest` (already in deps, or axum `TestClient`). Test: approve resolves gate, reject resolves gate, concurrent POST races return 423, second POST after first returns 409, gate context shows pending JSON, server URL printed to stdout, timeout auto-approves (low severity with 1ms timeout), high severity blocks indefinitely until cancellation.
**Acceptance:** `cargo test --test hitl_server` passes.
**Estimate:** M

### ST7 Pre-merge regression gate
**Files:** all
**What:** Run `make check` (fmt + clippy + test). Verify all existing HumanVerifier tests still pass. Verify `cargo test --workspace` passes. No warnings or test failures.
**Acceptance:** `make check` exits 0.
**Estimate:** S

## Dependency DAG

```
ST1 (Cargo deps)
  │
  ▼
ST2 (Module scaffold)
  │
  ▼
ST3 (Routes) ──► ST4 (One-shot) ──► ST5 (HumanVerifier wiring) ──► ST6 (Integration tests)
                                                                  │
                                                                  ▼
                                                                ST7 (make check)
```

ST3 and ST4 are parallel in theory but axum routers require route handlers, so ST3 must compile before ST4 can be fully exercised. For the plan, treat as sequential: ST2 → ST3 → ST4 → ST5 → ST6 → ST7.

## Pre-merge gate
- `make check` (fmt + clippy + test)
- All existing tests unchanged (no regressions in human_verifier, phase_runner, phase_lock, trace, markers)
- Integration test `hitl_server.rs` passes

## Risks

| Risk | Mitigation |
|------|-----------|
| axum version 0.7 vs 0.8 breaking API changes | Pin `"0.7"` in Cargo.toml to match existing ecosystem; upgrade in M11 if needed. |
| `PhaseLock::acquire` is sync, route handlers are async — blocking in async handler may stall other requests | Acceptable for v0: the server is one-shot with exactly 1 concurrent POST target. `PhaseLock::acquire` is fast (file open + `try_lock_exclusive`), and routes are localhost-only. |
| Binary size increase from axum (~2MB estimated) | Documented in design non-goals; `fallback_server = false` by default so unused code is dead-stripped by linker in normal builds. |
| Ctrl-C during gate loses the server but leaves pending file | Documented in design: `loker resume` will re-spawn the server. No data loss. |
| HTML form injection (XSS) via pending JSON fields | All fields rendered through `html_escape()` which escapes `& < > "`. No unescaped interpolation. |

## Notes

- **Additive only** — no changes to existing `pending.schema.json`, `response.schema.json`, marker schemas, manifest layout.
- **Default-off** — `HumanVerifierConfig::fallback_server` defaults to `false`; all existing tests and workflows behave identically.
- **Shared routes** — `src/hitl_server/routes.rs` is written so T-052 / CLO-321 (daemon mode) can import it directly into its axum router.
