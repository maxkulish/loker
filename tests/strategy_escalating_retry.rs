//! Integration tests for `Strategy::EscalatingRetry` (CLO-258).
//!
//! Pin the AC contract: walk the ladder cheap → strong, stop at the first
//! verify pass, record every attempt with its tier, and on full exhaustion
//! return `StrategyError::Exhausted` carrying the schema-shaped output.
//! Backend errors (retryable or not) record an attempt and do *not* skip
//! later rungs.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jsonschema::Validator;
use serde_json::Value;

use loker::backend::{Backend, BackendError, QueryOutput, TokenUsage};
use loker::strategy::escalating_retry::{EscalatingRetry, Rung};
use loker::strategy::verify::{VerifyError, VerifyHook, VerifyResult};
use loker::strategy::{
    FinalStatus, PhaseContext, Prompt, Strategy, StrategyError, StrategyKind, Tier, VerifyStatus,
    SCHEMA_VERSION,
};

const SCHEMA_PATH: &str = "docs/schemas/phase_result_escalating.schema.json";

struct MockBackend {
    name: String,
    calls: AtomicUsize,
    response: Box<dyn Fn() -> Result<QueryOutput, BackendError> + Send + Sync>,
}

impl MockBackend {
    fn ok(name: &str, model: &str, text: &str) -> Arc<Self> {
        let backend_name = name.to_string();
        let model_owned = model.to_string();
        let text_owned = text.to_string();
        Arc::new(Self {
            name: name.to_string(),
            calls: AtomicUsize::new(0),
            response: Box::new(move || {
                Ok(QueryOutput::from_text(
                    text_owned.clone(),
                    backend_name.clone(),
                    Duration::from_millis(12),
                )
                .with_model(Some(model_owned.clone()))
                .with_usage(Some(TokenUsage {
                    prompt_tokens: 800,
                    completion_tokens: 400,
                    total_tokens: 1200,
                })))
            }),
        })
    }

    fn fail(name: &str, err: impl Fn() -> BackendError + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            calls: AtomicUsize::new(0),
            response: Box::new(move || Err(err())),
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
        self.calls.fetch_add(1, Ordering::SeqCst);
        (self.response)()
    }

    fn is_available(&self) -> bool {
        true
    }
}

struct SequenceVerify {
    results: Vec<Result<VerifyResult, VerifyError>>,
    calls: AtomicUsize,
}

impl SequenceVerify {
    fn new(results: Vec<Result<VerifyResult, VerifyError>>) -> Arc<Self> {
        Arc::new(Self {
            results,
            calls: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl VerifyHook for SequenceVerify {
    fn name(&self) -> &str {
        "make check"
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
        "make check"
    }

    async fn verify(&self, _output: &QueryOutput) -> Result<VerifyResult, VerifyError> {
        Ok(VerifyResult::pass())
    }
}

fn ctx() -> PhaseContext {
    PhaseContext::new("implement", uuid::Uuid::new_v4())
}

fn run<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Runtime::new().unwrap().block_on(f)
}

#[test]
fn first_pass_success_returns_immediately() {
    let cheap = MockBackend::ok("cheap", "cheap-model", "ok");
    let medium = MockBackend::ok("medium", "medium-model", "should not run");
    let backends: Vec<Arc<dyn Backend>> = vec![cheap.clone(), medium.clone()];

    let strategy = EscalatingRetry::new(
        vec![
            Rung::new(Tier::Cheap, "cheap"),
            Rung::new(Tier::Medium, "medium"),
        ],
        "render-me",
        SequenceVerify::new(vec![Ok(VerifyResult::pass())]),
    );

    let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();

    assert_eq!(out.strategy, StrategyKind::Escalating);
    assert_eq!(out.final_status, Some(FinalStatus::Succeeded));
    assert_eq!(out.attempts.len(), 1);
    assert_eq!(out.attempts[0].tier, Some(Tier::Cheap));
    assert_eq!(out.attempts[0].verify.status, VerifyStatus::Pass);
    assert_eq!(cheap.calls(), 1);
    assert_eq!(medium.calls(), 0, "ladder must short-circuit on first pass");
}

#[test]
fn mid_list_pass_returns_winner_and_captures_earlier_attempts() {
    let cheap = MockBackend::ok("cheap", "cheap-model", "bad");
    let medium = MockBackend::ok("medium", "medium-model", "good");
    let strong = MockBackend::ok("strong", "strong-model", "unused");
    let backends: Vec<Arc<dyn Backend>> = vec![cheap.clone(), medium.clone(), strong.clone()];

    let strategy = EscalatingRetry::new(
        vec![
            Rung::new(Tier::Cheap, "cheap"),
            Rung::new(Tier::Medium, "medium"),
            Rung::new(Tier::Strong, "strong"),
        ],
        "render-me",
        SequenceVerify::new(vec![
            Ok(VerifyResult::fail("needs work")),
            Ok(VerifyResult::pass()),
        ]),
    );

    let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();

    assert_eq!(out.attempts.len(), 2);
    assert_eq!(out.attempts[0].tier, Some(Tier::Cheap));
    assert_eq!(out.attempts[0].verify.status, VerifyStatus::Fail);
    assert_eq!(out.attempts[1].tier, Some(Tier::Medium));
    assert_eq!(out.attempts[1].backend, "medium");
    assert_eq!(out.attempts[1].verify.status, VerifyStatus::Pass);
    assert_eq!(out.final_status, Some(FinalStatus::Succeeded));
    assert_eq!(strong.calls(), 0, "must stop after first pass");
}

#[test]
fn full_exhaustion_returns_exhausted_error_with_all_attempts() {
    let cheap = MockBackend::ok("cheap", "a", "no");
    let medium = MockBackend::ok("medium", "b", "no");
    let backends: Vec<Arc<dyn Backend>> = vec![cheap.clone(), medium.clone()];

    let strategy = EscalatingRetry::new(
        vec![
            Rung::new(Tier::Cheap, "cheap"),
            Rung::new(Tier::Medium, "medium"),
        ],
        "render-me",
        SequenceVerify::new(vec![
            Ok(VerifyResult::fail("bad")),
            Ok(VerifyResult::fail("still bad")),
        ]),
    );

    let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();

    match err {
        StrategyError::Exhausted { output } => {
            assert_eq!(output.attempts.len(), 2);
            assert!(output
                .attempts
                .iter()
                .all(|a| a.verify.status == VerifyStatus::Fail));
            assert_eq!(output.final_status, Some(FinalStatus::Exhausted));
            assert_eq!(output.strategy, StrategyKind::Escalating);
        }
        other => panic!("expected Exhausted, got {other:?}"),
    }
}

#[test]
fn non_retryable_backend_error_does_not_skip_subsequent_backends() {
    let auth = MockBackend::fail("cheap", || BackendError::Auth {
        message: "bad key".to_string(),
    });
    let medium = MockBackend::ok("medium", "medium-model", "ok");
    let backends: Vec<Arc<dyn Backend>> = vec![auth.clone(), medium.clone()];

    let strategy = EscalatingRetry::new(
        vec![
            Rung::new(Tier::Cheap, "cheap"),
            Rung::new(Tier::Medium, "medium"),
        ],
        "render-me",
        SequenceVerify::new(vec![Ok(VerifyResult::pass())]),
    );

    let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();

    assert_eq!(out.attempts.len(), 2);
    assert_eq!(out.attempts[0].tier, Some(Tier::Cheap));
    assert_eq!(out.attempts[0].verify.status, VerifyStatus::Skipped);
    assert_eq!(out.attempts[1].verify.status, VerifyStatus::Pass);
    assert_eq!(auth.calls(), 1);
    assert_eq!(medium.calls(), 1);
}

#[test]
fn pass_failure_context_defaults_false() {
    let strategy = EscalatingRetry::new(
        vec![Rung::new(Tier::Cheap, "cheap")],
        "render-me",
        Arc::new(AlwaysPass),
    );

    assert!(!strategy.pass_failure_context);
    assert!(
        strategy
            .with_pass_failure_context(true)
            .pass_failure_context
    );
}

#[test]
fn phase_result_json_validates_against_escalating_schema() {
    let cheap = MockBackend::ok("openai", "gpt-5-mini", "needs work");
    let medium = MockBackend::ok("anthropic", "claude-sonnet-4-6", "ok");
    let backends: Vec<Arc<dyn Backend>> = vec![cheap, medium];

    let strategy = EscalatingRetry::new(
        vec![
            Rung::new(Tier::Cheap, "openai"),
            Rung::new(Tier::Medium, "anthropic"),
        ],
        "render-me",
        SequenceVerify::new(vec![
            Ok(VerifyResult::fail("nope")),
            Ok(VerifyResult::pass()),
        ]),
    );

    let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
    assert_eq!(out.schema_version, SCHEMA_VERSION);

    let json = serde_json::to_value(&out).unwrap();
    let schema = load_schema();
    if let Err(e) = schema.validate(&json) {
        panic!(
            "EscalatingRetry output failed schema validation: {}\npayload: {}",
            e,
            serde_json::to_string_pretty(&json).unwrap()
        );
    }
}

#[test]
fn exhausted_payload_validates_against_schema() {
    let cheap = MockBackend::ok("openai", "gpt-5-mini", "no");
    let medium = MockBackend::ok("anthropic", "claude-sonnet-4-6", "no");
    let strong = MockBackend::ok("anthropic-strong", "claude-opus-4-7", "still no");
    let backends: Vec<Arc<dyn Backend>> = vec![cheap, medium, strong];

    let strategy = EscalatingRetry::new(
        vec![
            Rung::new(Tier::Cheap, "openai"),
            Rung::new(Tier::Medium, "anthropic"),
            Rung::new(Tier::Strong, "anthropic-strong"),
        ],
        "render-me",
        SequenceVerify::new(vec![
            Ok(VerifyResult::fail("a")),
            Ok(VerifyResult::fail("b")),
            Ok(VerifyResult::fail("c")),
        ]),
    );

    let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
    let output = match err {
        StrategyError::Exhausted { output } => output,
        other => panic!("expected Exhausted, got {other:?}"),
    };

    let json = serde_json::to_value(&*output).unwrap();
    let schema = load_schema();
    if let Err(e) = schema.validate(&json) {
        panic!(
            "Exhausted output failed schema validation: {}\npayload: {}",
            e,
            serde_json::to_string_pretty(&json).unwrap()
        );
    }
}

#[test]
fn empty_rungs_yields_no_backends_error() {
    let backends: Vec<Arc<dyn Backend>> = vec![MockBackend::ok("present", "m", "x")];
    let strategy = EscalatingRetry::new(vec![], "x", Arc::new(AlwaysPass));

    let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
    assert!(matches!(err, StrategyError::NoBackends));
}

#[test]
fn missing_backend_in_pool_returns_backend_not_found() {
    let backends: Vec<Arc<dyn Backend>> = vec![MockBackend::ok("present", "m", "x")];
    let strategy = EscalatingRetry::new(
        vec![Rung::new(Tier::Cheap, "absent")],
        "x",
        Arc::new(AlwaysPass),
    );

    let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
    assert!(matches!(err, StrategyError::BackendNotFound { name } if name == "absent"));
}

fn load_schema() -> Validator {
    let path = repo_root().join(SCHEMA_PATH);
    let raw =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
    let schema: Value = serde_json::from_str(&raw).expect("parse schema");
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&schema)
        .expect("compile schema")
}

fn repo_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
