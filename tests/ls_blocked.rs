#![cfg(test)]

use std::path::Path;

use chrono::{TimeZone, Utc};
use loker::commands::ls_blocked::{render_table, scan_blocked};
use serde_json::json;

fn write_pending(root: &Path, run_id: &str, phase: &str, severity: &str, opened_at: &str) {
    let path = root
        .join("runs")
        .join(run_id)
        .join("pending")
        .join(format!("{phase}.json"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let payload = json!({
        "schema_version": 1,
        "run_id": run_id,
        "workflow": "wf",
        "phase": phase,
        "severity": severity,
        "opened_at": opened_at,
        "timeout_at": null,
        "artefact": {
            "path": "review.md",
            "kind": "text/markdown",
            "preview_lines": 20
        },
        "context": {
            "preceded_by": [],
            "next_phase": null,
            "prompt_summary": "summary"
        },
        "decision_options": ["approve", "reject"]
    });
    std::fs::write(path, serde_json::to_vec_pretty(&payload).unwrap()).unwrap();
}

fn write_response(root: &Path, run_id: &str, phase: &str) {
    let path = root
        .join("runs")
        .join(run_id)
        .join("responses")
        .join(format!("{phase}.json"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, "{}").unwrap();
}

#[test]
fn snapshot_mixed_blocked_and_completed() {
    let tmp = tempfile::tempdir().unwrap();

    write_pending(
        tmp.path(),
        "resolved-run",
        "design",
        "low",
        "2026-05-06T08:00:00Z",
    );
    write_response(tmp.path(), "resolved-run", "design");

    write_pending(
        tmp.path(),
        "blocked-old",
        "review",
        "high",
        "2026-05-06T09:00:00Z",
    );

    write_pending(
        tmp.path(),
        "mixed-run",
        "implement",
        "medium",
        "2026-05-06T09:30:00Z",
    );
    write_response(tmp.path(), "mixed-run", "implement");
    write_pending(
        tmp.path(),
        "mixed-run",
        "verify",
        "medium",
        "2026-05-06T10:00:00Z",
    );

    let entries = scan_blocked(tmp.path()).unwrap();
    let mut out = Vec::new();
    render_table(
        &entries,
        Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap(),
        &mut out,
    )
    .unwrap();
    let out = String::from_utf8(out).unwrap();

    insta::assert_snapshot!(out, @r###"
RUN                      PHASE            SEVERITY AGE    RESPONSE
blocked-old              review           high     3h     runs/blocked-old/responses/review.json
mixed-run                verify           medium   2h     runs/mixed-run/responses/verify.json
"###);
}
