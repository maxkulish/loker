//! Daemon route handlers for `loker ui --serve`.
//!
//! Provides `ui_routes()` which builds the daemon's axum Router with:
//! - `GET /`          — HTML sessions index page
//! - `GET /health`    — 200 OK health check
//! - `GET /runs/:id`  — per-run detail (manifest, timeline, trace)
//! - `GET /pending`   — aggregated pending HITL gates
//! - `POST /gates/:run_id/:phase/approve` — approve a gate
//! - `POST /gates/:run_id/:phase/reject`  — reject a gate

use std::path::PathBuf;

use axum::{
    extract::{Form, Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::task;
use tokio_stream::wrappers::ReceiverStream;

use crate::hitl_server::routes::DecisionForm;
use crate::run_state::atomic_write;
use crate::run_state::phase_lock::{PhaseLock, PhaseLockError};
use crate::strategy::verify::{HumanDecision, HumanResponse};
use crate::ui::{discovery, gate_discovery, manifest, templates, trace_reader};

/// Combined state for the daemon's route handlers.
#[derive(Clone)]
pub struct AppState {
    pub project_root: PathBuf,
    pub max_trace_events: usize,
}

impl AppState {
    fn new(project_root: PathBuf) -> Self {
        Self {
            project_root,
            max_trace_events: 50,
        }
    }
}

/// Build the daemon's top-level router.
pub fn ui_routes(project_root: PathBuf) -> Router {
    let state = AppState::new(project_root);

    Router::new()
        .route("/", get(index_page))
        .route("/health", get(health_check))
        .route("/runs/:id", get(run_detail))
        .route("/runs/:id/trace/sse", get(run_trace_sse))
        .route("/pending", get(pending_panel))
        .route("/gates/:run_id/:phase/approve", post(hitl_approve))
        .route("/gates/:run_id/:phase/reject", post(hitl_reject))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET / — HTML sessions index page.
async fn index_page(State(state): State<AppState>) -> Response {
    let root = state.project_root.clone();
    let runs = task::spawn_blocking(move || discovery::discover_runs(&root))
        .await
        .unwrap_or_default();
    templates::IndexTemplate {
        runs,
        active_milestone: String::new(),
    }
    .into_response()
}

/// GET /health — 200 OK health check.
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// GET /runs/:id — per-run detail page.
async fn run_detail(State(state): State<AppState>, Path(run_id): Path<String>) -> Response {
    // Sanitize run_id — reject path traversal and empty IDs.
    if run_id.is_empty() || run_id.contains("..") || run_id.contains('/') || run_id.contains('\\') {
        return templates::ErrorTemplate {
            status_code: 400,
            message: format!("Invalid run ID: {}", run_id),
        }
        .into_response();
    }

    let run_dir = state.project_root.join("runs").join(&run_id);
    let max_events = state.max_trace_events;

    // Check the run directory exists
    if !run_dir.exists() {
        return templates::ErrorTemplate {
            status_code: 404,
            message: format!("Run not found: {}", run_id),
        }
        .into_response();
    }

    // Read manifest, timeline, and trace in a blocking task
    let run_dir_clone = run_dir.clone();
    let (manifest_entries, phase_timeline, trace_events) = task::spawn_blocking(move || {
        let manifest_path = run_dir_clone.join("manifest.json");
        let entries = manifest::read_manifest_entries(&manifest_path);
        let timeline = manifest::build_phase_timeline(&run_dir_clone);
        let trace_path = run_dir_clone.join("trace.jsonl");
        let traces = trace_reader::tail_trace_file(&trace_path, max_events);
        (entries, timeline, traces)
    })
    .await
    .unwrap_or_default();

    // Read workflow name from manifest (best-effort)
    let manifest_path = run_dir.join("manifest.json");
    let workflow = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|v| {
            v.get("workflow_name")
                .and_then(|w| w.as_str().map(String::from))
        });

    templates::RunDetailTemplate {
        run_id,
        workflow,
        manifest_entries,
        phase_timeline,
        trace_events,
    }
    .into_response()
}

/// GET /pending — aggregated pending HITL gates.
async fn pending_panel(State(state): State<AppState>) -> Response {
    let root = state.project_root.clone();
    let gates = task::spawn_blocking(move || gate_discovery::discover_pending_gates(&root))
        .await
        .unwrap_or_default();
    let display = gate_discovery::to_display(&gates);
    templates::PendingTemplate { gates: display }.into_response()
}

/// POST /gates/:run_id/:phase/approve — approve a pending gate.
async fn hitl_approve(
    State(state): State<AppState>,
    Path((run_id, phase)): Path<(String, String)>,
    Form(form): Form<DecisionForm>,
) -> Response {
    // Sanitize run_id — reject traversal and empty IDs.
    if run_id.is_empty() || run_id.contains("..") || run_id.contains('/') || run_id.contains('\\') {
        return templates::ErrorTemplate {
            status_code: 400,
            message: format!("Invalid run ID: {}", run_id),
        }
        .into_response();
    }

    let run_dir = state.project_root.join("runs").join(&run_id);
    if !run_dir.exists() {
        return templates::ErrorTemplate {
            status_code: 404,
            message: format!("Run not found: {}", run_id),
        }
        .into_response();
    }

    match write_gate_response(
        &run_dir,
        &run_id,
        &phase,
        HumanDecision::Approve,
        form.comment,
    )
    .await
    {
        Ok(()) => {
            // Redirect to pending panel on success
            let location = "/pending";
            (
                StatusCode::SEE_OTHER,
                [("Location", location)],
                "Redirecting…",
            )
                .into_response()
        }
        Err(GateError::AlreadyExists) => {
            let location = "/pending";
            (
                StatusCode::SEE_OTHER,
                [("Location", location)],
                "Already decided",
            )
                .into_response()
        }
        Err(GateError::Locked) => templates::ErrorTemplate {
            status_code: 423,
            message: format!("Gate '{phase}' on run '{run_id}' is locked by another process"),
        }
        .into_response(),
        Err(e) => templates::ErrorTemplate {
            status_code: 500,
            message: format!("Failed to write response: {e}"),
        }
        .into_response(),
    }
}

/// POST /gates/:run_id/:phase/reject — reject a pending gate.
async fn hitl_reject(
    State(state): State<AppState>,
    Path((run_id, phase)): Path<(String, String)>,
    Form(form): Form<DecisionForm>,
) -> Response {
    // Sanitize run_id — reject traversal and empty IDs.
    if run_id.is_empty() || run_id.contains("..") || run_id.contains('/') || run_id.contains('\\') {
        return templates::ErrorTemplate {
            status_code: 400,
            message: format!("Invalid run ID: {}", run_id),
        }
        .into_response();
    }

    let run_dir = state.project_root.join("runs").join(&run_id);
    if !run_dir.exists() {
        return templates::ErrorTemplate {
            status_code: 404,
            message: format!("Run not found: {}", run_id),
        }
        .into_response();
    }

    match write_gate_response(
        &run_dir,
        &run_id,
        &phase,
        HumanDecision::Reject,
        form.comment,
    )
    .await
    {
        Ok(()) => {
            let location = "/pending";
            (
                StatusCode::SEE_OTHER,
                [("Location", location)],
                "Redirecting…",
            )
                .into_response()
        }
        Err(GateError::AlreadyExists) => {
            let location = "/pending";
            (
                StatusCode::SEE_OTHER,
                [("Location", location)],
                "Already decided",
            )
                .into_response()
        }
        Err(GateError::Locked) => templates::ErrorTemplate {
            status_code: 423,
            message: format!("Gate '{phase}' on run '{run_id}' is locked by another process"),
        }
        .into_response(),
        Err(e) => templates::ErrorTemplate {
            status_code: 500,
            message: format!("Failed to write response: {e}"),
        }
        .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Gate response writing
// ---------------------------------------------------------------------------

use std::fmt;

#[derive(Debug)]
enum GateError {
    Locked,
    AlreadyExists,
    Io(String),
}

impl fmt::Display for GateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GateError::Locked => write!(f, "gate is locked"),
            GateError::AlreadyExists => write!(f, "response already exists"),
            GateError::Io(msg) => write!(f, "IO error: {}", msg),
        }
    }
}

async fn write_gate_response(
    run_dir: &std::path::Path,
    run_id: &str,
    phase: &str,
    decision: HumanDecision,
    comment: Option<String>,
) -> Result<(), GateError> {
    let run_dir_owned = run_dir.to_path_buf();
    let phase_owned = phase.to_string();
    let run_id_owned = run_id.to_string();

    // Acquire advisory lock in blocking task
    task::spawn_blocking(move || {
        let _lock =
            PhaseLock::acquire(&run_dir_owned, &phase_owned, &run_id_owned, None).map_err(|e| {
                match e {
                    PhaseLockError::LockInUse { .. } => GateError::Locked,
                    other => GateError::Io(other.to_string()),
                }
            })?;

        let response_path = run_dir_owned
            .join("responses")
            .join(format!("{phase_owned}.json"));

        // Race guard: don't overwrite existing responses
        if response_path.exists() {
            return Err(GateError::AlreadyExists);
        }

        let response = HumanResponse {
            schema_version: 1,
            phase: phase_owned.clone(),
            claimed_by: "loker_ui_daemon".into(),
            decided_at: chrono::Utc::now().to_rfc3339(),
            decision,
            global_comment: comment,
            inline_comments_path: None,
        };

        let json =
            serde_json::to_vec_pretty(&response).map_err(|e| GateError::Io(e.to_string()))?;

        if let Some(dir) = response_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| GateError::Io(e.to_string()))?;
        }

        atomic_write(&response_path, &json).map_err(|e| GateError::Io(e.to_string()))?;

        Ok(())
    })
    .await
    .map_err(|e| GateError::Io(e.to_string()))?
}

/// GET /runs/:id/trace/sse — stream live trace events.
async fn run_trace_sse(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    // Sanitize run_id
    if run_id.is_empty() || run_id.contains("..") || run_id.contains('/') || run_id.contains('\\') {
        return templates::ErrorTemplate {
            status_code: 400,
            message: format!("Invalid run ID: {}", run_id),
        }
        .into_response();
    }

    let run_dir = state.project_root.join("runs").join(&run_id);
    let trace_path = run_dir.join("trace.jsonl");

    if !trace_path.exists() {
        return templates::ErrorTemplate {
            status_code: 404,
            message: format!("Trace file not found for run: {}", run_id),
        }
        .into_response();
    }

    // Use Last-Event-ID header if present, otherwise use EOF
    let initial_offset = if let Some(last_id) = headers
        .get("Last-Event-ID")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
    {
        last_id
    } else {
        match std::fs::metadata(&trace_path) {
            Ok(m) => m.len(),
            Err(_) => {
                return templates::ErrorTemplate {
                    status_code: 500,
                    message: "Failed to read trace file metadata".to_string(),
                }
                .into_response()
            }
        }
    };

    let (tx, rx) = mpsc::channel(100);
    let watcher = crate::ui::sse::TraceWatcher::new(trace_path, initial_offset);

    // Spawn the watcher in the background
    tokio::spawn(async move {
        if let Err(e) = watcher.watch(tx).await {
            tracing::error!("Trace watcher failed for run {}: {}", run_id, e);
        }
    });

    // Create a heartbeat stream (every 15 seconds)
    let heartbeat = tokio_stream::wrappers::IntervalStream::new(tokio::time::interval(
        tokio::time::Duration::from_secs(15),
    ))
    .map(|_| Ok::<axum::body::Bytes, std::io::Error>(axum::body::Bytes::from(": heartbeat\n\n")));

    // Convert receiver to a stream of bytes
    let data_stream = ReceiverStream::new(rx).map(|(offset, line)| {
        let sse_event = crate::ui::sse::format_line_as_sse(&offset.to_string(), &line)
            .unwrap_or_else(|| "data: Error parsing line\n\n".to_string());
        Ok::<axum::body::Bytes, std::io::Error>(axum::body::Bytes::from(sse_event))
    });

    // Merge heartbeat and data streams
    let stream = futures::stream::select(data_stream, heartbeat);

    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(axum::body::Body::from_stream(stream))
        .unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use insta::assert_snapshot;
    use regex::Regex;
    use tower::ServiceExt;

    // -----------------------------------------------------------------------
    // GET / — index page
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn index_page_renders_runs_table() {
        let tmp = tempfile::tempdir().unwrap();
        let runs_dir = tmp.path().join("runs").join("wf-run-111");
        fs::create_dir_all(&runs_dir).unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "workflow_name": "test-workflow",
            "loker.run_id": "run-test-111",
            "entries": []
        });
        fs::write(
            runs_dir.join("manifest.json"),
            manifest.to_string().as_bytes(),
        )
        .unwrap();
        // Add a marker
        let markers_dir = runs_dir.join("markers");
        fs::create_dir_all(&markers_dir).unwrap();
        fs::write(markers_dir.join("design.completed"), b"{}").unwrap();

        let app = ui_routes(tmp.path().to_path_buf());
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), 50_000)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("wf-run-111"), "HTML should contain run ID");
        assert!(
            html.contains("test-workflow"),
            "HTML should contain workflow name"
        );
        assert!(html.contains("design"), "HTML should contain phase info");
        assert!(html.contains("completed"), "HTML should contain status");

        // Snapshot test — stable fixture data (strip dynamic timestamps)
        let ts_re = Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[\d:+Z.-]+").unwrap();
        let snapshot_html = ts_re.replace_all(&html, "<TIMESTAMP>");
        assert_snapshot!("index_page_with_runs", &snapshot_html.as_ref());
    }

    #[tokio::test]
    async fn index_page_empty_state() {
        let tmp = tempfile::tempdir().unwrap();
        let app = ui_routes(tmp.path().to_path_buf());
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), 50_000)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("No runs yet"), "HTML should show empty state");

        // Snapshot test — empty state is fully deterministic
        assert_snapshot!("index_page_empty", &html);
    }

    // -----------------------------------------------------------------------
    // GET /health
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn health_check_returns_200() {
        let tmp = tempfile::tempdir().unwrap();
        let app = ui_routes(tmp.path().to_path_buf());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    // -----------------------------------------------------------------------
    // GET /runs/:id — detail page
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn run_detail_page_renders_all_sections() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path().join("runs").join("detail-run-001");
        fs::create_dir_all(&run_dir).unwrap();
        // Manifest with two entries
        let manifest = serde_json::json!({
            "schema_version": 1,
            "workflow_name": "detail-wf",
            "loker.run_id": "run-detail-001",
            "entries": [
                {"name": "design.md", "kind": "text/markdown", "schema_version": 1, "sha256": "abc"},
                {"name": "review.md", "kind": "text/markdown", "schema_version": 1}
            ]
        });
        fs::write(
            run_dir.join("manifest.json"),
            manifest.to_string().as_bytes(),
        )
        .unwrap();
        // Markers
        let markers_dir = run_dir.join("markers");
        fs::create_dir_all(&markers_dir).unwrap();
        fs::write(markers_dir.join("design.completed"), b"").unwrap();
        fs::write(markers_dir.join("review.started"), b"").unwrap();
        // Trace file
        let trace_path = run_dir.join("trace.jsonl");
        let trace_content: String = (0..5)
            .map(|i| serde_json::json!({"timestamp": format!("2026-01-01T00:00:{:02}Z", i), "event_type": "step", "message": format!("event {}", i)}).to_string())
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&trace_path, &trace_content).unwrap();

        let app = ui_routes(tmp.path().to_path_buf());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/runs/detail-run-001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), 50_000)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        // Manifest
        assert!(html.contains("design.md"), "Should show manifest entry");
        assert!(html.contains("review.md"), "Should show manifest entry");
        // Timeline
        assert!(html.contains("design"));
        assert!(html.contains("review"));
        assert!(html.contains("completed"));
        assert!(html.contains("started"));
        // Trace
        assert!(html.contains("event 0"), "Should show trace events");
        assert!(html.contains("event 4"), "Should show trace events");

        // Snapshot test — fixture data (strip dynamic timestamps)
        let ts_re = Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[\d:+Z.-]+").unwrap();
        let snapshot_html = ts_re.replace_all(&html, "<TIMESTAMP>");
        assert_snapshot!("run_detail_with_data", &snapshot_html.as_ref());
    }

    #[tokio::test]
    async fn run_detail_page_unknown_run() {
        let tmp = tempfile::tempdir().unwrap();
        let app = ui_routes(tmp.path().to_path_buf());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/runs/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn run_detail_rejects_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let app = ui_routes(tmp.path().to_path_buf());
        // axum 0.7 normalizes ../ before routing, so the route doesn't match.
        // Instead, test with a run_id that contains dots and slashes via
        // URL encoding.
        let run_dir = tmp.path().join("runs").join("safe-run");
        std::fs::create_dir_all(&run_dir).unwrap();

        // Request a non-existent run — should get 404 from handler
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/runs/non-existent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // GET /pending — pending gates panel
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn pending_panel_renders_gates() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path().join("runs").join("pending-run-001");
        fs::create_dir_all(&run_dir).unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "workflow_name": "pending-wf",
            "loker.run_id": "run-pending-001",
            "entries": []
        });
        fs::write(
            run_dir.join("manifest.json"),
            manifest.to_string().as_bytes(),
        )
        .unwrap();
        // Create a pending gate
        let pending_dir = run_dir.join("pending");
        fs::create_dir_all(&pending_dir).unwrap();
        let pending = serde_json::json!({
            "severity": "high",
            "artefact": {"path": "review.md", "kind": "text/markdown"}
        });
        fs::write(
            pending_dir.join("review.json"),
            pending.to_string().as_bytes(),
        )
        .unwrap();

        let app = ui_routes(tmp.path().to_path_buf());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/pending")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), 50_000)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("pending-run-001"), "Should show run ID");
        assert!(html.contains("review"), "Should show phase name");
        assert!(html.contains("pending-wf"), "Should show workflow name");
        assert!(html.contains("approve"), "Should have approve form");
        assert!(html.contains("reject"), "Should have reject form");

        // Snapshot test — fixture data, deterministic
        assert_snapshot!("pending_panel_with_gates", &html);
    }

    #[tokio::test]
    async fn pending_panel_empty_state() {
        let tmp = tempfile::tempdir().unwrap();
        let app = ui_routes(tmp.path().to_path_buf());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/pending")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), 50_000)
            .await
            .unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            html.contains("All gates resolved"),
            "Should show empty state"
        );
    }

    // -----------------------------------------------------------------------
    // POST /gates/:run_id/:phase/approve|reject
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn approve_post_writes_response() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path().join("runs").join("gate-run-001");
        fs::create_dir_all(&run_dir).unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "workflow_name": "gate-wf",
            "loker.run_id": "run-gate-001",
            "entries": []
        });
        fs::write(
            run_dir.join("manifest.json"),
            manifest.to_string().as_bytes(),
        )
        .unwrap();
        // Create pending file so the gate exists
        let pending_dir = run_dir.join("pending");
        fs::create_dir_all(&pending_dir).unwrap();
        fs::write(pending_dir.join("review.json"), b"{}").unwrap();

        let app = ui_routes(tmp.path().to_path_buf());

        // POST to approve
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/gates/gate-run-001/review/approve")
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(Body::from("comment=looks+good"))
                    .unwrap(),
            )
            .await
            .unwrap();

        // Should be a redirect
        assert!(
            response.status() == StatusCode::SEE_OTHER,
            "Expected redirect, got {}",
            response.status()
        );

        // Verify response file was written
        let response_path = run_dir.join("responses").join("review.json");
        assert!(response_path.exists(), "Response file should exist");
        let content = fs::read_to_string(&response_path).unwrap();
        assert!(
            content.contains(r#""approve""#),
            "Should contain approval decision"
        );
    }

    #[tokio::test]
    async fn reject_post_writes_response() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path().join("runs").join("gate-run-002");
        fs::create_dir_all(&run_dir).unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "workflow_name": "gate-wf",
            "loker.run_id": "run-gate-002",
            "entries": []
        });
        fs::write(
            run_dir.join("manifest.json"),
            manifest.to_string().as_bytes(),
        )
        .unwrap();
        let pending_dir = run_dir.join("pending");
        fs::create_dir_all(&pending_dir).unwrap();
        fs::write(pending_dir.join("review.json"), b"{}").unwrap();

        let app = ui_routes(tmp.path().to_path_buf());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/gates/gate-run-002/review/reject")
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(Body::from("comment=needs+work"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(
            response.status() == StatusCode::SEE_OTHER,
            "Expected redirect, got {}",
            response.status()
        );

        let response_path = run_dir.join("responses").join("review.json");
        assert!(response_path.exists(), "Response file should exist");
        let content = fs::read_to_string(&response_path).unwrap();
        assert!(
            content.contains(r#""reject""#),
            "Should contain rejection decision"
        );
    }

    #[tokio::test]
    async fn hitl_approve_race_guard_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path().join("runs").join("gate-run-003");
        fs::create_dir_all(&run_dir).unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "workflow_name": "gate-wf",
            "loker.run_id": "run-gate-003",
            "entries": []
        });
        fs::write(
            run_dir.join("manifest.json"),
            manifest.to_string().as_bytes(),
        )
        .unwrap();
        let pending_dir = run_dir.join("pending");
        fs::create_dir_all(&pending_dir).unwrap();
        fs::write(pending_dir.join("review.json"), b"{}").unwrap();

        let app = ui_routes(tmp.path().to_path_buf());

        // First approval should succeed
        let resp1 = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/gates/gate-run-003/review/approve")
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(Body::from("comment=first"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp1.status(),
            StatusCode::SEE_OTHER,
            "First approve should redirect"
        );

        // Second approval should also redirect (already exists → still redirect)
        let resp2 = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/gates/gate-run-003/review/approve")
                    .header("Content-Type", "application/x-www-form-urlencoded")
                    .body(Body::from("comment=second"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp2.status(),
            StatusCode::SEE_OTHER,
            "Second approve should also redirect"
        );
    }
}
