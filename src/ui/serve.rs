//! Daemon bootstrap for `loker ui --serve`.
//!
//! Binds to a localhost address, starts axum, and waits for SIGINT/SIGTERM.

use std::path::PathBuf;

use anyhow::Result;
use tokio::net::TcpListener;

use crate::ui::routes;

/// Bind to `bind`, serve `ui_routes(project_root)`, and run until SIGINT/SIGTERM.
///
/// Prints the listening address to stderr (daemon convention — stdout may be
/// piped; status messages go to stderr).
pub async fn serve(bind: &str, project_root: PathBuf) -> Result<()> {
    let app = routes::ui_routes(project_root);
    let listener = TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;

    // stderr: daemon convention — status messages go to stderr.
    eprintln!("loker UI daemon listening on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Wait for SIGINT or SIGTERM, then return.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    eprintln!("shutting down loker UI daemon");
}

/// Spawn the daemon on port 0 and return the join handle + bound address.
///
/// Used by integration tests to avoid port collisions. The daemon runs until
/// the returned `JoinHandle` is dropped or the spawned task panics.
#[cfg(test)]
pub async fn spawn_test_daemon(
    project_root: PathBuf,
) -> (tokio::task::JoinHandle<()>, std::net::SocketAddr) {
    let app = routes::ui_routes(project_root);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind test daemon");
    let addr = listener.local_addr().expect("failed to get test daemon addr");

    let handle = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                // Listen for Ctrl-C or SIGTERM, but tests can abort via drop.
                tokio::signal::ctrl_c()
                    .await
                    .ok();
            })
            .await
        {
            eprintln!("test daemon exited with error: {e}");
        }
    });

    (handle, addr)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use reqwest::Client;

    #[tokio::test]
    async fn serve_binds_and_responds() {
        let tmp = tempfile::tempdir().unwrap();
        let (_handle, addr) = spawn_test_daemon(tmp.path().to_path_buf()).await;

        let client = Client::new();
        let resp = client
            .get(format!("http://{addr}/"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body, serde_json::json!([]));
    }

    #[tokio::test]
    async fn serve_returns_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let runs_dir = tmp.path().join("runs").join("my-run-456");
        fs::create_dir_all(&runs_dir).unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "workflow_name": "test-wf",
            "run_id": "run-test-wf",
            "created_at": "2026-05-07T00:00:00Z",
            "entries": []
        });
        fs::write(runs_dir.join("manifest.json"), manifest.to_string().as_bytes()).unwrap();

        let (_handle, addr) = spawn_test_daemon(tmp.path().to_path_buf()).await;

        let client = Client::new();
        let resp = client
            .get(format!("http://{addr}/"))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);

        let body: serde_json::Value = resp.json().await.unwrap();
        let runs = body.as_array().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].get("id").and_then(|v| v.as_str()),
            Some("my-run-456")
        );
    }
}
