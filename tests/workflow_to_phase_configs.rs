//! Unit tests for `Workflow::to_phase_configs()` (CLO-301, P1).
//!
//! Uses JSON to construct `mod::Workflow` instances, bypassing the need
//! to know exact internal field details.

use loker::manifest::{Kind, Producer};
use loker::phase_runner::{AggregatorName, StrategyName, VerifyHookName};
use loker::strategy::Tier;
use loker::workflow::Workflow;

/// Build a Workflow from a Vec of phase JSON objects.
fn workflow_phases(phases_json: &[serde_json::Value]) -> Workflow {
    let obj = serde_json::json!({
        "name": "test-workflow",
        "steps": phases_json
    });
    serde_json::from_value(obj).expect("valid workflow JSON")
}

/// Build a single-phase workflow with the given phase name and base fields.
#[allow(dead_code)]
fn phase(name: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "prompt": "test prompt",
        "backend": "claude"
    })
}

/// Build a single-phase workflow with name, prompt, backend.
fn llm_phase(name: &str, backend: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "prompt": format!("prompt for {}", name),
        "backend": backend
    })
}

// ---------------------------------------------------------------------------
// Single backend
// ---------------------------------------------------------------------------

#[test]
fn to_phase_configs_single() {
    let wf = workflow_phases(&[llm_phase("design", "claude")]);
    let configs = wf.to_phase_configs();
    assert_eq!(configs.len(), 1);
    let cfg = &configs[0];
    assert_eq!(cfg.phase, "design");
    assert_eq!(cfg.backend.as_deref(), Some("claude"));
    assert_eq!(cfg.strategy, StrategyName::Single);
    assert_eq!(cfg.verify, VerifyHookName::None);
    assert!(cfg.rungs.is_empty());
    assert_eq!(cfg.min_responses, 1);
}

#[test]
fn to_phase_configs_verify_apply_edits() {
    let wf = workflow_phases(&[serde_json::json!({
        "name": "edit",
        "prompt": "edit prompt",
        "backend": "claude",
        "apply_edits": true
    })]);
    let configs = wf.to_phase_configs();
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].verify, VerifyHookName::RunCommand);
}

#[test]
fn to_phase_configs_verify_shell_command() {
    let wf = workflow_phases(&[serde_json::json!({
        "name": "check",
        "prompt": "check prompt",
        "backend": "claude",
        "verify": "make check"
    })]);
    let configs = wf.to_phase_configs();
    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].verify, VerifyHookName::RunCommand);
}

// ---------------------------------------------------------------------------
// Parallel (multi-backend)
// ---------------------------------------------------------------------------

#[test]
fn to_phase_configs_parallel() {
    let wf = workflow_phases(&[serde_json::json!({
        "name": "parallel",
        "prompt": "query prompt",
        "backends": ["claude", "gemini"]
    })]);
    let configs = wf.to_phase_configs();
    assert_eq!(configs.len(), 1);
    let cfg = &configs[0];
    assert_eq!(cfg.strategy, StrategyName::Parallel);
    assert_eq!(cfg.targets.len(), 2);
    assert_eq!(cfg.producer, Producer::Parallel);
}

// ---------------------------------------------------------------------------
// Escalating retry
// ---------------------------------------------------------------------------

#[test]
fn to_phase_configs_escalating() {
    let wf = workflow_phases(&[serde_json::json!({
        "name": "retry",
        "prompt": "retry prompt",
        "backend": "claude",
        "retries": 2
    })]);
    let configs = wf.to_phase_configs();
    assert_eq!(configs.len(), 1);
    let cfg = &configs[0];
    assert_eq!(cfg.strategy, StrategyName::EscalatingRetry);
    assert_eq!(cfg.rungs.len(), 1);
    assert_eq!(cfg.rungs[0].tier, Tier::Medium);
}

// ---------------------------------------------------------------------------
// Consensus strategies
// ---------------------------------------------------------------------------

#[test]
fn to_phase_configs_consensus_first() {
    let wf = workflow_phases(&[serde_json::json!({
        "name": "first",
        "prompt": "first prompt",
        "backend": "claude",
        "consensus": "first"
    })]);
    let configs = wf.to_phase_configs();
    assert_eq!(configs[0].aggregator, AggregatorName::First);
}

#[test]
fn to_phase_configs_consensus_synthesis() {
    // Default (Synthesis) → First aggregator (design doc §4.2)
    let wf = workflow_phases(&[llm_phase("synth", "claude")]);
    let configs = wf.to_phase_configs();
    assert_eq!(configs[0].aggregator, AggregatorName::First);
}

#[test]
fn to_phase_configs_consensus_vote() {
    let wf = workflow_phases(&[serde_json::json!({
        "name": "vote",
        "prompt": "vote prompt",
        "backend": "claude",
        "consensus": "vote"
    })]);
    let configs = wf.to_phase_configs();
    assert_eq!(configs[0].aggregator, AggregatorName::Vote);
}

// ---------------------------------------------------------------------------
// Artefact kind
// ---------------------------------------------------------------------------

#[test]
fn to_phase_configs_artefact_kind_json() {
    let wf = workflow_phases(&[serde_json::json!({
        "name": "json-step",
        "prompt": "json prompt",
        "backend": "claude",
        "output_format": "json"
    })]);
    let configs = wf.to_phase_configs();
    assert_eq!(configs[0].artefact_kind, Kind::VerifyJson);
}

#[test]
fn to_phase_configs_artefact_kind_lines() {
    let wf = workflow_phases(&[serde_json::json!({
        "name": "lines-step",
        "prompt": "lines prompt",
        "backend": "claude",
        "output_format": "lines"
    })]);
    let configs = wf.to_phase_configs();
    assert_eq!(configs[0].artefact_kind, Kind::ResponseJson);
}

#[test]
fn to_phase_configs_artefact_kind_default() {
    // No output_format → default DesignMd
    let wf = workflow_phases(&[llm_phase("default-kind", "claude")]);
    let configs = wf.to_phase_configs();
    assert_eq!(configs[0].artefact_kind, Kind::DesignMd);
}

// ---------------------------------------------------------------------------
// Shell steps excluded
// ---------------------------------------------------------------------------

#[test]
fn to_phase_configs_shell_skipped() {
    let wf = workflow_phases(&[serde_json::json!({
        "name": "shell-step",
        "prompt": "do shell",
        "shell": "echo hi"
    })]);
    let configs = wf.to_phase_configs();
    assert!(configs.is_empty(), "shell steps must be excluded");
}

// ---------------------------------------------------------------------------
// Multiple steps
// ---------------------------------------------------------------------------

#[test]
fn to_phase_configs_multiple_steps() {
    let wf = workflow_phases(&[
        llm_phase("phase1", "claude"),
        llm_phase("phase2", "claude"),
        llm_phase("phase3", "claude"),
    ]);
    let configs = wf.to_phase_configs();
    assert_eq!(configs.len(), 3);
    assert_eq!(configs[0].phase, "phase1");
    assert_eq!(configs[1].phase, "phase2");
    assert_eq!(configs[2].phase, "phase3");
}

// ---------------------------------------------------------------------------
// Mixed: shell + LLM steps
// ---------------------------------------------------------------------------

#[test]
fn to_phase_configs_mixed_shell_and_llm() {
    let wf = workflow_phases(&[
        llm_phase("llm-step", "claude"),
        serde_json::json!({
            "name": "shell-step",
            "prompt": "shell",
            "shell": "make check"
        }),
    ]);
    let configs = wf.to_phase_configs();
    assert_eq!(configs.len(), 1, "only the LLM step should be included");
    assert_eq!(configs[0].phase, "llm-step");
}
