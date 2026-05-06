#![cfg(test)]

use std::path::Path;
use std::process::Command;

fn run_loker_explain(args: &[&str]) -> (bool, String) {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut cmd_args = vec!["run", "--quiet", "--bin", "loker", "--", "explain"];
    cmd_args.extend(args.iter().copied());

    let output = Command::new("cargo")
        .args(&cmd_args)
        .current_dir(manifest_dir)
        .output()
        .expect("failed to execute loker explain");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), format!("{}{}", stdout, stderr))
}

#[test]
fn test_explain_design_doc_tdd_snapshot() {
    let (success, output) = run_loker_explain(&["design-doc-tdd"]);

    assert!(success, "loker explain design-doc-tdd failed:\n{output}");
    insta::assert_snapshot!(output, @r###"
Workflow: design-doc-tdd
Source: ./.lok/workflows/design-doc-tdd.toml
Description: Four-phase design → review → implement → verify pipeline.

Warnings:
  - WARN: phase.contract reserved for post-v0; ignored (phase 'review')

Phase order:
  1. design
  2. review (depends on: design)
  3. implement (depends on: design, review)
  4. verify (depends on: implement, design)

Phases:
  design
    strategy: single
    backends: ollama/qwen3-coder-next
    inputs: spec
    prompt_template: ../prompts/design-doc-tdd/design.md.tmpl
    output: design.md
    verify: none

  review
    strategy: parallel (min_responses: 2)
    backends: claude/, gemini/, codex/, ollama/qwen3-coder-next
    inputs: phase:design
    depends_on: design
    prompt_template: ../prompts/design-doc-tdd/review.md.tmpl
    output: review.md
    verify: contract reserved

  implement
    strategy: escalating (pass_failure_context: true)
    backends: ollama/qwen3-coder-next, claude/, codex/
    inputs: phase:design, phase:review
    depends_on: design, review
    prompt_template: ../prompts/design-doc-tdd/implement.md.tmpl
    output: changes
    verify: none

  verify
    strategy: parallel (min_responses: 1)
    backends: codex/, gemini/
    inputs: phase:implement, phase:design
    depends_on: implement, design
    prompt_template: ../prompts/design-doc-tdd/verify.md.tmpl
    output: verify.json
    verify: none
"###);
}

#[test]
fn test_explain_workflow_path_snapshot() {
    let (success, output) = run_loker_explain(&["tests/fixtures/workflows/design-doc-tdd.toml"]);

    assert!(success, "loker explain workflow path failed:\n{output}");
    assert!(output.contains("Workflow: design-doc-tdd"));
    assert!(output.contains("Source: ./tests/fixtures/workflows/design-doc-tdd.toml"));
    assert!(output.contains("4. verify (depends on: implement, design)"));
    assert!(output.contains("strategy: escalating (pass_failure_context: true)"));
}

#[test]
fn test_explain_missing_reference_errors() {
    let (success, output) =
        run_loker_explain(&["tests/fixtures/workflows/explain-missing-ref.toml"]);

    assert!(
        !success,
        "invalid workflow unexpectedly succeeded:\n{output}"
    );
    assert!(
        output.contains("Invalid workflow:"),
        "missing invalid-workflow header:\n{output}"
    );
    assert!(
        output.contains("references unknown phase 'missing'"),
        "missing unknown phase error:\n{output}"
    );
}

#[test]
fn test_explain_forward_reference_errors() {
    let (success, output) =
        run_loker_explain(&["tests/fixtures/workflows/explain-forward-ref.toml"]);

    assert!(
        !success,
        "invalid workflow unexpectedly succeeded:\n{output}"
    );
    assert!(
        output.contains("references phase 'design' which is declared later"),
        "missing forward reference error:\n{output}"
    );
}

#[test]
fn test_explain_unknown_workflow_name_reports_lookup_error() {
    let (success, output) = run_loker_explain(&["definitely-not-a-workflow"]);

    assert!(!success, "unknown workflow unexpectedly succeeded");
    assert!(
        output.contains("Workflow 'definitely-not-a-workflow' not found"),
        "missing workflow lookup error:\n{output}"
    );
    assert!(
        !output.contains("Lok Explain"),
        "unknown workflow name fell through to codebase mode:\n{output}"
    );
}

#[test]
fn test_explain_falls_back_to_codebase_mode_for_directory() {
    let (success, output) =
        run_loker_explain(&[".", "--focus", "workflow", "--backend", "does-not-exist"]);

    assert!(
        !success,
        "unknown backend should fail after codebase fallback"
    );
    assert!(
        output.contains("Lok Explain"),
        "did not enter codebase explain mode:\n{output}"
    );
    assert!(
        !output.contains("Workflow:"),
        "directory was treated as a workflow:\n{output}"
    );
    assert!(
        !output.contains("Invalid workflow:"),
        "directory was parsed as a workflow:\n{output}"
    );
}
