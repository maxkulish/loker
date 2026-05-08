//! One-shot axum server bootstrap for a single HITL gate.
//!
//! Binds to `127.0.0.1:0`, serves exactly one gate, and shuts down after
//! the first human decision or a caller-initiated cancellation.

use std::net::SocketAddr;

use tokio::net::TcpListener;
use tokio::sync::oneshot;

use crate::hitl_server::routes::{router, AppState};
use crate::hitl_server::{GateConfig, ServerError, ServerOutcome};

/// Handle to a running one-shot server.
pub struct ServerHandle {
    /// The bound localhost address (includes the auto-allocated port).
    pub addr: SocketAddr,
    /// Receiver that resolves when the server gets a decision.
    decision_rx: oneshot::Receiver<ServerOutcome>,
    /// Sender to trigger graceful shutdown (cancellation path).
    shutdown_tx: tokio::sync::watch::Sender<()>,
}

impl ServerHandle {
    /// Wait for the server to resolve (decision, timeout, or cancellation).
    pub async fn outcome(self) -> ServerOutcome {
        match self.decision_rx.await {
            Ok(outcome) => {
                let _ = self.shutdown_tx.send(());
                outcome
            }
            Err(_) => {
                let _ = self.shutdown_tx.send(());
                ServerOutcome::Cancelled
            }
        }
    }

    /// Cancel the server gracefully.
    pub fn cancel(self) {
        let _ = self.shutdown_tx.send(());
    }
}

/// Bind to a free localhost port, spawn the server, and return the address
/// plus a handle to await the result.
///
/// The server exits when:
/// - A POST handler successfully writes a response and sends a decision.
/// - The caller invokes [`ServerHandle::cancel`].
/// - The background task panics.
pub async fn start(config: GateConfig) -> Result<ServerHandle, ServerError> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(ServerError::Bind)?;
    let addr = listener.local_addr().map_err(ServerError::Bind)?;

    let (dec_tx, dec_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(());

    let state = AppState::new(config, dec_tx, Some(addr.port()));
    let app = router(state);

    tokio::spawn(async move {
        let mut shutdown_rx = shutdown_rx;
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.changed().await;
            })
            .await;
    });

    Ok(ServerHandle {
        addr,
        decision_rx: dec_rx,
        shutdown_tx,
    })
}
