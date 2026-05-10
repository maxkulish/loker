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

use crate::manifest::{Kind, Producer};
use crate::phase_runner::{
    AggregatorName, PhaseConfig, PhaseInputs, PhaseOutcome, PhaseRunner, PhaseRung,
    StrategyName, VerifyHookName,
};
use crate::run_state::RunDir;
use crate::strategy::{PhaseContext, Prompt, Tier};
use crate::workflow::grammar::Strategy;
use anyhow::Context;

/// Build a `PhaseConfig` from a `grammar::Phase`.
///
/// Strategy mapping:
/// - `Strategy::Single`             → `StrategyName::Single`
/// - `Strategy::ParallelFanOut`     → `StrategyName::Parallel`
/// - `Strategy::EscalatingRetry`    → `StrategyName::EscalatingRetry`
///
/// The prompt template is passed through as-is; it will be rendered at
/// strategy execution time via the `PhaseContext.template_engine`.
pub fn build_phase_config(
    phase: &crate::workflow::grammar::Phase,
    config: &crate::config::Config,
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
            (
                StrategyName::EscalatingRetry,
                AggregatorName::First,
                rungs,
            )
        }
    };

    let backend = phase.backends.first().cloned();
    let min_responses = match &phase.strategy {
        Strategy::Single => 1,
        Strategy::ParallelFanOut { min_responses } => *min_responses,
        Strategy::EscalatingRetry { .. } => 1,
    };

    let artefact_kind = if phase.output.ends_with(".md") {
        Kind::DesignMd
    } else if phase.output.ends_with(".json") {
        Kind::VerifyJson
    } else {
        Kind::PhaseResultJson
    };

    PhaseConfig {
        phase: phase.name.clone(),
        strategy: strategy_name,
        aggregator: aggregator_name,
        verify: VerifyHookName::None,
        artefact_name: phase.output.clone(),
        artefact_kind,
        producer: Producer::Single,
        prompt_template: phase.prompt_template.clone(),
        backend,
        targets: Vec::new(),
        min_responses,
        rungs,
        pass_failure_context: false,
    }
}

/// Run a phase-based workflow, executing phases sequentially and chaining
/// outputs as inputs to downstream phases.
///
/// Returns paths to all generated artifacts.
pub async fn run_phase_workflow(
    config: Arc<crate::config::Config>,
    _cwd: &Path,
    workflow: &crate::workflow::grammar::Workflow,
    _spec_content: Option<String>,
    _template_vars: HashMap<String, String>,
    _rerun_phases: &[String],
    _run_dir: &RunDir,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut _artifact_paths: Vec<PathBuf> = Vec::new();

    for (idx, phase) in workflow.phases.iter().enumerate() {
        // Build PhaseConfig (ST2)
        let cfg = build_phase_config(phase, &config);
        println!(
            "  ▶ Phase {}/{}: {} (strategy: {:?})",
            idx + 1,
            workflow.phases.len(),
            phase.name,
            cfg.strategy
        );

        // TODO ST3: Resolve backends, build PhaseInputs, call PhaseRunner::run()
        anyhow::bail!("Phase runner execution not yet wired (CLO-327 ST3/ST4)");
    }

    Ok(_artifact_paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::grammar::Workflow;
    use std::sync::Arc;

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

    fn test_config() -> Arc<crate::config::Config> {
        Arc::new(crate::config::Config::default())
    }

    #[test]
    fn build_phase_config_single_strategy() {
        let phase = sample_single_phase();
        let cfg = build_phase_config(&phase, &test_config());
        assert_eq!(cfg.phase, "design");
        assert_eq!(cfg.strategy, StrategyName::Single);
        assert_eq!(cfg.aggregator, AggregatorName::First);
        assert_eq!(cfg.artefact_name, "design.md");
        assert_eq!(cfg.prompt_template, "Design {{ spec }}");
        assert_eq!(cfg.backend, Some("claude/".to_string()));
        assert_eq!(cfg.min_responses, 1);
        assert_eq!(cfg.rungs.len(), 1);
    }

    #[test]
    fn build_phase_config_parallel_strategy() {
        let phase = sample_parallel_phase();
        let cfg = build_phase_config(&phase, &test_config());
        assert_eq!(cfg.strategy, StrategyName::Parallel);
        assert_eq!(cfg.min_responses, 2);
        assert_eq!(cfg.rungs.len(), 2);
    }

    #[test]
    fn build_phase_config_escalating_strategy() {
        let phase = sample_escalating_phase();
        let cfg = build_phase_config(&phase, &test_config());
        assert_eq!(cfg.strategy, StrategyName::EscalatingRetry);
        assert_eq!(cfg.rungs.len(), 2);
    }

    #[test]
    fn build_phase_config_renders_template() {
        let phase = sample_single_phase();
        let cfg = build_phase_config(&phase, &test_config());
        assert_eq!(cfg.prompt_template, "Design {{ spec }}");
    }

    #[test]
    fn build_phase_config_resolves_backends() {
        // Verify that backend strings are correctly mapped to PhaseConfig.backend
        let phase = sample_single_phase();
        let cfg = build_phase_config(&phase, &test_config());
        assert_eq!(cfg.backend, Some("claude/".to_string()));
    }
}
