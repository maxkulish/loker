use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use loker::backend::{Backend, BackendCapabilities, BackendError, QueryOutput};
use loker::manifest::Kind;
use loker::strategy::verify::{HumanDecision, HumanResponse, HumanSeverity, HumanVerifierConfig};
use loker::strategy::{PhaseContext, Prompt};
use loker::{PhaseConfig, PhaseInputs, PhaseRunner, VerifyHookName};

#[derive(Debug)]
struct MockBackend {
    name: &'static str,
    output: &'static str,
}

#[async_trait]
impl Backend for MockBackend {
    fn name(&self) -> &str {
        self.name
    }

    async fn query(
        &self,
        _prompt: &str,
        _cwd: &std::path::Path,
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

fn write_response(path: &PathBuf, decision: HumanDecision) {
    let response = HumanResponse {
        schema_version: 1,
        phase: "review".into(),
        claimed_by: "human".into(),
        decided_at: "2026-05-04T00:00:00Z".into(),
        decision,
        global_comment: None,
        inline_comments_path: None,
    };

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, serde_json::to_vec_pretty(&response).unwrap()).unwrap();
}

#[tokio::test]
async fn phase_human_verifier_blocks_until_response() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = PhaseConfig::single("review", "mock", "prompt", "review.md");
    cfg.verify = VerifyHookName::HumanVerifier(HumanVerifierConfig {
        run_dir: tmp.path().to_path_buf(),
        run_id: "run-1".into(),
        workflow: "wf".into(),
        phase: "review".into(),
        artefact_name: "review.md".into(),
        artefact_kind: Kind::ReviewMd,
        severity: HumanSeverity::Medium,
        decision_options: vec![HumanDecision::Approve, HumanDecision::Reject],
    });

    let backend: Arc<dyn Backend> = Arc::new(MockBackend {
        name: "mock",
        output: "candidate output",
    });
    let backends = vec![backend];

    let err = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "review"),
                verify: None,
                run_dir: tmp.path().to_path_buf(),
                trace: None,
            },
            0,
        )
        .await
        .unwrap_err();

    assert_eq!(err.error_class(), "verify_failed");
    assert!(tmp.path().join("pending").join("review.json").is_file());

    let pending_text = fs::read_to_string(tmp.path().join("pending/review.json")).unwrap();
    let pending: serde_json::Value = serde_json::from_str(&pending_text).unwrap();
    assert_eq!(pending["artefact"]["path"], "review.md");
    assert_eq!(pending["artefact"]["kind"], "text/markdown");
}

#[tokio::test]
async fn phase_human_verifier_ignores_stale_response_after_consume() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = PhaseConfig::single("review", "mock", "prompt", "review.md");
    cfg.verify = VerifyHookName::HumanVerifier(HumanVerifierConfig {
        run_dir: tmp.path().to_path_buf(),
        run_id: "run-1".into(),
        workflow: "wf".into(),
        phase: "review".into(),
        artefact_name: "review.md".into(),
        artefact_kind: Kind::ReviewMd,
        severity: HumanSeverity::Medium,
        decision_options: vec![HumanDecision::Approve, HumanDecision::Reject],
    });

    let response_path = tmp.path().join("responses").join("review.json");
    write_response(&response_path, HumanDecision::Approve);

    let backend: Arc<dyn Backend> = Arc::new(MockBackend {
        name: "mock",
        output: "candidate output",
    });
    let backends = vec![backend];

    let outcome = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "review"),
                verify: None,
                run_dir: tmp.path().to_path_buf(),
                trace: None,
            },
            0,
        )
        .await
        .expect("phase run passes with approval");

    assert!(outcome.artefact_path.is_file());
    assert!(!response_path.exists());

    let consumed = tmp
        .path()
        .join("responses")
        .read_dir()
        .unwrap()
        .filter_map(|entry| entry.ok())
        .any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .contains("review.json.handled.")
        });
    assert!(consumed);

    let err = PhaseRunner::new()
        .run(
            &cfg,
            PhaseInputs {
                backends: &backends,
                prompt: Prompt::new(),
                ctx: context(&tmp, "review"),
                verify: None,
                run_dir: tmp.path().to_path_buf(),
                trace: None,
            },
            1,
        )
        .await
        .unwrap_err();

    assert_eq!(err.error_class(), "verify_failed");
    assert!(tmp.path().join("pending").join("review.json").is_file());
}
