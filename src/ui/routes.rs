//! Daemon route handlers for `loker ui --serve`.
//!
//! Provides `ui_routes()` which builds the daemon's axum Router. Extended
//! in T-053 (sessions list, per-run views, SSE endpoints).

use std::path::PathBuf;

use axum::{routing::get, Json, Router};
use serde_json::Value;

use crate::ui::discovery;

/// Combined state for the daemon's route handlers.
///
/// Extends in T-053 with runs-cache and notification channels.
#[derive(Clone)]
pub struct AppState {
    pub project_root: PathBuf,
}

/// Build the daemon's top-level router.
///
/// - `GET /`  -> JSON runs list (via `discovery::discover_runs`)
pub fn ui_routes(project_root: PathBuf) -> Router {
    let state = AppState { project_root };

    Router::new().route("/", get(runs_list)).with_state(state)
}

/// Handler for `GET /` — returns a JSON array of run summaries.
async fn runs_list(state: axum::extract::State<AppState>) -> Json<Value> {
    let runs = discovery::discover_runs(&state.project_root);
    Json(serde_json::to_value(runs).unwrap_or_else(|_| serde_json::json!([])))
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
    use tower::ServiceExt;

    #[tokio::test]
    async fn runs_list_returns_json_array() {
        let tmp = tempfile::tempdir().unwrap();
        let runs_dir = tmp.path().join("runs").join("my-run-123");
        fs::create_dir_all(&runs_dir).unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "workflow_name": "test-wf",
            "loker.run_id": "run-test-wf",
            "entries": []
        });
        fs::write(
            runs_dir.join("manifest.json"),
            manifest.to_string().as_bytes(),
        )
        .unwrap();

        let app = ui_routes(tmp.path().to_path_buf());
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), 10_000)
            .await
            .unwrap();
        let json: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(json.len(), 1);
        assert_eq!(
            json[0].get("id").and_then(|v| v.as_str()),
            Some("my-run-123")
        );
        assert_eq!(
            json[0].get("workflow").and_then(|v| v.as_str()),
            Some("test-wf")
        );
    }

    #[tokio::test]
    async fn runs_list_empty_when_runs_missing() {
        let tmp = tempfile::tempdir().unwrap();

        let app = ui_routes(tmp.path().to_path_buf());
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = axum::body::to_bytes(response.into_body(), 10_000)
            .await
            .unwrap();
        let json: Vec<Value> = serde_json::from_slice(&body).unwrap();
        assert!(json.is_empty());
    }
}
