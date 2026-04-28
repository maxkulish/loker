//! Aggregator primitives.
//!
//! Three aggregator behaviors live here:
//! - [`concat`]: fold multiple branch outputs into one Markdown artefact.
//! - [`any_fail_evaluate`] / [`AnyFailReason`]: short-circuit on the first
//!   rejected verdict.
//! - [`llm_judge`]: use a separate-family LLM to pick the best candidate.

mod concat;
mod llm_judge;

pub use concat::{
    AggregateInput, AggregatedArtifact, Aggregator, AggregatorError, BranchFailure, BranchOutcome,
    BranchSuccess, EMPTY_CONCAT_SENTINEL,
};

pub use llm_judge::{
    aggregate_llm_judge, check_cross_family, clamp_chosen_index, parse_ballot,
    render_ballot_prompt, Ballot, Candidate, LLMJudgeError,
};

use serde_json::Value;

#[non_exhaustive]
#[derive(Debug, Clone, thiserror::Error)]
pub enum AnyFailReason {
    #[error("verdict rejected")]
    VerdictRejected { payload: String },
    #[error("verdict contract violation: {message}")]
    VerdictContract { message: String },
    #[error("backend error: {detail}")]
    BackendError { detail: String },
}

/// Parse a backend response text as a JSON verdict and check `pass`.
///
/// Strips markdown fences (` ```json ... ``` `) before parsing so
/// LLM backends that wrap JSON in code blocks work out of the box.
/// Accepts extra keys (forward compatible). Missing or malformed `pass`
/// is a contract violation, not a panic.
pub fn any_fail_evaluate(text: &str) -> Result<(), AnyFailReason> {
    let stripped = strip_markdown_fences(text.trim());
    let value: Value =
        serde_json::from_str(stripped).map_err(|e| AnyFailReason::VerdictContract {
            message: format!("JSON parse error: {e}"),
        })?;

    match value.get("pass") {
        Some(Value::Bool(true)) => Ok(()),
        Some(Value::Bool(false)) => Err(AnyFailReason::VerdictRejected {
            payload: stripped.to_string(),
        }),
        Some(other) => Err(AnyFailReason::VerdictContract {
            message: format!("expected bool, got {}", other),
        }),
        None => Err(AnyFailReason::VerdictContract {
            message: "missing required field 'pass'".to_string(),
        }),
    }
}

/// Strip leading/trailing markdown fences if present.
pub(crate) fn strip_markdown_fences(text: &str) -> &str {
    let text = text.strip_prefix("```json").unwrap_or(text);
    let text = text.strip_prefix("```").unwrap_or(text);
    let text = text.strip_suffix("```").unwrap_or(text);
    text.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_true() {
        assert!(any_fail_evaluate(r#"{"pass": true}"#).is_ok());
    }

    #[test]
    fn pass_false() {
        let err = any_fail_evaluate(r#"{"pass": false}"#).unwrap_err();
        assert!(matches!(err, AnyFailReason::VerdictRejected { .. }));
    }

    #[test]
    fn missing_pass() {
        let err = any_fail_evaluate(r#"{"status": "ok"}"#).unwrap_err();
        assert!(matches!(err, AnyFailReason::VerdictContract { .. }));
    }

    #[test]
    fn wrong_pass_type() {
        let err = any_fail_evaluate(r#"{"pass": "yes"}"#).unwrap_err();
        assert!(matches!(err, AnyFailReason::VerdictContract { .. }));
    }

    #[test]
    fn empty_text() {
        let err = any_fail_evaluate("").unwrap_err();
        assert!(matches!(err, AnyFailReason::VerdictContract { .. }));
    }

    #[test]
    fn markdown_fenced_json() {
        let text = "```json\n{\"pass\": true}\n```";
        assert!(any_fail_evaluate(text).is_ok());
    }

    #[test]
    fn markdown_fenced_fail() {
        let text = "```json\n{\"pass\": false}\n```";
        let err = any_fail_evaluate(text).unwrap_err();
        assert!(matches!(err, AnyFailReason::VerdictRejected { .. }));
    }

    #[test]
    fn extra_keys_ok() {
        assert!(any_fail_evaluate(r#"{"pass": true, "note": "lgtm"}"#).is_ok());
    }
}
