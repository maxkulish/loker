//! Askama template structs for the loker UI daemon.
//!
//! Template rendering is driven by the `#[derive(Template)]` macro from
//! askama. All templates live in `templates/`.

use askama::Template;
use axum::response::{Html, IntoResponse, Response};

pub use types::*;

mod types {
    use askama::Template;

    #[derive(Template)]
    #[template(path = "index.html")]
    pub struct IndexTemplate {
        pub runs: Vec<crate::ui::discovery::RunSummary>,
        pub active_milestone: String,
    }

    #[derive(Template)]
    #[template(path = "run_detail.html")]
    pub struct RunDetailTemplate {
        pub run_id: String,
        pub workflow: Option<String>,
        pub manifest_entries: Vec<ManifestEntry>,
        pub phase_timeline: Vec<PhaseStep>,
        pub trace_events: Vec<TraceEventDisplay>,
    }

    #[derive(Template)]
    #[template(path = "pending.html")]
    pub struct PendingTemplate {
        pub gates: Vec<PendingGateDisplay>,
    }

    #[derive(Template)]
    #[template(path = "error.html")]
    pub struct ErrorTemplate {
        pub status_code: u16,
        pub message: String,
    }

    // ------------------------------------------------------------------
    // Data types shared between templates and route handlers
    // ------------------------------------------------------------------

    #[derive(Debug, Clone, serde::Serialize)]
    pub struct ManifestEntry {
        pub name: String,
        pub kind: String,
        pub schema_version: u32,
        pub sha256: Option<String>,
    }

    #[derive(Debug, Clone, serde::Serialize)]
    pub struct PhaseStep {
        pub name: String,
        pub status: String,
        pub started_at: Option<String>,
        pub completed_at: Option<String>,
    }

    #[derive(Debug, Clone, serde::Serialize)]
    pub struct TraceEventDisplay {
        pub timestamp: String,
        pub event_type: String,
        pub summary: String,
    }

    #[derive(Debug, Clone, serde::Serialize)]
    pub struct PendingGateDisplay {
        pub run_id: String,
        pub phase: String,
        pub workflow: String,
        pub severity: String,
        pub artefact_path: String,
    }
}

// ---------------------------------------------------------------------------
// IntoResponse implementations (without askama_axum)
// ---------------------------------------------------------------------------

impl IntoResponse for IndexTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(e) => {
                let msg = format!("Template error: {}", e);
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

impl IntoResponse for RunDetailTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(e) => {
                let msg = format!("Template error: {}", e);
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

impl IntoResponse for PendingTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => Html(html).into_response(),
            Err(e) => {
                let msg = format!("Template error: {}", e);
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}

impl IntoResponse for ErrorTemplate {
    fn into_response(self) -> Response {
        match self.render() {
            Ok(html) => (axum::http::StatusCode::from_u16(self.status_code).unwrap_or(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            ), Html(html)).into_response(),
            Err(e) => {
                let msg = format!("Template error: {}", e);
                (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response()
            }
        }
    }
}
