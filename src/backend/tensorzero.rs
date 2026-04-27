//! TensorZero backend - HTTP gateway via the `genai` crate (M1).
//!
//! Routes chat calls through a TensorZero gateway that exposes an
//! OpenAI-compatible Chat Completions surface at
//! `POST {gateway}/openai/v1/chat/completions`. Authenticates via
//! `Authorization: Bearer <token>`; `Content-Type: application/json`. The
//! `model` field on the wire carries the configured TensorZero function name
//! (e.g. `tensorzero::function_name::loker_d1_openai`).
//!
//! 5xx responses from the gateway frequently wrap upstream errors (a 401 from
//! Anthropic, a 429 from OpenAI, ...). We body-inspect 502s to classify them
//! as `BackendError::Auth` / `BackendError::RateLimit` /
//! `BackendError::Network` rather than collapsing all 5xx into
//! `BackendError::Network`, which would trigger pointless retries on
//! credential failures.
//!
//! Path prefix, header shape, error-mapping verdict, and the function-name
//! family-suffix convention come from the D1 round-trip spike.
//!
//! **Source of truth**: `docs/spikes/2026-04-25-tensorzero-roundtrip.md`
//! (CLO-243).

use super::{Backend, BackendError, QueryOutput, TokenUsage};
use async_trait::async_trait;
use genai::adapter::AdapterKind;
use genai::chat::{ChatRequest, Usage};
use genai::resolver::{AuthData, Endpoint, ServiceTargetResolver};
use genai::{Client, ModelIden, ServiceTarget, WebConfig};
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct TensorZeroConfig {
    pub endpoint: String,
    pub model: String,
    pub api_key: Option<String>,
    pub timeout: Duration,
}

pub struct TensorZeroBackend {
    client: Client,
    model: String,
}

impl TensorZeroBackend {
    /// Static capability advertisement for `tensorzero`. Mirrors the body of
    /// `Backend::capabilities()` below and the entry in
    /// `crate::backend::capabilities_for_name`. Exposing it as a constant lets
    /// the pinning test assert the values without constructing a backend
    /// (which requires a `TensorZeroConfig` literal that has tripped CI's
    /// fresh stable-x86_64 rustc with a fuzzy E0422 import suggestion).
    pub(crate) const CAPABILITIES: super::BackendCapabilities = super::BackendCapabilities {
        tool_use: false,
        streaming: false,
        file_edit: true,
    };

    pub fn new(cfg: TensorZeroConfig) -> Result<Self, BackendError> {
        let endpoint = Endpoint::from_owned(normalize_endpoint(&cfg.endpoint));
        let auth = AuthData::from_single(cfg.api_key.clone().unwrap_or_default());

        let resolver = ServiceTargetResolver::from_resolver_fn(
            move |service_target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
                let ServiceTarget { model, .. } = service_target;
                let wire_model = canonicalize_wire_model(model.model_name.as_str());
                let model = ModelIden::new(AdapterKind::OpenAI, wire_model);
                Ok(ServiceTarget {
                    endpoint: endpoint.clone(),
                    auth: auth.clone(),
                    model,
                })
            },
        );

        let web_config = WebConfig::default().with_timeout(cfg.timeout);

        let client = Client::builder()
            .with_web_config(web_config)
            .with_service_target_resolver(resolver)
            .build();

        Ok(Self {
            client,
            model: cfg.model,
        })
    }
}

#[async_trait]
impl Backend for TensorZeroBackend {
    fn name(&self) -> &str {
        "tensorzero"
    }

    async fn query(
        &self,
        prompt: &str,
        _cwd: &Path,
        model: Option<&str>,
    ) -> Result<QueryOutput, BackendError> {
        let effective_model = model
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.model.clone());

        let chat_req = ChatRequest::from_user(prompt);
        let start = std::time::Instant::now();

        let response = self
            .client
            .exec_chat(&effective_model, chat_req, None)
            .await
            .map_err(|e| BackendError::from(e).with_elapsed(start.elapsed()))?;

        let elapsed = start.elapsed();

        let usage = token_usage_from_genai(&response.usage);
        let text = response.into_first_text().unwrap_or_default();

        Ok(QueryOutput::from_text(text, "tensorzero", elapsed)
            .with_model(Some(effective_model))
            .with_usage(usage))
    }

    fn is_available(&self) -> bool {
        // Server-side gateway. We don't probe synchronously; runtime call
        // surfaces connectivity errors as `BackendError::Network`.
        true
    }

    fn capabilities(&self) -> super::BackendCapabilities {
        // genai `exec_chat` (non-streaming), no tool param wired today; the
        // gateway forwards to capable models, but the call site is single-shot.
        // `file_edit=true` because Claude/Sonnet-class models behind the
        // gateway reliably emit JSON edit blocks.
        Self::CAPABILITIES
    }
}

fn token_usage_from_genai(usage: &Usage) -> Option<TokenUsage> {
    let p = usage.prompt_tokens?;
    let c = usage.completion_tokens?;
    Some(TokenUsage::new(p.max(0) as u32, c.max(0) as u32))
}

/// Normalize a configured endpoint URL to the canonical TensorZero
/// OpenAI-compat base, `…/openai/v1/`.
///
/// Per D1 spike (`docs/spikes/2026-04-25-tensorzero-roundtrip.md`) the gateway
/// exposes the OpenAI surface at `/openai/v1/chat/completions`, and `genai`
/// appends `chat/completions` to whatever `Endpoint` it is given. We therefore
/// guarantee the configured endpoint ends in exactly `/openai/v1/`.
fn normalize_endpoint(raw: &str) -> String {
    let trimmed = raw.trim_end_matches('/');
    if trimmed.ends_with("/openai/v1") {
        format!("{trimmed}/")
    } else {
        format!("{trimmed}/openai/v1/")
    }
}

/// Canonicalize the value sent on the wire as the OpenAI-compat `model` field.
///
/// Per the D1 spike (`docs/spikes/2026-04-25-tensorzero-roundtrip.md` §2 and
/// `tests/fixtures/tensorzero/openai_success_request.json`) the gateway expects
/// the literal value `tensorzero::function_name::<fn>` in the request body.
///
/// `genai` 0.6.0-beta.17 always splits a `ModelName` on the first `::` and
/// emits only the suffix as the wire `model` field (see
/// `adapter_shared::ChatRequest::to_request_payload` and
/// `ModelName::split_as_namespace_and_name`). To make the wire body match the
/// D1 fixture we therefore prepend a sacrificial `loker::` namespace: genai
/// strips it during payload assembly, leaving exactly the canonical
/// `tensorzero::function_name::<fn>` on the wire.
///
/// Accepted input forms (canonicalize before prepending the sacrificial
/// namespace):
/// - `tensorzero::function_name::<fn>`  -> already canonical, kept verbatim
/// - `tensorzero::model_name::<m>`      -> kept verbatim (model_name routing)
/// - `function_name::<fn>` / `model_name::<m>` -> wrapped with `tensorzero::`
/// - bare `<fn>`                         -> wrapped as
///   `tensorzero::function_name::<fn>`
///
/// Config canonicalization (rejecting unknown family suffixes, etc.) is not in
/// this task's scope (CLO-250 / CLO-251).
fn canonicalize_wire_model(model: &str) -> String {
    let canonical = if model.starts_with("tensorzero::") {
        model.to_string()
    } else if model.starts_with("function_name::") || model.starts_with("model_name::") {
        format!("tensorzero::{model}")
    } else if !model.contains("::") {
        format!("tensorzero::function_name::{model}")
    } else {
        model.to_string()
    };
    format!("loker::{canonical}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn canonicalize_wire_model_strips_to_canonical_on_wire() {
        // genai's OpenAI adapter strips the first `::` namespace from
        // `ModelName` at payload assembly time. The wire `model` field is
        // therefore `<input>` minus the leading `loker::`. Each case below
        // documents the expected wire form (post-strip).
        let strip_first_ns = |s: &str| -> String {
            s.find("::")
                .map(|i| s[i + 2..].to_string())
                .unwrap_or_else(|| s.to_string())
        };

        let cases = [
            (
                "loker_d1_openai",
                "tensorzero::function_name::loker_d1_openai",
            ),
            (
                "function_name::loker_d1_openai",
                "tensorzero::function_name::loker_d1_openai",
            ),
            (
                "model_name::loker_anthropic_haiku",
                "tensorzero::model_name::loker_anthropic_haiku",
            ),
            (
                "tensorzero::function_name::loker_d1_openai",
                "tensorzero::function_name::loker_d1_openai",
            ),
            (
                "tensorzero::model_name::loker_anthropic_haiku",
                "tensorzero::model_name::loker_anthropic_haiku",
            ),
        ];

        for (input, expected_wire) in cases {
            let resolved = canonicalize_wire_model(input);
            let wire = strip_first_ns(&resolved);
            assert_eq!(
                wire, expected_wire,
                "input={input:?} resolved={resolved:?} wire={wire:?}"
            );
        }
    }

    fn config_for(server: &MockServer) -> TensorZeroConfig {
        TensorZeroConfig {
            endpoint: server.uri(),
            model: "test-model".to_string(),
            api_key: Some("test-key".to_string()),
            timeout: Duration::from_secs(5),
        }
    }

    fn openai_success_body() -> serde_json::Value {
        json!({
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "created": 1_700_000_000,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "hello from tensorzero"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 11, "completion_tokens": 22, "total_tokens": 33}
        })
    }

    #[tokio::test]
    async fn returns_text_on_200_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .and(header("authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(openai_success_body()))
            .mount(&server)
            .await;

        let backend = TensorZeroBackend::new(config_for(&server)).expect("backend builds");
        let out = backend
            .query("hi", Path::new("."), None)
            .await
            .expect("200 succeeds");
        assert_eq!(out.stdout, "hello from tensorzero");
        assert_eq!(out.backend, "tensorzero");
        assert_eq!(out.model.as_deref(), Some("test-model"));
    }

    #[tokio::test]
    async fn maps_429_to_rate_limit_retryable() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .mount(&server)
            .await;

        let backend = TensorZeroBackend::new(config_for(&server)).unwrap();
        let err = backend
            .query("hi", Path::new("."), None)
            .await
            .expect_err("429 fails");
        assert!(
            matches!(err, BackendError::RateLimit { .. }),
            "expected RateLimit, got {err:?}"
        );
        assert!(err.is_retryable());
    }

    #[tokio::test]
    async fn maps_500_to_retryable_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("server error"))
            .mount(&server)
            .await;

        let backend = TensorZeroBackend::new(config_for(&server)).unwrap();
        let err = backend
            .query("hi", Path::new("."), None)
            .await
            .expect_err("500 fails");
        assert!(
            matches!(err, BackendError::Network { .. }),
            "expected Network for 5xx, got {err:?}"
        );
        assert!(err.is_retryable());
    }

    #[tokio::test]
    async fn maps_401_to_auth_not_retryable() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let backend = TensorZeroBackend::new(config_for(&server)).unwrap();
        let err = backend
            .query("hi", Path::new("."), None)
            .await
            .expect_err("401 fails");
        assert!(
            matches!(err, BackendError::Auth { .. }),
            "expected Auth, got {err:?}"
        );
        assert!(!err.is_retryable());
    }

    #[tokio::test]
    async fn maps_malformed_json_to_parse_error() {
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
            .query("hi", Path::new("."), None)
            .await
            .expect_err("malformed JSON fails");
        assert!(
            matches!(err, BackendError::Parse { .. }),
            "expected Parse, got {err:?}"
        );
        assert!(!err.is_retryable());
    }

    #[tokio::test]
    async fn maps_request_timeout_to_timeout_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(openai_success_body())
                    .set_delay(Duration::from_millis(800)),
            )
            .mount(&server)
            .await;

        let mut cfg = config_for(&server);
        cfg.timeout = Duration::from_millis(150);
        let backend = TensorZeroBackend::new(cfg).unwrap();
        let err = backend
            .query("hi", Path::new("."), None)
            .await
            .expect_err("slow response should time out");
        assert!(
            matches!(err, BackendError::Timeout { .. }),
            "expected Timeout, got {err:?}"
        );
        assert!(err.is_retryable());
    }

    #[test]
    fn capabilities_match_current_wiring() {
        // Pin the const that `Backend::capabilities()` returns, without
        // constructing `TensorZeroBackend`. Matches the bedrock pattern at
        // `bedrock.rs:269-282`.
        assert_eq!(
            TensorZeroBackend::CAPABILITIES,
            super::super::BackendCapabilities {
                tool_use: false,
                streaming: false,
                file_edit: true,
            }
        );
    }

    #[test]
    fn name_is_tensorzero() {
        let cfg = TensorZeroConfig {
            endpoint: "http://localhost:3000".to_string(),
            model: "test-model".to_string(),
            api_key: Some("k".to_string()),
            timeout: Duration::from_secs(1),
        };
        let backend = TensorZeroBackend::new(cfg).unwrap();
        assert_eq!(backend.name(), "tensorzero");
        assert!(backend.is_available());
    }

    #[test]
    fn normalize_endpoint_appends_when_missing() {
        assert_eq!(
            normalize_endpoint("http://localhost:3000"),
            "http://localhost:3000/openai/v1/"
        );
        assert_eq!(
            normalize_endpoint("http://localhost:3000/"),
            "http://localhost:3000/openai/v1/"
        );
    }

    #[test]
    fn normalize_endpoint_does_not_double_suffix() {
        assert_eq!(
            normalize_endpoint("http://gateway/openai/v1"),
            "http://gateway/openai/v1/"
        );
        assert_eq!(
            normalize_endpoint("http://gateway/openai/v1/"),
            "http://gateway/openai/v1/"
        );
    }

    #[tokio::test]
    async fn endpoint_normalizes_to_openai_v1_at_runtime() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(openai_success_body()))
            .mount(&server)
            .await;

        let backend = TensorZeroBackend::new(config_for(&server)).expect("backend builds");
        let out = backend
            .query("hi", Path::new("."), None)
            .await
            .expect("base-URL endpoint should normalize to /openai/v1/");
        assert_eq!(out.stdout, "hello from tensorzero");
    }

    #[tokio::test]
    async fn sends_bearer_auth_json_content_and_function_name_model() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(openai_success_body()))
            .mount(&server)
            .await;

        let mut cfg = config_for(&server);
        cfg.model = "tensorzero::function_name::loker_d1_openai".to_string();
        let backend = TensorZeroBackend::new(cfg).expect("backend builds");
        backend
            .query("hi", Path::new("."), None)
            .await
            .expect("200 path matches");

        let recorded = server.received_requests().await.expect("requests captured");
        assert_eq!(recorded.len(), 1, "expected exactly one request");
        let req = &recorded[0];

        let auth = req
            .headers
            .get("authorization")
            .expect("Authorization header present")
            .to_str()
            .expect("Authorization is ascii");
        assert!(
            auth.starts_with("Bearer "),
            "Authorization must be Bearer-shaped, got {auth:?}"
        );
        assert_eq!(auth, "Bearer test-key");

        let content_type = req
            .headers
            .get("content-type")
            .expect("Content-Type header present")
            .to_str()
            .expect("content-type is ascii");
        assert!(
            content_type.starts_with("application/json"),
            "Content-Type must be application/json, got {content_type:?}"
        );

        let body: serde_json::Value =
            serde_json::from_slice(&req.body).expect("request body is JSON");
        assert_eq!(
            body.get("model").and_then(|v| v.as_str()),
            Some("tensorzero::function_name::loker_d1_openai"),
            "model field must echo the configured function name verbatim; body was {body}"
        );
    }

    #[tokio::test]
    async fn maps_502_upstream_auth_to_auth_not_retryable() {
        let server = MockServer::start().await;
        let body =
            include_str!("../../tests/fixtures/tensorzero/anthropic_auth_failure_response.json");
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(502).set_body_string(body))
            .mount(&server)
            .await;

        let backend = TensorZeroBackend::new(config_for(&server)).unwrap();
        let err = backend
            .query("hi", Path::new("."), None)
            .await
            .expect_err("502 fails");
        assert!(
            matches!(err, BackendError::Auth { .. }),
            "expected Auth (upstream auth wrapped in 502), got {err:?}"
        );
        assert!(!err.is_retryable());
    }

    #[tokio::test]
    async fn maps_502_upstream_rate_limit_to_rate_limit_retryable() {
        let server = MockServer::start().await;
        let body = r#"{"error":{"message":"All variants failed: rate_limit exceeded upstream"}}"#;
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(502).set_body_string(body))
            .mount(&server)
            .await;

        let backend = TensorZeroBackend::new(config_for(&server)).unwrap();
        let err = backend
            .query("hi", Path::new("."), None)
            .await
            .expect_err("502 fails");
        assert!(
            matches!(err, BackendError::RateLimit { .. }),
            "expected RateLimit (upstream rate limit wrapped in 502), got {err:?}"
        );
        assert!(err.is_retryable());
    }

    #[tokio::test]
    async fn maps_502_generic_to_network_retryable() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(502).set_body_string("bad gateway"))
            .mount(&server)
            .await;

        let backend = TensorZeroBackend::new(config_for(&server)).unwrap();
        let err = backend
            .query("hi", Path::new("."), None)
            .await
            .expect_err("502 fails");
        assert!(
            matches!(err, BackendError::Network { .. }),
            "expected Network for generic 502, got {err:?}"
        );
        assert!(err.is_retryable());
    }

    #[tokio::test]
    async fn maps_404_unknown_function_to_config_not_retryable() {
        let server = MockServer::start().await;
        let body = include_str!("../../tests/fixtures/tensorzero/unknown_function_response.json");
        Mock::given(method("POST"))
            .and(path("/openai/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(404).set_body_string(body))
            .mount(&server)
            .await;

        let backend = TensorZeroBackend::new(config_for(&server)).unwrap();
        let err = backend
            .query("hi", Path::new("."), None)
            .await
            .expect_err("404 fails");
        assert!(
            matches!(err, BackendError::Config { .. }),
            "expected Config for unknown function 404, got {err:?}"
        );
        assert!(!err.is_retryable());
    }
}
