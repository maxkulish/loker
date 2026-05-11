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
/// # Template rendering (dual-engine architecture)
///
/// This function pre-renders the prompt template using `workflow::template::Template`
/// (regex-based, handles `{{ spec }}`, `{{ phase.NAME.output }}`, `{{ var.X }}`)
/// BEFORE the strategy executes. The pre-rendered text is stored in
/// `PhaseConfig.prompt_template`. When the strategy's MiniJinja engine later renders
/// it, no `{{ }}` remain, so the render is effectively a no-op.
///
/// This dual-pass architecture is intentional: the two engines handle
/// disjoint syntax. The pre-render resolves workflow-level placeholders
/// (`{{ spec }}`, `{{ phase.NAME.output }}`, `{{ var.X }}`). The strategy
/// engine receives plain text with no remaining `{{ ... }}`.
///
/// **Important:** Only `spec`, `phase.*`, and `var.*` placeholders are
/// allowed in phase-based prompt templates. MiniJinja constructs like
/// `{{ steps.NAME.output }}` or control flow cannot survive the strict
/// workflow Template pass and will produce a render error. Phase prompts
/// should use the workflow-level template syntax exclusively.
///
/// # Errors
///
/// Returns an error if the prompt template fails to render (e.g. undefined variable
/// reference), enforcing strict-mode failure per the design.
pub fn build_phase_config(
    phase: &crate::workflow::grammar::Phase,
    phase_outputs: &HashMap<String, (String, PathBuf)>, // name -> (content, path)
    spec_content: Option<&str>,
    vars: &HashMap<String, String>,
) -> Result<PhaseConfig> {
    let (strategy_name, aggregator_name, rungs, targets) = match &phase.strategy {
        Strategy::Single => {
            let backend_s = phase.backends.first().cloned().unwrap_or_default();
            let normalized = backend_scheme(&backend_s).to_string();
            (
                StrategyName::Single,
                AggregatorName::First,
                vec![PhaseRung::new(Tier::Medium, &normalized)],
                Vec::new(),
            )
        }
        Strategy::ParallelFanOut { .. } => {
            let rungs: Vec<PhaseRung> = phase
                .backends
                .iter()
                .map(|b| PhaseRung::new(Tier::Medium, backend_scheme(b)))
                .collect();
            let targets: Vec<crate::strategy::TargetSpec> = phase
                .backends
                .iter()
                .map(|b| crate::strategy::TargetSpec::new(backend_scheme(b)))
                .collect();
            (
                StrategyName::Parallel,
                AggregatorName::First,
                rungs,
                targets,
            )
        }
        Strategy::EscalatingRetry { .. } => {
            let rungs: Vec<PhaseRung> = phase
                .backends
                .iter()
                .map(|b| PhaseRung::new(Tier::Medium, backend_scheme(b)))
                .collect();
            (
                StrategyName::EscalatingRetry,
                AggregatorName::First,
                rungs,
                Vec::new(),
            )
        }
    };

    let backend = phase
        .backends
        .first()
        .map(|b| backend_scheme(b).to_string());
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

    let output_lower = phase.output.to_lowercase();
    let artefact_kind = if output_lower.ends_with(".md") {
        Kind::DesignMd
    } else if output_lower.ends_with(".json") {
        Kind::VerifyJson
    } else {
        Kind::PhaseResultJson
    };

    let pass_failure_context = match &phase.strategy {
        Strategy::EscalatingRetry {
            pass_failure_context,
        } => *pass_failure_context,
        _ => false,
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
        .with_context(|| format!("Template render failed for phase '{}'", phase.name))?;

    Ok(PhaseConfig {
        phase: phase.name.clone(),
        strategy: strategy_name,
        aggregator: aggregator_name,
        verify: VerifyHookName::None,
        artefact_name: phase.output.clone(),
        artefact_kind,
        producer,
        prompt_template: rendered_prompt,
        backend,
        targets,
        min_responses,
        rungs,
        pass_failure_context,
    })
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

/// Read the output artefact produced by a phase run (async).
async fn read_phase_output(outcome: &crate::phase_runner::PhaseOutcome) -> Result<String> {
    tokio::fs::read_to_string(&outcome.artefact_path)
        .await
        .with_context(|| {
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
#[allow(clippy::too_many_arguments)]
pub async fn run_phase_workflow(
    config: Arc<crate::config::Config>,
    cwd: &Path,
    workflow: &crate::workflow::grammar::Workflow,
    spec_content: Option<String>,
    template_vars: HashMap<String, String>,
    _rerun_phases: &[String],
    run_dir_path: &Path,
    run_id: uuid::Uuid,
) -> Result<Vec<PathBuf>> {
    if !_rerun_phases.is_empty() {
        println!(
            "{} --rerun phase={} is a no-op for phase-based workflows (CLO-327 follow-up)",
            "↻".yellow(),
            _rerun_phases.join(", "),
        );
    }

    let mut phase_outputs: HashMap<String, (String, PathBuf)> = HashMap::new();
    let mut artifact_paths: Vec<PathBuf> = Vec::new();
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
        )
        .with_context(|| format!("Failed to build config for phase '{}'", phase.name))?;

        // Resolve backends
        let backends = resolve_phase_backends(phase, &config)
            .with_context(|| format!("Failed to resolve backends for phase '{}'", phase.name))?;

        // Phase-level run directory — PhaseRunner will create
        // attempts/<phase>/<n>/ under this root.
        let phase_dir = run_dir_path.to_path_buf();

        // Build PhaseContext with a template context that includes spec, vars,
        // and backend names (phase outputs are already baked into the pre-rendered prompt).
        let backend_names: Vec<String> = backends.iter().map(|b| b.name().to_string()).collect();
        let tmpl_ctx = crate::template::TemplateContext::new_with_extras(
            &HashMap::new(), // steps — empty, phase template is pre-rendered
            &[],             // args
            &backend_names,  // backends
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
        let output_content = read_phase_output(&outcome).await?;
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
prompt_template = "Design the system"
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
        let cfg = build_phase_config(&phase, &empty_phase_outputs(), None, &empty_vars()).unwrap();
        assert_eq!(cfg.phase, "design");
        assert_eq!(cfg.strategy, StrategyName::Single);
        assert_eq!(cfg.aggregator, AggregatorName::First);
        assert_eq!(cfg.artefact_name, "design.md");
        // Backend should be normalized: "claude/" -> "claude"
        assert_eq!(cfg.backend, Some("claude".to_string()));
        assert_eq!(cfg.min_responses, 1);
        assert_eq!(cfg.rungs.len(), 1);
        // Rung backend should also be normalized
        assert_eq!(cfg.rungs[0].backend, "claude");
        assert!(!cfg.pass_failure_context);
        assert!(cfg.targets.is_empty());
    }

    #[test]
    fn build_phase_config_parallel_strategy() {
        let phase = sample_parallel_phase();
        let cfg = build_phase_config(&phase, &empty_phase_outputs(), None, &empty_vars()).unwrap();
        assert_eq!(cfg.strategy, StrategyName::Parallel);
        assert_eq!(cfg.min_responses, 2);
        assert_eq!(cfg.rungs.len(), 2);
        // Rungs should be normalized
        assert_eq!(cfg.rungs[0].backend, "claude");
        assert_eq!(cfg.rungs[1].backend, "gemini");
        // Targets should be populated
        assert_eq!(cfg.targets.len(), 2);
        assert_eq!(cfg.targets[0].backend, "claude");
        assert_eq!(cfg.targets[1].backend, "gemini");
    }

    #[test]
    fn build_phase_config_escalating_strategy() {
        let phase = sample_escalating_phase();
        let cfg = build_phase_config(&phase, &empty_phase_outputs(), None, &empty_vars()).unwrap();
        assert_eq!(cfg.strategy, StrategyName::EscalatingRetry);
        assert_eq!(cfg.rungs.len(), 2);
        assert!(!cfg.pass_failure_context);
    }

    #[test]
    fn build_phase_config_resolves_backends() {
        let phase = sample_single_phase();
        let cfg = build_phase_config(&phase, &empty_phase_outputs(), None, &empty_vars()).unwrap();
        // Backend should be normalized
        assert_eq!(cfg.backend, Some("claude".to_string()));
    }

    #[test]
    fn build_phase_config_pre_renders_template_with_spec() {
        let spec_phase_toml = r#"
name = "test"
[[phases]]
name = "design"
strategy = { single = {} }
backends = ["claude/"]
prompt_template = "Design {{ spec }}"
inputs = ["spec"]
output = "design.md"
"#;
        let wf: Workflow = spec_phase_toml.parse().unwrap();
        let phase = wf.phases.into_iter().next().unwrap();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "test-workflow".to_string());
        let cfg = build_phase_config(
            &phase,
            &empty_phase_outputs(),
            Some("Build the system"),
            &vars,
        )
        .unwrap();
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

        let cfg = build_phase_config(&review_phase, &phase_outputs, None, &empty_vars()).unwrap();
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
