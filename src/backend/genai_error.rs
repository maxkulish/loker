//! Centralised translation from `genai::Error` (and `genai::webc::Error`) to
//! `BackendError`.
//!
//! TensorZero is currently the only consumer of the `genai` crate, but the
//! mapping is intentionally backend-agnostic: anything emitted by `genai`
//! lands here, retryability is preserved on the resulting `BackendError`,
//! and 5xx body inspection (originally a TensorZero overlay) is applied
//! uniformly because all observed body shapes come from the TensorZero
//! gateway today.
//!
//! ## Variant mapping
//!
//! | `genai::Error` variant | `BackendError`                | retryable |
//! |------------------------|-------------------------------|-----------|
//! | `WebModelCall`         | (delegates to webc mapping)   | ...       |
//! | `WebAdapterCall`       | (delegates to webc mapping)   | ...       |
//! | `ChatResponseGeneration` | `Parse`                     | no        |
//! | `StreamParse`          | `Parse`                       | no        |
//! | `Resolver`             | `Config`                      | no        |
//! | `RequiresApiKey`       | `Auth`                        | no        |
//! | `NoAuthData`           | `Auth`                        | no        |
//! | `NoAuthResolver`       | `Auth`                        | no        |
//! | `HttpError`            | (delegates to status mapping) | ...       |
//! | other (catch-all)      | `ExecutionFailed`             | no        |
//!
//! | `genai::webc::Error` variant         | `BackendError`        | retryable |
//! |--------------------------------------|-----------------------|-----------|
//! | `ResponseFailedStatus`               | (delegates to status) | ...       |
//! | `ResponseFailedInvalidJson`          | `Parse`               | no        |
//! | `ResponseFailedNotJson`              | `Parse`               | no        |
//! | `Reqwest` (timeout)                  | `Timeout`             | yes       |
//! | `Reqwest` (connect / other)          | `Network`             | yes       |
//! | `JsonValueExt`                       | `Parse`               | no        |
//!
//! ## HTTP status mapping
//!
//! | Status   | `BackendError`                                                | retryable |
//! |----------|---------------------------------------------------------------|-----------|
//! | 401, 403 | `Auth`                                                        | no        |
//! | 404      | `Config` if body matches "unknown function", else `ExecutionFailed` | no |
//! | 429      | `RateLimit`                                                   | yes       |
//! | 502      | body-inspected: upstream auth -> `Auth`; upstream rate-limit -> `RateLimit`; else `Network` | mixed |
//! | 500-599  | `Network`                                                     | yes       |
//! | other    | `ExecutionFailed`                                             | no        |
//!
//! ## Elapsed-time stamping
//!
//! `From<genai::Error>` cannot observe wall-clock duration at the call site,
//! so the `Timeout` branch emits `elapsed_ms: 0`. Callers chain
//! `BackendError::with_elapsed(start.elapsed())` to attach the measured
//! duration, which is a no-op on every other variant.
//!
//! Source-of-truth for the 502 body shapes and the family-suffix routing:
//! `docs/spikes/2026-04-25-tensorzero-roundtrip.md` (CLO-243).

use super::BackendError;

impl From<genai::Error> for BackendError {
    fn from(err: genai::Error) -> Self {
        match err {
            genai::Error::WebModelCall { webc_error, .. }
            | genai::Error::WebAdapterCall { webc_error, .. } => map_webc_error(webc_error),
            genai::Error::ChatResponseGeneration { cause, .. } => BackendError::Parse {
                message: format!("genai response parse error: {cause}"),
            },
            genai::Error::StreamParse { serde_error, .. } => BackendError::Parse {
                message: format!("genai stream parse error: {serde_error}"),
            },
            genai::Error::Resolver { resolver_error, .. } => BackendError::Config {
                message: format!("genai resolver error: {resolver_error}"),
            },
            genai::Error::RequiresApiKey { .. }
            | genai::Error::NoAuthData { .. }
            | genai::Error::NoAuthResolver { .. } => BackendError::Auth {
                message: format!("genai auth missing: {err}"),
            },
            genai::Error::HttpError { status, body, .. } => map_status(status.as_u16(), body),
            other => BackendError::ExecutionFailed {
                message: format!("genai call failed: {other}"),
                exit_code: None,
            },
        }
    }
}

fn map_webc_error(err: genai::webc::Error) -> BackendError {
    use genai::webc::Error as W;
    match err {
        W::ResponseFailedStatus { status, body, .. } => map_status(status.as_u16(), body),
        W::ResponseFailedInvalidJson { body, cause } => BackendError::Parse {
            message: format!("genai invalid JSON response: {cause}; body: {body}"),
        },
        W::ResponseFailedNotJson { content_type, body } => BackendError::Parse {
            message: format!("genai non-JSON response (content-type {content_type}): {body}"),
        },
        W::Reqwest(e) => {
            if e.is_timeout() {
                BackendError::Timeout {
                    message: format!("genai request timed out: {e}"),
                    elapsed_ms: 0,
                }
            } else if e.is_connect() {
                BackendError::Network {
                    message: format!("genai connection failed: {e}"),
                }
            } else {
                BackendError::Network {
                    message: format!("genai request failed: {e}"),
                }
            }
        }
        W::JsonValueExt(e) => BackendError::Parse {
            message: format!("genai JSON value error: {e}"),
        },
    }
}

/// Inspect a 502 response body and reclassify gateway-wrapped upstream
/// failures. The TensorZero gateway forwards a 401 from Anthropic / OpenAI as
/// a 502 carrying the upstream error JSON in the body; the same goes for
/// upstream rate-limit (429) signals. Returns `None` to fall through to the
/// default 5xx classification (`Network`).
fn classify_5xx_body(status: u16, body: &str) -> Option<BackendError> {
    let lower = body.to_lowercase();
    let auth_match = lower.contains("authentication_error")
        || lower.contains("unauthorized")
        || lower.contains("invalid x-api-key")
        || lower.contains("invalid api key")
        || contains_status_code(&lower, "401");
    if auth_match {
        return Some(BackendError::Auth {
            message: format!("genai HTTP {status} (upstream auth failure): {body}"),
        });
    }
    let rate_match = lower.contains("rate_limit")
        || lower.contains("rate limit")
        || lower.contains("too many requests")
        || contains_status_code(&lower, "429");
    if rate_match {
        return Some(BackendError::RateLimit {
            message: format!("genai HTTP {status} (upstream rate limit): {body}"),
            retry_after_ms: None,
        });
    }
    None
}

/// Boundary-aware HTTP status-code substring check.
///
/// Matches `code` when surrounded by non-ASCII-alphanumeric boundaries
/// (start/end of string or any non-ASCII-alphanumeric character). This
/// catches forms like `" 401 "`, `"\"401\""`, `"401:"`, `"(429)"`, and
/// `{"status":429}` in minified JSON, while avoiding false positives such
/// as `"4011"` or `"H429X"` where the digits are part of a longer
/// alphanumeric identifier.
fn contains_status_code(haystack: &str, code: &str) -> bool {
    let bytes = haystack.as_bytes();
    let needle = code.as_bytes();
    let mut start = 0;
    while let Some(rel) = haystack[start..].find(code) {
        let i = start + rel;
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let after = i + needle.len();
        let after_ok = after == bytes.len() || !bytes[after].is_ascii_alphanumeric();
        if before_ok && after_ok {
            return true;
        }
        start = after;
    }
    false
}

/// Reclassify a 404 body that signals an "Unknown function:" config error
/// (see `tests/fixtures/tensorzero/unknown_function_response.json`). Such
/// failures will not become healthy on retry; they require config edits.
fn classify_404_body(body: &str) -> Option<BackendError> {
    if body.to_lowercase().contains("unknown function") {
        return Some(BackendError::Config {
            message: format!("genai unknown function: {body}"),
        });
    }
    None
}

fn map_status(status: u16, body: String) -> BackendError {
    let msg = format!("genai HTTP {status}: {body}");
    match status {
        401 | 403 => BackendError::Auth { message: msg },
        404 => classify_404_body(&body).unwrap_or(BackendError::ExecutionFailed {
            message: msg,
            exit_code: None,
        }),
        429 => BackendError::RateLimit {
            message: msg,
            retry_after_ms: None,
        },
        502 => classify_5xx_body(status, &body).unwrap_or(BackendError::Network { message: msg }),
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

    #[test]
    fn classify_5xx_body_detects_anthropic_auth_fixture() {
        let fixture =
            include_str!("../../tests/fixtures/tensorzero/anthropic_auth_failure_response.json");
        let classified = classify_5xx_body(502, fixture);
        assert!(
            matches!(classified, Some(BackendError::Auth { .. })),
            "expected Auth from anthropic 502 auth fixture, got {classified:?}"
        );
    }

    #[test]
    fn classify_5xx_body_detects_rate_limit_signature() {
        let body = r#"{"error":{"message":"All variants failed: rate_limit hit upstream"}}"#;
        let classified = classify_5xx_body(502, body);
        assert!(
            matches!(classified, Some(BackendError::RateLimit { .. })),
            "expected RateLimit, got {classified:?}"
        );
    }

    #[test]
    fn classify_5xx_body_returns_none_for_generic_5xx() {
        assert!(classify_5xx_body(500, "internal server error").is_none());
        assert!(classify_5xx_body(502, "").is_none());
    }

    #[test]
    fn classify_404_body_detects_unknown_function_fixture() {
        let fixture =
            include_str!("../../tests/fixtures/tensorzero/unknown_function_response.json");
        let classified = classify_404_body(fixture);
        assert!(
            matches!(classified, Some(BackendError::Config { .. })),
            "expected Config from unknown_function fixture, got {classified:?}"
        );
    }

    #[test]
    fn contains_status_code_handles_punctuation_boundaries() {
        for body in [
            r#"{"error":"401 unauthorized"}"#,
            r#"{"error":"\"401\""}"#,
            "status 401:",
            "(401)",
            r#"{"status":401}"#,
            "401",
        ] {
            assert!(
                contains_status_code(&body.to_lowercase(), "401"),
                "expected match in {body:?}"
            );
        }
        for body in ["4011", "H401X", "no auth here", "code=14010"] {
            assert!(
                !contains_status_code(&body.to_lowercase(), "401"),
                "unexpected match in {body:?}"
            );
        }
    }

    #[test]
    fn map_status_401_to_auth() {
        let err = map_status(401, "unauthorized".to_string());
        assert!(matches!(err, BackendError::Auth { .. }));
        assert!(!err.is_retryable());
    }

    #[test]
    fn map_status_403_to_auth() {
        let err = map_status(403, "forbidden".to_string());
        assert!(matches!(err, BackendError::Auth { .. }));
        assert!(!err.is_retryable());
    }

    #[test]
    fn map_status_404_unknown_function_to_config() {
        let err = map_status(404, "Unknown function: foo".to_string());
        assert!(matches!(err, BackendError::Config { .. }));
        assert!(!err.is_retryable());
    }

    #[test]
    fn map_status_404_other_to_execution_failed() {
        let err = map_status(404, "not found".to_string());
        assert!(matches!(err, BackendError::ExecutionFailed { .. }));
        assert!(!err.is_retryable());
    }

    #[test]
    fn map_status_429_to_rate_limit_retryable() {
        let err = map_status(429, "rate limit".to_string());
        assert!(matches!(err, BackendError::RateLimit { .. }));
        assert!(err.is_retryable());
    }

    #[test]
    fn map_status_502_upstream_auth_to_auth_not_retryable() {
        let err = map_status(
            502,
            r#"{"error":{"type":"authentication_error"}}"#.to_string(),
        );
        assert!(matches!(err, BackendError::Auth { .. }));
        assert!(!err.is_retryable());
    }

    #[test]
    fn map_status_502_upstream_rate_limit_to_rate_limit_retryable() {
        let err = map_status(
            502,
            r#"{"error":{"message":"rate_limit hit upstream"}}"#.to_string(),
        );
        assert!(matches!(err, BackendError::RateLimit { .. }));
        assert!(err.is_retryable());
    }

    #[test]
    fn map_status_502_generic_to_network_retryable() {
        let err = map_status(502, "bad gateway".to_string());
        assert!(matches!(err, BackendError::Network { .. }));
        assert!(err.is_retryable());
    }

    #[test]
    fn map_status_500_to_network_retryable() {
        let err = map_status(500, "server error".to_string());
        assert!(matches!(err, BackendError::Network { .. }));
        assert!(err.is_retryable());
    }

    #[test]
    fn map_status_503_to_network_retryable() {
        let err = map_status(503, "service unavailable".to_string());
        assert!(matches!(err, BackendError::Network { .. }));
        assert!(err.is_retryable());
    }

    #[test]
    fn map_status_unknown_to_execution_failed() {
        let err = map_status(418, "teapot".to_string());
        assert!(matches!(err, BackendError::ExecutionFailed { .. }));
        assert!(!err.is_retryable());
    }
}
