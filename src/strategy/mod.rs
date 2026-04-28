//! Strategy primitives for orchestrating one or more backend calls within a phase.
//!
//! Per loker-design.md §4.2 a `Strategy` decides how a phase consumes backends:
//! `SingleModel` (one backend, one prompt, one response), `EscalatingRetry`
//! (CLO-258 - this module's second variant: try a cheap-to-strong ladder of
//! backends and stop at the first verify pass), `ParallelFanOut` (CLO-259).
//! All three implement the same `Strategy` trait so the phase runner (T-029)
//! can dispatch over them uniformly.
//!
//! ## Capability invariant
//!
//! Strategy code does *not* check `BackendCapabilities` at execute time.
//! Capability validation runs at workflow load (FR-4 / CLO-251's
//! `validate_with_capabilities`). By the time `Strategy::execute` runs the
//! caller has already proven that every backend it passes can do what the
//! strategy needs.

use crate::backend::{Backend, BackendError, QueryOutput, TokenUsage};
use crate::template::{TemplateContext, TemplateEngine, TemplateError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

pub mod escalating_retry;
pub mod parallel_fanout;
pub mod single_model;
pub mod verify;

pub use escalating_retry::EscalatingRetry;
pub use parallel_fanout::{ParallelFanOut, TargetSpec};
pub use single_model::SingleModel;
pub use verify::{VerifyError, VerifyHook, VerifyResult};

/// `schema_version` value emitted by every `StrategyOutput`. Pinned to the
/// const declared in `docs/schemas/phase_result_*.schema.json`.
pub const SCHEMA_VERSION: u32 = 1;

/// Phase-level prompt overrides applied on top of the strategy's own
/// template. Currently carries an optional model override that passes
/// through to `Backend::query(.., model)`.
///
/// `#[non_exhaustive]` so future fields (system prompt, tool definitions)
/// land additively without breaking call sites.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct Prompt {
    pub model: Option<String>,
}

impl Prompt {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
}

/// Per-phase metadata the strategy needs to render templates and stamp
/// the resulting `StrategyOutput`.
///
/// Production callers (CLO-261 / T-029 phase runner) build this from the
/// workflow definition; tests build it via `PhaseContext::new_for_test`.
///
/// `#[non_exhaustive]` so future fields (run dir path, verify config)
/// land additively.
#[non_exhaustive]
pub struct PhaseContext {
    pub phase_name: String,
    pub run_id: uuid::Uuid,
    pub cwd: PathBuf,
    pub template_engine: Arc<TemplateEngine>,
    pub template_context: TemplateContext,
}

impl PhaseContext {
    /// Builds a `PhaseContext` with a fresh `TemplateEngine`, empty
    /// `TemplateContext`, and `cwd` set to the current directory. Used
    /// by integration tests today; production callers (CLO-261 / T-029)
    /// can either call this and override fields or construct the struct
    /// literally from the workflow definition.
    pub fn new(phase_name: impl Into<String>, run_id: uuid::Uuid) -> Self {
        Self {
            phase_name: phase_name.into(),
            run_id,
            cwd: PathBuf::from("."),
            template_engine: Arc::new(TemplateEngine::new()),
            template_context: TemplateContext::new(&Default::default(), &[], &[]),
        }
    }
}

/// Strategy primitive: how a phase consumes backends.
///
/// Implementations must be `Send + Sync` so the phase runner can hold them
/// behind `Arc<dyn Strategy>` and drive them across async tasks.
///
/// `prompt: &Prompt` is borrowed so the same prompt can be replayed by
/// `EscalatingRetry` / `ParallelFanOut` without cloning. `backends:
/// &[Arc<dyn Backend>]` matches the in-tree convention - the engine resolves
/// backends to `Arc<dyn Backend>` via `create_backend`
/// (`src/backend/mod.rs:346`). Design doc §4.2 sketches `&[Box<dyn Backend>]`;
/// the `Arc` deviation is a deliberate alignment with the rest of the codebase.
#[async_trait]
pub trait Strategy: Send + Sync {
    async fn execute(
        &self,
        backends: &[Arc<dyn Backend>],
        prompt: &Prompt,
        ctx: &PhaseContext,
    ) -> Result<StrategyOutput, StrategyError>;
}

/// Discriminator for the active strategy, serialised into the
/// `loker.strategy` field of `StrategyOutput`.
///
/// `#[non_exhaustive]` - new variants land alongside new strategy
/// implementations (CLO-259).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    Single,
    Escalating,
    Parallel,
}

/// Tier of a single attempt in an `EscalatingRetry` ladder.
///
/// The escalating phase-result schema requires every attempt to declare
/// which tier produced it; `SingleModel` attempts omit the field via
/// `#[serde(skip_serializing_if = "Option::is_none")]` on `Attempt::tier`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Cheap,
    Medium,
    Strong,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cheap => "cheap",
            Self::Medium => "medium",
            Self::Strong => "strong",
        }
    }
}

/// Outcome of an `EscalatingRetry` ladder, recorded in the escalating
/// phase result. SingleModel omits the field via
/// `#[serde(skip_serializing_if = "Option::is_none")]`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalStatus {
    Succeeded,
    Exhausted,
    Aborted,
}

/// Phase result emitted by `Strategy::execute`.
///
/// Field names and rename attributes are pinned to the
/// `docs/schemas/phase_result_*.schema.json` set. `final_status` is only
/// populated by ladder strategies (currently `EscalatingRetry`); SingleModel
/// omits the field so the same struct serialises against the single schema.
///
/// For the `Parallel` strategy, `attempts` is serialised as `branches`, and
/// `aggregator`, `aggregate_output_path`, and `verify` are emitted at the
/// top level (per `phase_result_parallel.schema.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregator {
    Concat,
    AnyFail,
    Vote,
    LLMJudge,
}

impl Aggregator {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Concat => "concat",
            Self::AnyFail => "any_fail",
            Self::Vote => "vote",
            Self::LLMJudge => "llm_judge",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StrategyOutput {
    pub schema_version: u32,
    pub strategy: StrategyKind,
    pub phase: String,
    pub run_id: uuid::Uuid,
    pub attempts: Vec<Attempt>,
    pub final_status: Option<FinalStatus>,
    pub aggregator: Option<Aggregator>,
    pub aggregate_output_path: Option<String>,
    pub verify: Option<VerifyOutcome>,
}

impl Default for StrategyOutput {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            strategy: StrategyKind::Single,
            phase: String::new(),
            run_id: uuid::Uuid::nil(),
            attempts: Vec::new(),
            final_status: None,
            aggregator: None,
            aggregate_output_path: None,
            verify: None,
        }
    }
}

impl Serialize for StrategyOutput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let is_parallel = matches!(self.strategy, StrategyKind::Parallel);
        let mut field_count = 4; // schema_version, loker.strategy, loker.phase, loker.run_id
        if is_parallel {
            field_count += 4; // branches, aggregator, aggregate_output_path, verify
        } else {
            field_count += 1; // attempts
            if self.final_status.is_some() {
                field_count += 1;
            }
        }

        let mut state = serializer.serialize_struct("StrategyOutput", field_count)?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("loker.strategy", &self.strategy)?;
        state.serialize_field("loker.phase", &self.phase)?;
        state.serialize_field("loker.run_id", &self.run_id)?;

        if is_parallel {
            #[derive(Serialize)]
            struct Branch<'a> {
                backend: &'a str,
                family: &'a str,
                model: &'a str,
                finish_reasons: &'a [FinishReason],
                usage: &'a TokenUsageReport,
                output_path: &'a str,
            }

            let branches: Vec<Branch> = self
                .attempts
                .iter()
                .map(|a| Branch {
                    backend: &a.backend,
                    family: a.family.as_deref().unwrap_or("local"),
                    model: &a.model,
                    finish_reasons: &a.finish_reasons,
                    usage: &a.usage,
                    output_path: &a.output_path,
                })
                .collect();

            state.serialize_field("branches", &branches)?;
            state.serialize_field(
                "aggregator",
                self.aggregator
                    .as_ref()
                    .map(|a| a.as_str())
                    .unwrap_or("concat"),
            )?;
            state.serialize_field(
                "aggregate_output_path",
                self.aggregate_output_path
                    .as_deref()
                    .unwrap_or("aggregated.txt"),
            )?;
            state.serialize_field(
                "verify",
                self.verify.as_ref().unwrap_or(&VerifyOutcome::skipped()),
            )?;
        } else {
            state.serialize_field("attempts", &self.attempts)?;
            if let Some(ref fs) = self.final_status {
                state.serialize_field("final_status", fs)?;
            }
        }

        state.end()
    }
}

/// Per-call record. SingleModel emits exactly one; `EscalatingRetry` emits
/// one per ladder rung. `tier` is `Some` only on escalating attempts.
#[derive(Debug, Clone, Serialize)]
pub struct Attempt {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<Tier>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
    pub backend: String,
    pub model: String,
    pub finish_reasons: Vec<FinishReason>,
    pub usage: TokenUsageReport,
    pub output_path: String,
    pub verify: VerifyOutcome,
}

/// Schema-aligned reason a backend stopped producing tokens.
///
/// v0 maps every successful backend call to `FinishReason::Stop` and every
/// backend error to `FinishReason::Error`; the value becomes data-driven
/// once backends start surfacing real finish reasons in `QueryOutput`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Error,
}

/// Token counts using the schema's wire names (`input_tokens`,
/// `output_tokens`), distinct from `crate::backend::TokenUsage` which
/// follows the OpenAI shape (`prompt_tokens`, `completion_tokens`).
///
/// Backends without usage reporting yield `{ input_tokens: 0,
/// output_tokens: 0 }`; the schema requires the keys, not non-zero values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TokenUsageReport {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl From<&TokenUsage> for TokenUsageReport {
    fn from(u: &TokenUsage) -> Self {
        Self {
            input_tokens: u.prompt_tokens,
            output_tokens: u.completion_tokens,
        }
    }
}

/// Verify-hook outcome recorded for an attempt. SingleModel always emits
/// `VerifyOutcome::skipped()`; `EscalatingRetry` populates Pass / Fail
/// using the configured `VerifyHook`.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyOutcome {
    pub status: VerifyStatus,
    pub hook: Option<String>,
}

impl VerifyOutcome {
    pub fn skipped() -> Self {
        Self {
            status: VerifyStatus::Skipped,
            hook: None,
        }
    }

    pub fn passed(hook: impl Into<String>) -> Self {
        Self {
            status: VerifyStatus::Pass,
            hook: Some(hook.into()),
        }
    }

    pub fn failed(hook: impl Into<String>) -> Self {
        Self {
            status: VerifyStatus::Fail,
            hook: Some(hook.into()),
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyStatus {
    Pass,
    Fail,
    Skipped,
}

/// Errors a `Strategy::execute` call can return. `Backend` carries the
/// inner `BackendError` unchanged so callers can pattern-match the
/// original variant (the FR-5 acceptance criterion: "error surfaces
/// unchanged"). `Exhausted` carries the full `StrategyOutput` so the
/// caller can persist the schema-shaped JSON even on failure.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum StrategyError {
    #[error("backend not found: {name}")]
    BackendNotFound { name: String },

    #[error("no backends available")]
    NoBackends,

    #[error("prompt render failed: {0}")]
    PromptRender(#[from] TemplateError),

    #[error("backend error: {0}")]
    Backend(#[from] BackendError),

    #[error("escalating retry exhausted all backends")]
    Exhausted { output: Box<StrategyOutput> },

    #[error("parallel floor violated: {successes}/{min_responses} targets succeeded")]
    FloorViolation {
        successes: usize,
        min_responses: usize,
        output: Box<StrategyOutput>,
    },
}

/// Errors that can surface at phase-runner level, **above** individual
/// strategy execution. These are distinct from `StrategyError` because
/// they describe invariants the phase runner enforces (e.g. cross-family
/// diversity) rather than runtime backend or rendering failures inside a
/// single strategy.
///
/// `#[non_exhaustive]` so new phase-level invariants can be added without
/// breaking downstream consumers.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum PhaseError {
    #[error("family overlap: found {family} on {count} backends")]
    FamilyOverlap {
        family: crate::family::Family,
        count: usize,
    },
}

/// Build the `model` field that lands in `Attempt`, applying the v0
/// priority rule: prefer the value the backend reports, fall back to the
/// model the prompt requested, otherwise the literal `"default"`.
///
/// The schema requires `model` to be a non-empty string; this helper
/// treats empty strings from either source as missing so the result is
/// guaranteed non-empty.
pub(crate) fn pick_model(query: &QueryOutput, prompt: &Prompt) -> String {
    query
        .model
        .as_deref()
        .filter(|m| !m.is_empty())
        .or_else(|| prompt.model.as_deref().filter(|m| !m.is_empty()))
        .map(str::to_string)
        .unwrap_or_else(|| "default".to_string())
}

#[cfg(test)]
mod future_variant_compiles {
    //! Compile-time check that the `Strategy` trait shape accommodates
    //! `ParallelFanOut` (CLO-259) without modifying `SingleModel`,
    //! `EscalatingRetry`, or their call sites.
    //!
    //! Never executed; only ensures the trait remains object-safe and
    //! does not bake in single-call assumptions in its signature.

    use super::*;

    struct StubFanOut;

    #[async_trait]
    impl Strategy for StubFanOut {
        async fn execute(
            &self,
            _backends: &[Arc<dyn Backend>],
            _prompt: &Prompt,
            _ctx: &PhaseContext,
        ) -> Result<StrategyOutput, StrategyError> {
            unimplemented!("compile-time stub for future-variant check")
        }
    }

    #[allow(dead_code)]
    fn assert_object_safe(_: &dyn Strategy) {}

    #[test]
    fn stub_fan_out_implements_strategy() {
        let stub: Arc<dyn Strategy> = Arc::new(StubFanOut);
        assert_object_safe(&*stub);
    }
}
