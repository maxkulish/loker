//! Integration tests for `Strategy::ParallelFanOut` (CLO-259).
//!
//! Pin the AC contract: dispatch N backends concurrently, short-circuit on
//! min_responses, tolerate partial failure, record per-branch outcomes in
//! completion order, and validate against the parallel phase-result schema.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use jsonschema::Validator;
use serde_json::Value;

use loker::aggregator::Aggregator;
use loker::backend::{Backend, BackendError, QueryOutput, TokenUsage};
use loker::strategy::parallel_fanout::{ParallelFanOut, TargetSpec};
use loker::strategy::{
    FinishReason, PhaseContext, Prompt, Strategy, StrategyError, StrategyKind, SCHEMA_VERSION,
};

const SCHEMA_PATH: &str = "docs/schemas/phase_result_parallel.schema.json";

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
                .with_model(Some("mock-1"))
                .with_usage(Some(TokenUsage {
                    prompt_tokens: 7,
                    completion_tokens: 11,
                    total_tokens: 18,
                })))
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
    PhaseContext::new("review", uuid::Uuid::new_v4())
}

fn run<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Runtime::new().unwrap().block_on(f)
}

#[test]
fn happy_path_all_targets_succeed() {
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

    assert_eq!(out.schema_version, SCHEMA_VERSION);
    assert_eq!(out.strategy, StrategyKind::Parallel);
    assert_eq!(out.attempts.len(), 2);
    assert!(out
        .attempts
        .iter()
        .all(|a| a.finish_reasons == vec![FinishReason::Stop]));
    assert_eq!(a.calls(), 1);
    assert_eq!(b.calls(), 1);
}

#[test]
fn one_fails_min_responses_still_satisfied() {
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

    assert_eq!(out.strategy, StrategyKind::Parallel);
    // Short-circuit may drop the failing future; attempts is 1 or 2.
    assert!(
        out.attempts.len() >= 1 && out.attempts.len() <= 2,
        "expected 1-2 attempts, got {}",
        out.attempts.len()
    );
    let ok_count = out
        .attempts
        .iter()
        .filter(|a| a.finish_reasons == vec![FinishReason::Stop])
        .count();
    assert_eq!(ok_count, 1);
}

#[test]
fn too_many_failures_returns_floor_violation() {
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
fn fast_targets_cancel_slow() {
    let fast1 = MockBackend::ok("fast1", "fast1-out");
    let fast2 = MockBackend::ok("fast2", "fast2-out");
    let slow = MockBackend::slow("slow", "slow-out", 5000);
    let backends: Vec<Arc<dyn Backend>> = vec![fast1.clone(), fast2.clone(), slow.clone()];
    let strategy = ParallelFanOut::new(
        vec![
            TargetSpec::new("fast1"),
            TargetSpec::new("fast2"),
            TargetSpec::new("slow"),
        ],
        2,
        "render-me",
        Aggregator::concat("## {backend_id}"),
    );

    let start = std::time::Instant::now();
    let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(out.attempts.len(), 2);
    assert!(
        elapsed < Duration::from_secs(2),
        "expected fast return, took {:?}",
        elapsed
    );
    assert_eq!(fast1.calls(), 1);
    assert_eq!(fast2.calls(), 1);
    // slow may have been polled but not awaited to completion
}

#[test]
fn outcomes_contain_all_backends() {
    let slow = MockBackend::slow("slow", "slow-out", 50);
    let fast = MockBackend::ok("fast", "fast-out");
    let backends: Vec<Arc<dyn Backend>> = vec![slow.clone(), fast.clone()];
    let strategy = ParallelFanOut::new(
        vec![TargetSpec::new("slow"), TargetSpec::new("fast")],
        2,
        "render-me",
        Aggregator::concat("## {backend_id}"),
    );

    let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();

    assert_eq!(out.attempts.len(), 2);
    let names: Vec<&str> = out.attempts.iter().map(|a| a.backend.as_str()).collect();
    assert!(names.contains(&"slow"));
    assert!(names.contains(&"fast"));
}

#[test]
fn phase_result_validates_against_parallel_schema() {
    let a = MockBackend::ok("a", "out-a");
    let b = MockBackend::ok("b", "out-b");
    let backends: Vec<Arc<dyn Backend>> = vec![a, b];
    let strategy = ParallelFanOut::new(
        vec![TargetSpec::new("a"), TargetSpec::new("b")],
        2,
        "render-me",
        Aggregator::concat("## {backend_id}"),
    );

    let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
    let json = serde_json::to_value(&out).expect("serialize");
    assert_eq!(json["aggregator"], "concat");
    assert!(json["aggregate_output_path"]
        .as_str()
        .is_some_and(|p| !p.is_empty()));

    let schema = load_schema();
    if let Err(e) = schema.validate(&json) {
        panic!(
            "ParallelFanOut output failed schema validation: {}\npayload: {}",
            e,
            serde_json::to_string_pretty(&json).unwrap()
        );
    }
}

#[test]
fn floor_violation_payload_validates_against_schema() {
    let a = MockBackend::ok("a", "out-a");
    let b = MockBackend::fail("b", || BackendError::Network {
        message: "boom".into(),
    });
    let backends: Vec<Arc<dyn Backend>> = vec![a, b];
    let strategy = ParallelFanOut::new(
        vec![TargetSpec::new("a"), TargetSpec::new("b")],
        2,
        "render-me",
        Aggregator::concat("## {backend_id}"),
    );

    let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
    let output = match err {
        StrategyError::FloorViolation { output, .. } => output,
        other => panic!("expected FloorViolation, got {other:?}"),
    };

    let json = serde_json::to_value(&*output).unwrap();
    let schema = load_schema();
    if let Err(e) = schema.validate(&json) {
        panic!(
            "FloorViolation output failed schema validation: {}\npayload: {}",
            e,
            serde_json::to_string_pretty(&json).unwrap()
        );
    }
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
