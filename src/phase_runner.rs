//! Phase-level coordinator composing strategies, aggregators, verifiers,
//! manifest persistence, and run-state markers.

use std::path::PathBuf;
use std::sync::Arc;

use crate::backend::Backend;
use crate::manifest::{Kind, ManifestEntry, ManifestError, Producer};
use crate::run_state::markers::MarkerError;
use crate::strategy::verify::{VerifyError, VerifyHook, VerifyResult};
use crate::strategy::{
    PhaseContext, Prompt, StrategyError, StrategyKind, StrategyOutput, TargetSpec, Tier,
};

pub mod dispatch;
pub mod persist;

/// Strategy dispatch label.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyName {
    Single,
    Parallel,
    EscalatingRetry,
}

/// Aggregator dispatch label.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregatorName {
    First,
    Concat,
    Vote,
    AnyFail,
    AllPass,
}

/// Verify hook dispatch label. `None` is the explicit no-op.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyHookName {
    None,
    RunCommand,
    LlmVerifier,
}

/// Phase-level configuration. Built by tests today and by the workflow parser later.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PhaseConfig {
    pub phase: String,
    pub strategy: StrategyName,
    pub aggregator: AggregatorName,
    pub verify: VerifyHookName,
    pub artefact_name: String,
    pub artefact_kind: Kind,
    pub producer: Producer,
    pub prompt_template: String,
    pub backend: Option<String>,
    pub targets: Vec<TargetSpec>,
    pub min_responses: usize,
    pub rungs: Vec<PhaseRung>,
    pub pass_failure_context: bool,
}

impl PhaseConfig {
    pub fn single(
        phase: impl Into<String>,
        backend: impl Into<String>,
        prompt_template: impl Into<String>,
        artefact_name: impl Into<String>,
    ) -> Self {
        Self {
            phase: phase.into(),
            strategy: StrategyName::Single,
            aggregator: AggregatorName::First,
            verify: VerifyHookName::None,
            artefact_name: artefact_name.into(),
            artefact_kind: Kind::DesignMd,
            producer: Producer::Single,
            prompt_template: prompt_template.into(),
            backend: Some(backend.into()),
            targets: Vec::new(),
            min_responses: 1,
            rungs: Vec::new(),
            pass_failure_context: false,
        }
    }
}

/// Config rung avoiding leakage of strategy-internal constructor details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhaseRung {
    pub tier: Tier,
    pub backend: String,
}

impl PhaseRung {
    pub fn new(tier: Tier, backend: impl Into<String>) -> Self {
        Self {
            tier,
            backend: backend.into(),
        }
    }
}

/// Per-call inputs. Backends are resolved by the caller.
pub struct PhaseInputs<'a> {
    pub backends: &'a [Arc<dyn Backend>],
    pub prompt: Prompt,
    pub ctx: PhaseContext,
    pub verify: Option<Arc<dyn VerifyHook>>,
    pub run_dir: PathBuf,
}

/// Result of a successful phase run.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PhaseOutcome {
    pub artefact_path: PathBuf,
    pub manifest_entry: ManifestEntry,
    pub strategy_output: StrategyOutput,
    pub strategy_kind: StrategyKind,
    pub verify: Option<VerifyResult>,
}

/// Typed terminal failure.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum PhaseError {
    #[error("invalid phase config: {0}")]
    InvalidConfig(String),

    #[error("strategy failed: {0}")]
    StrategyFailed(#[from] StrategyError),

    #[error("verify hook failed: {0}")]
    VerifyHook(#[from] VerifyError),

    #[error("verify failed in phase `{phase}` at attempt {attempt}")]
    VerifyFailed {
        phase: String,
        attempt: u32,
        result: VerifyResult,
    },

    #[error("phase `{phase}` failed: {message}")]
    PhaseFailed { phase: String, message: String },

    #[error("aggregator failed: {0}")]
    Aggregator(#[from] crate::aggregator::AggregatorError),

    #[error("manifest write failed: {0}")]
    Manifest(#[from] ManifestError),

    #[error("marker write failed: {0}")]
    Marker(#[from] MarkerError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

impl PhaseError {
    pub fn error_class(&self) -> &'static str {
        match self {
            Self::StrategyFailed(_) => "strategy_failed",
            Self::VerifyHook(_) | Self::VerifyFailed { .. } => "verify_failed",
            Self::Aggregator(_) => "aggregator_failed",
            Self::Manifest(_) => "manifest_failed",
            Self::Marker(_) => "marker_failed",
            Self::Io(_) => "io_failed",
            Self::Json(_) => "phase_failed",
            Self::InvalidConfig(_) | Self::PhaseFailed { .. } => "phase_failed",
        }
    }
}

/// Phase-level coordinator.
#[derive(Debug, Default, Clone, Copy)]
pub struct PhaseRunner;

impl PhaseRunner {
    pub const fn new() -> Self {
        Self
    }

    pub async fn run(
        &self,
        cfg: &PhaseConfig,
        inputs: PhaseInputs<'_>,
    ) -> Result<PhaseOutcome, PhaseError> {
        validate_config(cfg)?;
        let markers = crate::run_state::markers::MarkerWriter::new(&inputs.run_dir);
        persist::start_attempt(&markers, &cfg.phase, 0)?;

        let mut ctx = inputs.ctx;
        ctx.cwd = inputs.run_dir.clone();
        let run_id = ctx.run_id;

        let strategy = dispatch::resolve_strategy(cfg, inputs.verify.clone())?;
        let strategy_output = match strategy
            .execute(inputs.backends, &inputs.prompt, &ctx)
            .await
        {
            Ok(output) => output,
            Err(err) => {
                let phase_err = match err {
                    StrategyError::AnyFail { reason, .. } => PhaseError::Aggregator(
                        crate::aggregator::AggregatorError::BranchFailures(reason.to_string()),
                    ),
                    other => PhaseError::StrategyFailed(other),
                };
                if let Err(persist_err) = persist::record_terminal_failure(
                    &markers,
                    &inputs.run_dir,
                    cfg,
                    1,
                    phase_err.error_class(),
                ) {
                    eprintln!("failed to persist terminal failure marker: {persist_err}");
                }
                return Err(phase_err);
            }
        };

        for attempt in 1..strategy_output.attempts.len() {
            persist::start_attempt(&markers, &cfg.phase, attempt as u32)?;
        }

        let aggregator = dispatch::resolve_aggregator(cfg.aggregator)?;
        let (bytes, selected_attempt) =
            match dispatch::canonical_bytes(&inputs.run_dir, &strategy_output, &aggregator) {
                Ok(bytes) => bytes,
                Err(err) => {
                    if let Err(persist_err) = persist::record_terminal_failure(
                        &markers,
                        &inputs.run_dir,
                        cfg,
                        strategy_output.attempts.len().max(1) as u32,
                        err.error_class(),
                    ) {
                        eprintln!("failed to persist terminal failure marker: {persist_err}");
                    }
                    return Err(err);
                }
            };

        let verify_result = if matches!(cfg.verify, VerifyHookName::None) {
            None
        } else {
            let hook = dispatch::resolve_verify_hook(cfg, inputs.verify.clone())?;
            let hook = hook.ok_or_else(|| {
                PhaseError::InvalidConfig(format!(
                    "verify hook {:?} requires PhaseInputs::verify",
                    cfg.verify
                ))
            })?;
            let selected = strategy_output.attempts.get(selected_attempt as usize);
            let ctx = crate::strategy::VerifyContext {
                stdout: String::from_utf8_lossy(&bytes).into_owned(),
                stderr: None,
                exit_code: None,
                backend_name: selected
                    .map(|a| a.backend.clone())
                    .unwrap_or_else(|| "aggregate".into()),
                model: selected.map(|a| a.model.clone()),
                structured: None,
                duration: std::time::Duration::ZERO,
            };
            let result = hook.verify(&ctx).await?;
            if !result.is_pass() {
                let phase_err = PhaseError::VerifyFailed {
                    phase: cfg.phase.clone(),
                    attempt: selected_attempt,
                    result,
                };
                if let Err(persist_err) = persist::record_terminal_failure(
                    &markers,
                    &inputs.run_dir,
                    cfg,
                    strategy_output.attempts.len().max(1) as u32,
                    phase_err.error_class(),
                ) {
                    eprintln!("failed to persist terminal failure marker: {persist_err}");
                }
                return Err(phase_err);
            }
            Some(result)
        };

        let attempt = selected_attempt;
        let (artefact_path, manifest_entry) =
            persist::commit_success(&inputs.run_dir, cfg, &bytes, attempt, run_id)?;
        markers.write_completed(
            &cfg.phase,
            attempt,
            &manifest_entry.sha256,
            std::slice::from_ref(&cfg.artefact_name),
        )?;

        let strategy_kind = strategy_output.strategy;
        Ok(PhaseOutcome {
            artefact_path,
            manifest_entry,
            strategy_output,
            strategy_kind,
            verify: verify_result,
        })
    }
}

fn validate_config(cfg: &PhaseConfig) -> Result<(), PhaseError> {
    let expected = match cfg.strategy {
        StrategyName::Single => Producer::Single,
        StrategyName::Parallel => Producer::Parallel,
        StrategyName::EscalatingRetry => Producer::Escalating,
    };
    if cfg.producer != expected {
        return Err(PhaseError::InvalidConfig(format!(
            "producer {:?} does not match strategy {:?}; expected {:?}",
            cfg.producer, cfg.strategy, expected
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_runner_api_surface_compiles() {
        let cfg = PhaseConfig::single("design", "mock", "hello", "design.md");
        assert_eq!(cfg.strategy, StrategyName::Single);
        let runner = PhaseRunner::new();
        let _ = runner;
    }
}
