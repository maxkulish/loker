# PRD: CLO-320 — Per-gate fallback axum server for HITL approval

| Field | Value |
|-------|-------|
| Author | pi (discovery phase) |
| Status | Draft |
| Created | 2026-05-06 |
| Task | CLO-320 |
| Depends on | CLO-317 (T-048 HumanVerifier scaffold), CLO-318 (T-049 severity ladder), CLO-319 (T-050 advisory lock) |

## 1. Goal

Provide a minimal per-gate axum HTTP server that lets a human approve or reject a paused `HumanVerifier` gate without needing the full M11 UI daemon. The server is one-shot (serves exactly one gate) and localhost-only. When a gate triggers, the server binds to a free port, prints a clickable URL, blocks until a decision arrives, writes the response through the advisory lock, and shuts down.

## 2. Scope

### In scope
- One-shot axum server scoped to a single pending gate, binding to `127.0.0.1:0` (free port).
- Routes:
  - `GET /` — returns gate context (pending JSON rendered as minimal HTML with Approve / Reject buttons).
  - `POST /approve` — accepts optional `comment` form field, writes `responses/<phase>.json` with `decision: Approve`.
  - `POST /reject` — accepts optional `comment` form field, writes `responses/<phase>.json` with `decision: Reject`.
- Server acquires `PhaseLock` before writing the response file (first-write-wins).
- Server exits cleanly after the first successful POST or after the gate timeout expires.
- URL printed on stdout when the gate triggers, before the server starts accepting connections.
- Shared route handlers module so M11 daemon mode (T-052) can reuse 100% of the logic.
- Integration tests using `reqwest` client against the spawned server.
- `make check` green.

### Out of scope (deferred to T-052 / M11)
- Multi-run / multi-tenant daemon mode.
- Authentication beyond loopback binding.
- Styled UI or artefact preview rendering (minimal HTML form is sufficient).
- SSE / real-time updates.
- Session list or trace viewer.

## 3. Acceptance criteria

1. A workflow with `verify = "human"` and no pre-existing response file prints a `http://127.0.0.1:<port>/` URL when the gate triggers.
2. Opening the URL in a browser shows the gate context and Approve / Reject buttons.
3. Clicking Approve writes `responses/<phase>.json` with `decision: Approve` and the run continues.
4. Clicking Reject writes `responses/<phase>.json` with `decision: Reject` and the run fails with a clear message.
5. Two concurrent POSTs to the same gate result in exactly one winner; the loser sees an HTTP 423 (Locked) or 409 (Conflict) response.
6. Decision is durable in the run's trace (`loker.hitl.*` fields) and status markers.
7. Server shuts down within 1 second of the first successful POST.
8. If the gate times out before human interaction, the server shuts down and the timeout action (auto-approve / auto-fail / block) is applied.
9. `make check` passes (fmt + clippy + test).

## 4. Design direction

### 4.1 Module layout

```
src/
  hitl_server/
    mod.rs           # Re-exports
    routes.rs        # Shared route handlers (GET /, POST /approve, POST /reject)
    one_shot.rs      # One-shot server bootstrap: bind, spawn, shutdown
  strategy/verify/
    human_verifier.rs  # Extended to optionally spawn the one-shot server and block
```

### 4.2 Route handlers (shared with M11)

```rust
// src/hitl_server/routes.rs
use axum::{extract::Form, response::Html, routing::{get, post}, Router};
use serde::Deserialize;

#[derive(Deserialize)]
struct DecisionForm {
    comment: Option<String>,
}

pub fn router(config: GateConfig) -> Router {
    Router::new()
        .route("/", get(gate_context_handler))
        .route("/approve", post(approve_handler))
        .route("/reject", post(reject_handler))
        .with_state(config)
}
```

The handlers:
- Read `pending/<phase>.json` to render context.
- On POST, acquire `PhaseLock`, write `responses/<phase>.json` in the correct schema, release lock.
- Notify the blocked `HumanVerifier` via a `tokio::sync::oneshot::Sender`.

### 4.3 One-shot server bootstrap

```rust
// src/hitl_server/one_shot.rs
pub async fn serve(
    gate_config: GateConfig,
    shutdown_rx: tokio::sync::oneshot::Receiver<HumanDecision>,
) -> Result<std::net::SocketAddr, ServerError>;
```

- Binds to `127.0.0.1:0`.
- Returns the actual socket address so the caller can print the URL.
- Runs the axum server until `shutdown_rx` receives a decision or a timeout signal.
- Graceful shutdown on drop or signal.

### 4.4 HumanVerifier integration

Add an optional `fallback_server: bool` to `HumanVerifierConfig`. When `true` and no response exists:

1. Write `pending/<phase>.json` (existing behaviour).
2. Spawn the one-shot server via `hitl_server::one_shot::serve`.
3. Print URL to stdout (via `println!` or a configurable output sink).
4. `tokio::select!` between:
   - `shutdown_rx` receiving a decision from the server
   - `timeout` firing based on `timeout_policy.rule_for(severity)`
5. If server resolves first: read the response file, consume it, return mapped `VerifyResult`.
6. If timeout fires first: apply `HumanTimeoutAction` (auto-approve / auto-fail / block), return mapped `VerifyResult`.

### 4.5 Concurrent approval protection

The server's POST handlers acquire `PhaseLock` before writing. If `PhaseLockError::LockInUse` is returned, the handler responds with HTTP 423 (Locked). If the lock is acquired but the response file already exists (race between two POSTs where one finished just before the other), respond with HTTP 409 (Conflict).

### 4.6 Minimal HTML form

```html
<!doctype html>
<title>Gate: {phase}</title>
<h1>{phase} — {workflow}</h1>
<p>Severity: {severity}</p>
<p>Artefact: {artefact_path}</p>
<form method="post" action="/approve">
  <textarea name="comment" placeholder="Optional comment"></textarea>
  <button type="submit">Approve</button>
</form>
<form method="post" action="/reject">
  <textarea name="comment" placeholder="Optional comment"></textarea>
  <button type="submit">Reject</button>
</form>
```

No CSS, no JS. Pure HTML5. This satisfies the "clickable URL" acceptance criterion without requiring M11 UI assets.

## 5. Test plan

### Unit tests (`src/hitl_server/one_shot.rs`)
- `binds_to_free_port` — `serve` returns a valid localhost address with port > 0.
- `shuts_down_after_shutdown_signal` — spawn server, send decision via `shutdown_tx`, assert server task completes within 1s.

### Integration tests (`tests/hitl_server.rs`)
- `one_shot_approve_resolves_gate` — spawn server with temp run_dir, POST /approve, assert response file exists and schema is valid.
- `one_shot_reject_resolves_gate` — POST /reject, assert response file exists with `decision: Reject`.
- `concurrent_approve_races_return_423` — two simultaneous POSTs, one wins with 200, the other gets 423.
- `gate_context_shows_pending_json` — GET / returns HTML containing the phase name and artefact path.
- `server_url_printed_to_stdout` — capture stdout during verify, assert URL pattern `http://127.0.0.1:\d+/` is present.
- `timeout_auto_approves_without_human` — low severity with 1ms timeout, server starts, no POST arrives, assert auto-approve result after timeout.

### Regression
- `cargo test` all existing HumanVerifier tests still pass.
- `make check` (fmt + clippy + test) passes.

## 6. Migration / rollout

- Additive only: new `src/hitl_server/` module, new dependencies (`axum`, `tower`).
- No changes to marker schema, manifest schema, or run directory layout.
- `HumanVerifierConfig` gains an optional `fallback_server: bool` field; default `false` preserves existing behaviour.
- No feature flag required; the server is only spawned when the config field is true.

## 7. Security / threat model

- **Binding:** `127.0.0.1` only. No `0.0.0.0`. This is enforced in code, not just config.
- **CSRF:** Not a concern for v0. The server is one-shot, localhost-only, and serves exactly one gate. No session cookies, no stateful auth.
- **Path traversal:** Phase names are validated by `PhaseLock::validate_phase_name` (rejects `/`, `\`, `\0`, `.`, `..`).
- **Symlink attack:** `PhaseLock::acquire` already validates that `run_dir` is a real directory (not a symlink).
- **Concurrent race:** `PhaseLock` + atomic write of response file prevents double-approval.
- **DoS:** A malicious local process could hold the `PhaseLock` indefinitely. Mitigation: 60s TTL with PID-liveness check (Unix) or timestamp check (Windows). The attacker would need to hold the OS advisory lock, which is detectable via `loker ls --blocked` (T-044).
