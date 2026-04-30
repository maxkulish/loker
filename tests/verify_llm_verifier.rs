use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use loker::backend::{Backend, BackendError, QueryOutput};
use loker::strategy::verify::LLMVerifier;
use loker::strategy::{VerifyContext, VerifyHook, VerifyResult};

struct MockBackend {
    name: String,
    calls: AtomicUsize,
    prompts: Mutex<Vec<String>>, // record rendered prompts for assertions
    response: Box<dyn Fn() -> Result<QueryOutput, BackendError> + Send + Sync>,
}

impl MockBackend {
    fn ok(name: &str, text: &str) -> Arc<Self> {
        let name = name.to_string();
        let response_text = text.to_string();
        Arc::new(Self {
            name: name.clone(),
            calls: AtomicUsize::new(0),
            prompts: Mutex::new(Vec::new()),
            response: Box::new(move || {
                Ok(QueryOutput::from_text(
                    response_text.clone(),
                    name.clone(),
                    Duration::from_millis(5),
                ))
            }),
        })
    }

    fn fail(name: &str, err: BackendError) -> Arc<Self> {
        let name = name.to_string();
        Arc::new(Self {
            name: name.clone(),
            calls: AtomicUsize::new(0),
            prompts: Mutex::new(Vec::new()),
            response: Box::new(move || Err(err.clone())),
        })
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn last_prompt(&self) -> Option<String> {
        self.prompts.lock().unwrap().last().cloned()
    }

    fn all_prompts(&self) -> Vec<String> {
        self.prompts.lock().unwrap().clone()
    }
}

#[async_trait]
impl Backend for MockBackend {
    fn name(&self) -> &str {
        &self.name
    }

    async fn query(
        &self,
        prompt: &str,
        _cwd: &Path,
        _model: Option<&str>,
    ) -> Result<QueryOutput, BackendError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.prompts.lock().unwrap().push(prompt.to_string());
        (self.response)()
    }

    fn is_available(&self) -> bool {
        true
    }
}

fn ctx_with_candidate(candidate: impl Into<String>) -> VerifyContext {
    let mut ctx = VerifyContext::from_query_output(&QueryOutput::from_text(
        candidate.into(),
        "source-backend",
        Duration::ZERO,
    ));
    ctx.structured = None;
    ctx
}

#[tokio::test]
async fn yes_is_pass() {
    let backend = MockBackend::ok("judge", "yes");
    let verifier = LLMVerifier::new("judge", backend.clone(), "{candidate}");

    let result = verifier
        .verify(&ctx_with_candidate("anything"))
        .await
        .unwrap();

    assert!(matches!(result, VerifyResult::Pass));
}

#[tokio::test]
async fn no_is_fail() {
    let backend = MockBackend::ok("judge", "no");
    let verifier = LLMVerifier::new("judge", backend.clone(), "{candidate}");

    let result = verifier
        .verify(&ctx_with_candidate("anything"))
        .await
        .unwrap();

    match result {
        VerifyResult::Fail { reason } => {
            assert_eq!(reason.summary, "no");
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[tokio::test]
async fn yes_variants_pass() {
    for response in ["Yes.", "YES\n", " yes - because..."] {
        let backend = MockBackend::ok("judge", response);
        let verifier = LLMVerifier::new("judge", backend.clone(), "{candidate}");
        let result = verifier
            .verify(&ctx_with_candidate("any output"))
            .await
            .unwrap();
        assert!(
            matches!(result, VerifyResult::Pass),
            "{response} should parse to pass"
        );
    }
}

#[tokio::test]
async fn unparseable_response_fails() {
    let backend = MockBackend::ok("judge", "maybe");
    let verifier = LLMVerifier::new("judge", backend.clone(), "{candidate}");

    let result = verifier
        .verify(&ctx_with_candidate("anything"))
        .await
        .unwrap();

    match result {
        VerifyResult::Fail { reason } => {
            assert!(reason.summary.contains("unparseable verifier response"));
        }
        other => panic!("expected Fail, got {other:?}"),
    }

    let backend = MockBackend::ok("judge", "");
    let verifier = LLMVerifier::new("judge", backend.clone(), "{candidate}");
    let result = verifier
        .verify(&ctx_with_candidate("anything"))
        .await
        .unwrap();
    match result {
        VerifyResult::Fail { reason } => {
            assert!(reason.summary.contains("unparseable verifier response"));
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[tokio::test]
async fn backend_error_is_fail() {
    let backend = MockBackend::fail(
        "judge",
        BackendError::Network {
            message: "service unavailable".into(),
        },
    );

    let verifier = LLMVerifier::new("judge", backend.clone(), "{candidate}");
    let result = verifier
        .verify(&ctx_with_candidate("anything"))
        .await
        .unwrap();

    match result {
        VerifyResult::Fail { reason } => {
            assert!(reason.summary.contains("backend error"));
            assert!(reason.stdout.contains("service unavailable"));
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[tokio::test]
async fn non_candidate_braces_passthrough() {
    let backend = MockBackend::ok("judge", "yes");
    let verifier = LLMVerifier::new(
        "judge",
        backend.clone(),
        "{candidate} and {raw} and literal {not-a-key}",
    )
    .with_param("raw", "left");

    let _ = verifier
        .verify(&ctx_with_candidate("candidate value"))
        .await
        .unwrap();

    let prompt = backend
        .last_prompt()
        .expect("backend should have been queried");
    assert_eq!(prompt, "candidate value and left and literal {not-a-key}");
}

#[tokio::test]
async fn forwards_system_prompt() {
    let backend = MockBackend::ok("judge", "yes");
    let verifier = LLMVerifier::new("judge", backend.clone(), "{candidate}")
        .with_system_prompt("SYSTEM: evaluate only the output text");

    let _ = verifier
        .verify(&ctx_with_candidate("candidate blob"))
        .await
        .unwrap();

    let prompt = backend
        .last_prompt()
        .expect("backend should have been queried");
    assert!(prompt.starts_with("SYSTEM: evaluate only the output text\n\n"));
    assert!(prompt.ends_with("candidate blob"));
}

#[tokio::test]
async fn candidate_substitution_and_prompt_params() {
    let backend = MockBackend::ok("judge", "yes");
    let verifier = LLMVerifier::new(
        "judge",
        backend.clone(),
        "Policy: {policy}\nCandidate: {candidate}",
    )
    .with_param("policy", "strict");

    let _ = verifier
        .verify(&ctx_with_candidate("actual output"))
        .await
        .unwrap();

    let prompt = backend
        .last_prompt()
        .expect("backend should have been queried");
    assert_eq!(prompt, "Policy: strict\nCandidate: actual output");
    assert_eq!(backend.calls(), 1);
    assert_eq!(backend.all_prompts().len(), 1);
}
