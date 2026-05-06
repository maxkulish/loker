//! HITL fallback server — one-shot axum gate for human approval.
//!
//! `hitl_server` provides a minimal localhost-only HTTP server that spawns
//! when a `HumanVerifier` gate triggers with `fallback_server = true`.
//! It serves exactly one gate and shuts down after the first human decision.
//!
//! Architecture:
//! - [`routes::router`] — shared axum route handlers (reused by M11 daemon).
//! - [`one_shot::start`] — bind to a free localhost port, spawn server, await outcome.

pub mod one_shot;
pub mod routes;

use std::path::PathBuf;

use crate::strategy::verify::HumanDecision;

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
    Decided {
        decision: HumanDecision,
        comment: Option<String>,
    },
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
    #[error("lock error: {0}")]
    Lock(#[from] crate::run_state::phase_lock::PhaseLockError),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
