//! `ParallelFanOut`: dispatch a single prompt to N backends concurrently.
//!
//! Per loker-design.md §4.2 this is the third `Strategy` variant after
//! `SingleModel` (CLO-257) and `EscalatingRetry` (CLO-258).  The runner
//! renders the prompt once, spawns one `Backend::query` future per target
//! via `FuturesUnordered`, and collects per-target outcomes in completion
//! order.  Once `min_responses` successful responses have arrived the
//! strategy short-circuits; remaining in-flight requests are dropped.
//!
//! If fewer than `min_responses` targets succeed before the whole set
//! settles, a structured `StrategyError::FloorViolation` is returned so
//! callers can still persist the schema-shaped JSON.

use crate::backend::{Backend, QueryOutput};
use crate::strategy::{
    Attempt, FinishReason, PhaseContext, Prompt, Strategy, StrategyError, StrategyKind,
    StrategyOutput, TokenUsageReport, VerifyOutcome, SCHEMA_VERSION,
};
use async_trait::async_trait;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use serde::Serialize;
use std::sync::Arc;

/// Target specification for one branch of the fan-out.
#[derive(Debug, Clone)]
pub struct TargetSpec {
    pub backend: String,
    pub model: Option<String>,
}

impl TargetSpec {
    pub fn new(backend: impl Into<String>) -> Self {
        Self {
            backend: backend.into(),
            model: None,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
}

/// Aggregator label for schema compliance.  Actual logic deferred to M3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregator {
    Concat,
    AnyFail,
    Vote,
    LLMJudge,
}

impl Aggregator {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Concat => "concat",
            Self::AnyFail => "any_fail",
            Self::Vote => "vote",
            Self::LLMJudge => "llm_judge",
        }
    }
}

/// Parallel fan-out strategy.
#[derive(Debug, Clone)]
pub struct ParallelFanOut {
    pub targets: Vec<TargetSpec>,
    pub min_responses: usize,
    pub prompt_template: String,
    pub aggregator: Aggregator,
}

impl ParallelFanOut {
    pub fn new(
        targets: Vec<TargetSpec>,
        min_responses: usize,
        prompt_template: impl Into<String>,
        aggregator: Aggregator,
    ) -> Self {
        Self {
            targets,
            min_responses,
            prompt_template: prompt_template.into(),
            aggregator,
        }
    }
}

#[async_trait]
impl Strategy for ParallelFanOut {
    async fn execute(
        &self,
        backends: &[Arc<dyn Backend>],
        prompt: &Prompt,
        ctx: &PhaseContext,
    ) -> Result<StrategyOutput, StrategyError> {
        if self.targets.is_empty() || backends.is_empty() {
            return Err(StrategyError::NoBackends);
        }

        if self.min_responses == 0 {
            return Err(StrategyError::NoBackends);
        }

        let rendered = ctx
            .template_engine
            .render(&self.prompt_template, &ctx.template_context)?;

        // Build FuturesUnordered: each future resolves to (target_index, result).
        let mut futures = FuturesUnordered::new();
        for (idx, target) in self.targets.iter().enumerate() {
            let backend = backends
                .iter()
                .find(|b| b.name() == target.backend)
                .ok_or_else(|| StrategyError::BackendNotFound {
                    name: target.backend.clone(),
                })?;

            let rendered = rendered.clone();
            let cwd = ctx.cwd.clone();
            let model_override = target
                .model
                .as_deref()
                .filter(|m| !m.is_empty())
                .or(prompt.model.as_deref().filter(|m| !m.is_empty()));

            let fut = async move {
                let result = backend.query(&rendered, &cwd, model_override).await;
                (idx, result)
            };
            futures.push(fut);
        }

        let mut attempts: Vec<Attempt> = Vec::with_capacity(self.targets.len());
        let mut successes = 0;

        while let Some((idx, result)) = futures.next().await {
            let target = &self.targets[idx];

            match result {
                Ok(query) => {
                    successes += 1;
                    let usage = query
                        .usage
                        .as_ref()
                        .map(TokenUsageReport::from)
                        .unwrap_or_default();
                    let model = pick_model_override(&query, prompt, target);
                    let output_path = format!("{}/attempts/{}-parallel.txt", ctx.phase_name, idx);

                    attempts.push(Attempt {
                        tier: None,
                        family: Some("local".to_string()),
                        backend: target.backend.clone(),
                        model,
                        finish_reasons: vec![FinishReason::Stop],
                        usage,
                        output_path,
                        verify: VerifyOutcome::skipped(),
                    });

                    if successes >= self.min_responses {
                        // Short-circuit: drop remaining futures.
                        break;
                    }
                }
                Err(_err) => {
                    // Record the error as a failed attempt but keep polling
                    // remaining futures so long as the floor has not been met.
                    let model = target
                        .model
                        .as_ref()
                        .filter(|m| !m.is_empty())
                        .cloned()
                        .or_else(|| prompt.model.clone().filter(|m| !m.is_empty()))
                        .unwrap_or_else(|| "default".to_string());
                    let output_path = format!("{}/attempts/{}-parallel.txt", ctx.phase_name, idx);

                    attempts.push(Attempt {
                        tier: None,
                        family: Some("local".to_string()),
                        backend: target.backend.clone(),
                        model,
                        finish_reasons: vec![FinishReason::Error],
                        usage: TokenUsageReport::default(),
                        output_path,
                        verify: VerifyOutcome::skipped(),
                    });
                }
            }
        }

        // Dropped futures are cancelled (cooperatively) when `futures` falls
        // out of scope here.

        if successes < self.min_responses {
            let output = StrategyOutput {
                schema_version: SCHEMA_VERSION,
                strategy: StrategyKind::Parallel,
                phase: ctx.phase_name.clone(),
                run_id: ctx.run_id,
                attempts,
                final_status: None,
                aggregator: Some(self.aggregator.as_str().to_string()),
                aggregate_output_path: Some(format!("{}/aggregated.txt", ctx.phase_name)),
                verify: Some(VerifyOutcome::skipped()),
            };
            return Err(StrategyError::FloorViolation {
                successes,
                min_responses: self.min_responses,
                output: Box::new(output),
            });
        }

        Ok(StrategyOutput {
            schema_version: SCHEMA_VERSION,
            strategy: StrategyKind::Parallel,
            phase: ctx.phase_name.clone(),
            run_id: ctx.run_id,
            attempts,
            final_status: None,
            aggregator: Some(self.aggregator.as_str().to_string()),
            aggregate_output_path: Some(format!("{}/aggregated.txt", ctx.phase_name)),
            verify: Some(VerifyOutcome::skipped()),
        })
    }
}

/// Build the `model` field that lands in an attempt, applying the priority:
/// backend-reported > target.model > prompt.model > "default".
fn pick_model_override(query: &QueryOutput, prompt: &Prompt, target: &TargetSpec) -> String {
    query
        .model
        .as_deref()
        .filter(|m| !m.is_empty())
        .or_else(|| target.model.as_deref().filter(|m| !m.is_empty()))
        .or_else(|| prompt.model.as_deref().filter(|m| !m.is_empty()))
        .map(str::to_string)
        .unwrap_or_else(|| "default".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::BackendError;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct MockBackend {
        name: String,
        calls: AtomicUsize,
        response: Box<dyn Fn(usize) -> Result<QueryOutput, BackendError> + Send + Sync>,
    }

    impl MockBackend {
        fn ok(name: &str, text: &str) -> Arc<Self> {
            let backend_name = name.to_string();
            let text_owned = text.to_string();
            Arc::new(Self {
                name: name.to_string(),
                calls: AtomicUsize::new(0),
                response: Box::new(move |_| {
                    Ok(QueryOutput::from_text(
                        text_owned.clone(),
                        backend_name.clone(),
                        Duration::from_millis(1),
                    )
                    .with_model(Some("mock-1")))
                }),
            })
        }

        fn fail(name: &str, err: impl Fn() -> BackendError + Send + Sync + 'static) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                calls: AtomicUsize::new(0),
                response: Box::new(move |_| Err(err())),
            })
        }

        fn slow(name: &str, text: &str, delay_ms: u64) -> Arc<Self> {
            let backend_name = name.to_string();
            let text_owned = text.to_string();
            Arc::new(Self {
                name: name.to_string(),
                calls: AtomicUsize::new(0),
                response: Box::new(move |_| {
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    Ok(QueryOutput::from_text(
                        text_owned.clone(),
                        backend_name.clone(),
                        Duration::from_millis(delay_ms),
                    )
                    .with_model(Some("mock-1")))
                }),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Backend for MockBackend {
        fn name(&self) -> &str {
            &self.name
        }

        async fn query(
            &self,
            _prompt: &str,
            _cwd: &Path,
            _model: Option<&str>,
        ) -> Result<QueryOutput, BackendError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            (self.response)(n)
        }

        fn is_available(&self) -> bool {
            true
        }
    }

    fn ctx() -> PhaseContext {
        PhaseContext::new("phase-1", uuid::Uuid::new_v4())
    }

    fn run<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(f)
    }

    #[test]
    fn happy_path_all_succeed() {
        let a = MockBackend::ok("a", "out-a");
        let b = MockBackend::ok("b", "out-b");
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            2,
            "render-me",
            Aggregator::Concat,
        );

        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        assert_eq!(out.strategy, StrategyKind::Parallel);
        assert_eq!(out.attempts.len(), 2);
        assert!(out
            .attempts
            .iter()
            .all(|a| a.finish_reasons == vec![FinishReason::Stop]));
    }

    #[test]
    fn one_fails_floor_still_met() {
        let a = MockBackend::ok("a", "out-a");
        let b = MockBackend::fail("b", || BackendError::Network {
            message: "boom".into(),
        });
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::Concat,
        );

        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        // Short-circuit means we may return before the failing backend
        // settles; attempt count is therefore >= min_responses and <= targets.
        assert!(
            out.attempts.len() >= 1 && out.attempts.len() <= 2,
            "expected 1 or 2 attempts, got {}",
            out.attempts.len()
        );
        let ok_count = out
            .attempts
            .iter()
            .filter(|a| a.finish_reasons == vec![FinishReason::Stop])
            .count();
        assert_eq!(ok_count, 1, "expected exactly 1 success");
        assert_eq!(a.calls(), 1);
        // `b` may or may not have been polled before short-circuit.
        assert!(b.calls() <= 1);
    }

    #[test]
    fn floor_violation() {
        let a = MockBackend::ok("a", "out-a");
        let b = MockBackend::fail("b", || BackendError::Network {
            message: "boom".into(),
        });
        let c = MockBackend::fail("c", || BackendError::Auth {
            message: "bad key".into(),
        });
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone(), c.clone()];
        let strategy = ParallelFanOut::new(
            vec![
                TargetSpec::new("a"),
                TargetSpec::new("b"),
                TargetSpec::new("c"),
            ],
            3,
            "render-me",
            Aggregator::Concat,
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::FloorViolation {
                successes,
                min_responses,
                output,
            } => {
                assert_eq!(successes, 1);
                assert_eq!(min_responses, 3);
                assert_eq!(output.attempts.len(), 3);
                assert_eq!(output.strategy, StrategyKind::Parallel);
            }
            other => panic!("expected FloorViolation, got {other:?}"),
        }
    }

    #[test]
    fn empty_targets_yields_no_backends() {
        let backends: Vec<Arc<dyn Backend>> = vec![MockBackend::ok("a", "x")];
        let strategy = ParallelFanOut::new(vec![], 1, "x", Aggregator::Concat);

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        assert!(matches!(err, StrategyError::NoBackends));
    }

    #[test]
    fn prompt_render_failure_no_dispatch() {
        let a = MockBackend::ok("a", "x");
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a")],
            1,
            "{{ steps.missing.output }}",
            Aggregator::Concat,
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        assert!(matches!(err, StrategyError::PromptRender(_)));
        assert_eq!(a.calls(), 0);
    }

    #[test]
    fn backend_not_found() {
        let present = MockBackend::ok("present", "x");
        let backends: Vec<Arc<dyn Backend>> = vec![present.clone()];
        let strategy =
            ParallelFanOut::new(vec![TargetSpec::new("absent")], 1, "x", Aggregator::Concat);

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        assert!(matches!(err, StrategyError::BackendNotFound { name } if name == "absent"));
        assert_eq!(present.calls(), 0);
    }
}
