//! Dispatcher-level coverage for `create_backend("tensorzero", ...)`.
//!
//! `tests/tensorzero_backend.rs` pins the public TensorZero backend wire
//! contract via direct construction. This file proves the runtime dispatcher
//! takes the same path instead of falling through to `Unknown backend`.

use std::path::Path;
use std::time::Duration;

use loker::backend::{create_backend, BackendConfig, RetryPolicy};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn zero_retry_policy() -> RetryPolicy {
    RetryPolicy {
        max_retries: 0,
        base_delay: Duration::from_millis(1),
        max_delay: Duration::from_millis(1),
    }
}

fn config_for(server: &MockServer) -> BackendConfig {
    BackendConfig {
        enabled: true,
        command: Some(server.uri()),
        args: vec![],
        skip_lines: 0,
        api_key_env: None,
        model: Some("test-model".to_string()),
        timeout: Some(5),
        max_retries: None,
        retry_delay_ms: None,
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
async fn create_backend_tensorzero_queries_wiremock_gateway() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/openai/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(openai_success_body("hello")))
        .expect(1)
        .mount(&server)
        .await;

    let config = config_for(&server);
    let backend = create_backend("tensorzero", &config, zero_retry_policy())
        .expect("tensorzero dispatcher arm builds backend");

    assert_eq!(backend.name(), "tensorzero");
    assert!(backend.is_available());

    let out = backend
        .query("ping", Path::new("."), None)
        .await
        .expect("dispatcher-built backend queries successfully");

    assert_eq!(out.stdout, "hello");
    assert_eq!(out.backend, "tensorzero");
    assert_eq!(out.model.as_deref(), Some("test-model"));
}
