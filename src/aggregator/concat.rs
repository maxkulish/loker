use std::sync::Arc;

use crate::backend::Backend;
use crate::strategy::PhaseContext;

// Aggregator implementations for folding multiple branch outputs into one artefact.
//
// This module contains behavioral aggregator config and pure aggregation logic.
// It is intentionally separate from [`crate::strategy::Aggregator`], which is
// the schema-facing label serialized into phase-result JSON.

/// Sentinel emitted when concat aggregation receives no branch outcomes.
///
/// Empty input is valid and never panics. The sentinel is a Markdown comment so
/// downstream phases can consume it as a string artefact without rendering noisy
/// prose to users.
pub const EMPTY_CONCAT_SENTINEL: &str =
    "<!-- loker: concat aggregator received no target outputs -->";

/// Behavioral aggregator configuration.
///
/// `Concat` joins successful branch outputs under rendered headings and appends
/// failed branches to a structured `## Errors` footer. Supported heading
/// placeholders are exactly:
///
/// - `{backend_id}`: source backend identifier
/// - `{family}`: resolved model family label
/// - `{index}`: caller-provided 1-based branch index
///
/// Unknown placeholders are preserved literally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aggregator {
    Concat {
        heading_template: String,
    },
    AnyFail,
    LLMJudge {
        judge_backend: String,
        prompt_template: String,
        require_judge_different_family: bool,
    },
}

impl Aggregator {
    /// Build a concat aggregator with the provided heading template.
    pub fn concat(heading_template: impl Into<String>) -> Self {
        Self::Concat {
            heading_template: heading_template.into(),
        }
    }

    /// Build an LLM judge aggregator with the provided configuration.
    pub fn llm_judge(
        judge_backend: impl Into<String>,
        prompt_template: impl Into<String>,
        require_judge_different_family: bool,
    ) -> Self {
        Self::LLMJudge {
            judge_backend: judge_backend.into(),
            prompt_template: prompt_template.into(),
            require_judge_different_family,
        }
    }

    /// Build an AnyFail aggregator (no configuration needed).
    pub fn any_fail() -> Self {
        Self::AnyFail
    }

    /// Return the schema-facing strategy aggregator label for this behavior.
    pub fn kind(&self) -> crate::strategy::Aggregator {
        match self {
            Self::Concat { .. } => crate::strategy::Aggregator::Concat,
            Self::AnyFail => crate::strategy::Aggregator::AnyFail,
            Self::LLMJudge { .. } => crate::strategy::Aggregator::LLMJudge,
        }
    }

    /// Aggregate ordered branch outcomes into one string artefact.
    pub fn aggregate(
        &self,
        input: AggregateInput,
        _backends: &[Arc<dyn Backend>],
        _ctx: &PhaseContext,
    ) -> Result<AggregatedArtifact, AggregatorError> {
        match self {
            Self::Concat { heading_template } => aggregate_concat(heading_template, input),
            Self::AnyFail => Err(AggregatorError::Unsupported(
                "AnyFail is evaluated inline by ParallelFanOut, not via aggregate()".into(),
            )),
            Self::LLMJudge { .. } => Err(AggregatorError::Unsupported(
                "LLMJudge requires async backend access; use aggregate_llm_judge()".into(),
            )),
        }
    }
}

/// Ordered input to an aggregator.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AggregateInput {
    pub branches: Vec<BranchOutcome>,
}

/// Per-branch outcome supplied by the phase runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchOutcome {
    Success(BranchSuccess),
    Failure(BranchFailure),
}

/// Successful branch text plus source metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchSuccess {
    pub backend_id: String,
    pub family: String,
    /// 1-based caller-visible index. For `ParallelFanOut`, this should be the
    /// arrival-order position supplied by the phase runner.
    pub index: usize,
    pub output: String,
}

/// Failed branch reason plus source metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchFailure {
    pub backend_id: String,
    pub family: String,
    /// 1-based caller-visible index.
    pub index: usize,
    pub reason: String,
}

/// Aggregate artefact and summary counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregatedArtifact {
    pub text: String,
    pub successful: usize,
    pub failed: usize,
}

/// Errors produced by aggregators.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AggregatorError {
    #[error("unsupported aggregator operation: {0}")]
    Unsupported(String),
}

fn aggregate_concat(
    heading_template: &str,
    input: AggregateInput,
) -> Result<AggregatedArtifact, AggregatorError> {
    if input.branches.is_empty() {
        return Ok(AggregatedArtifact {
            text: EMPTY_CONCAT_SENTINEL.to_string(),
            successful: 0,
            failed: 0,
        });
    }

    let mut sections = Vec::new();
    let mut failures = Vec::new();

    for branch in input.branches {
        match branch {
            BranchOutcome::Success(success) => {
                sections.push(render_success(heading_template, success))
            }
            BranchOutcome::Failure(failure) => failures.push(failure),
        }
    }

    let successful = sections.len();
    let failed = failures.len();
    let mut parts = sections;

    if !failures.is_empty() {
        parts.push(render_errors(&failures));
    }

    let mut text = if parts.is_empty() {
        String::new()
    } else {
        parts.join("\n\n")
    };
    text.push('\n');

    Ok(AggregatedArtifact {
        text,
        successful,
        failed,
    })
}

fn render_success(heading_template: &str, success: BranchSuccess) -> String {
    let heading = render_heading(
        heading_template,
        &success.backend_id,
        &success.family,
        success.index,
    );
    let output = success.output.trim();
    if output.is_empty() {
        heading
    } else {
        format!("{}\n\n{}", heading, output)
    }
}

fn render_heading(template: &str, backend_id: &str, family: &str, index: usize) -> String {
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start) = rest.find('{') {
        rendered.push_str(&rest[..start]);
        rest = &rest[start..];

        if rest.starts_with("{backend_id}") {
            rendered.push_str(backend_id);
            rest = &rest["{backend_id}".len()..];
        } else if rest.starts_with("{family}") {
            rendered.push_str(family);
            rest = &rest["{family}".len()..];
        } else if rest.starts_with("{index}") {
            rendered.push_str(&index.to_string());
            rest = &rest["{index}".len()..];
        } else if let Some(end) = rest.find('}') {
            rendered.push_str(&rest[..=end]);
            rest = &rest[end + '}'.len_utf8()..];
        } else {
            rendered.push_str(rest);
            rest = "";
        }
    }

    rendered.push_str(rest);
    rendered
}

fn render_errors(failures: &[BranchFailure]) -> String {
    let mut out = String::from("## Errors");
    for failure in failures {
        out.push_str("\n\n");
        out.push_str(&format!(
            "- backend_id: {}\n  family: {}\n  index: {}\n  reason: {}",
            failure.backend_id,
            failure.family,
            failure.index,
            render_reason(&failure.reason)
        ));
    }
    out
}

fn render_reason(reason: &str) -> String {
    reason
        .trim()
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn success(backend_id: &str, family: &str, index: usize, output: &str) -> BranchOutcome {
        BranchOutcome::Success(BranchSuccess {
            backend_id: backend_id.into(),
            family: family.into(),
            index,
            output: output.into(),
        })
    }

    fn failure(backend_id: &str, family: &str, index: usize, reason: &str) -> BranchOutcome {
        BranchOutcome::Failure(BranchFailure {
            backend_id: backend_id.into(),
            family: family.into(),
            index,
            reason: reason.into(),
        })
    }

    #[test]
    fn concat_renders_success_sections_in_input_order() {
        let artifact = Aggregator::concat("## {index}. {backend_id} ({family})")
            .aggregate(
                AggregateInput {
                    branches: vec![
                        success("claude", "anthropic", 1, " first "),
                        success("gemini", "google", 2, "second\n"),
                    ],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(
            artifact.text,
            "## 1. claude (anthropic)\n\nfirst\n\n## 2. gemini (google)\n\nsecond\n"
        );
        assert_eq!(artifact.successful, 2);
        assert_eq!(artifact.failed, 0);
    }

    #[test]
    fn concat_preserves_unknown_placeholders() {
        let artifact = Aggregator::concat("## {backend_id} {unknown}")
            .aggregate(
                AggregateInput {
                    branches: vec![success("claude", "anthropic", 1, "text")],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(artifact.text, "## claude {unknown}\n\ntext\n");
    }

    #[test]
    fn concat_preserves_braced_unknown_expressions_containing_known_tokens() {
        let artifact = Aggregator::concat("## {{backend_id}} {unknown {family}}")
            .aggregate(
                AggregateInput {
                    branches: vec![success("claude", "anthropic", 1, "text")],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(
            artifact.text,
            "## {{backend_id}} {unknown {family}}\n\ntext\n"
        );
    }

    #[test]
    fn concat_does_not_reexpand_placeholders_inside_metadata() {
        let artifact = Aggregator::concat("## {backend_id} ({family})")
            .aggregate(
                AggregateInput {
                    branches: vec![success("review-{index}", "other-{backend_id}", 3, "text")],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(
            artifact.text,
            "## review-{index} (other-{backend_id})\n\ntext\n"
        );
    }

    #[test]
    fn concat_escapes_multiline_failure_reason() {
        let artifact = Aggregator::concat("## {backend_id}")
            .aggregate(
                AggregateInput {
                    branches: vec![failure(
                        "codex",
                        "openai",
                        1,
                        "network: timeout\nretry exhausted",
                    )],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert!(artifact
            .text
            .contains("reason: network: timeout\\nretry exhausted"));
    }

    #[test]
    fn concat_normalizes_crlf_failure_reason() {
        let artifact = Aggregator::concat("## {backend_id}")
            .aggregate(
                AggregateInput {
                    branches: vec![failure("codex", "openai", 1, "line1\r\nline2\rline3")],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert!(artifact.text.contains("reason: line1\\nline2\\nline3"));
        assert!(!artifact.text.contains('\r'));
    }

    #[test]
    fn concat_whitespace_only_success_output_keeps_newline_invariants() {
        let artifact = Aggregator::concat("## {backend_id}")
            .aggregate(
                AggregateInput {
                    branches: vec![
                        success("claude", "anthropic", 1, "   \n"),
                        success("gemini", "google", 2, "ok"),
                    ],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(artifact.text, "## claude\n\n## gemini\n\nok\n");
    }

    #[test]
    fn concat_empty_input_returns_sentinel() {
        let artifact = Aggregator::concat("## {backend_id}")
            .aggregate(
                AggregateInput::default(),
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(artifact.text, EMPTY_CONCAT_SENTINEL);
        assert_eq!(artifact.successful, 0);
        assert_eq!(artifact.failed, 0);
    }

    #[test]
    fn concat_counts_success_and_failure() {
        let artifact = Aggregator::concat("## {backend_id}")
            .aggregate(
                AggregateInput {
                    branches: vec![
                        success("claude", "anthropic", 1, "ok"),
                        failure("codex", "openai", 2, "network: timeout"),
                    ],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(artifact.successful, 1);
        assert_eq!(artifact.failed, 1);
        assert!(artifact.text.contains("## Errors"));
        assert!(artifact.text.contains("- backend_id: codex"));
    }

    #[test]
    fn concat_kind_maps_to_strategy_label() {
        assert_eq!(
            Aggregator::concat("## {backend_id}").kind(),
            crate::strategy::Aggregator::Concat
        );
    }

    #[test]
    fn llm_judge_kind_maps_to_strategy_label() {
        assert_eq!(
            Aggregator::llm_judge("judge", "template", true).kind(),
            crate::strategy::Aggregator::LLMJudge
        );
    }

    #[test]
    fn concat_mixed_success_failure_snapshot() {
        let artifact = Aggregator::concat("## {index}. {backend_id} ({family})")
            .aggregate(
                AggregateInput {
                    branches: vec![
                        success("claude", "anthropic", 1, "Claude review text."),
                        failure("codex", "openai", 2, "network: timeout"),
                        success("gemini", "google", 3, "Gemini review text."),
                    ],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        insta::assert_snapshot!(artifact.text);
    }
}
