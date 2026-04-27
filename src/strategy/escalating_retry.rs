//! `EscalatingRetry`: walk a cheap-to-strong ladder of backends, stop at the
//! first verify pass.
//!
//! Per loker-design.md §4.2 this is the second `Strategy` variant after
//! `SingleModel`. The walker calls each rung in order; on a verify pass it
//! returns immediately with `final_status: Succeeded`, and on full
//! exhaustion it returns `StrategyError::Exhausted` carrying a fully-shaped
//! `StrategyOutput` (with `final_status: Exhausted`) so the caller can
//! still persist the schema-shaped JSON.
//!
//! Backend errors do **not** abort the ladder - they are recorded as a
//! failed attempt and the walker advances to the next rung. This matches
//! AC: "non-retryable backend error does not skip subsequent backends".

use crate::backend::Backend;
use crate::strategy::{
    pick_model, Attempt, FinalStatus, FinishReason, PhaseContext, Prompt, Strategy, StrategyError,
    StrategyKind, StrategyOutput, Tier, TokenUsageReport, VerifyHook, VerifyOutcome,
    SCHEMA_VERSION,
};
use async_trait::async_trait;
use std::sync::Arc;

/// Single rung of the ladder: which tier this slot represents and which
/// backend (matched against `Backend::name()`) should serve it.
#[derive(Debug, Clone)]
pub struct Rung {
    pub tier: Tier,
    pub backend: String,
}

impl Rung {
    pub fn new(tier: Tier, backend: impl Into<String>) -> Self {
        Self {
            tier,
            backend: backend.into(),
        }
    }
}

/// Ladder strategy. `rungs` must be non-empty; capability validation runs
/// at workflow load time (CLO-251), so by the time `execute` runs every
/// referenced backend has already been proven capable.
///
/// `pass_failure_context` is reserved for the planner work in CLO-261:
/// when set, the walker will (eventually) thread the previous rung's
/// failure reason into the next rung's prompt. v0 records the flag but
/// does not yet inject context.
pub struct EscalatingRetry {
    pub rungs: Vec<Rung>,
    pub prompt_template: String,
    pub verify: Arc<dyn VerifyHook>,
    pub pass_failure_context: bool,
}

impl EscalatingRetry {
    pub fn new(
        rungs: Vec<Rung>,
        prompt_template: impl Into<String>,
        verify: Arc<dyn VerifyHook>,
    ) -> Self {
        Self {
            rungs,
            prompt_template: prompt_template.into(),
            verify,
            pass_failure_context: false,
        }
    }

    pub fn with_pass_failure_context(mut self, enabled: bool) -> Self {
        self.pass_failure_context = enabled;
        self
    }
}

#[async_trait]
impl Strategy for EscalatingRetry {
    async fn execute(
        &self,
        backends: &[Arc<dyn Backend>],
        prompt: &Prompt,
        ctx: &PhaseContext,
    ) -> Result<StrategyOutput, StrategyError> {
        if self.rungs.is_empty() || backends.is_empty() {
            return Err(StrategyError::NoBackends);
        }

        let rendered = ctx
            .template_engine
            .render(&self.prompt_template, &ctx.template_context)?;

        let mut attempts: Vec<Attempt> = Vec::with_capacity(self.rungs.len());

        for (idx, rung) in self.rungs.iter().enumerate() {
            let backend = backends
                .iter()
                .find(|b| b.name() == rung.backend)
                .ok_or_else(|| StrategyError::BackendNotFound {
                    name: rung.backend.clone(),
                })?;

            let output_path = format!(
                "{}/attempts/{}-{}.md",
                ctx.phase_name,
                idx + 1,
                rung.tier.as_str()
            );

            match backend
                .query(&rendered, &ctx.cwd, prompt.model.as_deref())
                .await
            {
                Ok(query) => {
                    let usage = query
                        .usage
                        .as_ref()
                        .map(TokenUsageReport::from)
                        .unwrap_or_default();
                    let model = pick_model(&query, prompt);

                    match self.verify.verify(&query).await {
                        Ok(result) => {
                            let passed = result.is_pass();
                            let verify_outcome = if passed {
                                VerifyOutcome::passed(self.verify.name())
                            } else {
                                VerifyOutcome::failed(self.verify.name())
                            };
                            attempts.push(Attempt {
                                tier: Some(rung.tier),
                                backend: backend.name().to_string(),
                                model,
                                finish_reasons: vec![FinishReason::Stop],
                                usage,
                                output_path,
                                verify: verify_outcome,
                            });

                            if passed {
                                return Ok(StrategyOutput {
                                    schema_version: SCHEMA_VERSION,
                                    strategy: StrategyKind::Escalating,
                                    phase: ctx.phase_name.clone(),
                                    run_id: ctx.run_id,
                                    attempts,
                                    final_status: Some(FinalStatus::Succeeded),
                                });
                            }
                        }
                        Err(_err) => {
                            // Hook itself blew up: record as a failed attempt
                            // (status=fail, hook name preserved) and keep
                            // walking the ladder.
                            attempts.push(Attempt {
                                tier: Some(rung.tier),
                                backend: backend.name().to_string(),
                                model,
                                finish_reasons: vec![FinishReason::Stop],
                                usage,
                                output_path,
                                verify: VerifyOutcome::failed(self.verify.name()),
                            });
                        }
                    }
                }
                Err(_err) => {
                    // Backend errored. Record it as an error-finished attempt
                    // and advance to the next rung. Per AC: non-retryable
                    // backend errors must not abort the ladder.
                    let model = prompt
                        .model
                        .as_ref()
                        .filter(|m| !m.is_empty())
                        .cloned()
                        .unwrap_or_else(|| "default".to_string());
                    attempts.push(Attempt {
                        tier: Some(rung.tier),
                        backend: rung.backend.clone(),
                        model,
                        finish_reasons: vec![FinishReason::Error],
                        usage: TokenUsageReport::default(),
                        output_path,
                        verify: VerifyOutcome::skipped(),
                    });
                }
            }
        }

        let exhausted = StrategyOutput {
            schema_version: SCHEMA_VERSION,
            strategy: StrategyKind::Escalating,
            phase: ctx.phase_name.clone(),
            run_id: ctx.run_id,
            attempts,
            final_status: Some(FinalStatus::Exhausted),
        };
        Err(StrategyError::Exhausted {
            output: Box::new(exhausted),
        })
    }
}
