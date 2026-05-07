use std::fs;

#[tokio::main]
async fn main() {
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = tmp.path().to_path_buf();

    let pending = serde_json::json!({
        "schema_version": 1,
        "run_id": "run-1",
        "workflow": "design-doc-tdd",
        "phase": "review",
        "severity": "high",
        "opened_at": "2026-05-06T12:00:00Z",
        "timeout_at": null,
        "artefact": {"path": "review.md", "kind": "text/markdown", "preview_lines": 17},
        "context": {"preceded_by": [], "next_phase": null, "prompt_summary": "candidate output preview"},
        "decision_options": ["approve", "reject"]
    });
    let path = run_dir.join("pending").join("review.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(&pending).unwrap()).unwrap();

    let config = loker::hitl_server::GateConfig {
        run_dir: run_dir.clone(),
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
    };

    let handle = loker::hitl_server::one_shot::start(config).await.unwrap();
    let addr = handle.addr;
    println!("Server bound to {}", addr);

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}/approve", addr))
        .form(&[("comment", "looks good")])
        .send()
        .await
        .unwrap();
    println!("Status: {}", resp.status());
    println!("Body: {}", resp.text().await.unwrap());
}
