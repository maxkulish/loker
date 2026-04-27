//! `EscalatingRetry`: walk a cheap-to-strong ladder of backends, stop at the
//! first verify pass.
//!
//! Per loker-design.md §4.2 this is the second `Strategy` variant after
//! `SingleModel`. The walker calls each rung in order; on a verify pass it
//! returns immediately with `final_status: Succeeded`, and on full
//! exhaustion it returns `StrategyError::Exhausted` carrying a fully-shaped
//! `StrategyOutput` (with `final_status: Exhausted`) so the caller can
//! still persist the schema-shaped JSON.
//!
//! Backend errors do **not** abort the ladder - they are recorded as a
//! failed attempt and the walker advances to the next rung. This matches
//! AC: "non-retryable backend error does not skip subsequent backends".

use crate::backend::Backend;
use crate::strategy::{
    pick_model, Attempt, BackendError, FinalStatus, FinishReason, PhaseContext, Prompt, Strategy,
    StrategyError, StrategyKind, StrategyOutput, Tier, TokenUsageReport, VerifyError, VerifyHook,
    VerifyOutcome, VerifyResult, SCHEMA_VERSION,
};
use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// 4 KiB excerpt fits inside an 8 KiB envelope while leaving headroom for
/// verifier reason and backend error class.
const MAX_RESPONSE_EXCERPT_BYTES: usize = 4096;

/// Soft cap on the rendered failure-context block (including the original
/// prompt body). Enforced by `build_failure_envelope`.
const MAX_FAILURE_CONTEXT_BYTES: usize = 8192;

/// Single rung of the ladder: which tier this slot represents and which
/// backend (matched against `Backend::name()`) should serve it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rung {
    pub tier: Tier,
    pub backend: String,
}

impl Rung {
    pub fn new(tier: Tier, backend: impl Into<String>) -> Self {
        Self {
            tier,
            backend: backend.into(),
        }
    }
}

/// Wire-serializable subset of `EscalatingRetry` used for config round-trip
/// tests. Does *not* carry the `VerifyHook` trait object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EscalatingRetryConfig {
    pub rungs: Vec<Rung>,
    pub prompt_template: String,
    #[serde(default)]
    pub pass_failure_context: bool,
}

/// Captures the signals from a failed ladder rung so the *next* rung can
/// receive them when `pass_failure_context` is enabled.
///
/// Only the **most recent** failure is retained per `execute()` call. The
/// full history is *not* accumulated into one envelope - that scales linearly
/// with rung count and is intentionally deferred to a future task if needed.
#[derive(Debug, Clone, PartialEq)]
pub struct FailureContext {
    pub previous_tier: Tier,
    pub previous_backend: String,
    pub verify_reason: Option<String>,
    pub backend_error_class: Option<String>,
    pub response_excerpt: Option<String>,
}

impl FailureContext {
    /// Build from a verify-fail outcome. `response` is the raw backend output
    /// (redaction + truncation applied internally).
    pub fn from_verify_fail(
        tier: Tier,
        backend: impl Into<String>,
        reason: impl Into<String>,
        response: Option<impl Into<String>>,
    ) -> Self {
        let reason = redact_secrets(&reason.into());
        let response_excerpt = response.map(|r| {
            truncate_excerpt(&redact_secrets(&r.into()), MAX_RESPONSE_EXCERPT_BYTES)
        });
        Self {
            previous_tier: tier,
            previous_backend: backend.into(),
            verify_reason: Some(reason),
            backend_error_class: None,
            response_excerpt,
        }
    }

    /// Build from a backend error. No response excerpt is captured because
    /// the backend did not produce one.
    pub fn from_backend_error(
        tier: Tier,
        backend: impl Into<String>,
        error: &BackendError,
    ) -> Self {
        let class = backend_error_class(error);
        let redacted = redact_secrets(&class);
        Self {
            previous_tier: tier,
            previous_backend: backend.into(),
            verify_reason: None,
            backend_error_class: Some(redacted),
            response_excerpt: None,
        }
    }
}

/// Returns the discriminant name of a `BackendError` variant (e.g. `"Timeout"`).
fn backend_error_class(err: &BackendError) -> String {
    match err {
        BackendError::Timeout { .. } => "Timeout",
        BackendError::RateLimit { .. } => "RateLimited",
        BackendError::Auth { .. } => "Auth",
        BackendError::Network { .. } => "Network",
        BackendError::Parse { .. } => "Parse",
        BackendError::ExecutionFailed { .. } => "ExecutionFailed",
        BackendError::Unavailable { .. } => "Unavailable",
        BackendError::Config { .. } => "Config",
    }
    .to_string()
}

/// Redact common secret shapes from text before they reach the next rung's
/// prompt envelope. Applied to *every* byte of `FailureContext` text
/// (verify_reason, response_excerpt, and the final assembled header).
///
/// This is the module-local helper scoped to `escalating_retry.rs`. A
/// future centralised secret-scrubbing service should absorb this function
/// rather than invent a second one.
pub fn redact_secrets(input: &str) -> String {
    let mut result = input.to_string();

    // Pattern 1: AWS access keys
    let aws = Regex::new(r"AKIA[0-9A-Z]{16}").expect("valid regex");
    result = aws.replace_all(&result, "[REDACTED]").to_string();

    // Pattern 2: generic key=value (case-insensitive; redacts only the value side)
    let key_val = Regex::new(r"(?i)((?:api[_-]?key|secret|token|password)\s*[=:]\s*)[^\s'\"]+")
        .expect("valid regex");
    result = key_val.replace_all(&result, "${1}[REDACTED]").to_string();

    // Pattern 3: Bearer tokens
    let bearer = Regex::new(r"(?i)Bearer\s+[A-Za-z0-9._\-~+/=]+")
        .expect("valid regex");
    result = bearer.replace_all(&result, "[REDACTED]").to_string();

    // Pattern 4: heuristic – long base64-ish blob preceded by key/secret/token
    let heuristic = Regex::new(r"(?i)\b(key|secret|token)[\s:=]+([A-Za-z0-9+/=_\-]{32,})")
        .expect("valid regex");
    result = heuristic.replace_all(&result, "$1 [REDACTED]").to_string();

    result
}

/// Truncate `s` to at most `max_bytes`, cutting at a UTF-8 character
/// boundary. If truncation occurs, appends ` …[truncated, N bytes elided]`.
fn truncate_excerpt(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let elided = s.len() - boundary;
    format!("{} …[truncated, {} bytes elided]", &s[..boundary], elided)
}

/// Assemble the `<previous-attempt>...<original-prompt>` envelope.
///
/// Enforces `MAX_FAILURE_CONTEXT_BYTES`. If the envelope exceeds the cap,
/// fields are progressively truncated: `response_excerpt` first, then
/// `verify_reason` (capped at 1024), then `backend_error_class`.
pub fn build_failure_envelope(ctx: &FailureContext, body: &str) -> String {
    let mut excerpt_budget = MAX_RESPONSE_EXCERPT_BYTES;
    let mut verify_cap = 1024usize;

    loop {
        let excerpt = truncate_excerpt(
            ctx.response_excerpt.as_deref().unwrap_or(""),
            excerpt_budget,
        );
        let verify = truncate_excerpt(
            ctx.verify_reason.as_deref().unwrap_or(""),
            verify_cap,
        );
        let err = ctx.backend_error_class.as_deref().unwrap_or("null");
        let verify_field = if ctx.verify_reason.is_some() {
            format!("{:?}", verify)
        } else {
            "null".to_string()
        };
        let err_field = if ctx.backend_error_class.is_some() {
            format!("{:?}", err)
        } else {
            "null".to_string()
        };
        let response_field = if ctx.response_excerpt.is_some() {
            let indented = excerpt.replace('\n', "\n    ");
            format!("|\n    {}", indented)
        } else {
            "null".to_string()
        };

        let envelope = format!(
            "<previous-attempt>\n  tier: {}\n  backend: {}\n  verify_reason: {}\n  backend_error: {}\n  response_excerpt: {}\n</previous-attempt>\n\n<original-prompt>\n{}\n</original-prompt>",
            ctx.previous_tier.as_str(),
            ctx.previous_backend,
            verify_field,
            err_field,
            response_field,
            body,
        );

        if envelope.len() <= MAX_FAILURE_CONTEXT_BYTES
            || (excerpt_budget == 0 && verify_cap <= 64)
        {
            return envelope;
        }

        if excerpt_budget > 0 {
            excerpt_budget = excerpt_budget.saturating_sub(512);
        } else if verify_cap > 64 {
            verify_cap = verify_cap.saturating_sub(256);
        }
    }
}

/// Ladder strategy. `rungs` must be non-empty; capability validation runs
/// at workflow load time (CLO-251), so by the time `execute` runs every
/// referenced backend has already been proven capable.
///
/// `pass_failure_context` controls whether a failed rung's failure context
/// is injected into the next rung's prompt. Off by default.
pub struct EscalatingRetry {
    pub rungs: Vec<Rung>,
    pub prompt_template: String,
    pub verify: Arc<dyn VerifyHook>,
    pub pass_failure_context: bool,
}

impl EscalatingRetry {
    pub fn new(
        rungs: Vec<Rung>,
        prompt_template: impl Into<String>,
        verify: Arc<dyn VerifyHook>,
    ) -> Self {
        Self {
            rungs,
            prompt_template: prompt_template.into(),
            verify,
            pass_failure_context: false,
        }
    }

    /// When to enable: turn this on for workflows where escalation
    /// quality measurably improves with prior context (typical of
    /// strict-output TDD-style flows). The reference `design-doc-tdd`
    /// workflow ships with `pass_failure_context = true`. Off by
    /// default in v0 because (a) it widens the input prompt and so
    /// has a token cost, (b) it can leak failure-mode noise into the
    /// next rung if the verifier is too chatty.
    pub fn with_pass_failure_context(mut self, enabled: bool) -> Self {
        self.pass_failure_context = enabled;
        self
    }
}

#[async_trait]
impl Strategy for EscalatingRetry {
    async fn execute(
        &self,
        backends: &[Arc<dyn Backend>],
        prompt: &Prompt,
        ctx: &PhaseContext,
    ) -> Result<StrategyOutput, StrategyError> {
        if self.rungs.is_empty() || backends.is_empty() {
            return Err(StrategyError::NoBackends);
        }

        let rendered = ctx
            .template_engine
            .render(&self.prompt_template, &ctx.template_context)?;

        let mut attempts: Vec<Attempt> = Vec::with_capacity(self.rungs.len());
        let mut previous_failure: Option<FailureContext> = None;

        for (idx, rung) in self.rungs.iter().enumerate() {
            let backend = backends
                .iter()
                .find(|b| b.name() == rung.backend)
                .ok_or_else(|| StrategyError::BackendNotFound {
                    name: rung.backend.clone(),
                })?;

            let output_path = format!(
                "{}/attempts/{}-{}.md",
                ctx.phase_name,
                idx + 1,
                rung.tier.as_str()
            );

            // Build the prompt for this rung. Rung 1 always gets the bare
            // rendered prompt. Later rungs get the failure envelope prepended
            // when the flag is on and there is a previous failure.
            let rung_prompt = if idx > 0
                && self.pass_failure_context
                && let Some(ref fail_ctx) = previous_failure
            {
                let envelope = build_failure_envelope(fail_ctx, &rendered);
                redact_secrets(&envelope)
            } else {
                rendered.clone()
            };

            match backend
                .query(&rung_prompt, &ctx.cwd, prompt.model.as_deref())
                .await
            {
                Ok(query) => {
                    let usage = query
                        .usage
                        .as_ref()
                        .map(TokenUsageReport::from)
                        .unwrap_or_default();
                    let model = pick_model(&query, prompt);

                    match self.verify.verify(&query).await {
                        Ok(result) => {
                            let passed = result.is_pass();
                            let verify_outcome = if passed {
                                VerifyOutcome::passed(self.verify.name())
                            } else {
                                VerifyOutcome::failed(self.verify.name())
                            };
                            attempts.push(Attempt {
                                tier: Some(rung.tier),
                                family: None,
                                backend: backend.name().to_string(),
                                model: model.clone(),
                                finish_reasons: vec![FinishReason::Stop],
                                usage,
                                output_path,
                                verify: verify_outcome,
                            });

                            if passed {
                                return Ok(StrategyOutput {
                                    schema_version: SCHEMA_VERSION,
                                    strategy: StrategyKind::Escalating,
                                    phase: ctx.phase_name.clone(),
                                    run_id: ctx.run_id,
                                    attempts,
                                    final_status: Some(FinalStatus::Succeeded),
                                    aggregator: None,
                                    aggregate_output_path: None,
                                    verify: None,
                                });
                            }

                            // Verify failed - record failure context for next rung
                            let reason = match &result {
                                VerifyResult::Fail { reason } => reason.clone(),
                                _ => "verify did not pass".to_string(),
                            };
                            previous_failure = Some(FailureContext::from_verify_fail(
                                rung.tier,
                                backend.name(),
                                reason,
                                Some(query.text.clone()),
                            ));
                        }
                        Err(verify_err) => {
                            // Hook itself blew up: record as a failed attempt
                            // (status=fail, hook name preserved) and keep
                            // walking the ladder.
                            attempts.push(Attempt {
                                tier: Some(rung.tier),
                                family: None,
                                backend: backend.name().to_string(),
                                model: model.clone(),
                                finish_reasons: vec![FinishReason::Stop],
                                usage,
                                output_path,
                                verify: VerifyOutcome::failed(self.verify.name()),
                            });

                            previous_failure = Some(FailureContext::from_verify_fail(
                                rung.tier,
                                backend.name(),
                                &verify_err.message,
                                Some(query.text.clone()),
                            ));
                        }
                    }
                }
                Err(err) => {
                    // Backend errored. Record it as an error-finished attempt
                    // and advance to the next rung. Per AC: non-retryable
                    // backend errors must not abort the ladder.
                    let model = prompt
                        .model
                        .as_ref()
                        .filter(|m| !m.is_empty())
                        .cloned()
                        .unwrap_or_else(|| "default".to_string());
                    attempts.push(Attempt {
                        tier: Some(rung.tier),
                        family: None,
                        backend: rung.backend.clone(),
                        model,
                        finish_reasons: vec![FinishReason::Error],
                        usage: TokenUsageReport::default(),
                        output_path,
                        verify: VerifyOutcome::skipped(),
                    });

                    previous_failure =
                        Some(FailureContext::from_backend_error(
                            rung.tier,
                            &rung.backend,
                            &err,
                        ));
                }
            }
        }

        let exhausted = StrategyOutput {
            schema_version: SCHEMA_VERSION,
            strategy: StrategyKind::Escalating,
            phase: ctx.phase_name.clone(),
            run_id: ctx.run_id,
            attempts,
            final_status: Some(FinalStatus::Exhausted),
            aggregator: None,
            aggregate_output_path: None,
            verify: None,
        };
        Err(StrategyError::Exhausted {
            output: Box::new(exhausted),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_aws_key() {
        let s = "key: AKIAIOSFODNN7EXAMPLE rest";
        assert_eq!(redact_secrets(s), "key: [REDACTED] rest");
    }

    #[test]
    fn redaction_api_key_value() {
        let s = "api_key=AKIA0123456789ABCDEF other";
        assert_eq!(redact_secrets(s), "api_key=[REDACTED] other");
    }

    #[test]
    fn redaction_bearer_token() {
        let s = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI";
        assert_eq!(redact_secrets(s), "Authorization: [REDACTED]");
    }

    #[test]
    fn redaction_long_blob_heuristic() {
        let s = "token abcdef1234567890abcdef1234567890abcdef1234567890abcdef12345678 extra";
        assert_eq!(
            redact_secrets(s),
            "token [REDACTED] extra"
        );
    }

    #[test]
    fn redaction_does_not_false_positive_short_text() {
        let s = "The user's name is John and their secret is just a word.";
        assert_eq!(redact_secrets(s), s);
    }

    #[test]
    fn truncate_no_op_when_under_budget() {
        assert_eq!(truncate_excerpt("hello", 10), "hello");
    }

    #[test]
    fn truncate_multibyte_safe() {
        // 🎉 is 4 bytes; place it right at the 5-byte boundary.
        let s = "ab🎉cd";
        assert_eq!(truncate_excerpt(s, 5), "ab …[truncated, 4 bytes elided]");
    }

    #[test]
    fn truncate_exact_boundary() {
        let s = "abcdef";
        assert_eq!(truncate_excerpt(s, 3), "abc …[truncated, 3 bytes elided]");
    }

    #[test]
    fn envelope_under_budget_no_truncation() {
        let ctx = FailureContext::from_verify_fail(
            Tier::Cheap,
            "ollama-local",
            "expected JSON object",
            Some("short body"),
        );
        let out = build_failure_envelope(&ctx, "original prompt");
        assert!(out.len() <= MAX_FAILURE_CONTEXT_BYTES);
        assert!(out.contains("<previous-attempt>"));
        assert!(out.contains("<original-prompt>\noriginal prompt\n</original-prompt>"));
    }

    #[test]
    fn envelope_over_budget_truncates_excerpt() {
        let huge = "x".repeat(100_000);
        let ctx = FailureContext::from_verify_fail(
            Tier::Cheap,
            "ollama-local",
            "fail",
            Some(&huge),
        );
        let out = build_failure_envelope(&ctx, "prompt");
        assert!(out.len() <= MAX_FAILURE_CONTEXT_BYTES);
        assert!(out.contains("…[truncated,"));
    }

    #[test]
    fn envelope_verify_reason_only_when_no_response() {
        let ctx = FailureContext::from_verify_fail::<String>(
            Tier::Cheap,
            "backend",
            "bad",
            None,
        );
        let out = build_failure_envelope(&ctx, "p");
        assert!(out.contains("response_excerpt: null"));
        assert!(out.contains(r#"verify_reason: "bad""#));
    }

    #[test]
    fn envelope_backend_error_shows_null_response() {
        let ctx = FailureContext::from_backend_error(
            Tier::Cheap,
            "backend",
            &BackendError::Timeout {
                message: "timed out".to_string(),
                elapsed_ms: 100,
            },
        );
        let out = build_failure_envelope(&ctx, "p");
        assert!(out.contains("response_excerpt: null"));
        assert!(out.contains(r#"backend_error: "Timeout""#));
        assert!(out.contains("verify_reason: null"));
    }

    #[test]
    fn config_round_trip_true() {
        let cfg = EscalatingRetryConfig {
            rungs: vec![Rung::new(Tier::Cheap, "cheap")],
            prompt_template: "render-me".to_string(),
            pass_failure_context: true,
        };
        let s = toml::to_string(&cfg).unwrap();
        let restored: EscalatingRetryConfig = toml::from_str(&s).unwrap();
        assert_eq!(cfg, restored);
        assert!(restored.pass_failure_context);
    }

    #[test]
    fn config_round_trip_false() {
        let cfg = EscalatingRetryConfig {
            rungs: vec![Rung::new(Tier::Cheap, "cheap")],
            prompt_template: "render-me".to_string(),
            pass_failure_context: false,
        };
        let s = toml::to_string(&cfg).unwrap();
        let restored: EscalatingRetryConfig = toml::from_str(&s).unwrap();
        assert_eq!(cfg, restored);
        assert!(!restored.pass_failure_context);
    }

    #[test]
    fn config_default_false() {
        let s = r#"
rungs = [{ tier = "cheap", backend = "cheap" }]
prompt_template = " hi"
"#;
        let restored: EscalatingRetryConfig = toml::from_str(s).unwrap();
        assert!(!restored.pass_failure_context);
    }
}
