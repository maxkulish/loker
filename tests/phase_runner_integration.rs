use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use loker::backend::{Backend, BackendCapabilities, BackendError, QueryOutput};
use loker::manifest::Manifest;
use loker::strategy::{PhaseContext, Prompt};
use loker::{PhaseConfig, PhaseInputs, PhaseRunner};

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

    assert_eq!(std::fs::read_to_string(&outcome.artefact_path).unwrap(), "canonical design");
    assert!(tmp.path().join("markers/design.started.0").is_file());
    assert!(tmp.path().join("markers/design.completed").is_file());

    let manifest = Manifest::load(&tmp.path().join("manifest.json")).unwrap();
    assert_eq!(manifest.entries.len(), 1);
    assert_eq!(manifest.entries[0].name, "design.md");
}
