//! Core verification types and trait.
//!
//! This module defines the `VerifyHook` trait and supporting types
//! (`VerifyResult`, `FailureReason`, `VerifyError`, `VerifyContext`).
//! Concrete implementations live in sibling modules (`llm_verifier`,
//! `run_command`).
//!
//! v0 hooks only need `Pass` and `Fail`. `Repair` and `Score` are reserved so
//! later hook implementations can evolve without changing the public enum.
//!
//! ## Security: redaction
//!
//! `FailureReason.stdout` and `FailureReason.stderr` carry raw output that may
//! contain secrets. Redaction is deferred to the consumer — CLO-260's
//! `pass_failure_context` path runs `redact_secrets()` before flowing the
//! reason into the next prompt. Hook implementations that log or persist
//! `FailureReason` fields directly must apply their own redaction.

/// Sandbox-level signal captured from command execution failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxViolation {
    /// Wall-clock timeout expired while the command was running.
    Timeout,
    /// Process exited due to an OS signal (Unix-only).
    Signal { signal: i32 },
    /// Non-zero process exit code.
    NonZeroExit { code: i32 },
}

// ── FailureReason ────────────────────────────────────────────

use std::time::Duration;

use async_trait::async_trait;

use crate::backend::QueryOutput;

// ── FailureReason ────────────────────────────────────────────

/// Structured reason a verification hook returned `Fail`.
///
/// Carries enough detail to feed `pass_failure_context` (CLO-260).
/// Fields are `pub` so callers can extract individual signals without
/// parsing the combined `display()` string.
///
/// ## Security: Redaction
///
/// `stdout` and `stderr` carry raw output that may contain secrets
/// (API keys in LLM responses, stack traces with env vars). Redaction
/// is **deferred to the consumer** — CLO-260's `pass_failure_context`
/// path runs `redact_secrets()` on the reason before flowing it into
/// the next prompt. Hook implementations that log or persist
/// `FailureReason` fields directly must apply their own redaction.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct FailureReason {
    /// Human-readable summary (e.g. "test `it_adds` failed").
    pub summary: String,
    /// Captured stdout from the verification run (may be truncated).
    /// **Unredacted** — consumers must apply redaction before prompt injection.
    pub stdout: String,
    /// Captured stderr from the verification run (may be truncated).
    /// **Unredacted** — consumers must apply redaction before prompt injection.
    pub stderr: String,
    /// `true` iff stdout or stderr was truncated at `MAX_OUTPUT_BYTES`.
    pub truncated: bool,
    /// Exit code if the verifier ran as a process. `None` for in‑process
    /// verifiers (e.g. `LLMVerifier`).
    pub exit_code: Option<i32>,
    /// Optional sandbox signal that further explains process termination.
    pub sandbox_violation: Option<SandboxViolation>,
}

impl FailureReason {
    /// Create a new failure reason with a human-readable summary.
    /// All other fields default to empty / `None`.
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            stdout: String::new(),
            stderr: String::new(),
            truncated: false,
            exit_code: None,
            sandbox_violation: None,
        }
    }

    /// Attach captured stdout (builder-pattern).
    pub fn with_stdout(mut self, stdout: impl Into<String>) -> Self {
        self.stdout = stdout.into();
        self
    }

    /// Attach captured stderr (builder-pattern).
    pub fn with_stderr(mut self, stderr: impl Into<String>) -> Self {
        self.stderr = stderr.into();
        self
    }

    /// Mark the output as truncated (builder-pattern).
    pub fn with_truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }

    /// Attach an exit code (builder-pattern).
    pub fn with_exit_code(mut self, exit_code: i32) -> Self {
        self.exit_code = Some(exit_code);
        self
    }

    /// Attach sandbox metadata (builder-pattern).
    pub fn with_sandbox_violation(mut self, sandbox_violation: SandboxViolation) -> Self {
        self.sandbox_violation = Some(sandbox_violation);
        self
    }
}

impl std::fmt::Display for FailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.summary)?;
        if self.truncated {
            write!(f, " (truncated)")?;
        }
        Ok(())
    }
}

// ── VerifyResult ─────────────────────────────────────────────

/// Verdict returned by a `VerifyHook::verify()` call.
///
/// **Variant lifecycle:**
///
/// | Variant | v0 status | Notes |
/// |---------|-----------|-------|
/// | `Pass`  | **live** — emitted by v0 hooks | |
/// | `Fail { reason }` | **live** — `reason` is `FailureReason` | |
/// | `Repair { suggestion }` | **reserved** — compiles, no caller acts on it yet | M10 `HumanVerifier` will emit this |
/// | `Score(f32)` | **reserved** — compiles, no caller acts on it yet. Higher values = better quality. | Future cascadeflow‑style semantic gates |
///
/// Callers that pattern‑match MUST include arms for reserved variants;
/// the recommended pattern is a documented fallthrough (see
/// `escalating_retry.rs` for the reference consumer).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum VerifyResult {
    Pass,
    Fail { reason: FailureReason },
    Repair { suggestion: String },
    Score(f32),
}

impl VerifyResult {
    /// Convenience constructor for a `Pass` variant.
    pub fn pass() -> Self {
        Self::Pass
    }

    /// Convenience constructor for a `Fail` variant with a simple summary.
    /// Other `FailureReason` fields default to empty.
    pub fn fail(summary: impl Into<String>) -> Self {
        Self::Fail {
            reason: FailureReason::new(summary),
        }
    }

    /// Convenience constructor for a `Fail` variant with a fully populated reason.
    pub fn fail_with(reason: FailureReason) -> Self {
        Self::Fail { reason }
    }

    /// `true` iff this is a `Pass` variant.
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    /// `true` iff this is a `Fail` variant.
    pub fn is_fail(&self) -> bool {
        matches!(self, Self::Fail { .. })
    }
}

// ── VerifyError ──────────────────────────────────────────────

/// Error surfaced when a `VerifyHook` implementation itself fails.
///
/// Distinct from `VerifyResult::Fail`:
/// - `Fail` means the hook ran, decided "that output isn't good enough",
///   and produced a structured `FailureReason`.
/// - `VerifyError` means the hook could not run at all: sandbox crash,
///   backend unreachable, `make` missing from `$PATH`, etc.
///
/// ## Future: error source chain
/// For v0 the `message` string suffices. When CLO-271 (RunCommand)
/// introduces I/O errors and subprocess failures, `VerifyError` should
/// gain a `#[source]`-annotated field (e.g. `source: Option<Box<dyn std::error::Error + Send + Sync>>`)
/// to preserve the original error chain for debugging.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("verify hook failed: {message}")]
pub struct VerifyError {
    pub message: String,
}

impl VerifyError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

// ── VerifyContext ────────────────────────────────────────────

/// Input passed to every `VerifyHook::verify()` call.
///
/// Carries the output under verification plus metadata about the phase
/// and backend that produced it. Does **not** carry credentials
/// (API keys, tokens) — those live in `BackendConfig` and are never
/// exposed to verify hooks.
///
/// `#[non_exhaustive]` so the phase runner (T-029) can add fields
/// (manifest pointer, run‑dir paths) without breaking hook implementations.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct VerifyContext {
    /// Raw stdout from the backend call under verification.
    pub stdout: String,
    /// Raw stderr from the backend call, if any.
    pub stderr: Option<String>,
    /// Exit code if the backend ran as a subprocess.
    pub exit_code: Option<i32>,
    /// Name of the backend that produced this output (e.g. `"claude-3"`).
    pub backend_name: String,
    /// Model name reported by the backend, if known.
    pub model: Option<String>,
    /// Parsed JSON if the output was successfully deserialized.
    pub structured: Option<serde_json::Value>,
    /// Wall‑clock duration of the backend call, as measured by `Backend::query()`.
    pub duration: Duration,
}

impl VerifyContext {
    /// Build a `VerifyContext` from a `QueryOutput` plus a backend name.
    ///
    /// This is the EscalatingRetry call‑site constructor. When the phase
    /// runner (T-029) replaces EscalatingRetry as the direct caller, it
    /// can build `VerifyContext` from other sources (manifest, run dir)
    /// without touching hook implementations.
    pub fn from_query_output(query: &QueryOutput) -> Self {
        Self {
            stdout: query.stdout.clone(),
            stderr: query.stderr.clone(),
            exit_code: query.exit_code,
            backend_name: query.backend.clone(),
            model: query.model.clone(),
            structured: query.structured.clone(),
            duration: query.duration,
        }
    }
}

// ── VerifyHook trait ─────────────────────────────────────────

/// Verification hook that gates strategy progress.
///
/// Implementations are `Send + Sync` so they can be shared behind `Arc`
/// and driven across async tasks by the phase runner.
///
/// ## Method contract
///
/// - `name()` returns a stable, human‑readable label for trace output
///   (e.g. `"TestRunner"`, `"LLMVerifier"`).
/// - `verify(ctx)` inspects the backend output in `ctx` and returns a
///   verdict. `Err(VerifyError)` signals the hook itself failed;
///   `Ok(VerifyResult::Fail { .. })` signals the hook ran successfully
///   but judged the output insufficient.
///
/// ## Required context contract
///
/// As `VerifyContext` gains fields over time (via `#[non_exhaustive]`),
/// a hook may receive a context that lacks a field required for its
/// operation. In that case the hook MUST return `Err(VerifyError)` with
/// a descriptive message — it MUST NOT panic.
///
/// ## Cancellation safety
///
/// `verify()` implementors are responsible for cancellation safety.
/// If a `tokio::timeout` (or similar) drops the future mid-execution,
/// any spawned subprocesses or in-flight HTTP requests must be cleaned
/// up (e.g. via `tokio::spawn` with a cancellation token, or by
/// ensuring the future is abort-safe).
#[async_trait]
pub trait VerifyHook: Send + Sync {
    fn name(&self) -> &str;

    async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError>;
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// A stub hook that returns a predetermined result.
    struct StubHook {
        name: &'static str,
        result: Result<VerifyResult, VerifyError>,
    }

    #[async_trait]
    impl VerifyHook for StubHook {
        fn name(&self) -> &str {
            self.name
        }

        async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
            match &self.result {
                Ok(r) => Ok(r.clone()),
                Err(e) => Err(e.clone()),
            }
        }
    }

    fn dummy_context() -> VerifyContext {
        VerifyContext {
            stdout: String::new(),
            stderr: None,
            exit_code: None,
            backend_name: "test".to_string(),
            model: None,
            structured: None,
            duration: Duration::ZERO,
        }
    }

    #[tokio::test]
    async fn stub_verify_hook_returns_pass() {
        let hook = Arc::new(StubHook {
            name: "pass_stub",
            result: Ok(VerifyResult::Pass),
        });
        let result = hook.verify(&dummy_context()).await.unwrap();
        assert!(result.is_pass());
        assert!(!result.is_fail());
    }

    #[tokio::test]
    async fn stub_verify_hook_returns_fail() {
        let reason = FailureReason::new("test failed");
        let hook = Arc::new(StubHook {
            name: "fail_stub",
            result: Ok(VerifyResult::Fail {
                reason: reason.clone(),
            }),
        });
        let result = hook.verify(&dummy_context()).await.unwrap();
        assert!(result.is_fail());
        assert!(!result.is_pass());
        if let VerifyResult::Fail { reason: r } = result {
            assert_eq!(r.summary, "test failed");
        } else {
            panic!("expected Fail variant");
        }
    }

    #[tokio::test]
    async fn stub_verify_hook_returns_fail_with_full_reason() {
        let reason = FailureReason::new("validation error")
            .with_stdout("line 1: unexpected token")
            .with_stderr("error[E0308]: type mismatch")
            .with_truncated(false)
            .with_exit_code(1);
        let hook = Arc::new(StubHook {
            name: "full_fail",
            result: Ok(VerifyResult::Fail {
                reason: reason.clone(),
            }),
        });
        let result = hook.verify(&dummy_context()).await.unwrap();
        if let VerifyResult::Fail { reason: r } = result {
            assert_eq!(r.summary, "validation error");
            assert_eq!(r.stdout, "line 1: unexpected token");
            assert_eq!(r.stderr, "error[E0308]: type mismatch");
            assert!(!r.truncated);
            assert_eq!(r.exit_code, Some(1));
        } else {
            panic!("expected Fail variant");
        }
    }

    #[tokio::test]
    async fn stub_verify_hook_returns_error() {
        let hook = Arc::new(StubHook {
            name: "error_stub",
            result: Err(VerifyError::new("sandbox crashed")),
        });
        let err = hook.verify(&dummy_context()).await.unwrap_err();
        assert_eq!(err.message, "sandbox crashed");
    }

    #[tokio::test]
    async fn reserved_repair_compiles_but_not_pass() {
        let hook = Arc::new(StubHook {
            name: "repair_stub",
            result: Ok(VerifyResult::Repair {
                suggestion: "retry with temperature 0.5".to_string(),
            }),
        });
        let result = hook.verify(&dummy_context()).await.unwrap();
        assert!(!result.is_pass());
        assert!(!result.is_fail());
    }

    #[tokio::test]
    async fn reserved_score_compiles_but_not_pass() {
        let hook = Arc::new(StubHook {
            name: "score_stub",
            result: Ok(VerifyResult::Score(0.9)),
        });
        let result = hook.verify(&dummy_context()).await.unwrap();
        assert!(!result.is_pass());
        assert!(!result.is_fail());
    }

    #[tokio::test]
    async fn failure_reason_builder_api() {
        let reason = FailureReason::new("msg")
            .with_stdout("out")
            .with_stderr("err")
            .with_truncated(true)
            .with_exit_code(1);
        assert_eq!(reason.summary, "msg");
        assert_eq!(reason.stdout, "out");
        assert_eq!(reason.stderr, "err");
        assert!(reason.truncated);
        assert_eq!(reason.exit_code, Some(1));
    }

    #[tokio::test]
    async fn verify_context_from_query_output() {
        let query = QueryOutput {
            stdout: "hello world".to_string(),
            stderr: Some("warnings".to_string()),
            exit_code: Some(0),
            backend: "test-backend".to_string(),
            model: Some("gpt-4".to_string()),
            structured: Some(serde_json::json!({ "key": "value" })),
            duration: Duration::from_secs(1),
            usage: None,
        };
        let ctx = VerifyContext::from_query_output(&query);
        assert_eq!(ctx.stdout, "hello world");
        assert_eq!(ctx.stderr, Some("warnings".to_string()));
        assert_eq!(ctx.exit_code, Some(0));
        assert_eq!(ctx.backend_name, "test-backend");
        assert_eq!(ctx.model, Some("gpt-4".to_string()));
        assert_eq!(ctx.structured, Some(serde_json::json!({ "key": "value" })));
        assert_eq!(ctx.duration, Duration::from_secs(1));
    }

    #[test]
    fn failure_reason_display() {
        let r = FailureReason::new("test failed".to_string());
        assert_eq!(r.to_string(), "test failed");

        let r = FailureReason::new("test failed".to_string()).with_truncated(true);
        assert_eq!(r.to_string(), "test failed (truncated)");
    }
}
