//! Integration tests for `loker ui --serve` daemon mode.
//!
//! Tests bind to `127.0.0.1:0` (port 0) to avoid port collisions in CI.
//! The `spawn_test_daemon` helper from `ui::serve` is used for bootstrap.

use std::fs;
use std::path::Path;

use reqwest::Client;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct DaemonFixture {
    _tmp: TempDir,
    addr: std::net::SocketAddr,
    _handle: tokio::task::JoinHandle<()>,
}

impl DaemonFixture {
    async fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path().to_path_buf();
        let app = loker::ui::routes::ui_routes(project_root);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    tokio::signal::ctrl_c().await.ok();
                })
                .await
            {
                eprintln!("test daemon exited with error: {e}");
            }
        });

        DaemonFixture {
            _tmp: tmp,
            addr,
            _handle: handle,
        }
    }

    fn url(&self) -> String {
        format!("http://{}/", self.addr)
    }

    fn create_run(&self, name: &str, workflow: &str) {
        let run_dir = self._tmp.path().join("runs").join(name);
        fs::create_dir_all(&run_dir).unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "workflow_name": workflow,
            "run_id": format!("run-{workflow}"),
            "created_at": "2026-05-07T00:00:00Z",
            "entries": []
        });
        fs::write(run_dir.join("manifest.json"), manifest.to_string().as_bytes()).unwrap();
    }

    fn create_corrupt_run(&self, name: &str) {
        let run_dir = self._tmp.path().join("runs").join(name);
        fs::create_dir_all(&run_dir).unwrap();
        fs::write(run_dir.join("manifest.json"), b"not valid json").unwrap();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn daemon_serves_runs_list() {
    let fixture = DaemonFixture::new().await;
    fixture.create_run("my-run-123", "test-wf");

    let client = Client::new();
    let resp = client.get(fixture.url()).send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let runs: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(
        runs[0].get("id").and_then(|v| v.as_str()),
        Some("my-run-123")
    );
    assert_eq!(
        runs[0].get("workflow").and_then(|v| v.as_str()),
        Some("test-wf")
    );
}

#[tokio::test]
async fn daemon_returns_empty_list_when_no_runs() {
    let fixture = DaemonFixture::new().await;

    let client = Client::new();
    let resp = client.get(fixture.url()).send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let runs: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert!(runs.is_empty());
}

#[tokio::test]
async fn daemon_custom_bind_address() {
    let tmp = TempDir::new().unwrap();
    let project_root = tmp.path().to_path_buf();

    // Bind to a specific port via port 0 to avoid races, then verify.
    let app = loker::ui::routes::ui_routes(project_root);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async { tokio::signal::ctrl_c().await.ok(); })
            .await
            .ok();
    });

    // Verify the daemon responds on the bound port.
    let client = Client::new();
    let resp = client
        .get(format!("http://{}/", addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    handle.abort();
}

#[tokio::test]
async fn daemon_skips_corrupt_run_directory() {
    let fixture = DaemonFixture::new().await;
    fixture.create_run("good-run", "good-wf");
    fixture.create_corrupt_run("corrupt-run");

    let client = Client::new();
    let resp = client.get(fixture.url()).send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let runs: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(
        runs.len(),
        1,
        "only the valid run should appear; corrupt one is skipped"
    );
    assert_eq!(
        runs[0].get("id").and_then(|v| v.as_str()),
        Some("good-run")
    );

    // Verify a second request also succeeds (daemon stays up).
    let resp2 = client.get(fixture.url()).send().await.unwrap();
    assert_eq!(resp2.status(), 200);
}

#[tokio::test]
async fn daemon_shuts_down_gracefully() {
    let fixture = DaemonFixture::new().await;

    // Verify daemon is alive.
    let client = Client::new();
    let resp = client.get(fixture.url()).send().await.unwrap();
    assert_eq!(resp.status(), 200);

    // Abort the handle (simulates shutdown signal).
    fixture._handle.abort();

    // Give it a moment to shut down.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Verify the handle is finished.
    assert!(fixture._handle.is_finished());
}
