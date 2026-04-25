//! TensorZero backend - HTTP gateway via the `genai` crate (M1).
//!
//! Routes chat calls through a TensorZero gateway that exposes an
//! OpenAI-compatible Chat Completions API. We pin all calls to the
//! configured endpoint and auth via a `ServiceTargetResolver`, using
//! `AdapterKind::OpenAI` so genai builds the standard
//! `POST {endpoint}/chat/completions` request.

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
    pub fn new(cfg: TensorZeroConfig) -> Result<Self, BackendError> {
        let endpoint_url = if cfg.endpoint.ends_with('/') {
            cfg.endpoint.clone()
        } else {
            format!("{}/", cfg.endpoint)
        };
        let endpoint = Endpoint::from_owned(endpoint_url);
        let auth = AuthData::from_single(cfg.api_key.clone().unwrap_or_default());

        let resolver = ServiceTargetResolver::from_resolver_fn(
            move |service_target: ServiceTarget| -> Result<ServiceTarget, genai::resolver::Error> {
                let ServiceTarget { model, .. } = service_target;
                let model = ModelIden::new(AdapterKind::OpenAI, model.model_name);
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
            .map_err(|e| map_genai_error(e, start.elapsed()))?;

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
}

fn token_usage_from_genai(usage: &Usage) -> Option<TokenUsage> {
    let p = usage.prompt_tokens?;
    let c = usage.completion_tokens?;
    Some(TokenUsage::new(p.max(0) as u32, c.max(0) as u32))
}

fn map_genai_error(err: genai::Error, elapsed: Duration) -> BackendError {
    match err {
        genai::Error::WebModelCall { webc_error, .. }
        | genai::Error::WebAdapterCall { webc_error, .. } => map_webc_error(webc_error, elapsed),
        genai::Error::ChatResponseGeneration { cause, .. } => BackendError::Parse {
            message: format!("TensorZero response parse error: {cause}"),
        },
        genai::Error::StreamParse { serde_error, .. } => BackendError::Parse {
            message: format!("TensorZero stream parse error: {serde_error}"),
        },
        genai::Error::Resolver { resolver_error, .. } => BackendError::Config {
            message: format!("TensorZero resolver error: {resolver_error}"),
        },
        genai::Error::RequiresApiKey { .. }
        | genai::Error::NoAuthData { .. }
        | genai::Error::NoAuthResolver { .. } => BackendError::Auth {
            message: format!("TensorZero auth missing: {err}"),
        },
        genai::Error::HttpError { status, body, .. } => map_status(status.as_u16(), body),
        other => BackendError::ExecutionFailed {
            message: format!("TensorZero call failed: {other}"),
            exit_code: None,
        },
    }
}

fn map_webc_error(err: genai::webc::Error, elapsed: Duration) -> BackendError {
    use genai::webc::Error as W;
    match err {
        W::ResponseFailedStatus { status, body, .. } => map_status(status.as_u16(), body),
        W::ResponseFailedInvalidJson { body, cause } => BackendError::Parse {
            message: format!("TensorZero invalid JSON response: {cause}; body: {body}"),
        },
        W::ResponseFailedNotJson { content_type, body } => BackendError::Parse {
            message: format!("TensorZero non-JSON response (content-type {content_type}): {body}"),
        },
        W::Reqwest(e) => {
            if e.is_timeout() {
                BackendError::Timeout {
                    message: format!("TensorZero request timed out: {e}"),
                    elapsed_ms: elapsed.as_millis() as u64,
                }
            } else if e.is_connect() {
                BackendError::Network {
                    message: format!("TensorZero connection failed: {e}"),
                }
            } else {
                BackendError::Network {
                    message: format!("TensorZero request failed: {e}"),
                }
            }
        }
        W::JsonValueExt(e) => BackendError::Parse {
            message: format!("TensorZero JSON value error: {e}"),
        },
    }
}

fn map_status(status: u16, body: String) -> BackendError {
    let msg = format!("TensorZero HTTP {status}: {body}");
    match status {
        401 | 403 => BackendError::Auth { message: msg },
        429 => BackendError::RateLimit {
            message: msg,
            retry_after_ms: None,
        },
        500..=599 => BackendError::Network { message: msg },
        _ => BackendError::ExecutionFailed {
            message: msg,
            exit_code: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn config_for(server: &MockServer) -> TensorZeroConfig {
        TensorZeroConfig {
            endpoint: format!("{}/v1/", server.uri()),
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
            .and(path("/v1/chat/completions"))
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
            .and(path("/v1/chat/completions"))
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
            .and(path("/v1/chat/completions"))
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
            .and(path("/v1/chat/completions"))
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
            .and(path("/v1/chat/completions"))
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
            .and(path("/v1/chat/completions"))
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
    fn name_is_tensorzero() {
        let cfg = TensorZeroConfig {
            endpoint: "http://localhost:3000/v1/".to_string(),
            model: "test-model".to_string(),
            api_key: Some("k".to_string()),
            timeout: Duration::from_secs(1),
        };
        let backend = TensorZeroBackend::new(cfg).unwrap();
        assert_eq!(backend.name(), "tensorzero");
        assert!(backend.is_available());
    }
}
