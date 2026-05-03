//! `SingleModel`: the simplest `Strategy` variant.
//!
//! One backend, one prompt, one response - no retry, no aggregation.
//! Establishes the call shape that `EscalatingRetry` (CLO-258) and
//! `ParallelFanOut` (CLO-259) will build on without altering this type
//! or its call sites.

use crate::backend::Backend;
use crate::strategy::{
    pick_model, Attempt, FinishReason, PhaseContext, Prompt, Strategy, StrategyError, StrategyKind,
    StrategyOutput, TokenUsageReport, VerifyOutcome, SCHEMA_VERSION,
};
use async_trait::async_trait;
use std::path::Path;
use std::sync::Arc;

/// Single-call strategy keyed by `backend` name (matched against
/// `Backend::name()`) and `prompt_template` (a mini-jinja source rendered
/// against the phase's `TemplateContext` before dispatch).
///
/// Construction is intentionally trivial - workflow loaders (CLO-261)
/// will populate the fields directly from TOML; v0 has no validation
/// beyond the runtime backend lookup in `execute`.
#[derive(Debug, Clone)]
pub struct SingleModel {
    pub backend: String,
    pub prompt_template: String,
}

impl SingleModel {
    pub fn new(backend: impl Into<String>, prompt_template: impl Into<String>) -> Self {
        Self {
            backend: backend.into(),
            prompt_template: prompt_template.into(),
        }
    }
}

#[async_trait]
impl Strategy for SingleModel {
    async fn execute(
        &self,
        backends: &[Arc<dyn Backend>],
        prompt: &Prompt,
        ctx: &PhaseContext,
    ) -> Result<StrategyOutput, StrategyError> {
        if backends.is_empty() {
            return Err(StrategyError::NoBackends);
        }

        let chosen = backends
            .iter()
            .find(|b| b.name() == self.backend)
            .ok_or_else(|| StrategyError::BackendNotFound {
                name: self.backend.clone(),
            })?;

        let rendered = ctx
            .template_engine
            .render(&self.prompt_template, &ctx.template_context)?;

        let query = chosen
            .query(&rendered, &ctx.cwd, prompt.model.as_deref())
            .await?;

        let usage = query
            .usage
            .as_ref()
            .map(TokenUsageReport::from)
            .unwrap_or_default();

        let output_path = "attempts/0.txt".to_string();
        write_strategy_output(&ctx.cwd, &output_path, &query.stdout).await?;

        let attempt = Attempt {
            tier: None,
            family: None,
            backend: chosen.name().to_string(),
            model: pick_model(&query, prompt),
            finish_reasons: vec![FinishReason::Stop],
            usage,
            output_path,
            verify: VerifyOutcome::skipped(),
        };

        Ok(StrategyOutput {
            schema_version: SCHEMA_VERSION,
            strategy: StrategyKind::Single,
            phase: ctx.phase_name.clone(),
            run_id: ctx.run_id,
            attempts: vec![attempt],
            final_status: None,
            aggregator: None,
            aggregate_output_path: None,
            verify: None,
        })
    }
}

async fn write_strategy_output(
    cwd: &Path,
    output_path: &str,
    text: &str,
) -> Result<(), StrategyError> {
    let path = cwd.join(output_path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|err| {
            StrategyError::Backend(crate::backend::BackendError::ExecutionFailed {
                message: format!("failed to create output parent {}: {err}", parent.display()),
                exit_code: None,
            })
        })?;
    }
    tokio::fs::write(&path, text).await.map_err(|err| {
        StrategyError::Backend(crate::backend::BackendError::ExecutionFailed {
            message: format!("failed to write strategy output {}: {err}", path.display()),
            exit_code: None,
        })
    })
}
