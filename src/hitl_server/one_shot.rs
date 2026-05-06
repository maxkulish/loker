//! One-shot axum server bootstrap for a single HITL gate.
//!
//! Binds to `127.0.0.1:0`, serves exactly one gate, and shuts down after
//! the first human decision or a caller-initiated cancellation.

use std::net::SocketAddr;

use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::hitl_server::routes::{AppState, router};
use crate::hitl_server::{GateConfig, ServerError, ServerOutcome};

/// Handle to a running one-shot server.
pub struct ServerHandle {
    /// The bound localhost address (includes the auto-allocated port).
    pub addr: SocketAddr,
    /// Receiver that resolves when the server gets a decision.
    decision_rx: oneshot::Receiver<ServerOutcome>,
    /// The spawned server task. Aborted on outcome or cancel.
    task: tokio::task::JoinHandle<()>,
}

impl ServerHandle {
    /// Wait for the server to resolve (decision, timeout, or cancellation).
    ///
    /// Aborts the background server task before returning.
    pub async fn outcome(self) -> ServerOutcome {
        match self.decision_rx.await {
            Ok(outcome) => {
                self.task.abort();
                outcome
            }
            Err(_) => {
                self.task.abort();
                ServerOutcome::Cancelled
            }
        }
    }

    /// Cancel the server immediately (abort the background task).
    pub fn cancel(self) {
        self.task.abort();
    }
}

/// Bind to a free localhost port, spawn the server, and return the address
/// plus a handle to await the result.
///
/// The server exits when:
/// - A POST handler successfully writes a response and sends a decision.
/// - The caller invokes [`ServerHandle::cancel`] or drops the handle.
/// - The background task panics.
pub async fn start(config: GateConfig) -> Result<ServerHandle, ServerError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(ServerError::Bind)?;
    let addr = listener.local_addr().map_err(ServerError::Bind)?;

    let (dec_tx, dec_rx) = oneshot::channel();

    let state = AppState::new(config, dec_tx);
    let app = router(state);

    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    Ok(ServerHandle {
        addr,
        decision_rx: dec_rx,
        task,
    })
}
