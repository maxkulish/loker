use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::backend::{Backend, BackendError, QueryOutput};
use crate::verify::{VerifyHook, VerifyResult};

pub type BackendId = String;

#[derive(Clone)]
pub enum Strategy {
    EscalatingRetry(EscalatingRetry),
}

#[derive(Clone)]
pub struct EscalatingRetry {
    pub backends: Vec<BackendId>,
    pub verify: Arc<dyn VerifyHook>,
    pub pass_failure_context: bool,
}

impl EscalatingRetry {
    pub fn new(backends: Vec<BackendId>, verify: Arc<dyn VerifyHook>) -> Self {
        Self {
            backends,
            verify,
            pass_failure_context: false,
        }
    }

    pub fn with_pass_failure_context(mut self, enabled: bool) -> Self {
        self.pass_failure_context = enabled;
        self
    }
}

pub struct StrategyRunner {
    backends: HashMap<BackendId, Arc<dyn Backend>>,
}

impl StrategyRunner {
    pub fn new(backends: impl IntoIterator<Item = (BackendId, Arc<dyn Backend>)>) -> Self {
        Self {
            backends: backends.into_iter().collect(),
        }
    }

    pub async fn run(
        &self,
        strategy: &Strategy,
        prompt: &str,
        cwd: &Path,
    ) -> Result<PhaseResult, PhaseError> {
        match strategy {
            Strategy::EscalatingRetry(strategy) => {
                self.run_escalating_retry(strategy, prompt, cwd).await
            }
        }
    }

    pub async fn run_escalating_retry(
        &self,
        strategy: &EscalatingRetry,
        prompt: &str,
        cwd: &Path,
    ) -> Result<PhaseResult, PhaseError> {
        if strategy.backends.is_empty() {
            return Err(PhaseError::InvalidStrategy {
                message: "EscalatingRetry requires at least one backend".to_string(),
            });
        }

        let mut attempts = Vec::new();

        for backend_id in &strategy.backends {
            let backend =
                self.backends
                    .get(backend_id)
                    .ok_or_else(|| PhaseError::BackendNotFound {
                        backend: backend_id.clone(),
                    })?;

            match backend.query(prompt, cwd, None).await {
                Ok(output) => match strategy.verify.verify(&output).await {
                    Ok(verify) => {
                        let passed = verify.is_pass();
                        attempts.push(EscalatingAttempt::from_output(
                            backend_id.clone(),
                            output,
                            VerifyOutcome::Ran {
                                hook: strategy.verify.name().to_string(),
                                result: verify,
                            },
                        ));

                        if passed {
                            return Ok(PhaseResult::Escalating {
                                attempts,
                                final_status: EscalatingFinalStatus::Succeeded,
                            });
                        }
                    }
                    Err(err) => {
                        attempts.push(EscalatingAttempt::from_output(
                            backend_id.clone(),
                            output,
                            VerifyOutcome::HookError {
                                hook: strategy.verify.name().to_string(),
                                message: err.message,
                            },
                        ));
                    }
                },
                Err(error) => {
                    attempts.push(EscalatingAttempt::from_error(backend_id.clone(), error));
                }
            }
        }

        Err(PhaseError::Exhausted { attempts })
    }
}

#[derive(Debug, Clone)]
pub enum PhaseResult {
    Escalating {
        attempts: Vec<EscalatingAttempt>,
        final_status: EscalatingFinalStatus,
    },
}

impl PhaseResult {
    pub fn attempts(&self) -> &[EscalatingAttempt] {
        match self {
            Self::Escalating { attempts, .. } => attempts,
        }
    }

    pub fn to_json_value(&self, phase: &str, run_id: &str) -> Value {
        match self {
            Self::Escalating {
                attempts,
                final_status,
            } => json!({
                "schema_version": 1,
                "loker.strategy": "escalating",
                "loker.phase": phase,
                "loker.run_id": run_id,
                "attempts": attempts.iter().enumerate().map(|(idx, attempt)| {
                    attempt.to_json_value(phase, idx)
                }).collect::<Vec<_>>(),
                "final_status": final_status.as_str(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscalatingFinalStatus {
    Succeeded,
    Exhausted,
    Aborted,
}

impl EscalatingFinalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Exhausted => "exhausted",
            Self::Aborted => "aborted",
        }
    }
}

#[derive(Debug, Clone)]
pub struct EscalatingAttempt {
    pub backend: BackendId,
    pub output: Option<QueryOutput>,
    pub backend_error: Option<BackendError>,
    pub backend_error_retryable: Option<bool>,
    pub verify: VerifyOutcome,
}

impl EscalatingAttempt {
    fn from_output(backend: BackendId, output: QueryOutput, verify: VerifyOutcome) -> Self {
        Self {
            backend,
            output: Some(output),
            backend_error: None,
            backend_error_retryable: None,
            verify,
        }
    }

    fn from_error(backend: BackendId, error: BackendError) -> Self {
        Self {
            backend,
            output: None,
            backend_error_retryable: Some(error.is_retryable()),
            backend_error: Some(error),
            verify: VerifyOutcome::Skipped,
        }
    }

    fn to_json_value(&self, phase: &str, idx: usize) -> Value {
        let tier = tier_for_index(idx);
        let model = self
            .output
            .as_ref()
            .and_then(|output| output.model.as_ref())
            .cloned()
            .unwrap_or_else(|| self.backend.clone());
        let usage = self
            .output
            .as_ref()
            .and_then(|output| output.usage.as_ref())
            .map(|usage| {
                json!({
                    "input_tokens": usage.prompt_tokens,
                    "output_tokens": usage.completion_tokens,
                })
            })
            .unwrap_or_else(|| json!({ "input_tokens": 0, "output_tokens": 0 }));
        let finish_reasons = if self.backend_error.is_some() {
            vec!["error"]
        } else {
            vec!["stop"]
        };

        json!({
            "tier": tier,
            "backend": self.backend,
            "model": model,
            "finish_reasons": finish_reasons,
            "usage": usage,
            "output_path": format!("{phase}/attempts/{}-{tier}.md", idx + 1),
            "verify": self.verify.to_json_value(),
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum VerifyOutcome {
    Ran { hook: String, result: VerifyResult },
    HookError { hook: String, message: String },
    Skipped,
}

impl VerifyOutcome {
    pub fn is_pass(&self) -> bool {
        matches!(
            self,
            Self::Ran {
                result: VerifyResult::Pass { .. },
                ..
            }
        )
    }

    fn to_json_value(&self) -> Value {
        match self {
            Self::Ran { hook, result } => {
                json!({ "status": result.phase_status(), "hook": hook })
            }
            Self::HookError { hook, .. } => json!({ "status": "fail", "hook": hook }),
            Self::Skipped => json!({ "status": "skipped", "hook": null }),
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum PhaseError {
    #[error("invalid strategy: {message}")]
    InvalidStrategy { message: String },

    #[error("backend not found: {backend}")]
    BackendNotFound { backend: BackendId },

    #[error("escalating retry exhausted all backends")]
    Exhausted { attempts: Vec<EscalatingAttempt> },
}

fn tier_for_index(idx: usize) -> &'static str {
    match idx {
        0 => "cheap",
        1 => "medium",
        _ => "strong",
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;

    use crate::backend::{BackendCapabilities, QueryOutput, TokenUsage};
    use crate::verify::VerifyError;

    use super::*;

    #[tokio::test]
    async fn first_pass_success_returns_immediately() {
        let cheap = Arc::new(MockBackend::new(
            "cheap",
            Ok(output("cheap", "cheap-model", "ok")),
        ));
        let medium = Arc::new(MockBackend::new(
            "medium",
            Ok(output("medium", "medium-model", "should not run")),
        ));
        let verify = Arc::new(SequenceVerify::new(vec![Ok(VerifyResult::pass())]));
        let runner = StrategyRunner::new(vec![
            ("cheap".to_string(), cheap.clone() as Arc<dyn Backend>),
            ("medium".to_string(), medium.clone() as Arc<dyn Backend>),
        ]);
        let strategy = EscalatingRetry::new(vec!["cheap".into(), "medium".into()], verify);

        let result = runner
            .run_escalating_retry(&strategy, "prompt", Path::new("."))
            .await
            .expect("first backend should pass");

        assert_eq!(result.attempts().len(), 1);
        assert_eq!(cheap.calls(), 1);
        assert_eq!(medium.calls(), 0);
        assert!(result.attempts()[0].verify.is_pass());
    }

    #[tokio::test]
    async fn mid_list_pass_returns_winner_and_captures_earlier_attempts() {
        let cheap = Arc::new(MockBackend::new(
            "cheap",
            Ok(output("cheap", "cheap-model", "bad")),
        ));
        let medium = Arc::new(MockBackend::new(
            "medium",
            Ok(output("medium", "medium-model", "good")),
        ));
        let strong = Arc::new(MockBackend::new(
            "strong",
            Ok(output("strong", "strong-model", "unused")),
        ));
        let verify = Arc::new(SequenceVerify::new(vec![
            Ok(VerifyResult::fail("needs work")),
            Ok(VerifyResult::pass()),
        ]));
        let runner = StrategyRunner::new(vec![
            ("cheap".to_string(), cheap.clone() as Arc<dyn Backend>),
            ("medium".to_string(), medium.clone() as Arc<dyn Backend>),
            ("strong".to_string(), strong.clone() as Arc<dyn Backend>),
        ]);
        let strategy = EscalatingRetry::new(
            vec!["cheap".into(), "medium".into(), "strong".into()],
            verify,
        );

        let result = runner
            .run_escalating_retry(&strategy, "prompt", Path::new("."))
            .await
            .expect("second backend should pass");

        assert_eq!(result.attempts().len(), 2);
        assert!(!result.attempts()[0].verify.is_pass());
        assert!(result.attempts()[1].verify.is_pass());
        assert_eq!(result.attempts()[1].backend, "medium");
        assert_eq!(strong.calls(), 0);
    }

    #[tokio::test]
    async fn full_exhaustion_returns_all_attempts() {
        let runner = StrategyRunner::new(vec![
            (
                "cheap".to_string(),
                Arc::new(MockBackend::new("cheap", Ok(output("cheap", "a", "no"))))
                    as Arc<dyn Backend>,
            ),
            (
                "medium".to_string(),
                Arc::new(MockBackend::new("medium", Ok(output("medium", "b", "no"))))
                    as Arc<dyn Backend>,
            ),
        ]);
        let strategy = EscalatingRetry::new(
            vec!["cheap".into(), "medium".into()],
            Arc::new(SequenceVerify::new(vec![
                Ok(VerifyResult::fail("bad")),
                Ok(VerifyResult::fail("still bad")),
            ])),
        );

        let err = runner
            .run_escalating_retry(&strategy, "prompt", Path::new("."))
            .await
            .expect_err("all backends should fail verification");

        match err {
            PhaseError::Exhausted { attempts } => {
                assert_eq!(attempts.len(), 2);
                assert!(attempts.iter().all(|attempt| !attempt.verify.is_pass()));
            }
            other => panic!("expected exhaustion, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_retryable_backend_error_does_not_skip_subsequent_backends() {
        let auth = Arc::new(MockBackend::new(
            "cheap",
            Err(BackendError::Auth {
                message: "bad key".to_string(),
            }),
        ));
        let medium = Arc::new(MockBackend::new(
            "medium",
            Ok(output("medium", "medium-model", "ok")),
        ));
        let runner = StrategyRunner::new(vec![
            ("cheap".to_string(), auth.clone() as Arc<dyn Backend>),
            ("medium".to_string(), medium.clone() as Arc<dyn Backend>),
        ]);
        let strategy = EscalatingRetry::new(
            vec!["cheap".into(), "medium".into()],
            Arc::new(SequenceVerify::new(vec![Ok(VerifyResult::pass())])),
        );

        let result = runner
            .run_escalating_retry(&strategy, "prompt", Path::new("."))
            .await
            .expect("second backend should pass");

        assert_eq!(result.attempts().len(), 2);
        assert_eq!(result.attempts()[0].backend_error_retryable, Some(false));
        assert!(result.attempts()[1].verify.is_pass());
        assert_eq!(auth.calls(), 1);
        assert_eq!(medium.calls(), 1);
    }

    #[test]
    fn pass_failure_context_defaults_false() {
        let strategy = EscalatingRetry::new(vec!["cheap".into()], Arc::new(AlwaysPass));

        assert!(!strategy.pass_failure_context);
        assert!(
            strategy
                .with_pass_failure_context(true)
                .pass_failure_context
        );
    }

    #[test]
    fn phase_result_json_matches_escalating_shape() {
        let result = PhaseResult::Escalating {
            attempts: vec![
                EscalatingAttempt::from_output(
                    "openai".to_string(),
                    output("openai", "gpt-5-mini", "needs work"),
                    VerifyOutcome::Ran {
                        hook: "make check".to_string(),
                        result: VerifyResult::fail("failed"),
                    },
                ),
                EscalatingAttempt::from_output(
                    "anthropic".to_string(),
                    output("anthropic", "claude-sonnet-4-6", "ok"),
                    VerifyOutcome::Ran {
                        hook: "make check".to_string(),
                        result: VerifyResult::pass(),
                    },
                ),
            ],
            final_status: EscalatingFinalStatus::Succeeded,
        };

        let json = result.to_json_value("implement", "01f2a4e0-3b6f-7c8d-9e1a-2b3c4d5e6f70");

        assert_eq!(
            json,
            json!({
                "schema_version": 1,
                "loker.strategy": "escalating",
                "loker.phase": "implement",
                "loker.run_id": "01f2a4e0-3b6f-7c8d-9e1a-2b3c4d5e6f70",
                "attempts": [
                    {
                        "tier": "cheap",
                        "backend": "openai",
                        "model": "gpt-5-mini",
                        "finish_reasons": ["stop"],
                        "usage": { "input_tokens": 800, "output_tokens": 400 },
                        "output_path": "implement/attempts/1-cheap.md",
                        "verify": { "status": "fail", "hook": "make check" }
                    },
                    {
                        "tier": "medium",
                        "backend": "anthropic",
                        "model": "claude-sonnet-4-6",
                        "finish_reasons": ["stop"],
                        "usage": { "input_tokens": 800, "output_tokens": 400 },
                        "output_path": "implement/attempts/2-medium.md",
                        "verify": { "status": "pass", "hook": "make check" }
                    }
                ],
                "final_status": "succeeded",
            })
        );
    }

    struct MockBackend {
        name: String,
        result: Result<QueryOutput, BackendError>,
        calls: AtomicUsize,
    }

    impl MockBackend {
        fn new(name: &str, result: Result<QueryOutput, BackendError>) -> Self {
            Self {
                name: name.to_string(),
                result,
                calls: AtomicUsize::new(0),
            }
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
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.result.clone()
        }

        fn is_available(&self) -> bool {
            true
        }

        fn capabilities(&self) -> BackendCapabilities {
            BackendCapabilities::none()
        }
    }

    struct SequenceVerify {
        results: Vec<Result<VerifyResult, VerifyError>>,
        calls: AtomicUsize,
    }

    impl SequenceVerify {
        fn new(results: Vec<Result<VerifyResult, VerifyError>>) -> Self {
            Self {
                results,
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl VerifyHook for SequenceVerify {
        fn name(&self) -> &str {
            "verify"
        }

        async fn verify(&self, _output: &QueryOutput) -> Result<VerifyResult, VerifyError> {
            let idx = self.calls.fetch_add(1, Ordering::SeqCst);
            self.results
                .get(idx)
                .cloned()
                .unwrap_or_else(|| Ok(VerifyResult::fail("unexpected verify call")))
        }
    }

    struct AlwaysPass;

    #[async_trait]
    impl VerifyHook for AlwaysPass {
        fn name(&self) -> &str {
            "verify"
        }

        async fn verify(&self, _output: &QueryOutput) -> Result<VerifyResult, VerifyError> {
            Ok(VerifyResult::pass())
        }
    }

    fn output(backend: &str, model: &str, text: &str) -> QueryOutput {
        QueryOutput::from_text(text.to_string(), backend, Duration::from_millis(12))
            .with_model(Some(model))
            .with_usage(Some(TokenUsage::new(800, 400)))
    }
}
