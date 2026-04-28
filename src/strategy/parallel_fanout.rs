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

use crate::aggregator::{aggregate_llm_judge, Aggregator, BranchSuccess};
use crate::backend::{Backend, QueryOutput};
use crate::family::family_of;
use crate::strategy::{
    Attempt, FinishReason, PhaseContext, Prompt, Strategy, StrategyError, StrategyKind,
    StrategyOutput, TokenUsageReport, VerifyOutcome, SCHEMA_VERSION,
};
use async_trait::async_trait;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use std::path::Path;
use std::sync::Arc;
use tokio::fs;

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
        assert!(
            min_responses > 0,
            "min_responses must be greater than 0, got {min_responses}"
        );
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
        let mut successful_candidates: Vec<crate::aggregator::BranchSuccess> =
            Vec::with_capacity(self.targets.len());
        let is_any_fail = matches!(self.aggregator, Aggregator::AnyFail);
        let is_llm_judge = matches!(self.aggregator, Aggregator::LLMJudge { .. });

        while let Some((idx, result)) = futures.next().await {
            let target = &self.targets[idx];

            match result {
                Ok(query) => {
                    let usage = query
                        .usage
                        .as_ref()
                        .map(TokenUsageReport::from)
                        .unwrap_or_default();
                    let model = pick_model_override(&query, prompt, target);
                    let output_path = format!("{}/attempts/{}-parallel.txt", ctx.phase_name, idx);

                    if is_any_fail {
                        if let Err(reason) = crate::aggregator::any_fail_evaluate(&query.stdout) {
                            let offender = Attempt {
                                tier: None,
                                family: Some(family_of(&target.backend).to_string()),
                                backend: target.backend.clone(),
                                model,
                                finish_reasons: vec![FinishReason::Stop],
                                usage,
                                output_path,
                                verify: VerifyOutcome::skipped(),
                            };
                            attempts.push(offender.clone());
                            let output = StrategyOutput {
                                schema_version: SCHEMA_VERSION,
                                strategy: StrategyKind::Parallel,
                                phase: ctx.phase_name.clone(),
                                run_id: ctx.run_id,
                                attempts,
                                final_status: None,
                                aggregator: Some(self.aggregator.kind()),
                                aggregate_output_path: Some(format!(
                                    "{}/aggregated.txt",
                                    ctx.phase_name
                                )),
                                verify: Some(VerifyOutcome::failed("Aggregator::AnyFail")),
                            };
                            return Err(StrategyError::AnyFail {
                                backend: target.backend.clone(),
                                reason,
                                offender: Box::new(offender),
                                output: Box::new(output),
                            });
                        }
                    }

                    successes += 1;
                    successful_candidates.push(BranchSuccess {
                        backend_id: target.backend.clone(),
                        family: family_of(&target.backend).to_string(),
                        index: successful_candidates.len() + 1,
                        output: query.stdout.clone(),
                    });

                    attempts.push(Attempt {
                        tier: None,
                        family: Some(family_of(&target.backend).to_string()),
                        backend: target.backend.clone(),
                        model,
                        finish_reasons: vec![FinishReason::Stop],
                        usage,
                        output_path,
                        verify: VerifyOutcome::skipped(),
                    });

                    if !is_any_fail && !is_llm_judge && successes >= self.min_responses {
                        // For non-LLMJudge aggregation modes, stop once enough
                        // successes are in to meet the configured floor.
                        // LLMJudge must inspect all candidates first and therefore
                        // cannot short-circuit on min_responses.
                        break;
                    }
                }
                Err(err) => {
                    let model = target
                        .model
                        .as_ref()
                        .filter(|m| !m.is_empty())
                        .cloned()
                        .or_else(|| prompt.model.clone().filter(|m| !m.is_empty()))
                        .unwrap_or_else(|| "default".to_string());
                    let output_path = format!("{}/attempts/{}-parallel.txt", ctx.phase_name, idx);

                    let attempt = Attempt {
                        tier: None,
                        family: Some(family_of(&target.backend).to_string()),
                        backend: target.backend.clone(),
                        model,
                        finish_reasons: vec![FinishReason::Error],
                        usage: TokenUsageReport::default(),
                        output_path,
                        verify: VerifyOutcome::skipped(),
                    };

                    if is_any_fail {
                        attempts.push(attempt.clone());
                        let output = StrategyOutput {
                            schema_version: SCHEMA_VERSION,
                            strategy: StrategyKind::Parallel,
                            phase: ctx.phase_name.clone(),
                            run_id: ctx.run_id,
                            attempts,
                            final_status: None,
                            aggregator: Some(self.aggregator.kind()),
                            aggregate_output_path: Some(format!(
                                "{}/aggregated.txt",
                                ctx.phase_name
                            )),
                            verify: Some(VerifyOutcome::failed("Aggregator::AnyFail")),
                        };
                        return Err(StrategyError::AnyFail {
                            backend: target.backend.clone(),
                            reason: crate::aggregator::AnyFailReason::BackendError {
                                detail: err.to_string(),
                            },
                            offender: Box::new(attempt),
                            output: Box::new(output),
                        });
                    }

                    attempts.push(attempt);
                }
            }
        }

        // Dropped futures are cancelled (cooperatively) when `futures` falls
        // out of scope here.

        if is_any_fail {
            return Ok(StrategyOutput {
                schema_version: SCHEMA_VERSION,
                strategy: StrategyKind::Parallel,
                phase: ctx.phase_name.clone(),
                run_id: ctx.run_id,
                attempts,
                final_status: None,
                aggregator: Some(self.aggregator.kind()),
                aggregate_output_path: Some(format!("{}/aggregated.txt", ctx.phase_name)),
                verify: Some(VerifyOutcome::passed("Aggregator::AnyFail")),
            });
        }

        if successes < self.min_responses {
            let output = StrategyOutput {
                schema_version: SCHEMA_VERSION,
                strategy: StrategyKind::Parallel,
                phase: ctx.phase_name.clone(),
                run_id: ctx.run_id,
                attempts,
                final_status: None,
                aggregator: Some(self.aggregator.kind()),
                aggregate_output_path: Some(format!("{}/aggregated.txt", ctx.phase_name)),
                verify: Some(VerifyOutcome::skipped()),
            };
            return Err(StrategyError::FloorViolation {
                successes,
                min_responses: self.min_responses,
                output: Box::new(output),
            });
        }

        let aggregated_output_path = format!("{}/aggregated.txt", ctx.phase_name);
        let mut output = StrategyOutput {
            schema_version: SCHEMA_VERSION,
            strategy: StrategyKind::Parallel,
            phase: ctx.phase_name.clone(),
            run_id: ctx.run_id,
            attempts,
            final_status: None,
            aggregator: Some(self.aggregator.kind()),
            aggregate_output_path: Some(aggregated_output_path.clone()),
            verify: Some(VerifyOutcome::skipped()),
        };

        if let Aggregator::LLMJudge {
            judge_backend,
            prompt_template,
            require_judge_different_family,
        } = &self.aggregator
        {
            let aggregate = aggregate_llm_judge(
                &successful_candidates,
                judge_backend,
                prompt_template,
                *require_judge_different_family,
                backends,
                ctx,
            )
            .await
            .map_err(|err| {
                use crate::aggregator::LLMJudgeError;
                match err {
                    LLMJudgeError::FamilyOverlap { candidate, .. } => {
                        let fam = family_of(&candidate);
                        let count = successful_candidates
                            .iter()
                            .filter(|c| family_of(&c.backend_id) == fam)
                            .count()
                            + 1;
                        StrategyError::Phase(crate::family::PhaseError::FamilyOverlap {
                            family: fam,
                            count,
                        })
                    }
                    LLMJudgeError::Contract { message } => {
                        StrategyError::Phase(crate::family::PhaseError::AggregatorContract {
                            message,
                        })
                    }
                    LLMJudgeError::JudgeCall(err) => {
                        StrategyError::Phase(crate::family::PhaseError::JudgeUnavailable {
                            detail: err.to_string(),
                        })
                    }
                    LLMJudgeError::BackendNotFound(name) => StrategyError::BackendNotFound { name },
                }
            })?;

            let aggregate_output_path = aggregated_output_path.clone();

            if let Some(parent) = Path::new(&aggregate_output_path).parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent).await.map_err(|err| {
                        StrategyError::Backend(crate::backend::BackendError::ExecutionFailed {
                            message: format!(
                                "failed to create aggregate output parent {}: {err}",
                                parent.display()
                            ),
                            exit_code: None,
                        })
                    })?;
                }
            }

            let aggregate_output_path = aggregate_output_path.clone();
            let aggregate_output_path_ref = aggregate_output_path.as_str();
            fs::write(&aggregate_output_path, aggregate.text)
                .await
                .map_err(|err| {
                    StrategyError::Backend(crate::backend::BackendError::ExecutionFailed {
                        message: format!(
                            "failed to write aggregate output to {aggregate_output_path_ref}: {err}"
                        ),
                        exit_code: None,
                    })
                })?;

            output.verify = Some(VerifyOutcome::passed("LLMJudge"));
        }

        Ok(output)
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
    use crate::aggregator::AnyFailReason;
    use crate::backend::BackendError;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct MockBackend {
        name: String,
        calls: AtomicUsize,
        response: Box<dyn Fn(usize) -> Result<QueryOutput, BackendError> + Send + Sync>,
        delay_ms: Option<u64>,
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
                delay_ms: None,
            })
        }

        fn fail(name: &str, err: impl Fn() -> BackendError + Send + Sync + 'static) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                calls: AtomicUsize::new(0),
                response: Box::new(move |_| Err(err())),
                delay_ms: None,
            })
        }

        fn slow(name: &str, text: &str, delay_ms: u64) -> Arc<Self> {
            let backend_name = name.to_string();
            let text_owned = text.to_string();
            Arc::new(Self {
                name: name.to_string(),
                calls: AtomicUsize::new(0),
                response: Box::new(move |_| {
                    Ok(QueryOutput::from_text(
                        text_owned.clone(),
                        backend_name.clone(),
                        Duration::from_millis(delay_ms),
                    )
                    .with_model(Some("mock-1")))
                }),
                delay_ms: Some(delay_ms),
            })
        }

        fn delayed_ok(name: &str, text: &str, delay_ms: u64) -> Arc<Self> {
            Self::slow(name, text, delay_ms)
        }

        fn delayed_fail(
            name: &str,
            err: impl Fn() -> BackendError + Send + Sync + 'static,
            delay_ms: u64,
        ) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                calls: AtomicUsize::new(0),
                response: Box::new(move |_| Err(err())),
                delay_ms: Some(delay_ms),
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
            if let Some(ms) = self.delay_ms {
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
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
            Aggregator::concat("## {backend_id}"),
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
            Aggregator::concat("## {backend_id}"),
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
            Aggregator::concat("## {backend_id}"),
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
        let strategy = ParallelFanOut::new(vec![], 1, "x", Aggregator::concat("## {backend_id}"));

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
            Aggregator::concat("## {backend_id}"),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        assert!(matches!(err, StrategyError::PromptRender(_)));
        assert_eq!(a.calls(), 0);
    }

    #[test]
    fn backend_not_found() {
        let present = MockBackend::ok("present", "x");
        let backends: Vec<Arc<dyn Backend>> = vec![present.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("absent")],
            1,
            "x",
            Aggregator::concat("## {backend_id}"),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        assert!(matches!(err, StrategyError::BackendNotFound { name } if name == "absent"));
        assert_eq!(present.calls(), 0);
    }

    #[test]
    fn any_fail_all_pass() {
        let a = MockBackend::ok("a", r#"{"pass": true}"#);
        let b = MockBackend::ok("b", r#"{"pass": true}"#);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        assert_eq!(out.strategy, StrategyKind::Parallel);
        assert_eq!(out.attempts.len(), 2);
        assert_eq!(
            out.verify.as_ref().unwrap().status,
            crate::strategy::VerifyStatus::Pass
        );
    }

    #[test]
    fn any_fail_first_fails() {
        let b0 = MockBackend::delayed_ok("b0", r#"{"pass": false}"#, 1);
        let b1 = MockBackend::delayed_ok("b1", r#"{"pass": true}"#, 10);
        let b2 = MockBackend::delayed_ok("b2", r#"{"pass": true}"#, 10);
        let backends: Vec<Arc<dyn Backend>> = vec![b0.clone(), b1.clone(), b2.clone()];
        let strategy = ParallelFanOut::new(
            vec![
                TargetSpec::new("b0"),
                TargetSpec::new("b1"),
                TargetSpec::new("b2"),
            ],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "b0");
                assert!(matches!(reason, AnyFailReason::VerdictRejected { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_mid_list_fails() {
        let b0 = MockBackend::delayed_ok("b0", r#"{"pass": true}"#, 1);
        let b1 = MockBackend::delayed_ok("b1", r#"{"pass": false}"#, 5);
        let b2 = MockBackend::delayed_ok("b2", r#"{"pass": true}"#, 10);
        let backends: Vec<Arc<dyn Backend>> = vec![b0.clone(), b1.clone(), b2.clone()];
        let strategy = ParallelFanOut::new(
            vec![
                TargetSpec::new("b0"),
                TargetSpec::new("b1"),
                TargetSpec::new("b2"),
            ],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "b1");
                assert!(matches!(reason, AnyFailReason::VerdictRejected { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_all_fail() {
        let b0 = MockBackend::delayed_ok("b0", r#"{"pass": false}"#, 1);
        let b1 = MockBackend::delayed_ok("b1", r#"{"pass": false}"#, 5);
        let backends: Vec<Arc<dyn Backend>> = vec![b0.clone(), b1.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("b0"), TargetSpec::new("b1")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                // b0 has shorter delay, so it arrives first
                assert_eq!(backend, "b0");
                assert!(matches!(reason, AnyFailReason::VerdictRejected { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_backend_error_treated_as_failure() {
        let a = MockBackend::ok("a", r#"{"pass": true}"#);
        let b = MockBackend::delayed_fail(
            "b",
            || BackendError::Network {
                message: "boom".into(),
            },
            1,
        );
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "b");
                assert!(matches!(reason, AnyFailReason::BackendError { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_missing_pass_field() {
        let a = MockBackend::delayed_ok("a", r#"{"status": "ok"}"#, 1);
        let b = MockBackend::delayed_ok("b", r#"{"pass": true}"#, 10);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "a");
                assert!(matches!(reason, AnyFailReason::VerdictContract { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_wrong_pass_type() {
        let a = MockBackend::delayed_ok("a", r#"{"pass": "yes"}"#, 1);
        let b = MockBackend::delayed_ok("b", r#"{"pass": true}"#, 10);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "a");
                assert!(matches!(reason, AnyFailReason::VerdictContract { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_empty_query_text() {
        let a = MockBackend::delayed_ok("a", "", 1);
        let b = MockBackend::delayed_ok("b", r#"{"pass": true}"#, 10);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "a");
                assert!(matches!(reason, AnyFailReason::VerdictContract { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_markdown_fenced_json() {
        let a = MockBackend::ok("a", "```json\n{\"pass\": true}\n```");
        let b = MockBackend::ok("b", "```json\n{\"pass\": true}\n```");
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        assert_eq!(out.attempts.len(), 2);
        assert_eq!(
            out.verify.as_ref().unwrap().status,
            crate::strategy::VerifyStatus::Pass
        );
    }

    #[test]
    fn any_fail_markdown_fenced_fail() {
        let a = MockBackend::ok("a", "```json\n{\"pass\": false}\n```");
        let b = MockBackend::ok("b", r#"{"pass": true}"#);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "a");
                assert!(matches!(reason, AnyFailReason::VerdictRejected { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_valid_json_extra_keys() {
        let a = MockBackend::ok("a", r#"{"pass": true, "note": "lgtm"}"#);
        let b = MockBackend::ok("b", r#"{"pass": true}"#);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        assert_eq!(out.attempts.len(), 2);
        assert_eq!(
            out.verify.as_ref().unwrap().status,
            crate::strategy::VerifyStatus::Pass
        );
    }

    #[test]
    fn any_fail_non_deterministic_offender() {
        let b0 = MockBackend::delayed_ok("b0", r#"{"pass": false}"#, 1);
        let b1 = MockBackend::delayed_ok("b1", r#"{"pass": false}"#, 1);
        let backends: Vec<Arc<dyn Backend>> = vec![b0.clone(), b1.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("b0"), TargetSpec::new("b1")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert!(
                    matches!(&backend as &str, "b0" | "b1"),
                    "expected b0 or b1, got {backend}"
                );
                assert!(matches!(reason, AnyFailReason::VerdictRejected { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }
}
