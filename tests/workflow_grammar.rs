//! Integration tests for the TOML workflow grammar parser (CLO-287).
//!
//! Tests the phase-based workflow AST at `crate::workflow::grammar`.
//! Each test maps to a TDD test contract item from the CLO-287 spec.

use std::path::Path;

use loker::workflow::grammar::{BackendRef, InputRef, Strategy, Workflow, WorkflowError};

/// Helper: read a fixture file and parse it.
fn parse_fixture(name: &str) -> Result<Workflow, Vec<WorkflowError>> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("workflows")
        .join(name);
    let toml = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read fixture {}: {}", path.display(), e));
    toml.parse::<Workflow>()
}

// ---------------------------------------------------------------------------
// Test 1: Byte-for-byte design-doc-tdd — parse the fixture and assert all
// four phases round-trip into the AST.
// ---------------------------------------------------------------------------

#[test]
fn byte_for_byte_design_doc_tdd() {
    let wf = parse_fixture("design-doc-tdd.toml")
        .expect("design-doc-tdd.toml should parse successfully");

    assert_eq!(wf.name, "design-doc-tdd");
    assert_eq!(
        wf.description.as_deref(),
        Some("Design doc with test-driven development workflow")
    );
    assert_eq!(wf.phases.len(), 4, "Expected 4 phases");

    // Phase 1: research — Single, claude, spec input
    let research = &wf.phases[0];
    assert_eq!(research.name, "research");
    assert_eq!(research.strategy, Strategy::Single);
    assert_eq!(research.backends.len(), 1);
    assert_eq!(research.backends[0], "claude/");
    assert!(research.inputs.contains(&"spec".to_string()));
    assert_eq!(research.output, "research.md");

    // Phase 2: design — ParallelFanOut, 2 backends, research input
    let design = &wf.phases[1];
    assert_eq!(design.name, "design");
    assert_eq!(
        design.strategy,
        Strategy::ParallelFanOut { min_responses: 2 }
    );
    assert_eq!(design.backends.len(), 2);
    assert_eq!(design.backends[0], "claude/");
    assert_eq!(design.backends[1], "gemini/");
    assert!(design.inputs.contains(&"phase:research".to_string()));
    assert_eq!(design.output, "design.md");

    // Phase 3: implement — Single, codex, design + var inputs
    let implement = &wf.phases[2];
    assert_eq!(implement.name, "implement");
    assert_eq!(implement.strategy, Strategy::Single);
    assert_eq!(implement.backends.len(), 1);
    assert_eq!(implement.backends[0], "codex/");
    assert!(implement.inputs.contains(&"phase:design".to_string()));
    assert!(implement.inputs.contains(&"var:branch".to_string()));
    assert_eq!(implement.output, "patch.diff");

    // Phase 4: review — EscalatingRetry, tensorzero + ollama, contract
    let review = &wf.phases[3];
    assert_eq!(review.name, "review");
    assert_eq!(
        review.strategy,
        Strategy::EscalatingRetry {
            pass_failure_context: true
        }
    );
    assert_eq!(review.backends.len(), 2);
    assert_eq!(review.backends[0], "tensorzero/judge");
    assert_eq!(review.backends[1], "ollama/glm-5.1");
    assert!(review.inputs.contains(&"phase:implement".to_string()));
    assert!(review.inputs.contains(&"spec".to_string()));
    assert_eq!(review.output, "review.md");
    assert!(
        review.contract.is_some(),
        "Phase 'review' should have a contract block"
    );

    // Verify resolved backends
    let resolved = Workflow::resolve_backends(review).unwrap();
    assert_eq!(resolved[0], BackendRef::TensorZero("judge".into()));
    assert_eq!(resolved[1], BackendRef::Ollama("glm-5.1".into()));

    // Verify resolved inputs
    let resolved_inputs = Workflow::resolve_inputs(review).unwrap();
    assert!(resolved_inputs.contains(&InputRef::PhaseRef("implement".into())));
    assert!(resolved_inputs.contains(&InputRef::Spec));
}

// ---------------------------------------------------------------------------
// Test 2: Backend scheme matrix — parse a phase per scheme
// ---------------------------------------------------------------------------

#[test]
fn backend_scheme_matrix() {
    let toml = r#"
name = "scheme-matrix"

[[phases]]
name = "judge"
strategy = { single = {} }
backends = ["tensorzero/judge"]
prompt_template = "Judge"
inputs = ["spec"]
output = "judge.md"

[[phases]]
name = "code"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Code"
inputs = ["spec"]
output = "code.md"

[[phases]]
name = "exec"
strategy = { single = {} }
backends = ["codex/"]
prompt_template = "Exec"
inputs = ["spec"]
output = "exec.md"

[[phases]]
name = "think"
strategy = { single = {} }
backends = ["gemini/"]
prompt_template = "Think"
inputs = ["spec"]
output = "think.md"

[[phases]]
name = "run"
strategy = { single = {} }
backends = ["ollama/glm-5.1"]
prompt_template = "Run"
inputs = ["spec"]
output = "run.md"
"#;
    let wf = toml
        .parse::<Workflow>()
        .expect("Backend scheme matrix should parse");
    assert_eq!(wf.phases.len(), 5);
    assert_eq!(
        Workflow::resolve_backends(&wf.phases[0]).unwrap()[0],
        BackendRef::TensorZero("judge".into())
    );
    assert_eq!(
        Workflow::resolve_backends(&wf.phases[1]).unwrap()[0],
        BackendRef::Claude
    );
    assert_eq!(
        Workflow::resolve_backends(&wf.phases[2]).unwrap()[0],
        BackendRef::Codex
    );
    assert_eq!(
        Workflow::resolve_backends(&wf.phases[3]).unwrap()[0],
        BackendRef::Gemini
    );
    assert_eq!(
        Workflow::resolve_backends(&wf.phases[4]).unwrap()[0],
        BackendRef::Ollama("glm-5.1".into())
    );
}

// ---------------------------------------------------------------------------
// Test 3: Unknown scheme — resolver returns error
// ---------------------------------------------------------------------------

#[test]
fn unknown_scheme() {
    let toml = r#"
name = "unknown-scheme"

[[phases]]
name = "test"
strategy = { single = {} }
backends = ["unknownscheme/foo"]
prompt_template = "Test"
inputs = ["spec"]
output = "test.md"
"#;
    let err = toml
        .parse::<Workflow>()
        .expect_err("Unknown scheme should produce errors");
    assert!(
        err.iter().any(|e| matches!(
            e,
            WorkflowError::UnknownBackendScheme { scheme }
                if scheme == "unknownscheme/foo"
        )),
        "Expected UnknownBackendScheme for 'unknownscheme/foo', got: {:?}",
        err
    );
}

// ---------------------------------------------------------------------------
// Test 4: phase.contract ignored with WARN
// ---------------------------------------------------------------------------

#[test]
fn contract_ignored_with_warn() {
    let toml = r#"
name = "contract-test"

[[phases]]
name = "analysis"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Analyze"
inputs = ["spec"]
output = "analysis.md"
contract = { required_capability = "file_edit", timeout_ms = 30000 }
"#;
    let wf = toml
        .parse::<Workflow>()
        .expect("Workflow with contract should parse successfully");
    let phase = &wf.phases[0];
    assert!(
        phase.contract.is_some(),
        "contract should be stored on the AST"
    );

    // Lint output should contain the reservation warning
    let warnings = wf.lint();
    let has_warning = warnings
        .iter()
        .any(|w| w.contains("phase.contract reserved for post-v0; ignored"));
    assert!(
        has_warning,
        "Expected contract reservation warning in lint output, got: {:?}",
        warnings
    );
}

// ---------------------------------------------------------------------------
// Test 5: Forward input ref
// ---------------------------------------------------------------------------

#[test]
fn forward_input_ref() {
    let toml = r#"
name = "forward"

[[phases]]
name = "synthesis"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Synthesize"
inputs = ["phase:analysis"]
output = "synthesis.md"

[[phases]]
name = "analysis"
strategy = { single = {} }
backends = ["gemini/"]
prompt_template = "Analyze"
inputs = ["spec"]
output = "analysis.md"
"#;
    let err = toml
        .parse::<Workflow>()
        .expect_err("Forward ref should produce errors");
    assert!(
        err.iter().any(|e| matches!(
            e,
            WorkflowError::ForwardInputRef { from, to }
                if from == "synthesis" && to == "analysis"
        )),
        "Expected ForwardInputRef from 'synthesis' to 'analysis', got: {:?}",
        err
    );
}

// ---------------------------------------------------------------------------
// Test 6: Strategy constraint — min_responses exceeds backends
// ---------------------------------------------------------------------------

#[test]
fn strategy_constraint() {
    let toml = r#"
name = "min-resp"

[[phases]]
name = "review"
strategy = { parallel = { min_responses = 5 } }
backends = ["claude/", "gemini/"]
prompt_template = "Review"
inputs = ["spec"]
output = "review.md"
"#;
    let err = toml
        .parse::<Workflow>()
        .expect_err("Strategy constraint violation should produce errors");
    assert!(
        err.iter().any(|e| matches!(
            e,
            WorkflowError::MinResponsesExceedsBackends {
                phase,
                min,
                count,
            } if phase == "review" && *min == 5 && *count == 2
        )),
        "Expected MinResponsesExceedsBackends, got: {:?}",
        err
    );
}

// ---------------------------------------------------------------------------
// Test 7: Multi-error — workflow with two distinct violations returns both
// ---------------------------------------------------------------------------

#[test]
fn multi_error() {
    let toml = r#"
name = "multi-error"

[[phases]]
name = "synthesis"
strategy = { parallel = { min_responses = 10 } }
backends = ["claude/"]
prompt_template = "Synthesize"
inputs = ["phase:analysis"]
output = "synthesis.md"

[[phases]]
name = "analysis"
strategy = { single = {} }
backends = ["gemini/"]
prompt_template = "Analyze"
inputs = ["spec"]
output = "analysis.md"
"#;
    let err = toml
        .parse::<Workflow>()
        .expect_err("Multi-error workflow should produce errors");
    assert!(
        err.len() >= 2,
        "Expected at least 2 errors (forward ref + min_responses), got {}: {:?}",
        err.len(),
        err
    );
}

// ---------------------------------------------------------------------------
// Test 8: Empty workflow
// ---------------------------------------------------------------------------

#[test]
fn empty_workflow() {
    let toml = r#"
name = "empty"
phases = []
"#;
    let err = toml
        .parse::<Workflow>()
        .expect_err("Empty workflow should produce an error");
    assert!(
        err.iter().any(|e| matches!(e, WorkflowError::NoPhases)),
        "Expected NoPhases error, got: {:?}",
        err
    );
}
