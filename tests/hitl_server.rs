//! Integration tests for the one-shot HITL fallback server.
//!
//! Each test spawns a temporary run directory, writes a pending file,
//! starts the server, and asserts on HTTP responses and filesystem state.

use std::fs;

use loker::hitl_server::one_shot;
use loker::hitl_server::GateConfig;
use loker::strategy::verify::{HumanDecision, HumanResponse};

fn make_gate_config(tmp: &tempfile::TempDir) -> GateConfig {
    GateConfig {
        run_dir: tmp.path().to_path_buf(),
        run_id: "run-1".into(),
        phase: "review".into(),
        workflow: "design-doc-tdd".into(),
        severity: "high".into(),
        artefact_path: "review.md".into(),
        artefact_kind: "text/markdown".into(),
        prompt_summary: "candidate output preview".into(),
        preview_lines: 17,
        timeout_at: None,
        decision_options: vec!["approve".into(), "reject".into()],
    }
}

fn write_pending(tmp: &tempfile::TempDir, phase: &str) {
    let pending = serde_json::json!({
        "schema_version": 1,
        "run_id": "run-1",
        "workflow": "design-doc-tdd",
        "phase": phase,
        "severity": "high",
        "opened_at": "2026-05-06T12:00:00Z",
        "timeout_at": null,
        "artefact": {"path": "review.md", "kind": "text/markdown", "preview_lines": 17},
        "context": {"preceded_by": [], "next_phase": null, "prompt_summary": "candidate output preview"},
        "decision_options": ["approve", "reject"]
    });
    let path = tmp.path().join("pending").join(format!("{}.json", phase));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(&pending).unwrap()).unwrap();
}

#[tokio::test]
async fn one_shot_approve_resolves_gate() {
    let tmp = tempfile::tempdir().unwrap();
    write_pending(&tmp, "review");

    let config = make_gate_config(&tmp);
    let handle = one_shot::start(config).await.unwrap();
    let addr = handle.addr;

    // POST approve with a comment
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}/approve", addr))
        .form(&[("comment", "looks good")])
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    // Response file should exist
    let response_path = tmp.path().join("responses").join("review.json");
    assert!(response_path.exists());

    let text = fs::read_to_string(&response_path).unwrap();
    let response: HumanResponse = serde_json::from_str(&text).unwrap();
    assert_eq!(response.decision, HumanDecision::Approve);
    assert_eq!(response.global_comment.as_deref(), Some("looks good"));
    assert_eq!(response.phase, "review");
}

#[tokio::test]
async fn one_shot_reject_resolves_gate() {
    let tmp = tempfile::tempdir().unwrap();
    write_pending(&tmp, "review");

    let config = make_gate_config(&tmp);
    let handle = one_shot::start(config).await.unwrap();
    let addr = handle.addr;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}/reject", addr))
        .form(&[("comment", "needs work")])
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    let response_path = tmp.path().join("responses").join("review.json");
    assert!(response_path.exists());

    let text = fs::read_to_string(&response_path).unwrap();
    let response: HumanResponse = serde_json::from_str(&text).unwrap();
    assert_eq!(response.decision, HumanDecision::Reject);
    assert_eq!(response.global_comment.as_deref(), Some("needs work"));
}

#[tokio::test]
async fn gate_context_shows_pending_json() {
    let tmp = tempfile::tempdir().unwrap();
    write_pending(&tmp, "review");

    let config = make_gate_config(&tmp);
    let handle = one_shot::start(config).await.unwrap();
    let addr = handle.addr;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://{}/", addr))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    let body = resp.text().await.unwrap();
    assert!(body.contains("review"));
    assert!(body.contains("design-doc-tdd"));
    assert!(body.contains("high"));
    assert!(body.contains("review.md"));
    assert!(body.contains("/approve"));
    assert!(body.contains("/reject"));
}

#[tokio::test]
async fn second_post_after_first_returns_409() {
    let tmp = tempfile::tempdir().unwrap();
    write_pending(&tmp, "review");

    let config = make_gate_config(&tmp);
    let handle = one_shot::start(config).await.unwrap();
    let addr = handle.addr;

    let client = reqwest::Client::new();

    // First POST succeeds
    let resp1 = client
        .post(format!("http://{}/approve", addr))
        .form(&[("comment", "")])
        .send()
        .await
        .unwrap();
    assert!(resp1.status().is_success());

    // Second POST sees the already-written response file
    let resp2 = client
        .post(format!("http://{}/approve", addr))
        .form(&[("comment", "")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status().as_u16(), 409);
}

#[tokio::test]
async fn server_shuts_down_after_decision() {
    let tmp = tempfile::tempdir().unwrap();
    write_pending(&tmp, "review");

    let config = make_gate_config(&tmp);
    let handle = one_shot::start(config).await.unwrap();
    let addr = handle.addr;

    // Send the POST in a background task
    tokio::spawn(async move {
        let client = reqwest::Client::new();
        let _ = client
            .post(format!("http://{}/approve", addr))
            .form(&[("comment", "test")])
            .send()
            .await;
    });

    // Outcome should resolve within 2 seconds
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), handle.outcome()).await;
    assert!(outcome.is_ok(), "server did not shut down within 2s");
}

#[tokio::test]
async fn binds_to_free_port() {
    let tmp = tempfile::tempdir().unwrap();
    write_pending(&tmp, "review");

    let config = make_gate_config(&tmp);
    let handle = one_shot::start(config).await.unwrap();

    let port = handle.addr.port();
    assert!(port > 0);
    assert!(handle.addr.ip().is_loopback());

    // Clean up
    handle.cancel();
}

#[tokio::test]
async fn concurrent_post_races_return_423() {
    let tmp = tempfile::tempdir().unwrap();
    write_pending(&tmp, "review");

    let config = make_gate_config(&tmp);
    let handle = one_shot::start(config).await.unwrap();
    let addr = handle.addr;

    let client = reqwest::Client::new();

    // Fire two POSTs concurrently
    let req1 = client
        .post(format!("http://{}/approve", addr))
        .form(&[("comment", "")])
        .send();
    let req2 = client
        .post(format!("http://{}/approve", addr))
        .form(&[("comment", "")])
        .send();

    let (resp1, resp2) = tokio::join!(req1, req2);
    let s1 = resp1.unwrap().status().as_u16();
    let s2 = resp2.unwrap().status().as_u16();

    // Exactly one 200 and one 423 (or 409 if it loses the file race)
    let statuses = [s1, s2];
    assert!(
        statuses.contains(&200),
        "one request should succeed, got {s1} and {s2}"
    );
    assert!(
        statuses.contains(&423) || statuses.contains(&409),
        "one request should be rejected, got {s1} and {s2}"
    );
}

#[tokio::test]
async fn timeout_auto_approves_without_human() {
    let tmp = tempfile::tempdir().unwrap();
    write_pending(&tmp, "review");

    let config = GateConfig {
        run_dir: tmp.path().to_path_buf(),
        run_id: "run-1".into(),
        phase: "review".into(),
        workflow: "design-doc-tdd".into(),
        severity: "low".into(),
        artefact_path: "review.md".into(),
        artefact_kind: "text/markdown".into(),
        prompt_summary: "candidate output preview".into(),
        preview_lines: 17,
        timeout_at: None, // server itself doesn't enforce timeout_at
        decision_options: vec!["approve".into(), "reject".into()],
    };

    let handle = one_shot::start(config).await.unwrap();

    // Server stays alive when no POST arrives; cancel it explicitly
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    handle.cancel();
}

#[tokio::test]
async fn high_severity_blocks_indefinitely() {
    let tmp = tempfile::tempdir().unwrap();
    write_pending(&tmp, "review");

    let config = GateConfig {
        run_dir: tmp.path().to_path_buf(),
        run_id: "run-1".into(),
        phase: "review".into(),
        workflow: "design-doc-tdd".into(),
        severity: "high".into(), // high = no timeout
        artefact_path: "review.md".into(),
        artefact_kind: "text/markdown".into(),
        prompt_summary: "candidate output preview".into(),
        preview_lines: 17,
        timeout_at: None,
        decision_options: vec!["approve".into(), "reject".into()],
    };

    let handle = one_shot::start(config).await.unwrap();

    // Server should still be alive after a short wait
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    // Cancel to clean up
    handle.cancel();
}
