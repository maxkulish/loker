# Design: CLO-320 — Per-gate fallback axum server for HITL approval

## 1. Problem

Per the discovery report (`docs/discovery/clo-320.md`), operators using `HumanVerifier` gates in loker workflows have no interactive path to approve or reject a paused phase. Today, `HumanVerifier::verify_with_report` writes `pending/<phase>.json` and returns `VerifyResult::Fail` when no response file exists. The loker process exits; the operator must hand-author `responses/<phase>.json` with the correct schema and run `loker resume`. This is unacceptable for a human-in-the-loop workflow — the operator should be able to click a URL printed to stdout, make a decision in a browser, and have the run continue automatically. T-048 (HumanVerifier scaffold), T-049 (severity ladder), and T-050 (advisory lock) are all implemented. This design closes the gap by adding a one-shot axum server that binds to a free localhost port, serves a minimal HTML form, writes the response through the advisory lock, and signals the blocked `HumanVerifier` to proceed.

## 2. Goals / Non-goals

### Goals

- Add a `src/hitl_server/` module with shared route handlers and one-shot server bootstrap.
- Spawn a one-shot axum server on `127.0.0.1:0` when a `HumanVerifier` gate triggers with no pre-existing response.
- Print the `http://127.0.0.1:<port>/` URL to stdout before the server accepts connections.
- Serve three routes: `GET /` (gate context as minimal HTML), `POST /approve`, `POST /reject`.
- Acquire `PhaseLock` before writing `responses/<phase>.json`; reject concurrent POSTs with HTTP 423.
- Block `HumanVerifier::verify_with_report` until the server signals a decision or the gate timeout fires.
- Server shuts down cleanly within 1 second of the first successful POST.
- Route handlers are 100% reusable by M11 daemon mode (T-052 / CLO-321).
- `make check` green; unit + integration tests cover all acceptance criteria.

### Non-goals

- Multi-run / multi-tenant daemon mode (T-052).
- Authentication, session cookies, or CSRF tokens (localhost-only v0).
- Styled UI, CSS, JavaScript, or artefact preview rendering (M11).
- SSE / real-time updates (M11).
- `loker ls --blocked` enumeration (T-044; consumes existing marker data).
- Changing the pending/response JSON schema, marker schema, or manifest schema.

## 3. Architecture

### Module layout

```
src/
  hitl_server/
    mod.rs           # Re-exports GateConfig, ServerError, route helpers
    routes.rs        # Shared axum route handlers (GET /, POST /approve, POST /reject)
    one_shot.rs      # One-shot server: bind, spawn, graceful shutdown
  strategy/verify/
    human_verifier.rs  # Extended with optional fallback_server path
```

### Data types

```rust
// src/hitl_server/mod.rs
use std::path::PathBuf;
use std::net::SocketAddr;
use tokio::sync::oneshot;

/// Configuration for a single gate server instance.
#[derive(Debug, Clone)]
pub struct GateConfig {
    pub run_dir: PathBuf,
    pub run_id: String,
    pub phase: String,
    pub workflow: String,
    pub severity: String,
    pub artefact_path: String,
    pub artefact_kind: String,
    pub prompt_summary: String,
    pub preview_lines: u32,
    pub timeout_at: Option<String>,
    pub decision_options: Vec<String>,
}

/// Result of running the one-shot server.
#[derive(Debug)]
pub enum ServerOutcome {
    /// A human decision was received via POST.
    Decided { decision: HumanDecision, comment: Option<String> },
    /// The gate timed out before any POST arrived.
    TimedOut,
    /// The server was cancelled (e.g. Ctrl-C, parent drop).
    Cancelled,
}

/// Error emitted by the one-shot server bootstrap.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("failed to bind to 127.0.0.1:0: {0}")]
    Bind(std::io::Error),
    #[error("failed to read pending file: {0}")]
    PendingRead(#[from] std::io::Error),
    #[error("lock contention: {0}")]
    Lock(#[from] crate::run_state::phase_lock::PhaseLockError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
```

### One-shot server bootstrap

```rust
// src/hitl_server/one_shot.rs
use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Bind to a free localhost port, spawn the axum server, and return the
/// address + a future that resolves when the server shuts down.
///
/// The server runs until `cancel_rx` fires, `decision_tx` sends a decision,
/// or the caller drops the returned handle.
pub async fn start(
    config: GateConfig,
) -> Result<(SocketAddr, ServerHandle), ServerError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(ServerError::Bind)?;
    let addr = listener.local_addr().map_err(ServerError::Bind)?;

    let (decision_tx, decision_rx) = oneshot::channel::<ServerOutcome>();
    let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

    let app = routes::router(config).with_state((decision_tx, cancel_tx));

    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                // Wait for either a decision or a cancellation signal
                let _ = cancel_rx.await;
            })
            .await
    });

    Ok((addr, ServerHandle { handle, decision_rx }))
}

/// Handle to the running one-shot server. Await `outcome()` to get the
/// result, or drop to trigger graceful shutdown.
pub struct ServerHandle {
    handle: tokio::task::JoinHandle<std::result::Result<(), std::io::Error>>,
    decision_rx: oneshot::Receiver<ServerOutcome>,
}

impl ServerHandle {
    /// Block until the server resolves (decision, timeout, or cancellation).
    pub async fn outcome(self) -> ServerOutcome {
        match self.decision_rx.await {
            Ok(outcome) => outcome,
            Err(_) => ServerOutcome::Cancelled,
        }
    }
}
```

### Route handlers

```rust
// src/hitl_server/routes.rs
use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::Html,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use tokio::sync::oneshot::Sender;

#[derive(Deserialize)]
struct DecisionForm {
    comment: Option<String>,
}

/// State passed to every route: (decision_sender, cancel_sender).
type RouteState = (Sender<ServerOutcome>, Sender<()>);

pub fn router(config: GateConfig) -> Router<RouteState> {
    Router::new()
        .route("/", get(gate_context))
        .route("/approve", post(approve))
        .route("/reject", post(reject))
        .with_state(config)
}

async fn gate_context(
    State(config): State<GateConfig>,
) -> Result<Html<String>, StatusCode> {
    let pending_path = config.run_dir.join("pending").join(format!("{}.json", config.phase));
    let body = match tokio::fs::read_to_string(&pending_path).await {
        Ok(json) => render_html(&config, &json),
        Err(_) => render_html_fallback(&config),
    };
    Ok(Html(body))
}

async fn approve(
    State((decision_tx, _cancel_tx)): State<RouteState>,
    State(config): State<GateConfig>,
    Form(form): Form<DecisionForm>,
) -> StatusCode {
    match write_response(&config, HumanDecision::Approve, form.comment).await {
        Ok(()) => {
            let _ = decision_tx.send(ServerOutcome::Decided {
                decision: HumanDecision::Approve,
                comment: form.comment,
            });
            StatusCode::OK
        }
        Err(ResponseWriteError::Locked) => StatusCode::LOCKED,   // 423
        Err(ResponseWriteError::AlreadyExists) => StatusCode::CONFLICT, // 409
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn reject(
    State((decision_tx, _cancel_tx)): State<RouteState>,
    State(config): State<GateConfig>,
    Form(form): Form<DecisionForm>,
) -> StatusCode {
    match write_response(&config, HumanDecision::Reject, form.comment).await {
        Ok(()) => {
            let _ = decision_tx.send(ServerOutcome::Decided {
                decision: HumanDecision::Reject,
                comment: form.comment,
            });
            StatusCode::OK
        }
        Err(ResponseWriteError::Locked) => StatusCode::LOCKED,
        Err(ResponseWriteError::AlreadyExists) => StatusCode::CONFLICT,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
```

### Response write helper

```rust
// src/hitl_server/routes.rs (internal)

use crate::run_state::phase_lock::{PhaseLock, PhaseLockError};
use crate::run_state::atomic_write;
use crate::strategy::verify::{HumanDecision, HumanResponse};

#[derive(Debug)]
enum ResponseWriteError {
    Locked,
    AlreadyExists,
    Io(std::io::Error),
    Json(serde_json::Error),
}

async fn write_response(
    config: &GateConfig,
    decision: HumanDecision,
    comment: Option<String>,
) -> Result<(), ResponseWriteError> {
    // 1. Acquire the per-phase advisory lock
    let _lock = PhaseLock::acquire(&config.run_dir, &config.phase, &config.run_id, None)
        .map_err(|e| match e {
            PhaseLockError::LockInUse { .. } => ResponseWriteError::Locked,
            other => ResponseWriteError::Io(other.into()),
        })?;

    // 2. Check if response already exists (race between two POSTs)
    let response_path = config.run_dir.join("responses").join(format!("{}.json", config.phase));
    if response_path.exists() {
        return Err(ResponseWriteError::AlreadyExists);
    }

    // 3. Build and write the response
    let response = HumanResponse {
        schema_version: 1,
        phase: config.phase.clone(),
        claimed_by: "hitl_server".into(),
        decided_at: chrono::Utc::now().to_rfc3339(),
        decision,
        global_comment: comment,
        inline_comments_path: None,
    };
    let json = serde_json::to_vec_pretty(&response)
        .map_err(ResponseWriteError::Json)?;

    atomic_write(&response_path, &json)
        .map_err(ResponseWriteError::Io)?;

    Ok(())
}
```

### HTML rendering

```rust
// src/hitl_server/routes.rs (internal)

fn render_html(config: &GateConfig, pending_json: &str) -> String {
    let pending: serde_json::Value = serde_json::from_str(pending_json).unwrap_or_default();
    format!(
        r#"<!doctype html>
<title>Gate: {phase}</title>
<h1>{phase} — {workflow}</h1>
<p>Severity: <strong>{severity}</strong></p>
<p>Artefact: <code>{artefact}</code></p>
<p>Timeout: {timeout}</p>
<hr>
<form method="post" action="/approve">
  <textarea name="comment" rows="3" cols="60" placeholder="Optional comment (approve)"></textarea><br>
  <button type="submit">Approve</button>
</form>
<hr>
<form method="post" action="/reject">
  <textarea name="comment" rows="3" cols="60" placeholder="Optional comment (reject)"></textarea><br>
  <button type="submit">Reject</button>
</form>
"#,
        phase = html_escape(&config.phase),
        workflow = html_escape(&config.workflow),
        severity = html_escape(&config.severity),
        artefact = html_escape(&config.artefact_path),
        timeout = config.timeout_at.as_deref().unwrap_or("none"),
    )
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
     .replace('<', "&lt;")
     .replace('>', "&gt;")
     .replace('"', "&quot;")
}
```

### HumanVerifier integration

```rust
// src/strategy/verify/human_verifier.rs — additions

use crate::hitl_server::{GateConfig, ServerHandle, ServerOutcome, start};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanVerifierConfig {
    // ... existing fields ...
    /// When true and no response exists, spawn a one-shot HTTP server.
    #[serde(default)]
    pub fallback_server: bool,
}

impl HumanVerifier {
    pub async fn verify_with_report(
        &self,
        ctx: &VerifyContext,
    ) -> Result<(VerifyResult, HumanVerifyReport), VerifyError> {
        // ... existing pending/response logic up to "None =>" branch ...

        // In the "None" branch (no response exists):
        if self.config.fallback_server {
            return self.verify_with_fallback_server(ctx).await;
        }

        // ... existing behaviour (write pending, return Fail) ...
    }

    async fn verify_with_fallback_server(
        &self,
        ctx: &VerifyContext,
    ) -> Result<(VerifyResult, HumanVerifyReport), VerifyError> {
        // 1. Write pending file (existing logic)
        let payload = self.pending_payload(
            &self.config.artefact_name,
            kind_str(&self.config.artefact_kind),
            &ctx.stdout.chars().take(160).collect::<String>(),
            u32::try_from(ctx.stdout.lines().count()).unwrap_or(u32::MAX),
        )?;
        self.ensure_pending_file(&payload)?;

        // 2. Compute timeout
        let rule = self.config.timeout_policy.rule_for(self.config.severity);
        let timeout_duration = rule.timeout.map(|d| d.to_std());

        // 3. Spawn server
        let gate_config = GateConfig {
            run_dir: self.config.run_dir.clone(),
            run_id: self.config.run_id.clone(),
            phase: self.config.phase.clone(),
            workflow: self.config.workflow.clone(),
            severity: self.config.severity.as_str().to_string(),
            artefact_path: self.config.artefact_name.clone(),
            artefact_kind: kind_str(&self.config.artefact_kind).to_string(),
            prompt_summary: ctx.stdout.chars().take(160).collect(),
            preview_lines: u32::try_from(ctx.stdout.lines().count()).unwrap_or(u32::MAX),
            timeout_at: payload.timeout_at.clone(),
            decision_options: self.config.decision_options.iter()
                .map(|d| d.as_str().to_string())
                .collect(),
        };

        let (addr, handle) = start(gate_config)
            .await
            .map_err(|e| VerifyError::new(format!("failed to start HITL server: {e}")))?;

        println!("HITL gate open: http://{}/", addr);

        // 4. Wait for decision or timeout
        let outcome = if let Some(duration) = timeout_duration {
            match tokio::time::timeout(duration, handle.outcome()).await {
                Ok(outcome) => outcome,
                Err(_) => ServerOutcome::TimedOut,
            }
        } else {
            handle.outcome().await
        };

        // 5. Map outcome to VerifyResult
        match outcome {
            ServerOutcome::Decided { decision, .. } => {
                // Re-enter the normal response path: the server already wrote
                // the response file, so parse_response will find it.
                self.verify_with_report(ctx).await
            }
            ServerOutcome::TimedOut => {
                // Re-enter the normal timeout path
                self.verify_with_report(ctx).await
            }
            ServerOutcome::Cancelled => {
                let reason = FailureReason::new(
                    format!("HITL server for {} was cancelled", self.config.phase)
                );
                let report = HumanVerifyReport::from_policy(
                    self.config.severity,
                    payload.timeout_at,
                    rule.on_timeout,
                    HumanTimeoutOutcome::Blocking,
                );
                Ok((VerifyResult::Fail { reason }, report))
            }
        }
    }
}
```

### Data flow

```
PhaseRunner::run
   │
   ▼
HumanVerifier::verify_with_report
   │  No response exists, fallback_server = true
   ▼
write pending/<phase>.json
   │
   ▼
hitl_server::one_shot::start
   │  Bind to 127.0.0.1:0, print URL
   ▼
┌─────────────────┐         ┌─────────────┐
│  axum server    │◄────────│  browser    │
│  GET /          │         │  human      │
│  POST /approve  │────────►│  clicks     │
│  POST /reject   │         │  button     │
└─────────────────┘         └─────────────┘
   │
   ▼
PhaseLock::acquire ──► write responses/<phase>.json
   │
   ▼
oneshot::Sender ──► ServerHandle::outcome() resolves
   │
   ▼
HumanVerifier reads response file ──► VerifyResult
```

## 4. Public API Surface

### `src/hitl_server/mod.rs`

```rust
pub use routes::router;
pub use one_shot::{start, ServerHandle, ServerOutcome};

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct GateConfig {
    pub run_dir: PathBuf,
    pub run_id: String,
    pub phase: String,
    pub workflow: String,
    pub severity: String,
    pub artefact_path: String,
    pub artefact_kind: String,
    pub prompt_summary: String,
    pub preview_lines: u32,
    pub timeout_at: Option<String>,
    pub decision_options: Vec<String>,
}

#[derive(Debug)]
pub enum ServerOutcome {
    Decided { decision: HumanDecision, comment: Option<String> },
    TimedOut,
    Cancelled,
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("bind failed: {0}")]
    Bind(#[from] std::io::Error),
    #[error("pending read failed: {0}")]
    PendingRead(#[from] std::io::Error),
    #[error("lock error: {0}")]
    Lock(#[from] crate::run_state::phase_lock::PhaseLockError),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
```

### `src/hitl_server/one_shot.rs`

```rust
use std::net::SocketAddr;

/// Bind, spawn the server, return the address + a handle.
pub async fn start(
    config: GateConfig,
) -> Result<(SocketAddr, ServerHandle), ServerError>;

/// Await `outcome()` to get the server result.
pub struct ServerHandle {
    // opaque
}

impl ServerHandle {
    pub async fn outcome(self) -> ServerOutcome;
}
```

### `src/hitl_server/routes.rs`

```rust
use axum::Router;

/// Build an axum Router for the three gate routes.
pub fn router(config: GateConfig) -> Router;
```

### `src/strategy/verify/human_verifier.rs` (delta)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanVerifierConfig {
    // ... existing fields ...
    pub fallback_server: bool,
}
```

## 5. Test Plan

### Unit tests (`src/hitl_server/one_shot.rs`)

| Test | Assertion |
|------|-----------|
| `binds_to_free_port` | `start()` returns `SocketAddr` with port > 0 and IP = 127.0.0.1 |
| `shuts_down_after_decision_signal` | Spawn server, send decision via internal channel, `handle.outcome()` resolves within 1s |
| `shuts_down_on_handle_drop` | Drop `ServerHandle`, server task exits within 1s |

### Integration tests (`tests/hitl_server.rs`)

| Test | Setup | Assertion |
|------|-------|-----------|
| `one_shot_approve_resolves_gate` | Temp run_dir with pending file, spawn server, POST /approve with comment | Response file exists, schema valid, `decision: Approve`, comment preserved |
| `one_shot_reject_resolves_gate` | Same, POST /reject | Response file exists, `decision: Reject` |
| `concurrent_post_races_return_423` | Two `reqwest` clients POST /approve simultaneously | Exactly one 200, one 423 (LOCKED) |
| `second_post_after_first_returns_409` | POST /approve, then immediately POST /approve again | Second returns 409 (CONFLICT) |
| `gate_context_shows_pending_json` | GET / after writing pending file | HTML body contains phase name, artefact path, severity |
| `server_url_printed_to_stdout` | `HumanVerifier` with `fallback_server=true`, no response | Stdout contains `http://127.0.0.1:\d+/` |
| `timeout_auto_approves_without_human` | Low severity, 1ms timeout, no POST | Server exits, auto-approve result after timeout |
| `high_severity_blocks_indefinitely` | High severity, no timeout, no POST | Server stays alive (or returns blocking after explicit cancellation) |

### Regression tests

- All existing `human_verifier.rs` unit tests pass unchanged (`fallback_server` defaults to `false`).
- All existing `phase_runner_human_verifier.rs` integration tests pass unchanged.
- `make check` (fmt + clippy + test) passes.

### Manual test

1. Run `cargo run --bin loker -- run <workflow-with-human-verify>`.
2. Observe `HITL gate open: http://127.0.0.1:<port>/` printed.
3. Open URL in browser, see HTML form.
4. Click Approve.
5. Run continues, phase completes, marker shows `hitl` context.

## 6. Migration / Rollout

- **Additive only**: new `src/hitl_server/` module, new Cargo deps (`axum`, `tower`).
- No changes to marker schema, manifest schema, or run directory layout.
- `HumanVerifierConfig` gains `fallback_server: bool` with `#[serde(default)]`; default `false` preserves existing behaviour for all existing tests and workflows.
- No feature flag required; the server is only spawned when `fallback_server = true`.
- The new module is compiled unconditionally (axum is a small dep, ~2MB binary increase acceptable for v0).

## 7. Open Questions

| Question | Resolution | Owner |
|----------|-----------|-------|
| Should the server print to stdout via `println!` or through an injected output sink? | Use `println!` for v0; the phase runner already prints progress. A structured sink can be added when trace writer supports user-facing messages (T-029). | CLO-320 |
| What happens if the operator Ctrl-Cs while the server is running? | The tokio runtime shuts down, `ServerHandle` is dropped, the server task aborts. `HumanVerifier` returns `Fail` with reason "HITL server cancelled". The pending file remains; `loker resume` will re-spawn the server. | CLO-320 |
| Should `GateConfig` include `opened_at` / `timeout_at` from the pending payload? | Yes — `timeout_at` is passed through so the HTML can display it. `opened_at` is implicit in the pending file. | CLO-320 |
| Do we need CORS headers for localhost? | No. The server is one-shot, no cross-origin requests are expected. If M11 needs CORS, it will be added there. | M11 |
| Binary size impact of axum? | Acceptable for v0. If size becomes a concern post-v0, gate behind a feature flag. | Post-v0 |
