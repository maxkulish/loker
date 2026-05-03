use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use loker::backend::{Backend, BackendCapabilities, BackendError, QueryOutput};
use loker::manifest::Manifest;
use loker::strategy::verify::{VerifyContext, VerifyError, VerifyHook, VerifyResult};
use loker::strategy::{PhaseContext, Prompt, TargetSpec, Tier};
use loker::{AggregatorName, PhaseConfig, PhaseInputs, PhaseRunner, StrategyName, VerifyHookName};

#[derive(Debug)]
struct MockBackend {
    name: &'static str,
    output: &'static str,
}

struct StubVerifyHook {
    result: VerifyResult,
}

#[async_trait]
impl VerifyHook for StubVerifyHook {
    fn name(&self) -> &str {
        "StubVerifyHook"
    }

    async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
        Ok(self.result.clone())
    }
}

struct SequenceVerifyHook {
    results: Mutex<Vec<VerifyResult>>,
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

#[async_trait]
impl Backend for MockBackend {
    fn name(&self) -> &str {
        self.name
    }

    async fn query(
        &self,
        _prompt: &str,
        _cwd: &Path,
        _model: Option<&str>,
    ) -> Result<QueryOutput, BackendError> {
        Ok(QueryOutput::from_text(
            self.output.to_string(),
            self.name,
            Duration::from_millis(1),
        ))
    }

    fn is_available(&self) -> bool {
        true
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
    }
}

fn context(tmp: &tempfile::TempDir, phase: &str) -> PhaseContext {
    let mut ctx = PhaseContext::new(phase, uuid::Uuid::new_v4());
    ctx.cwd = tmp.path().to_path_buf();
    ctx
}

#[tokio::test]
async fn single_first_no_verify_emits_one_artefact_and_completed_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = PhaseConfig::single("design", "mock", "hello", "design.md");
    let backend: Arc<dyn Backend> = Arc::new(MockBackend {
        name: "mock",
        output: "canonical design",
    });
    let backends = vec![backend];

    let outcome = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "design"),
                verify: None,
                run_dir: tmp.path().to_path_buf(),
            },
        )
        .await
        .expect("phase run succeeds");

    assert_eq!(
        std::fs::read_to_string(&outcome.artefact_path).unwrap(),
        "canonical design"
    );
    assert!(tmp.path().join("markers/design.started.0").is_file());
    assert!(tmp.path().join("markers/design.completed").is_file());

    let manifest = Manifest::load(&tmp.path().join("manifest.json")).unwrap();
    assert_eq!(manifest.entries.len(), 1);
    assert_eq!(manifest.entries[0].name, "design.md");
}

#[tokio::test]
async fn phase_runner_parallel_concat_with_verifier_emits_completed_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = PhaseConfig::single("review", "unused", "hello", "review.md");
    cfg.strategy = StrategyName::Parallel;
    cfg.aggregator = AggregatorName::Concat;
    cfg.verify = VerifyHookName::RunCommand;
    cfg.targets = vec![TargetSpec::new("a"), TargetSpec::new("b")];
    cfg.min_responses = 2;

    let backends: Vec<Arc<dyn Backend>> = vec![
        Arc::new(MockBackend {
            name: "a",
            output: "A",
        }),
        Arc::new(MockBackend {
            name: "b",
            output: "B",
        }),
    ];
    let verify: Arc<dyn VerifyHook> = Arc::new(StubVerifyHook {
        result: VerifyResult::pass(),
    });

    let outcome = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "review"),
                verify: Some(verify),
                run_dir: tmp.path().to_path_buf(),
            },
        )
        .await
        .expect("parallel run succeeds");

    let text = std::fs::read_to_string(outcome.artefact_path).unwrap();
    assert!(text.contains("A"));
    assert!(text.contains("B"));
    assert!(tmp.path().join("markers/review.completed").is_file());
}

#[tokio::test]
async fn phase_runner_parallel_all_pass_collects_failures() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = PhaseConfig::single("review", "unused", "hello", "review.md");
    cfg.strategy = StrategyName::Parallel;
    cfg.aggregator = AggregatorName::AllPass;
    cfg.targets = vec![TargetSpec::new("a"), TargetSpec::new("missing")];
    cfg.min_responses = 1;

    let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(MockBackend {
        name: "a",
        output: "A",
    })];

    let err = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "review"),
                verify: None,
                run_dir: tmp.path().to_path_buf(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.error_class(), "strategy_failed");
    assert!(tmp.path().join("markers/review.failed").is_file());
}

#[tokio::test]
async fn phase_runner_retry_and_failure_escalating_recovers_then_terminal_failure_marks_failed() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = PhaseConfig::single("implement", "unused", "hello", "impl.md");
    cfg.strategy = StrategyName::EscalatingRetry;
    cfg.verify = VerifyHookName::LlmVerifier;
    cfg.rungs = vec![
        loker::PhaseRung::new(Tier::Cheap, "cheap"),
        loker::PhaseRung::new(Tier::Strong, "strong"),
    ];
    let backends: Vec<Arc<dyn Backend>> = vec![
        Arc::new(MockBackend {
            name: "cheap",
            output: "bad",
        }),
        Arc::new(MockBackend {
            name: "strong",
            output: "good",
        }),
    ];
    let verify: Arc<dyn VerifyHook> = Arc::new(SequenceVerifyHook {
        results: Mutex::new(vec![VerifyResult::fail("not yet"), VerifyResult::pass()]),
    });

    let outcome = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "implement"),
                verify: Some(verify),
                run_dir: tmp.path().to_path_buf(),
            },
        )
        .await
        .expect("escalating recovers");
    assert_eq!(
        std::fs::read_to_string(outcome.artefact_path).unwrap(),
        "good"
    );
    assert!(tmp.path().join("markers/implement.completed").is_file());

    let tmp = tempfile::tempdir().unwrap();
    let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(MockBackend {
        name: "cheap",
        output: "bad",
    })];
    let mut fail_cfg = cfg;
    fail_cfg.rungs = vec![loker::PhaseRung::new(Tier::Cheap, "cheap")];
    let verify: Arc<dyn VerifyHook> = Arc::new(SequenceVerifyHook {
        results: Mutex::new(vec![VerifyResult::fail("still bad")]),
    });
    let err = PhaseRunner::new()
        .run(
            &fail_cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "implement"),
                verify: Some(verify),
                run_dir: tmp.path().to_path_buf(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(err.error_class(), "strategy_failed");
    assert!(tmp.path().join("markers/implement.failed").is_file());
}
