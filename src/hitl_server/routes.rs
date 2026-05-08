//! Axum route handlers for the HITL gate server.
//!
//! Reused by both the one-shot fallback server (T-051) and the M11 daemon.

use std::sync::Arc;

use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use tokio::sync::oneshot::Sender;

use crate::hitl_server::{GateConfig, ServerOutcome};
use crate::run_state::atomic_write;
use crate::run_state::phase_lock::{PhaseLock, PhaseLockError};
use crate::strategy::verify::{HumanDecision, HumanResponse};

use crate::ui::security::{add_security_headers, check_post_origin, PostGuardConfig};

#[derive(Deserialize)]
pub struct DecisionForm {
    pub comment: Option<String>,
}

/// Combined application state: gate config + signalling channels.
#[derive(Clone)]
pub struct AppState {
    pub config: GateConfig,
    pub decision_tx: Arc<std::sync::Mutex<Option<Sender<ServerOutcome>>>>,
    pub post_guard_config: Option<PostGuardConfig>,
}

impl AppState {
    pub fn new(config: GateConfig, decision_tx: Sender<ServerOutcome>, port: Option<u16>) -> Self {
        Self {
            config,
            decision_tx: Arc::new(std::sync::Mutex::new(Some(decision_tx))),
            post_guard_config: port.map(PostGuardConfig::for_loopback),
        }
    }
}

/// Build an axum Router for the three gate routes.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(gate_context))
        .route("/approve", post(approve))
        .route("/reject", post(reject))
        .with_state(state)
}

/// Render the per-gate context view (HTML form for human approval/rejection).
///
/// Shared between the one-shot fallback server (T-051) and the daemon (T-052).
/// Reads `pending/<phase>.json` from the run directory and renders it as an
/// HTML page with Approve/Reject buttons. Falls back to an empty-context page
/// if the pending file cannot be read.
pub async fn render_gate_view(config: &GateConfig) -> Result<Html<String>, StatusCode> {
    let pending_path = config
        .run_dir
        .join("pending")
        .join(format!("{}.json", config.phase));
    let body = match tokio::fs::read_to_string(&pending_path).await {
        Ok(json) => render_html(config, &json),
        Err(_) => render_html_fallback(config),
    };
    Ok(Html(body))
}

async fn gate_context(State(state): State<AppState>) -> Response {
    match render_gate_view(&state.config).await {
        Ok(html) => {
            let mut response = html.into_response();
            add_security_headers(&mut response);
            response
        }
        Err(status) => {
            let mut response = status.into_response();
            add_security_headers(&mut response);
            response
        }
    }
}

async fn approve(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Form(form): Form<DecisionForm>,
) -> Response {
    // POST guard
    if let Some(ref config) = state.post_guard_config {
        if let Err((status, msg)) = check_post_origin(&headers, config) {
            let mut response = (status, msg).into_response();
            add_security_headers(&mut response);
            return response;
        }
    }

    let status =
        match write_response(&state.config, HumanDecision::Approve, form.comment.clone()).await {
            Ok(()) => {
                if let Ok(mut tx) = state.decision_tx.lock() {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(ServerOutcome::Decided {
                            decision: HumanDecision::Approve,
                            comment: form.comment,
                        });
                    }
                }
                StatusCode::OK
            }
            Err(ResponseWriteError::Locked) => StatusCode::LOCKED,
            Err(ResponseWriteError::AlreadyExists) => StatusCode::CONFLICT,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
    let mut response = status.into_response();
    add_security_headers(&mut response);
    response
}

async fn reject(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Form(form): Form<DecisionForm>,
) -> Response {
    // POST guard
    if let Some(ref config) = state.post_guard_config {
        if let Err((status, msg)) = check_post_origin(&headers, config) {
            let mut response = (status, msg).into_response();
            add_security_headers(&mut response);
            return response;
        }
    }

    let status =
        match write_response(&state.config, HumanDecision::Reject, form.comment.clone()).await {
            Ok(()) => {
                if let Ok(mut tx) = state.decision_tx.lock() {
                    if let Some(tx) = tx.take() {
                        let _ = tx.send(ServerOutcome::Decided {
                            decision: HumanDecision::Reject,
                            comment: form.comment,
                        });
                    }
                }
                StatusCode::OK
            }
            Err(ResponseWriteError::Locked) => StatusCode::LOCKED,
            Err(ResponseWriteError::AlreadyExists) => StatusCode::CONFLICT,
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
    let mut response = status.into_response();
    add_security_headers(&mut response);
    response
}

// ---------------------------------------------------------------------------
// Internal: response write
// ---------------------------------------------------------------------------

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
    // Acquire the per-phase advisory lock (PhaseLock::acquire is sync;
    // the oneshot server has exactly one target, so brief blocking is fine).
    let _lock =
        PhaseLock::acquire(&config.run_dir, &config.phase, &config.run_id, None).map_err(|e| {
            match e {
                PhaseLockError::LockInUse { .. } => ResponseWriteError::Locked,
                other => ResponseWriteError::Io(std::io::Error::other(other.to_string())),
            }
        })?;

    // Race guard: if another POST already wrote the response.
    let response_path = config
        .run_dir
        .join("responses")
        .join(format!("{}.json", config.phase));
    if response_path.exists() {
        return Err(ResponseWriteError::AlreadyExists);
    }

    let response = HumanResponse {
        schema_version: 1,
        phase: config.phase.clone(),
        claimed_by: "hitl_server".into(),
        decided_at: chrono::Utc::now().to_rfc3339(),
        decision,
        global_comment: comment,
        inline_comments_path: None,
    };
    let json = serde_json::to_vec_pretty(&response).map_err(ResponseWriteError::Json)?;

    if let Some(dir) = response_path.parent() {
        std::fs::create_dir_all(dir).map_err(ResponseWriteError::Io)?;
    }

    atomic_write(&response_path, &json).map_err(ResponseWriteError::Io)?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Internal: HTML rendering
// ---------------------------------------------------------------------------

fn render_html(config: &GateConfig, pending_json: &str) -> String {
    let pending: serde_json::Value = serde_json::from_str(pending_json).unwrap_or_default();
    let ctx = pending.get("context");
    let prompt_summary = ctx
        .and_then(|c| c.get("prompt_summary"))
        .and_then(|v| v.as_str())
        .unwrap_or(&config.prompt_summary);
    let artefact = pending
        .get("artefact")
        .and_then(|a| a.get("path"))
        .and_then(|v| v.as_str())
        .unwrap_or(&config.artefact_path);

    format!(
        r#"<!doctype html>
<title>Gate: {phase}</title>
<h1>{phase} — {workflow}</h1>
<p>Severity: <strong>{severity}</strong></p>
<p>Artefact: <code>{artefact}</code></p>
<p>Timeout: {timeout}</p>
<hr>
<p><strong>Summary:</strong> <pre>{summary}</pre></p>
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
        artefact = html_escape(artefact),
        timeout = config.timeout_at.as_deref().unwrap_or("none"),
        summary = html_escape(prompt_summary),
    )
}

fn render_html_fallback(config: &GateConfig) -> String {
    render_html(config, "{}")
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_html_contains_gate_context() {
        let config = GateConfig {
            run_dir: std::path::PathBuf::from("/tmp/run"),
            run_id: "run-1".into(),
            phase: "review".into(),
            workflow: "design-doc-tdd".into(),
            severity: "high".into(),
            artefact_path: "review.md".into(),
            artefact_kind: "text/markdown".into(),
            prompt_summary: "Candidate output preview".into(),
            preview_lines: 17,
            timeout_at: None,
            decision_options: vec!["approve".into(), "reject".into()],
        };
        let pending_json = serde_json::json!({
            "artefact": {"path": "review.md", "kind": "text/markdown"},
            "context": {"prompt_summary": "Candidate output preview"}
        });
        let html = render_html(&config, &pending_json.to_string());
        assert!(html.contains("review"));
        assert!(html.contains("design-doc-tdd"));
        assert!(html.contains("high"));
        assert!(html.contains("review.md"));
        assert!(html.contains("/approve"));
        assert!(html.contains("/reject"));
    }

    #[test]
    fn html_escape_sanitises_injection() {
        let dirty = "<script>alert('xss')</script>";
        let clean = html_escape(dirty);
        assert!(!clean.contains('<'));
        assert!(clean.contains("\u{0026}lt;"));
    }

    #[tokio::test]
    async fn render_gate_view_returns_gate_context() {
        let tmp = tempfile::tempdir().unwrap();
        let pending_dir = tmp.path().join("pending");
        std::fs::create_dir_all(&pending_dir).unwrap();
        let pending_json = serde_json::json!({
            "context": {"prompt_summary": "Candidate output preview"},
            "artefact": {"path": "review.md", "kind": "text/markdown"}
        });
        std::fs::write(
            pending_dir.join("review.json"),
            pending_json.to_string().as_bytes(),
        )
        .unwrap();

        let config = GateConfig {
            run_dir: tmp.path().to_path_buf(),
            run_id: "run-1".into(),
            phase: "review".into(),
            workflow: "design-doc-tdd".into(),
            severity: "high".into(),
            artefact_path: "review.md".into(),
            artefact_kind: "text/markdown".into(),
            prompt_summary: "fallback".into(),
            preview_lines: 17,
            timeout_at: None,
            decision_options: vec!["approve".into(), "reject".into()],
        };

        let result = render_gate_view(&config).await;
        assert!(result.is_ok());
        let html = result.unwrap().0;
        assert!(html.contains("review"));
        assert!(html.contains("design-doc-tdd"));
        assert!(html.contains("high"));
        assert!(html.contains("review.md"));
        assert!(html.contains("/approve"));
        assert!(html.contains("/reject"));
        // Uses the pending file content, not the fallback prompt_summary
        assert!(html.contains("Candidate output preview"));
    }

    #[tokio::test]
    async fn render_gate_view_fallback_when_pending_missing() {
        let tmp = tempfile::tempdir().unwrap();

        let config = GateConfig {
            run_dir: tmp.path().to_path_buf(),
            run_id: "run-2".into(),
            phase: "approval".into(),
            workflow: "test-wf".into(),
            severity: "low".into(),
            artefact_path: "report.md".into(),
            artefact_kind: "text/markdown".into(),
            prompt_summary: "Fallback summary".into(),
            preview_lines: 5,
            timeout_at: None,
            decision_options: vec!["approve".into()],
        };

        let result = render_gate_view(&config).await;
        assert!(result.is_ok());
        let html = result.unwrap().0;
        // Fallback uses the config's prompt_summary, not a file
        assert!(html.contains("Fallback summary"));
        assert!(html.contains("approval"));
        assert!(html.contains("test-wf"));
    }
}
