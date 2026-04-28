//! Integration tests for `Aggregator::LLMJudge`.

use async_trait::async_trait;
use jsonschema::Validator;
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use insta::assert_snapshot;
use loker::aggregator::{aggregate_llm_judge, Aggregator, BranchSuccess};
use loker::backend::{Backend, BackendError, QueryOutput};
use loker::family::PhaseError;
use loker::strategy::parallel_fanout::{ParallelFanOut, TargetSpec};
use loker::strategy::{
    PhaseContext, Prompt, Strategy, StrategyError, StrategyKind, StrategyOutput, VerifyStatus,
};

const SCHEMA_PATH: &str = "docs/schemas/phase_result_parallel.schema.json";

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
                ))
            }),
        })
    }

    fn fail(name: &str, error: impl Fn() -> BackendError + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            name: name.to_string(),
            calls: AtomicUsize::new(0),
            response: Box::new(move |_| Err(error())),
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

fn run_test<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Runtime::new().unwrap().block_on(f)
}

fn validate_with_schema(output: &StrategyOutput) {
    let validator = load_validator();
    let value = serde_json::to_value(output).unwrap();
    assert!(
        validator.is_valid(&value),
        "StrategyOutput should satisfy phase_result_parallel schema"
    );
}

fn load_validator() -> Validator {
    let schema = std::fs::read_to_string(SCHEMA_PATH).expect("load schema");
    let value: Value = serde_json::from_str(&schema).expect("parse schema");
    let validator = Validator::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&value)
        .expect("build validator");
    validator
}

#[test]
fn llm_judge_success() {
    let claude = MockBackend::ok("claude", "claude answer");
    let gemini = MockBackend::ok("gemini", "gemini answer");
    let judge = MockBackend::ok(
        "reviewer",
        r#"{"chosen_index": 1, "reason": "Gemini is better"}"#,
    );
    let backends: Vec<Arc<dyn Backend>> = vec![claude.clone(), gemini.clone(), judge.clone()];

    let strategy = ParallelFanOut::new(
        vec![TargetSpec::new("claude"), TargetSpec::new("gemini")],
        2,
        "judge candidates",
        Aggregator::llm_judge("reviewer", "{{ candidates | length }} candidates", false),
    );

    let out = run_test(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
    assert_eq!(out.strategy, StrategyKind::Parallel);
    assert!(out.verify.as_ref().is_some_and(|verify| {
        verify.status == VerifyStatus::Pass && verify.hook == Some("LLMJudge".to_string())
    }));
    validate_with_schema(&out);

    assert_eq!(claude.calls(), 1);
    assert_eq!(gemini.calls(), 1);
    assert_eq!(judge.calls(), 1);
}

#[test]
fn llm_judge_family_overlap_refused() {
    let loker_anthropic = MockBackend::ok("loker_review_anthropic", "judge unavailable");
    let anthropic_candidate = MockBackend::ok("claude", "candidate a");
    let backends: Vec<Arc<dyn Backend>> =
        vec![anthropic_candidate.clone(), loker_anthropic.clone()];

    let strategy = ParallelFanOut::new(
        vec![TargetSpec::new("claude")],
        1,
        "x",
        Aggregator::llm_judge("loker_review_anthropic", "{x}", true),
    );

    let err = run_test(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
    match err {
        StrategyError::Phase(PhaseError::FamilyOverlap { .. }) => {}
        other => panic!("expected phase family overlap, got {other:?}"),
    }
}

#[test]
fn llm_judge_backend_error_maps_to_judge_unavailable() {
    let claude = MockBackend::ok("claude", "candidate");
    let judge = MockBackend::fail("reviewer", || BackendError::Network {
        message: "judge down".to_string(),
    });
    let backends: Vec<Arc<dyn Backend>> = vec![claude.clone(), judge.clone()];

    let strategy = ParallelFanOut::new(
        vec![TargetSpec::new("claude")],
        1,
        "x",
        Aggregator::llm_judge("reviewer", "{x}", true),
    );

    let err = run_test(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
    match err {
        StrategyError::Phase(PhaseError::JudgeUnavailable { detail }) => {
            assert!(detail.contains("judge down"));
        }
        other => panic!("expected judge unavailable, got {other:?}"),
    }
}

#[test]
fn llm_judge_malformed_json() {
    let claude = MockBackend::ok("claude", "candidate");
    let judge = MockBackend::ok("reviewer", "not json");
    let backends: Vec<Arc<dyn Backend>> = vec![claude.clone(), judge.clone()];

    let strategy = ParallelFanOut::new(
        vec![TargetSpec::new("claude")],
        1,
        "x",
        Aggregator::llm_judge("reviewer", "{x}", true),
    );

    let err = run_test(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
    match err {
        StrategyError::Phase(PhaseError::AggregatorContract { message }) => {
            assert!(message.contains("JSON parse error"));
        }
        other => panic!("expected aggregator contract, got {other:?}"),
    }
}

#[test]
fn llm_judge_family_overlap_opt_out() {
    let claude = MockBackend::ok("claude", "candidate");
    let judge = MockBackend::ok(
        "loker_review_anthropic",
        r#"{"chosen_index": 0, "reason": "fallback ok"}"#,
    );
    let backends: Vec<Arc<dyn Backend>> = vec![claude.clone(), judge.clone()];

    let strategy = ParallelFanOut::new(
        vec![TargetSpec::new("claude")],
        1,
        "x",
        Aggregator::llm_judge("loker_review_anthropic", "{x}", false),
    );

    let out = run_test(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
    assert!(out.verify.as_ref().is_some_and(|verify| {
        verify.status == VerifyStatus::Pass && verify.hook == Some("LLMJudge".to_string())
    }));
    assert_eq!(judge.calls(), 1);
}

#[test]
fn llm_judge_aggregation_snapshot() {
    let judge = MockBackend::ok(
        "judge",
        r#"{"chosen_index": 1, "reason": "Gemini is best"}"#,
    );
    let backends: Vec<Arc<dyn Backend>> = vec![judge.clone()];

    let candidates = vec![
        BranchSuccess {
            backend_id: "claude".into(),
            family: "anthropic".into(),
            index: 1,
            output: "Claude draft".into(),
        },
        BranchSuccess {
            backend_id: "gemini".into(),
            family: "google".into(),
            index: 2,
            output: "Gemini draft".into(),
        },
    ];

    let artifact = run_test(aggregate_llm_judge(
        &candidates,
        "judge",
        "{{ candidates | length }} candidates. {{ candidates[1].backend_id }}",
        false,
        &backends,
        &ctx(),
    ))
    .unwrap();

    assert_snapshot!(artifact.text, @r#"
Gemini draft

<!-- loker: LLMJudge chose candidate 2 (gemini) -->
Gemini is best
"#);
}
