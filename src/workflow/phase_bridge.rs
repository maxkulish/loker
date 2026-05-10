//! Bridge between `grammar::Workflow` (phase-based TOML AST) and
//! `PhaseRunner` (execution engine) for CLO-327.
//!
//! Provides:
//! - `build_phase_config` — converts a `grammar::Phase` into a `PhaseConfig`
//! - `run_phase_workflow` — walks phases sequentially, executing each
//!   via `PhaseRunner` and chaining outputs to downstream phases.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::backend::{create_backend, get_retry_policy};
use crate::manifest::{Kind, Producer};
use crate::phase_runner::{
    AggregatorName, PhaseConfig, PhaseInputs, PhaseRung, PhaseRunner, StrategyName, VerifyHookName,
};
use crate::strategy::{PhaseContext, Prompt, Tier};
use crate::workflow::grammar::Strategy;
use crate::workflow::template::{PhaseOutput, Template, TemplateContext};
use anyhow::{Context, Result};
use colored::Colorize;

/// Extract the backend scheme name from a grammar backend string.
///
/// Grammar backends use format `"claude/"` or `"gemini/gemini-2.5-pro"`.
/// This extracts the base scheme: `"claude"` or `"gemini"`.
fn backend_scheme(raw: &str) -> &str {
    raw.trim_end_matches('/').split('/').next().unwrap_or(raw)
}

/// Build a `PhaseConfig` from a `grammar::Phase`.
///
/// Strategy mapping:
/// - `Strategy::Single`             → `StrategyName::Single`
/// - `Strategy::ParallelFanOut`     → `StrategyName::Parallel`
/// - `Strategy::EscalatingRetry`    → `StrategyName::EscalatingRetry`
///
/// The prompt template is pre-rendered so the strategy's MiniJinja engine
/// receives plain text (no phase-syntax `{{ }}` placeholders).
pub fn build_phase_config(
    phase: &crate::workflow::grammar::Phase,
    phase_outputs: &HashMap<String, (String, PathBuf)>, // name → (content, path)
    spec_content: Option<&str>,
    vars: &HashMap<String, String>,
) -> PhaseConfig {
    let (strategy_name, aggregator_name, rungs) = match &phase.strategy {
        Strategy::Single => {
            let backend = phase.backends.first().cloned().unwrap_or_default();
            (
                StrategyName::Single,
                AggregatorName::First,
                vec![PhaseRung::new(Tier::Medium, &backend)],
            )
        }
        Strategy::ParallelFanOut { .. } => {
            let rungs: Vec<PhaseRung> = phase
                .backends
                .iter()
                .map(|b| PhaseRung::new(Tier::Medium, b))
                .collect();
            (StrategyName::Parallel, AggregatorName::First, rungs)
        }
        Strategy::EscalatingRetry { .. } => {
            let rungs: Vec<PhaseRung> = phase
                .backends
                .iter()
                .map(|b| PhaseRung::new(Tier::Medium, b))
                .collect();
            (StrategyName::EscalatingRetry, AggregatorName::First, rungs)
        }
    };

    let backend = phase.backends.first().cloned();
    let min_responses = match &phase.strategy {
        Strategy::Single => 1,
        Strategy::ParallelFanOut { min_responses } => *min_responses,
        Strategy::EscalatingRetry { .. } => 1,
    };

    let producer = match &phase.strategy {
        Strategy::Single => Producer::Single,
        Strategy::ParallelFanOut { .. } => Producer::Parallel,
        Strategy::EscalatingRetry { .. } => Producer::Escalating,
    };

    let artefact_kind = if phase.output.ends_with(".md") {
        Kind::DesignMd
    } else if phase.output.ends_with(".json") {
        Kind::VerifyJson
    } else {
        Kind::PhaseResultJson
    };

    // Pre-render the prompt template to resolve {{ spec }}, {{ phase.NAME.output }},
    // {{ var.X }} using the workflow-level template engine. The rendered text
    // contains no phase-syntax placeholders, so the strategy's MiniJinja engine
    // will pass it through unchanged.
    let mut tmpl_ctx = TemplateContext::new();
    if let Some(spec) = spec_content {
        tmpl_ctx = tmpl_ctx.with_spec(spec.to_string());
    }
    for (name, (content, path)) in phase_outputs {
        tmpl_ctx = tmpl_ctx.with_phase_output(
            name,
            PhaseOutput {
                content: content.clone(),
                path: path.to_string_lossy().to_string(),
            },
        );
    }
    for (k, v) in vars {
        tmpl_ctx = tmpl_ctx.with_var(k, v.clone());
    }
    let rendered_prompt = Template::render(&phase.prompt_template, &tmpl_ctx)
        .unwrap_or_else(|_| phase.prompt_template.clone());

    PhaseConfig {
        phase: phase.name.clone(),
        strategy: strategy_name,
        aggregator: aggregator_name,
        verify: VerifyHookName::None,
        artefact_name: phase.output.clone(),
        artefact_kind,
        producer,
        prompt_template: rendered_prompt,
        backend,
        targets: Vec::new(),
        min_responses,
        rungs,
        pass_failure_context: false,
    }
}

/// Resolve backend strings from a grammar phase to concrete `Arc<dyn Backend>`.
pub fn resolve_phase_backends(
    phase: &crate::workflow::grammar::Phase,
    config: &crate::config::Config,
) -> Result<Vec<Arc<dyn crate::backend::Backend>>> {
    phase
        .backends
        .iter()
        .map(|raw| {
            let scheme = backend_scheme(raw);
            let backend_cfg = config.backends.get(scheme).ok_or_else(|| {
                anyhow::anyhow!(
                    "Backend '{}' not configured (available: {})",
                    scheme,
                    config
                        .backends
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
            let retry_policy = get_retry_policy(backend_cfg, &config.defaults);
            create_backend(scheme, backend_cfg, retry_policy)
                .with_context(|| format!("Failed to create backend '{}'", scheme))
        })
        .collect()
}

/// Read the output artefact produced by a phase run.
fn read_phase_output(outcome: &crate::phase_runner::PhaseOutcome) -> Result<String> {
    std::fs::read_to_string(&outcome.artefact_path).with_context(|| {
        format!(
            "Failed to read phase output: {}",
            outcome.artefact_path.display()
        )
    })
}

/// Run a phase-based workflow, executing phases sequentially and chaining
/// outputs as inputs to downstream phases.
///
/// Returns paths to all generated artifacts.
pub async fn run_phase_workflow(
    config: Arc<crate::config::Config>,
    cwd: &Path,
    workflow: &crate::workflow::grammar::Workflow,
    spec_content: Option<String>,
    template_vars: HashMap<String, String>,
    _rerun_phases: &[String],
    run_dir_path: &Path,
) -> Result<Vec<PathBuf>> {
    let mut phase_outputs: HashMap<String, (String, PathBuf)> = HashMap::new();
    let mut artifact_paths: Vec<PathBuf> = Vec::new();
    let run_id = uuid::Uuid::new_v4();
    let runner = PhaseRunner::new();

    for (idx, phase) in workflow.phases.iter().enumerate() {
        println!(
            "  {} Phase {}/{}: {}",
            "▶".cyan(),
            idx + 1,
            workflow.phases.len(),
            phase.name
        );

        // Build PhaseConfig with pre-rendered prompt
        let cfg = build_phase_config(
            phase,
            &phase_outputs,
            spec_content.as_deref(),
            &template_vars,
        );

        // Resolve backends
        let backends = resolve_phase_backends(phase, &config)
            .with_context(|| format!("Failed to resolve backends for phase '{}'", phase.name))?;

        // Determine the phase's run directory
        let phase_dir = run_dir_path.join("attempts").join(&phase.name);

        // Build PhaseContext with a template context that includes spec and vars
        // (phase outputs are already baked into the pre-rendered prompt).
        let tmpl_ctx = crate::template::TemplateContext::new_with_extras(
            &HashMap::new(), // steps — empty, phase template is pre-rendered
            &[],             // args
            &[],             // backends
            spec_content.clone(),
            &template_vars,
        );
        let mut ctx = PhaseContext::new(&phase.name, run_id);
        ctx.template_context = tmpl_ctx;
        ctx.cwd = cwd.to_path_buf();

        let inputs = PhaseInputs {
            backends: &backends,
            prompt: Prompt::new(),
            ctx,
            verify: None,
            run_dir: phase_dir.clone(),
            trace: None,
        };

        // Execute the phase
        let outcome = match runner.run(&cfg, inputs, 0).await {
            Ok(outcome) => outcome,
            Err(err) => {
                anyhow::bail!(
                    "Phase '{}' failed: {}\n  hint: fix the issue and re-run with --rerun phase={}",
                    phase.name,
                    err,
                    phase.name
                );
            }
        };

        println!(
            "  {} Phase {} completed: {}",
            "✓".green(),
            phase.name,
            outcome.artefact_path.display()
        );

        // Read and store output for downstream phases
        let output_content = read_phase_output(&outcome)?;
        phase_outputs.insert(
            phase.name.clone(),
            (output_content, outcome.artefact_path.clone()),
        );
        artifact_paths.push(outcome.artefact_path);
    }

    println!(
        "{} Phase workflow completed — {} artifacts generated",
        "✓".green(),
        artifact_paths.len()
    );
    Ok(artifact_paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::grammar::Workflow;

    fn sample_single_phase() -> crate::workflow::grammar::Phase {
        let toml = r#"
name = "test"
[[phases]]
name = "design"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Design {{ spec }}"
inputs = ["spec"]
output = "design.md"
"#;
        let wf: Workflow = toml.parse().unwrap();
        wf.phases.into_iter().next().unwrap()
    }

    fn sample_parallel_phase() -> crate::workflow::grammar::Phase {
        let toml = r#"
name = "parallel-test"
[[phases]]
name = "review"
strategy = { parallel = { min_responses = 2 } }
backends = ["claude/", "gemini/"]
prompt_template = "Review"
inputs = ["spec"]
output = "review.md"
"#;
        let wf: Workflow = toml.parse().unwrap();
        wf.phases.into_iter().next().unwrap()
    }

    fn sample_escalating_phase() -> crate::workflow::grammar::Phase {
        let toml = r#"
name = "escalating-test"
[[phases]]
name = "code"
strategy = { escalating = { pass_failure_context = false } }
backends = ["claude/", "gemini/"]
prompt_template = "Code"
inputs = ["spec"]
output = "code.md"
"#;
        let wf: Workflow = toml.parse().unwrap();
        wf.phases.into_iter().next().unwrap()
    }

    fn empty_phase_outputs() -> HashMap<String, (String, PathBuf)> {
        HashMap::new()
    }

    fn empty_vars() -> HashMap<String, String> {
        HashMap::new()
    }

    #[test]
    fn build_phase_config_single_strategy() {
        let phase = sample_single_phase();
        let cfg = build_phase_config(&phase, &empty_phase_outputs(), None, &empty_vars());
        assert_eq!(cfg.phase, "design");
        assert_eq!(cfg.strategy, StrategyName::Single);
        assert_eq!(cfg.aggregator, AggregatorName::First);
        assert_eq!(cfg.artefact_name, "design.md");
        assert_eq!(cfg.backend, Some("claude/".to_string()));
        assert_eq!(cfg.min_responses, 1);
        assert_eq!(cfg.rungs.len(), 1);
    }

    #[test]
    fn build_phase_config_parallel_strategy() {
        let phase = sample_parallel_phase();
        let cfg = build_phase_config(&phase, &empty_phase_outputs(), None, &empty_vars());
        assert_eq!(cfg.strategy, StrategyName::Parallel);
        assert_eq!(cfg.min_responses, 2);
        assert_eq!(cfg.rungs.len(), 2);
    }

    #[test]
    fn build_phase_config_escalating_strategy() {
        let phase = sample_escalating_phase();
        let cfg = build_phase_config(&phase, &empty_phase_outputs(), None, &empty_vars());
        assert_eq!(cfg.strategy, StrategyName::EscalatingRetry);
        assert_eq!(cfg.rungs.len(), 2);
    }

    #[test]
    fn build_phase_config_resolves_backends() {
        let phase = sample_single_phase();
        let cfg = build_phase_config(&phase, &empty_phase_outputs(), None, &empty_vars());
        assert_eq!(cfg.backend, Some("claude/".to_string()));
    }

    #[test]
    fn build_phase_config_pre_renders_template_with_spec() {
        let phase = sample_single_phase();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "test-workflow".to_string());
        let cfg = build_phase_config(
            &phase,
            &empty_phase_outputs(),
            Some("Build the system"),
            &vars,
        );
        // The template "Design {{ spec }}" should become "Design Build the system"
        assert!(
            cfg.prompt_template.contains("Build the system"),
            "Expected 'Build the system' in prompt, got: {}",
            cfg.prompt_template
        );
        assert!(
            !cfg.prompt_template.contains("{{ spec }}"),
            "{{ spec }} should have been resolved"
        );
    }

    #[test]
    fn build_phase_config_pre_renders_phase_output_chaining() {
        let mut phase_outputs = HashMap::new();
        phase_outputs.insert(
            "design".to_string(),
            (
                "Design output content".to_string(),
                PathBuf::from("design.md"),
            ),
        );
        // Parse a multi-phase workflow with both the source and dependent phases
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
        let wf: Workflow = review_toml.parse().unwrap();
        let review_phase = wf.phases.into_iter().nth(1).unwrap();

        let cfg = build_phase_config(&review_phase, &phase_outputs, None, &empty_vars());
        assert!(
            cfg.prompt_template.contains("Design output content"),
            "Expected phase output chaining, got: {}",
            cfg.prompt_template
        );
        assert!(
            !cfg.prompt_template.contains("{{ phase.design.output }}"),
            "{{ phase.design.output }} should have been resolved"
        );
    }
}
