//! ST5: Integration tests for the phase-based workflow runner (CLO-327).
//!
//! Exercises the full pipeline:
//! - Workflow file detection and grammar parse
//! - PhaseConfig builder (template pre-rendering, strategy mapping)
//! - Phase output chaining
//! - Step-workflow regression (grammar parse correctly rejects step-based files)

use std::collections::HashMap;
use std::path::PathBuf;

/// Test that the grammar parser correctly detects phase-based workflows.
#[test]
fn loker_run_phase_workflow_detection() {
    let phase_toml = r#"
name = "test-workflow"

[[phases]]
name = "design"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Design the system"
inputs = ["spec"]
output = "design.md"

[[phases]]
name = "review"
strategy = { single = {} }
backends = ["gemini/"]
prompt_template = "Review the design"
inputs = ["phase:design"]
output = "review.md"
"#;

    let wf: Result<loker::workflow::grammar::Workflow, _> = phase_toml.parse();
    assert!(
        wf.is_ok(),
        "Phase-based TOML should parse as grammar::Workflow"
    );
    let wf = wf.unwrap();
    assert_eq!(wf.name, "test-workflow");
    assert_eq!(wf.phases.len(), 2);
    assert_eq!(wf.phases[0].name, "design");
    assert_eq!(wf.phases[1].name, "review");
}

/// Test that step-based workflow TOML is rejected by the grammar parser.
#[test]
fn loker_run_step_workflow_unchanged() {
    let step_toml = r#"
name = "step-workflow"
description = "A step-based workflow with no [[phases]]"

[[steps]]
name = "build"
backend = "claude/"
prompt = "Build the system"
"#;

    let wf: Result<loker::workflow::grammar::Workflow, _> = step_toml.parse();
    assert!(
        wf.is_err(),
        "Step-based TOML should FAIL grammar parse (no [[phases]] sections)"
    );
}

/// Test that phase output chaining produces correct pre-rendered templates.
#[test]
fn loker_run_phase_with_input_chaining() {
    // Simulate what run_phase_workflow does: after executing design,
    // pass its output as context for review.
    let phase_outputs: HashMap<String, (String, PathBuf)> = [(
        "design".to_string(),
        (
            "Design output content".to_string(),
            PathBuf::from("design.md"),
        ),
    )]
    .into();

    let review_toml = r#"
name = "test"
[[phases]]
name = "design"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Design the system"
inputs = ["spec"]
output = "design.md"

[[phases]]
name = "review"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Review {{ phase.design.output }}"
inputs = ["phase:design"]
output = "review.md"
"#;
    let wf: loker::workflow::grammar::Workflow = review_toml.parse().unwrap();
    let review_phase = &wf.phases[1];

    let cfg = loker::workflow::phase_bridge::build_phase_config(
        review_phase,
        &phase_outputs,
        None,
        &HashMap::new(),
    )
    .unwrap();

    assert!(
        cfg.prompt_template.contains("Design output content"),
        "Phase output chaining should resolve the design output content"
    );
    assert!(
        !cfg.prompt_template.contains("{{ phase.design.output }}"),
        "Phase output placeholder should be resolved"
    );
    assert_eq!(cfg.phase, "review");
    assert_eq!(cfg.strategy, loker::phase_runner::StrategyName::Single);
}

/// Test that the grammar parser correctly handles various strategy types.
#[test]
fn loker_run_phase_workflow_strategy_variants() {
    // Single strategy
    let single_toml = r#"
name = "single-test"
[[phases]]
name = "single-phase"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Single prompt"
inputs = ["spec"]
output = "out.md"
"#;
    let wf: loker::workflow::grammar::Workflow = single_toml.parse().unwrap();
    assert_eq!(wf.phases.len(), 1);

    // Parallel strategy
    let parallel_toml = r#"
name = "parallel-test"
[[phases]]
name = "parallel-phase"
strategy = { parallel = { min_responses = 2 } }
backends = ["claude/", "gemini/"]
prompt_template = "Parallel prompt"
inputs = ["spec"]
output = "out.md"
"#;
    let wf: loker::workflow::grammar::Workflow = parallel_toml.parse().unwrap();
    assert_eq!(wf.phases.len(), 1);

    // Escalating strategy
    let escalating_toml = r#"
name = "escalating-test"
[[phases]]
name = "escalating-phase"
strategy = { escalating = { pass_failure_context = false } }
backends = ["claude/", "gemini/"]
prompt_template = "Escalating prompt"
inputs = ["spec"]
output = "out.md"
"#;
    let wf: loker::workflow::grammar::Workflow = escalating_toml.parse().unwrap();
    assert_eq!(wf.phases.len(), 1);
}
