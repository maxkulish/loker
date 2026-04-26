//! Wiremock contract tests for the TensorZero backend.
//!
//! Pins the M1 backend's public-API behaviour against an in-process
//! HTTP mock so the contract is reasserted from outside `src/backend/tensorzero.rs`.
//! Inline `#[cfg(test)] mod tests` covers private helpers; this file covers
//! only the public `Backend` surface that downstream crates depend on.

use std::path::Path;
use std::time::Duration;

use loker::backend::{Backend, BackendError, TensorZeroBackend, TensorZeroConfig};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config_for(server: &MockServer) -> TensorZeroConfig {
    TensorZeroConfig {
        endpoint: server.uri(),
        model: "test-model".to_string(),
        api_key: Some("test-key".to_string()),
        timeout: Duration::from_secs(5),
    }
}

fn openai_success_body(text: &str) -> serde_json::Value {
    json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "created": 1_700_000_000,
        "model": "test-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": text},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
    })
}

#[tokio::test]
async fn success_200_returns_text() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/openai/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_success_body("hello")))
        .expect(1)
        .mount(&server)
        .await;

    let backend = TensorZeroBackend::new(config_for(&server)).expect("backend builds");
    let out = backend
        .query("ping", Path::new("."), None)
        .await
        .expect("200 succeeds");

    assert_eq!(out.stdout, "hello");
    assert_eq!(out.backend, "tensorzero");
    assert_eq!(out.model.as_deref(), Some("test-model"));
}

#[tokio::test]
async fn rate_limit_429_is_retryable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/openai/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
        .mount(&server)
        .await;

    let backend = TensorZeroBackend::new(config_for(&server)).unwrap();
    let err = backend
        .query("ping", Path::new("."), None)
        .await
        .expect_err("429 fails");

    assert!(
        matches!(err, BackendError::RateLimit { .. }),
        "expected RateLimit, got {err:?}"
    );
    assert!(err.is_retryable());
}

#[tokio::test]
async fn server_error_500_is_retryable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/openai/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal error"))
        .mount(&server)
        .await;

    let backend = TensorZeroBackend::new(config_for(&server)).unwrap();
    let err = backend
        .query("ping", Path::new("."), None)
        .await
        .expect_err("500 fails");

    assert!(
        matches!(err, BackendError::Network { .. }),
        "expected Network for generic 5xx, got {err:?}"
    );
    assert!(err.is_retryable());
}

#[tokio::test]
async fn auth_failure_401_is_not_retryable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/openai/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let backend = TensorZeroBackend::new(config_for(&server)).unwrap();
    let err = backend
        .query("ping", Path::new("."), None)
        .await
        .expect_err("401 fails");

    assert!(
        matches!(err, BackendError::Auth { .. }),
        "expected Auth, got {err:?}"
    );
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn auth_failure_403_is_not_retryable() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/openai/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
        .mount(&server)
        .await;

    let backend = TensorZeroBackend::new(config_for(&server)).unwrap();
    let err = backend
        .query("ping", Path::new("."), None)
        .await
        .expect_err("403 fails");

    assert!(
        matches!(err, BackendError::Auth { .. }),
        "expected Auth, got {err:?}"
    );
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn malformed_json_returns_parse_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/openai/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string("{not valid json"),
        )
        .mount(&server)
        .await;

    let backend = TensorZeroBackend::new(config_for(&server)).unwrap();
    let err = backend
        .query("ping", Path::new("."), None)
        .await
        .expect_err("malformed JSON fails");

    assert!(
        matches!(err, BackendError::Parse { .. }),
        "expected Parse, got {err:?}"
    );
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn request_timeout_returns_timeout_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/openai/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(openai_success_body("too late"))
                .set_delay(Duration::from_millis(800)),
        )
        .mount(&server)
        .await;

    let mut cfg = config_for(&server);
    cfg.timeout = Duration::from_millis(150);
    let backend = TensorZeroBackend::new(cfg).unwrap();
    let err = backend
        .query("ping", Path::new("."), None)
        .await
        .expect_err("slow response should time out");

    assert!(
        matches!(err, BackendError::Timeout { .. }),
        "expected Timeout, got {err:?}"
    );
    assert!(err.is_retryable());
}
