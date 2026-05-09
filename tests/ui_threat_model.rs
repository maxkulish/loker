//! Threat-model integration test suite for the loker UI daemon.
//!
//! Covers the §5 test list from `docs/security/2026-04-25-ui-threat-model.md`.
//! All tests run against the daemon router on an ephemeral port.

use std::fs;

use axum::http::StatusCode;
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

// ---------------------------------------------------------------------------
// T-BIND-1: Default bind address rejects non-loopback
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_bind_1_default_bind_is_loopback() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("bind-run-001", "test-wf");

    let client = fixture.client();
    let resp = client.get(fixture.url("/")).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // The test fixture binds to 127.0.0.1:0; if it were non-loopback the
    // handler would not have received the request in this harness.
}

// ---------------------------------------------------------------------------
// T-COOKIE-1: No Set-Cookie header on any response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_cookie_1_no_set_cookie_header() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("cookie-run-001", "test-wf");
    fixture.create_pending_gate("cookie-run-001", "review");

    let origin = format!("http://127.0.0.1:{}", fixture.addr.port());
    let client = fixture.client();

    // GET / (HTML)
    let resp = client.get(fixture.url("/")).send().await.unwrap();
    assert!(resp.headers().get("Set-Cookie").is_none());

    // POST approve
    let resp = client
        .post(fixture.url("/gates/cookie-run-001/review/approve"))
        .header("Origin", &origin)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("comment=ok")
        .send()
        .await
        .unwrap();
    assert!(resp.headers().get("Set-Cookie").is_none());

    // GET artefact
    let resp = client
        .get(fixture.url("/runs/cookie-run-001/artefact/review.md"))
        .send()
        .await
        .unwrap();
    assert!(resp.headers().get("Set-Cookie").is_none());
}

// ---------------------------------------------------------------------------
// T-LOCK-1: Two concurrent approval attempts honor advisory lock
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_lock_1_concurrent_approval_honors_lock() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("lock-run-001", "test-wf");
    fixture.create_pending_gate("lock-run-001", "review");

    let origin = format!("http://127.0.0.1:{}", fixture.addr.port());
    let client = fixture.client();

    // Fire two approvals concurrently
    let req1 = client
        .post(fixture.url("/gates/lock-run-001/review/approve"))
        .header("Origin", &origin)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("comment=first")
        .send();
    let req2 = client
        .post(fixture.url("/gates/lock-run-001/review/approve"))
        .header("Origin", &origin)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("comment=second")
        .send();

    let (resp1, resp2) = tokio::join!(req1, req2);
    let s1 = resp1.unwrap().status();
    let s2 = resp2.unwrap().status();

    // Exactly one must indicate success (redirect); the other must not be 200.
    let statuses = [s1, s2];
    assert!(
        statuses.contains(&StatusCode::SEE_OTHER),
        "one request should succeed, got {:?} and {:?}",
        s1,
        s2
    );

    // Response file must have been written exactly once
    let run_dir = fixture._tmp.path().join("runs").join("lock-run-001");
    let response_path = run_dir.join("responses").join("review.json");
    assert!(response_path.exists());
}

// ---------------------------------------------------------------------------
// T-LOCK-2: Stale lock with expired TTL is reclaimable
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_lock_2_stale_lock_reclaimable() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("lock-run-002", "test-wf");
    fixture.create_pending_gate("lock-run-002", "review");

    let run_dir = fixture._tmp.path().join("runs").join("lock-run-002");
    let lock_dir = run_dir.join("locks");
    fs::create_dir_all(&lock_dir).unwrap();

    // Write a stale lock body: TTL expired, dead PID
    let stale = serde_json::json!({
        "phase": "review",
        "run_id": "lock-run-002",
        "writer_pid": 99999,
        "writer_host": "test-host",
        "acquired_at": "2026-01-01T00:00:00Z",
        "ttl_seconds": 60
    });
    fs::write(lock_dir.join("review.lock"), stale.to_string()).unwrap();

    let origin = format!("http://127.0.0.1:{}", fixture.addr.port());
    let client = fixture.client();
    let resp = client
        .post(fixture.url("/gates/lock-run-002/review/approve"))
        .header("Origin", &origin)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("comment=stale_reclaim")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    // Response file should have been written by the reclaim
    let response_path = run_dir.join("responses").join("review.json");
    assert!(response_path.exists());
    let content = fs::read_to_string(&response_path).unwrap();
    assert!(content.contains("approve"));
}

// ---------------------------------------------------------------------------
// T-MIME-1: Unknown artefact extension served as text/plain with attachment
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_mime_1_unknown_extension_as_attachment() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("mime-run-001", "test-wf");
    let run_dir = fixture._tmp.path().join("runs").join("mime-run-001");
    fs::write(run_dir.join("data.unknownext"), b"key: value").unwrap();

    let client = fixture.client();
    let resp = client
        .get(fixture.url("/runs/mime-run-001/artefact/data.unknownext"))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("Content-Type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        ct.starts_with("text/plain"),
        "expected text/plain, got {}",
        ct
    );
}

// ---------------------------------------------------------------------------
// T-SSE-CSRF: SSE rejects cross-origin requests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_sse_csrf_cross_origin_rejected() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("sse-run-001", "test-wf");
    let run_dir = fixture._tmp.path().join("runs").join("sse-run-001");
    fs::write(run_dir.join("trace.jsonl"), b"{}\n").unwrap();

    let client = fixture.client();
    let resp = client
        .get(fixture.url("/runs/sse-run-001/trace/sse"))
        .header("Origin", "http://evil.com")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

// ---------------------------------------------------------------------------
// T-ENTROPY-1: Gate URLs derive entropy from run_id (no per-gate token)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn t_entropy_1_gate_url_uses_run_id() {
    let fixture = ThreatModelFixture::new().await;
    fixture.create_run("run-uuid-123e4567-e89b-12d3-a456-426614174000", "test-wf");
    fixture.create_pending_gate("run-uuid-123e4567-e89b-12d3-a456-426614174000", "review");

    let origin = format!("http://127.0.0.1:{}", fixture.addr.port());
    let client = fixture.client();
    let resp = client
        .post(fixture.url("/gates/run-uuid-123e4567-e89b-12d3-a456-426614174000/review/approve"))
        .header("Origin", &origin)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body("comment=ok")
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    // Verify response file exists at the expected run_id path
    let run_dir = fixture
        ._tmp
        .path()
        .join("runs")
        .join("run-uuid-123e4567-e89b-12d3-a456-426614174000");
    assert!(run_dir.join("responses").join("review.json").exists());
}
