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
    StrategyError, StrategyKind, StrategyOutput, Tier, TokenUsageReport, VerifyContext, VerifyHook,
    VerifyOutcome, VerifyResult, SCHEMA_VERSION,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

/// 4 KiB excerpt fits inside an 8 KiB envelope while leaving headroom for
/// verifier reason and backend error class.
const MAX_RESPONSE_EXCERPT_BYTES: usize = 4096;

/// Soft cap on the rendered failure-context block (including the original
/// prompt body). Enforced by `build_failure_envelope`.
const MAX_FAILURE_CONTEXT_BYTES: usize = 8192;

/// Single rung of the ladder: which tier this slot represents and which
/// backend (matched against `Backend::name()`) should serve it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
///
/// `pub(crate)` because the only current consumers are this module's tests;
/// promote to `pub` once the workflow loader (T-029) needs to deserialize
/// it from TOML.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct EscalatingRetryConfig {
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
    /// Build from a verify-fail outcome. `response` is borrowed - the raw
    /// backend output (redaction + truncation applied internally produces
    /// the owned excerpt, so the caller does not need to clone first).
    pub fn from_verify_fail(
        tier: Tier,
        backend: impl Into<String>,
        reason: impl Into<String>,
        response: Option<&str>,
    ) -> Self {
        let reason = redact_secrets(&reason.into());
        let response_excerpt =
            response.map(|r| truncate_excerpt(&redact_secrets(r), MAX_RESPONSE_EXCERPT_BYTES));
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
pub(crate) fn redact_secrets(input: &str) -> String {
    crate::utils::redact_secrets(input)
}

/// Truncate `s` to at most `max_bytes` total (including the suffix),
/// cutting at a UTF-8 character boundary. If truncation occurs, appends
/// ` …[truncated, N bytes elided]`. The result is *guaranteed* to be at
/// most `max_bytes` bytes; if the suffix alone would exceed `max_bytes`,
/// returns a hard-cut prefix without the suffix.
fn truncate_excerpt(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    // Suffix template is " …[truncated, {N} bytes elided]" - 30 bytes plus
    // the digit count of the elided byte count. Upper-bound the digit count
    // by the digits in `s.len()` since `elided <= s.len()`.
    const SUFFIX_TEMPLATE_LEN: usize = " …[truncated,  bytes elided]".len();
    let digits = s.len().to_string().len();
    let suffix_len = SUFFIX_TEMPLATE_LEN + digits;
    if suffix_len >= max_bytes {
        let mut boundary = max_bytes;
        while boundary > 0 && !s.is_char_boundary(boundary) {
            boundary -= 1;
        }
        return s[..boundary].to_string();
    }
    let mut boundary = max_bytes - suffix_len;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let elided = s.len() - boundary;
    format!("{} …[truncated, {} bytes elided]", &s[..boundary], elided)
}

/// Assemble the `<previous-attempt>...<original-prompt>` envelope.
///
/// Enforces `MAX_FAILURE_CONTEXT_BYTES` as a hard cap. The shrink loop first
/// trims `response_excerpt` (in 512-byte steps) and then `verify_reason`
/// (down to a 64-byte floor in 256-byte steps). If the envelope is still
/// over budget after both budgets are exhausted (e.g. the original prompt
/// body alone exceeds the cap), the assembled envelope is truncated as a
/// final fallback so the returned string is *guaranteed* to fit.
///
/// `pub(crate)` because the only external consumers are this module's
/// `execute()` and tests; promote to `pub` once a foreign caller appears.
pub(crate) fn build_failure_envelope(ctx: &FailureContext, body: &str) -> String {
    let mut excerpt_budget = MAX_RESPONSE_EXCERPT_BYTES;
    let mut verify_cap = 1024usize;

    loop {
        let excerpt = truncate_excerpt(
            ctx.response_excerpt.as_deref().unwrap_or(""),
            excerpt_budget,
        );
        let verify = truncate_excerpt(ctx.verify_reason.as_deref().unwrap_or(""), verify_cap);
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

        if envelope.len() <= MAX_FAILURE_CONTEXT_BYTES {
            return envelope;
        }

        if excerpt_budget > 0 {
            excerpt_budget = excerpt_budget.saturating_sub(512);
        } else if verify_cap > 64 {
            verify_cap = verify_cap.saturating_sub(256);
        } else {
            // Both shrink budgets exhausted (the body itself is over the
            // cap). Fall through to a hard-cap truncation so the returned
            // envelope is guaranteed to fit MAX_FAILURE_CONTEXT_BYTES.
            return truncate_excerpt(&envelope, MAX_FAILURE_CONTEXT_BYTES);
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
            let rung_prompt = if idx > 0 && self.pass_failure_context {
                if let Some(ref fail_ctx) = previous_failure {
                    let envelope = build_failure_envelope(fail_ctx, &rendered);
                    redact_secrets(&envelope)
                } else {
                    rendered.clone()
                }
            } else {
                rendered.clone()
            };

            match backend
                .query(&rung_prompt, &ctx.cwd, prompt.model.as_deref())
                .await
            {
                Ok(query) => {
                    write_strategy_output(&ctx.cwd, &output_path, &query.stdout).await?;
                    let usage = query
                        .usage
                        .as_ref()
                        .map(TokenUsageReport::from)
                        .unwrap_or_default();
                    let model = pick_model(&query, prompt);

                    let verify_ctx = VerifyContext::from_query_output(&query);
                    match self.verify.verify(&verify_ctx).await {
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
                            // only when the flag is on; building it eagerly when
                            // disabled wastes work since the next rung will not
                            // consume it (see `rung_prompt` selector below).
                            previous_failure = self.pass_failure_context.then(|| {
                                let reason = match &result {
                                    VerifyResult::Fail { reason } => reason.summary.clone(),
                                    _ => "verify did not pass".to_string(),
                                };
                                FailureContext::from_verify_fail(
                                    rung.tier,
                                    backend.name(),
                                    reason,
                                    Some(&query.stdout),
                                )
                            });
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

                            previous_failure = self.pass_failure_context.then(|| {
                                FailureContext::from_verify_fail(
                                    rung.tier,
                                    backend.name(),
                                    &verify_err.message,
                                    Some(&query.stdout),
                                )
                            });
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

                    previous_failure = self.pass_failure_context.then(|| {
                        FailureContext::from_backend_error(rung.tier, backend.name(), &err)
                    });
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

async fn write_strategy_output(
    cwd: &Path,
    output_path: &str,
    text: &str,
) -> Result<(), StrategyError> {
    if cwd == Path::new(".") {
        return Ok(());
    }
    let path = cwd.join(output_path);
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await.map_err(|err| {
                StrategyError::Backend(crate::backend::BackendError::ExecutionFailed {
                    message: format!("failed to create output parent {}: {err}", parent.display()),
                    exit_code: None,
                })
            })?;
        }
    }
    tokio::fs::write(&path, text).await.map_err(|err| {
        StrategyError::Backend(crate::backend::BackendError::ExecutionFailed {
            message: format!("failed to write strategy output {}: {err}", path.display()),
            exit_code: None,
        })
    })
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
        assert_eq!(redact_secrets(s), "token [REDACTED] extra");
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
        // 🎉 is 4 bytes; place it right at the 5-byte boundary. The cap of
        // 5 is too small to fit the truncation suffix, so the function
        // returns a hard-cut prefix that stops before the multi-byte char.
        let s = "ab🎉cd"; // 8 bytes total
        let out = truncate_excerpt(s, 5);
        assert!(out.len() <= 5);
        assert_eq!(out, "ab");
    }

    #[test]
    fn truncate_exact_boundary() {
        // Cap of 3 is too small for the suffix; hard-cut prefix returned.
        let s = "abcdef";
        let out = truncate_excerpt(s, 3);
        assert!(out.len() <= 3);
        assert_eq!(out, "abc");
    }

    #[test]
    fn truncate_with_suffix_fits_within_budget() {
        // Cap large enough to fit the truncation suffix; verify the result
        // is at or below the cap and ends with the elided-bytes marker.
        let s = "a".repeat(200);
        let out = truncate_excerpt(&s, 64);
        assert!(out.len() <= 64, "got {} bytes, cap was 64", out.len());
        assert!(out.contains("[truncated,"));
        assert!(out.ends_with("bytes elided]"));
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
        let ctx =
            FailureContext::from_verify_fail(Tier::Cheap, "ollama-local", "fail", Some(&huge));
        let out = build_failure_envelope(&ctx, "prompt");
        assert!(out.len() <= MAX_FAILURE_CONTEXT_BYTES);
        assert!(out.contains("…[truncated,"));
    }

    /// Regression: when the original prompt body alone exceeds
    /// `MAX_FAILURE_CONTEXT_BYTES`, the excerpt/verify shrink loop cannot
    /// bring the envelope under the cap on its own. The function must still
    /// return a string at or below the hard cap rather than the previous
    /// behaviour of returning the oversized envelope verbatim.
    #[test]
    fn envelope_hard_caps_when_body_alone_exceeds_budget() {
        let body = "y".repeat(MAX_FAILURE_CONTEXT_BYTES * 2);
        let ctx = FailureContext::from_verify_fail(Tier::Cheap, "backend", "fail", Some("short"));
        let out = build_failure_envelope(&ctx, &body);
        assert!(
            out.len() <= MAX_FAILURE_CONTEXT_BYTES,
            "envelope must not exceed MAX_FAILURE_CONTEXT_BYTES; got {} bytes",
            out.len()
        );
    }

    #[test]
    fn envelope_verify_reason_only_when_no_response() {
        let ctx = FailureContext::from_verify_fail(Tier::Cheap, "backend", "bad", None);
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
