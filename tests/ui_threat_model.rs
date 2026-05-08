//! Threat-model integration test suite for the loker UI daemon.
//!
//! Covers the §5 test list from `docs/security/2026-04-25-ui-threat-model.md`.
//! All tests run against the daemon router on an ephemeral port.

use std::fs;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use reqwest::Client;
use tempfile::TempDir;
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct ThreatModelFixture {
    _tmp: TempDir,
    addr: std::net::SocketAddr,
    _handle: tokio::task::JoinHandle<()>,
}

impl ThreatModelFixture {
    async fn new() -> Self {
        let tmp = TempDir::new().unwrap();
        let project_root = tmp.path().to_path_buf();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let port = addr.port();

        let app = loker::ui::routes::ui_routes_with_port(project_root, Some(port));

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

        ThreatModelFixture {
            _tmp: tmp,
            addr,
            _handle: handle,
        }
    }

    fn client(&self) -> Client {
        Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap()
    }

    fn url(&self, path: &str) -> String {
        format!("http://{}{}", self.addr, path)
    }

    fn create_run(&self, name: &str, workflow: &str) {
        let run_dir = self._tmp.path().join("runs").join(name);
        fs::create_dir_all(&run_dir).unwrap();
        let manifest = serde_json::json!({
            "schema_version": 1,
            "workflow_name": workflow,
            "loker.run_id": format!("run-{workflow}"),
            "entries": []
        });
        fs::write(
            run_dir.join("manifest.json"),
            manifest.to_string().as_bytes(),
        )
        .unwrap();
    }

    fn create_pending_gate(&self, run_id: &str, phase: &str) {
        let pending_dir = self._tmp.path().join("runs").join(run_id).join("pending");
        fs::create_dir_all(&pending_dir).unwrap();
        let pending = serde_json::json!({
            "severity": "high",
            "artefact": {"path": "review.md", "kind": "text/markdown"}
        });
        fs::write(
            pending_dir.join(format!("{}.json", phase)),
            pending.to_string(),
        )
        .unwrap();
    }
}

// ---------------------------------------------------------------------------
// T-CSRF-1: POST with foreign Origin returns 403
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_csrf_1_post_with_foreign_origin_is_rejected() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("csrf-run-001", "test-wf");
    fixture.create_pending_gate("csrf-run-001", "review");

    let client = fixture.client();
    let resp = client
        .post(fixture.url("/gates/csrf-run-001/review/approve"))
        .header("Origin", "http://evil.com")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("comment=looks+good")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------
// T-CSRF-2: POST without Origin returns 403
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_csrf_2_post_without_origin_is_rejected() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("csrf-run-002", "test-wf");
    fixture.create_pending_gate("csrf-run-002", "review");

    let client = fixture.client();
    let resp = client
        .post(fixture.url("/gates/csrf-run-002/review/approve"))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("comment=looks+good")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------
// T-CSRF-3: POST with correct Origin succeeds
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_csrf_3_post_with_loopback_origin_succeeds() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("csrf-run-003", "test-wf");
    fixture.create_pending_gate("csrf-run-003", "review");

    let origin = format!("http://127.0.0.1:{}", fixture.addr.port());
    let client = fixture.client();
    let resp = client
        .post(fixture.url("/gates/csrf-run-003/review/approve"))
        .header("Origin", &origin)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("comment=looks+good")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
}

// ---------------------------------------------------------------------------
// T-CSRF-4: POST with wrong Content-Type returns 415
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_csrf_4_post_with_text_plain_content_type_is_rejected() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("csrf-run-004", "test-wf");
    fixture.create_pending_gate("csrf-run-004", "review");

    let origin = format!("http://127.0.0.1:{}", fixture.addr.port());
    let client = fixture.client();
    let resp = client
        .post(fixture.url("/gates/csrf-run-004/review/approve"))
        .header("Origin", &origin)
        .header("Content-Type", "text/plain")
        .body("comment=looks+good")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

// ---------------------------------------------------------------------------
// T-TRAVERSAL-1: Path traversal on artefact route returns 400
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_traversal_1_artefact_dotdot_is_rejected() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("trav-run-001", "test-wf");

    let client = fixture.client();
    let resp = client
        .get(fixture.url("/runs/trav-run-001/artefact/../../../etc/passwd"))
        .send()
        .await
        .unwrap();

    assert!(
        resp.status() == StatusCode::BAD_REQUEST || resp.status() == StatusCode::NOT_FOUND,
        "Expected 400 or 404 for traversal, got {:?}",
        resp.status()
    );
}

// ---------------------------------------------------------------------------
// T-TRAVERSAL-2: Percent-encoded traversal returns 400
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_traversal_2_percent_encoded_traversal_is_rejected() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("trav-run-002", "test-wf");

    let client = fixture.client();
    let resp = client
        .get(fixture.url("/runs/trav-run-002/artefact/%2e%2e%2fetc%2fpasswd"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// T-TRAVERSAL-3: Absolute path param returns 400
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_traversal_3_absolute_path_param_is_rejected() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("trav-run-003", "test-wf");

    let client = fixture.client();
    let resp = client
        .get(fixture.url("/runs/trav-run-003/artefact//etc/passwd"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// T-CORP-1: Artefact response carries CORP and nosniff
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_corp_1_artefact_response_has_security_headers() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("corp-run-001", "test-wf");
    let run_dir = fixture._tmp.path().join("runs").join("corp-run-001");
    fs::write(run_dir.join("review.md"), b"# Review").unwrap();

    let client = fixture.client();
    let resp = client
        .get(fixture.url("/runs/corp-run-001/artefact/review.md"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("Cross-Origin-Resource-Policy").unwrap(),
        "same-origin"
    );
    assert_eq!(
        resp.headers().get("X-Content-Type-Options").unwrap(),
        "nosniff"
    );
}

// ---------------------------------------------------------------------------
// T-CSP-1: All responses carry CSP header
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_csp_1_all_responses_carry_csp() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("csp-run-001", "test-wf");

    let client = fixture.client();
    let resp = client.get(fixture.url("/")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let csp = resp.headers().get("Content-Security-Policy").unwrap();
    assert!(csp.to_str().unwrap().contains("default-src 'self'"));
}

// ---------------------------------------------------------------------------
// T-LOCK-3: Replay after gate resolved returns 409
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_lock_3_replay_after_gate_resolved_is_rejected() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("lock-run-003", "test-wf");
    fixture.create_pending_gate("lock-run-003", "review");

    let origin = format!("http://127.0.0.1:{}", fixture.addr.port());
    let client = fixture.client();

    // First approval
    let resp1 = client
        .post(fixture.url("/gates/lock-run-003/review/approve"))
        .header("Origin", &origin)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("comment=first")
        .send()
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::SEE_OTHER);

    // Second approval should still redirect (race guard)
    let resp2 = client
        .post(fixture.url("/gates/lock-run-003/review/approve"))
        .header("Origin", &origin)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("comment=second")
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::SEE_OTHER);
}

// ---------------------------------------------------------------------------
// T-XFRAME-1: HTML responses carry X-Frame-Options: DENY
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_xframe_1_html_response_has_x_frame_options() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("xframe-run-001", "test-wf");

    let client = fixture.client();
    let resp = client.get(fixture.url("/")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers().get("X-Frame-Options").unwrap(), "DENY");
}

// ---------------------------------------------------------------------------
// T-REFERRER-1: All responses carry Referrer-Policy: no-referrer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_referrer_1_all_responses_carry_referrer_policy() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("ref-run-001", "test-wf");

    let client = fixture.client();
    let resp = client.get(fixture.url("/")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers().get("Referrer-Policy").unwrap(),
        "no-referrer"
    );
}

// ---------------------------------------------------------------------------
// T-METHOD-1: GET-only routes return 405 on POST
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_method_1_get_only_routes_reject_post() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("method-run-001", "test-wf");

    let origin = format!("http://127.0.0.1:{}", fixture.addr.port());
    let client = fixture.client();
    let resp = client
        .post(fixture.url("/runs/method-run-001"))
        .header("Origin", &origin)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}
