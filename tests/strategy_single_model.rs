//! Mock-backend tests for `Strategy::SingleModel`.
//!
//! Pin the AC11/AC12 contract from CLO-257: one call out, one response in,
//! no retry, no aggregation, error pass-through, schema-shaped output.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jsonschema::Validator;
use serde_json::Value;

use loker::backend::{Backend, BackendError, QueryOutput, TokenUsage};
use loker::strategy::single_model::SingleModel;
use loker::strategy::{
    PhaseContext, Prompt, Strategy, StrategyError, StrategyKind, SCHEMA_VERSION,
};

const SCHEMA_PATH: &str = "docs/schemas/phase_result_single.schema.json";

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
                .with_model(Some("mock-1"))
                .with_usage(Some(TokenUsage {
                    prompt_tokens: 7,
                    completion_tokens: 11,
                    total_tokens: 18,
                })))
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
fn happy_path_emits_one_attempt() {
    let backend = MockBackend::ok("mock", "hello");
    let backends: Vec<Arc<dyn Backend>> = vec![backend.clone()];
    let strategy = SingleModel::new("mock", "render-me");

    let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();

    assert_eq!(out.schema_version, SCHEMA_VERSION);
    assert_eq!(out.strategy, StrategyKind::Single);
    assert_eq!(out.attempts.len(), 1);
    assert_eq!(out.attempts[0].backend, "mock");
    assert_eq!(out.attempts[0].model, "mock-1");
    assert_eq!(out.attempts[0].usage.input_tokens, 7);
    assert_eq!(out.attempts[0].usage.output_tokens, 11);
    assert_eq!(backend.calls(), 1);
}

#[test]
fn no_retry_on_backend_error() {
    let backend = MockBackend::fail("mock", || BackendError::Network {
        message: "boom".into(),
    });
    let backends: Vec<Arc<dyn Backend>> = vec![backend.clone()];
    let strategy = SingleModel::new("mock", "x");

    let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();

    assert!(matches!(
        err,
        StrategyError::Backend(BackendError::Network { .. })
    ));
    assert_eq!(backend.calls(), 1, "SingleModel must not retry");
}

#[test]
fn no_aggregation_when_multiple_backends_present() {
    let chosen = MockBackend::ok("chosen", "out");
    let other = MockBackend::ok("other", "ignored");
    let backends: Vec<Arc<dyn Backend>> = vec![chosen.clone(), other.clone()];
    let strategy = SingleModel::new("chosen", "x");

    let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();

    assert_eq!(out.attempts.len(), 1);
    assert_eq!(out.attempts[0].backend, "chosen");
    assert_eq!(chosen.calls(), 1);
    assert_eq!(other.calls(), 0, "non-target backends must not be called");
}

#[test]
fn output_validates_against_d2_schema() {
    let backend = MockBackend::ok("mock", "hello");
    let backends: Vec<Arc<dyn Backend>> = vec![backend];
    let strategy = SingleModel::new("mock", "x");

    let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
    let json = serde_json::to_value(&out).expect("serialize");

    let schema = load_schema();
    if let Err(e) = schema.validate(&json) {
        panic!(
            "StrategyOutput failed schema validation: {}\npayload: {}",
            e,
            serde_json::to_string_pretty(&json).unwrap()
        );
    }
}

#[test]
fn prompt_render_failure_surfaces_template_error() {
    let backend = MockBackend::ok("mock", "x");
    let backends: Vec<Arc<dyn Backend>> = vec![backend.clone()];
    let strategy = SingleModel::new("mock", "{{ steps.missing.output }}");

    let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();

    assert!(matches!(err, StrategyError::PromptRender(_)));
    assert_eq!(
        backend.calls(),
        0,
        "backend must not be called when template render fails"
    );
}

#[test]
fn backend_not_found() {
    let backend = MockBackend::ok("present", "x");
    let backends: Vec<Arc<dyn Backend>> = vec![backend.clone()];
    let strategy = SingleModel::new("absent", "x");

    let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();

    assert!(matches!(err, StrategyError::BackendNotFound { name } if name == "absent"));
    assert_eq!(backend.calls(), 0);
}

#[test]
fn empty_backends_yields_no_backends_error() {
    let backends: Vec<Arc<dyn Backend>> = vec![];
    let strategy = SingleModel::new("mock", "x");

    let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();

    assert!(matches!(err, StrategyError::NoBackends));
}

#[test]
fn missing_usage_serialises_zeroes() {
    struct NoUsage;

    #[async_trait]
    impl Backend for NoUsage {
        fn name(&self) -> &str {
            "no-usage"
        }
        async fn query(
            &self,
            _: &str,
            _: &Path,
            _: Option<&str>,
        ) -> Result<QueryOutput, BackendError> {
            Ok(QueryOutput::from_text(
                "x".into(),
                "no-usage",
                Duration::from_millis(1),
            ))
        }
        fn is_available(&self) -> bool {
            true
        }
    }

    let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(NoUsage)];
    let strategy = SingleModel::new("no-usage", "x");
    let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();

    assert_eq!(out.attempts[0].usage.input_tokens, 0);
    assert_eq!(out.attempts[0].usage.output_tokens, 0);

    let json = serde_json::to_value(&out).unwrap();
    load_schema().validate(&json).expect("schema validates");
}

#[test]
fn prompt_model_override_falls_through_to_attempt() {
    struct NoModel;

    #[async_trait]
    impl Backend for NoModel {
        fn name(&self) -> &str {
            "nm"
        }
        async fn query(
            &self,
            _: &str,
            _: &Path,
            _: Option<&str>,
        ) -> Result<QueryOutput, BackendError> {
            Ok(QueryOutput::from_text(
                "x".into(),
                "nm",
                Duration::from_millis(1),
            ))
        }
        fn is_available(&self) -> bool {
            true
        }
    }

    let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(NoModel)];
    let strategy = SingleModel::new("nm", "x");
    let prompt = Prompt::new().with_model("override-model");

    let out = run(strategy.execute(&backends, &prompt, &ctx())).unwrap();
    assert_eq!(out.attempts[0].model, "override-model");
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
