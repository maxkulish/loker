use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use loker::backend::{Backend, BackendCapabilities, BackendError, QueryOutput, TokenUsage};
use loker::manifest::Kind;
use loker::strategy::verify::{VerifyContext, VerifyError, VerifyHook, VerifyResult};
use loker::strategy::{PhaseContext, Prompt, TargetSpec, Tier};
use loker::{
    AggregatorName, InMemorySink, PhaseConfig, PhaseInputs, PhaseRunner, StrategyName, TraceWriter,
    VerifyHookName,
};

#[derive(Debug)]
struct MockBackendWithUsage {
    name: String,
    output: String,
    usage: Option<TokenUsage>,
}

#[async_trait]
impl Backend for MockBackendWithUsage {
    fn name(&self) -> &str {
        &self.name
    }

    async fn query(
        &self,
        _prompt: &str,
        _cwd: &Path,
        _model: Option<&str>,
    ) -> Result<QueryOutput, BackendError> {
        let mut out =
            QueryOutput::from_text(self.output.clone(), &self.name, Duration::from_millis(10))
                .with_model(Some("mock-model"));
        if let Some(ref u) = self.usage {
            out = out.with_usage(Some(u.clone()));
        }
        Ok(out)
    }

    fn is_available(&self) -> bool {
        true
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
    }
}

struct FailingVerifyHook;

#[async_trait]
impl VerifyHook for FailingVerifyHook {
    fn name(&self) -> &str {
        "FailingVerifyHook"
    }

    async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
        Ok(VerifyResult::fail("verification failed"))
    }
}

fn context(tmp: &tempfile::TempDir, phase: &str) -> PhaseContext {
    let mut ctx = PhaseContext::new(phase, uuid::Uuid::new_v4());
    ctx.cwd = tmp.path().to_path_buf();
    ctx
}

#[tokio::test]
async fn single_phase_four_spans_parented() {
    let tmp = tempfile::tempdir().unwrap();
    let sink = Arc::new(InMemorySink::new());
    let cfg = PhaseConfig::single("design", "mock", "hello", "design.md");
    let backend: Arc<dyn Backend> = Arc::new(MockBackendWithUsage {
        name: "mock".to_string(),
        output: "design output".to_string(),
        usage: Some(TokenUsage::new(10, 20)),
    });
    let backends = vec![backend];

    let _ = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "design"),
                verify: None,
                run_dir: tmp.path().to_path_buf(),
                trace: Some(sink.as_ref()),
            },
            0,
        )
        .await
        .expect("phase run succeeds");

    let spans = sink.spans();
    assert_eq!(
        spans.len(),
        4,
        "expected phase + backend + aggregator + finished spans"
    );
    let phase_span = &spans[0];
    let backend_span = &spans[1];
    assert_eq!(phase_span["name"], "phase.design");
    assert_eq!(backend_span["name"], "backend.mock");
    assert_eq!(backend_span["parent_span_id"], phase_span["span_id"]);
}

#[tokio::test]
async fn parallel_three_replicas_six_spans() {
    let tmp = tempfile::tempdir().unwrap();
    let sink = Arc::new(InMemorySink::new());
    let mut cfg = PhaseConfig::single("review", "unused", "hello", "review.md");
    cfg.strategy = StrategyName::Parallel;
    cfg.producer = loker::manifest::Producer::Parallel;
    cfg.artefact_kind = Kind::ReviewMd;
    cfg.aggregator = AggregatorName::Concat;
    cfg.targets = vec![
        TargetSpec::new("b0"),
        TargetSpec::new("b1"),
        TargetSpec::new("b2"),
    ];
    cfg.min_responses = 3;

    let backends: Vec<Arc<dyn Backend>> = vec![
        Arc::new(MockBackendWithUsage {
            name: "b0".to_string(),
            output: "A".to_string(),
            usage: Some(TokenUsage::new(1, 2)),
        }),
        Arc::new(MockBackendWithUsage {
            name: "b1".to_string(),
            output: "B".to_string(),
            usage: Some(TokenUsage::new(3, 4)),
        }),
        Arc::new(MockBackendWithUsage {
            name: "b2".to_string(),
            output: "C".to_string(),
            usage: Some(TokenUsage::new(5, 6)),
        }),
    ];

    let _ = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "review"),
                verify: None,
                run_dir: tmp.path().to_path_buf(),
                trace: Some(sink.as_ref()),
            },
            0,
        )
        .await
        .expect("phase run succeeds");

    let spans = sink.spans();
    // phase + 3 backend + aggregator + finished = 6
    assert_eq!(
        spans.len(),
        6,
        "expected phase + 3 backend + aggregator + finished spans"
    );
    let phase_span = &spans[0];
    let backend_spans: Vec<_> = spans
        .iter()
        .filter(|s| s["name"].as_str().unwrap_or("").starts_with("backend."))
        .collect();
    assert_eq!(backend_spans.len(), 3);
    for b in &backend_spans {
        assert_eq!(b["parent_span_id"], phase_span["span_id"]);
    }
    let agg_span = spans
        .iter()
        .find(|s| s["name"] == "aggregator.concat")
        .expect("aggregator span exists");
    assert_eq!(agg_span["parent_span_id"], phase_span["span_id"]);
}

#[tokio::test]
async fn escalating_retry_attempt_spans() {
    let tmp = tempfile::tempdir().unwrap();
    let sink = Arc::new(InMemorySink::new());
    let mut cfg = PhaseConfig::single("test", "cheap", "hello", "test.md");
    cfg.strategy = StrategyName::EscalatingRetry;
    cfg.producer = loker::manifest::Producer::Escalating;
    cfg.verify = VerifyHookName::RunCommand;
    cfg.rungs = vec![
        loker::PhaseRung::new(Tier::Cheap, "cheap"),
        loker::PhaseRung::new(Tier::Strong, "strong"),
    ];

    let backends: Vec<Arc<dyn Backend>> = vec![
        Arc::new(MockBackendWithUsage {
            name: "cheap".to_string(),
            output: "cheap output".to_string(),
            usage: Some(TokenUsage::new(5, 5)),
        }),
        Arc::new(MockBackendWithUsage {
            name: "strong".to_string(),
            output: "strong output".to_string(),
            usage: Some(TokenUsage::new(10, 10)),
        }),
    ];

    let verify: Arc<dyn VerifyHook> = Arc::new(SequenceVerifyHook {
        results: std::sync::Mutex::new(vec![
            VerifyResult::fail("first attempt fails"),
            VerifyResult::pass(),
            VerifyResult::pass(),
        ]),
    });

    let _ = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "test"),
                verify: Some(verify),
                run_dir: tmp.path().to_path_buf(),
                trace: Some(sink.as_ref()),
            },
            0,
        )
        .await
        .expect("phase run succeeds");

    let spans = sink.spans();
    // phase + 2 backend + aggregator + verify + finished = 6
    assert_eq!(
        spans.len(),
        6,
        "expected phase + 2 backend + aggregator + verify + finished spans"
    );
    let phase_span = &spans[0];
    let backend_spans: Vec<_> = spans
        .iter()
        .filter(|s| s["name"].as_str().unwrap_or("").starts_with("backend."))
        .collect();
    assert_eq!(backend_spans.len(), 2);
    for b in &backend_spans {
        assert_eq!(b["parent_span_id"], phase_span["span_id"]);
    }
    // Attempt indices
    assert_eq!(backend_spans[0]["loker.attempt"], 0);
    assert_eq!(backend_spans[1]["loker.attempt"], 1);
}

#[tokio::test]
async fn verify_failure_outcome() {
    let tmp = tempfile::tempdir().unwrap();
    let sink = Arc::new(InMemorySink::new());
    let mut cfg = PhaseConfig::single("test", "mock", "hello", "test.md");
    cfg.verify = VerifyHookName::RunCommand;

    let backend: Arc<dyn Backend> = Arc::new(MockBackendWithUsage {
        name: "mock".to_string(),
        output: "output".to_string(),
        usage: None,
    });
    let backends = vec![backend];
    let verify: Arc<dyn VerifyHook> = Arc::new(FailingVerifyHook);

    let _ = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "test"),
                verify: Some(verify),
                run_dir: tmp.path().to_path_buf(),
                trace: Some(sink.as_ref()),
            },
            0,
        )
        .await;

    let spans = sink.spans();
    let error_span = spans
        .iter()
        .find(|s| s["name"].as_str().unwrap_or("").ends_with(".error"))
        .expect("error span exists");
    assert_eq!(error_span["error.kind"], "verify_failed");

    let verify_span = spans
        .iter()
        .find(|s| s["name"].as_str().unwrap_or("").starts_with("verify."))
        .expect("verify span exists");
    assert!(!verify_span
        .as_object()
        .unwrap()
        .contains_key("gen_ai.usage.input_tokens"));
    assert!(!verify_span
        .as_object()
        .unwrap()
        .contains_key("gen_ai.usage.output_tokens"));
}

#[tokio::test]
async fn token_counts_match_mock() {
    let tmp = tempfile::tempdir().unwrap();
    let sink = Arc::new(InMemorySink::new());
    let cfg = PhaseConfig::single("test", "mock", "hello", "test.md");
    let backend: Arc<dyn Backend> = Arc::new(MockBackendWithUsage {
        name: "mock".to_string(),
        output: "output".to_string(),
        usage: Some(TokenUsage::new(42, 84)),
    });
    let backends = vec![backend];

    let _ = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "test"),
                verify: None,
                run_dir: tmp.path().to_path_buf(),
                trace: Some(sink.as_ref()),
            },
            0,
        )
        .await
        .expect("phase run succeeds");

    let spans = sink.spans();
    let backend_span = spans
        .iter()
        .find(|s| s["name"].as_str().unwrap_or("").starts_with("backend."))
        .expect("backend span exists");
    assert_eq!(backend_span["gen_ai.usage.input_tokens"], 42);
    assert_eq!(backend_span["gen_ai.usage.output_tokens"], 84);
}

#[tokio::test]
async fn file_valid_jsonl() {
    let tmp = tempfile::tempdir().unwrap();
    let trace_path = tmp.path().join("trace.jsonl");
    let writer = TraceWriter::new(&trace_path, true).unwrap();
    let cfg = PhaseConfig::single("test", "mock", "hello", "test.md");
    let backend: Arc<dyn Backend> = Arc::new(MockBackendWithUsage {
        name: "mock".to_string(),
        output: "output".to_string(),
        usage: Some(TokenUsage::new(1, 2)),
    });
    let backends = vec![backend];

    let _ = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "test"),
                verify: None,
                run_dir: tmp.path().to_path_buf(),
                trace: Some(&writer),
            },
            0,
        )
        .await
        .expect("phase run succeeds");

    let content = std::fs::read_to_string(&trace_path).unwrap();
    let lines: Vec<&str> = content.lines().collect();
    assert!(!lines.is_empty());
    for line in &lines {
        let parsed: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
        assert!(parsed.is_object());
    }
}

#[tokio::test]
async fn schema_validation() {
    let tmp = tempfile::tempdir().unwrap();
    let trace_path = tmp.path().join("trace.jsonl");
    let writer = TraceWriter::new(&trace_path, true).unwrap();
    let cfg = PhaseConfig::single("test", "mock", "hello", "test.md");
    let backend: Arc<dyn Backend> = Arc::new(MockBackendWithUsage {
        name: "mock".to_string(),
        output: "output".to_string(),
        usage: Some(TokenUsage::new(1, 2)),
    });
    let backends = vec![backend];

    let _ = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "test"),
                verify: None,
                run_dir: tmp.path().to_path_buf(),
                trace: Some(&writer),
            },
            0,
        )
        .await
        .expect("phase run succeeds");

    let content = std::fs::read_to_string(&trace_path).unwrap();
    let schema_json = std::fs::read_to_string("docs/schemas/trace_span.schema.json").unwrap();
    let schema: serde_json::Value = serde_json::from_str(&schema_json).unwrap();
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");

    for line in content.lines() {
        let value: serde_json::Value = serde_json::from_str(line).expect("valid JSON");
        let result = validator.validate(&value);
        assert!(
            result.is_ok(),
            "schema validation failed for line: {line}\n{err:?}",
            err = result.err()
        );
    }
}

#[derive(Debug)]
struct SequenceVerifyHook {
    results: std::sync::Mutex<Vec<VerifyResult>>,
}

#[async_trait]
impl VerifyHook for SequenceVerifyHook {
    fn name(&self) -> &str {
        "SequenceVerifyHook"
    }

    async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
        let mut results = self.results.lock().unwrap();
        Ok(results.remove(0))
    }
}
