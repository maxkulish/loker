//! `Aggregator::LLMJudge` implementation.
//!
//! Constructs a ballot prompt from candidate branch outputs, enforces
//! cross-family separation between judge and candidates, calls the judge
//! backend, and parses the structured ballot response.

use std::sync::Arc;

use crate::aggregator::{strip_markdown_fences, AggregatedArtifact, BranchSuccess};
use crate::backend::Backend;
use crate::family::family_of;
use crate::strategy::PhaseContext;
use serde::{Deserialize, Serialize};

/// Candidate exposed to the prompt template.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub index: usize,
    pub backend_id: String,
    pub family: String,
    pub output: String,
}

impl Candidate {
    pub fn from_branch_success(success: &BranchSuccess) -> Self {
        Self {
            index: success.index,
            backend_id: success.backend_id.clone(),
            family: success.family.clone(),
            output: success.output.clone(),
        }
    }
}

/// Judge ballot response schema (v0).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Ballot {
    pub chosen_index: usize,
    pub reason: String,
}

/// Errors specific to LLMJudge aggregation.
#[derive(Debug, thiserror::Error)]
pub enum LLMJudgeError {
    #[error(
        "family overlap: judge backend {judge} shares family {family} with candidate {candidate}"
    )]
    FamilyOverlap {
        judge: String,
        candidate: String,
        family: String,
    },

    #[error("judge backend not found: {0}")]
    BackendNotFound(String),

    #[error("judge call failed: {0}")]
    JudgeCall(#[from] crate::backend::BackendError),

    #[error("aggregator contract violation: {message}")]
    Contract { message: String },
}

/// Check that the judge backend does not share a family with any candidate.
///
/// When `require_different` is `false`, overlapping families log a warning
/// and return `Ok(())`.
pub fn check_cross_family(
    judge_backend: &str,
    candidates: &[BranchSuccess],
    require_different: bool,
) -> Result<(), LLMJudgeError> {
    let judge_family = family_of(judge_backend);
    for c in candidates {
        let c_family = family_of(&c.backend_id);
        if c_family == judge_family && require_different {
            return Err(LLMJudgeError::FamilyOverlap {
                judge: judge_backend.into(),
                candidate: c.backend_id.clone(),
                family: judge_family.to_string(),
            });
        }
        // When opted out, we silently continue. The caller may log
        // a warning if observability is required.
    }
    Ok(())
}

/// Build the context object used to render the judge prompt template.
#[derive(Debug, Serialize)]
struct BallotContext {
    candidates: Vec<Candidate>,
    phase_name: String,
}

/// Render the judge ballot prompt from the user-supplied template.
pub fn render_ballot_prompt(
    template: &str,
    candidates: &[BranchSuccess],
    ctx: &PhaseContext,
) -> Result<String, LLMJudgeError> {
    let context = BallotContext {
        candidates: candidates
            .iter()
            .map(Candidate::from_branch_success)
            .collect(),
        phase_name: ctx.phase_name.clone(),
    };
    ctx.template_engine
        .render_serde(template, &context)
        .map_err(|err| LLMJudgeError::Contract {
            message: format!("template render failed: {err}"),
        })
}

/// Parse a judge response text into a structured ballot.
///
/// Strips markdown fences before parsing.  Accepts extra keys.
/// Missing or malformed `chosen_index` / `reason` is a contract violation.
pub fn parse_ballot(text: &str) -> Result<Ballot, LLMJudgeError> {
    let stripped = strip_markdown_fences(text.trim());
    let value: serde_json::Value =
        serde_json::from_str(stripped).map_err(|e| LLMJudgeError::Contract {
            message: format!("JSON parse error: {e}"),
        })?;

    let chosen_index = value
        .get("chosen_index")
        .and_then(|v| v.as_u64())
        .map(|u| u as usize)
        .ok_or_else(|| LLMJudgeError::Contract {
            message: "missing or non-integer 'chosen_index'".into(),
        })?;

    let reason = value
        .get("reason")
        .and_then(|v| v.as_str())
        .ok_or_else(|| LLMJudgeError::Contract {
            message: "missing or non-string 'reason'".into(),
        })?;

    Ok(Ballot {
        chosen_index,
        reason: reason.into(),
    })
}

/// Build aggregate text by calling a judge backend and selecting the winning
/// candidate.
pub async fn aggregate_llm_judge(
    candidates: &[BranchSuccess],
    judge_backend: &str,
    prompt_template: &str,
    require_judge_different_family: bool,
    backends: &[Arc<dyn Backend>],
    ctx: &PhaseContext,
) -> Result<AggregatedArtifact, LLMJudgeError> {
    let judge_family = family_of(judge_backend);
    let overlapping_candidates: Vec<&BranchSuccess> = candidates
        .iter()
        .filter(|candidate| family_of(&candidate.backend_id) == judge_family)
        .collect();

    if !overlapping_candidates.is_empty() && require_judge_different_family {
        let candidate = overlapping_candidates[0];
        return Err(LLMJudgeError::FamilyOverlap {
            judge: judge_backend.to_string(),
            candidate: candidate.backend_id.clone(),
            family: judge_family.to_string(),
        });
    }

    let ballot_prompt = render_ballot_prompt(prompt_template, candidates, ctx)?;

    let judge = backends
        .iter()
        .find(|backend| backend.name() == judge_backend)
        .ok_or_else(|| LLMJudgeError::BackendNotFound(judge_backend.to_string()))?;

    let judge_output = judge.query(&ballot_prompt, &ctx.cwd, None).await?;
    let ballot = parse_ballot(&judge_output.stdout)?;

    if candidates.is_empty() {
        return Err(LLMJudgeError::Contract {
            message: "no candidates available for judge decision".into(),
        });
    }

    let index = clamp_chosen_index(ballot.chosen_index, candidates.len());
    let chosen = &candidates[index];

    let mut text = format!(
        "{}\n\n<!-- loker: LLMJudge chose candidate {} ({}) -->\n{}",
        chosen.output, chosen.index, chosen.backend_id, ballot.reason,
    );

    if !overlapping_candidates.is_empty() && !require_judge_different_family {
        text.push_str(&format!(
            "\n<!-- loker: warning judge family '{}' shares family with candidate '{}' but require_judge_different_family=false -->",
            judge_family,
            overlapping_candidates[0].backend_id,
        ));
    }

    Ok(AggregatedArtifact {
        text,
        successful: candidates.len(),
        failed: 0,
    })
}

/// Clamp a chosen index to the valid candidate range.
pub fn clamp_chosen_index(chosen_index: usize, candidate_count: usize) -> usize {
    if candidate_count == 0 {
        0
    } else {
        chosen_index.min(candidate_count - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn success(backend_id: &str, family: &str, index: usize, output: &str) -> BranchSuccess {
        BranchSuccess {
            backend_id: backend_id.into(),
            family: family.into(),
            index,
            output: output.into(),
        }
    }

    #[test]
    fn llm_judge_family_diverse_ok() {
        let candidates = vec![
            success("claude", "anthropic", 1, "a"),
            success("gemini", "google", 2, "b"),
            success("openai", "openai", 3, "c"),
        ];
        assert!(check_cross_family("ollama", &candidates, true).is_ok());
    }

    #[test]
    fn llm_judge_family_overlap_blocks() {
        let candidates = vec![
            success("claude", "anthropic", 1, "a"),
            success("gemini", "google", 2, "b"),
        ];
        let err = check_cross_family("loker_review_anthropic", &candidates, true).unwrap_err();
        assert!(err.to_string().contains("family overlap"));
        assert!(err.to_string().contains("anthropic"));
    }

    #[test]
    fn llm_judge_family_overlap_opt_out_warns() {
        let candidates = vec![success("claude", "anthropic", 1, "a")];
        assert!(check_cross_family("anthropic", &candidates, false).is_ok());
    }

    #[test]
    fn llm_judge_prompt_renders_candidates() {
        let candidates = vec![
            success("claude", "anthropic", 1, "first"),
            success("gemini", "google", 2, "second"),
        ];
        let ctx = PhaseContext::new("review", uuid::Uuid::nil());
        let template = r#"
{% for c in candidates %}
## {{ c.index }} ({{ c.backend_id }} — {{ c.family }})
{{ c.output }}
{% endfor %}
"#;
        let rendered = render_ballot_prompt(template, &candidates, &ctx).unwrap();
        assert!(rendered.contains("## 1 (claude — anthropic)"));
        assert!(rendered.contains("first"));
        assert!(rendered.contains("## 2 (gemini — google)"));
        assert!(rendered.contains("second"));
    }

    #[test]
    fn llm_judge_prompt_includes_phase_name() {
        let candidates = vec![success("claude", "anthropic", 1, "a")];
        let ctx = PhaseContext::new("design", uuid::Uuid::nil());
        let template = "phase: {{ phase_name }}";
        let rendered = render_ballot_prompt(template, &candidates, &ctx).unwrap();
        assert_eq!(rendered.trim(), "phase: design");
    }

    #[test]
    fn llm_judge_parse_valid_ballot() {
        let ballot = parse_ballot(r#"{"chosen_index": 1, "reason": "better"}"#).unwrap();
        assert_eq!(
            ballot,
            Ballot {
                chosen_index: 1,
                reason: "better".into(),
            }
        );
    }

    #[test]
    fn llm_judge_parse_markdown_fenced_ballot() {
        let text = "```json\n{\"chosen_index\":0,\"reason\":\"ok\"}\n```";
        let ballot = parse_ballot(text).unwrap();
        assert_eq!(ballot.chosen_index, 0);
        assert_eq!(ballot.reason, "ok");
    }

    #[test]
    fn llm_judge_parse_missing_chosen_index() {
        let err = parse_ballot(r#"{"reason": "better"}"#).unwrap_err();
        assert!(err
            .to_string()
            .contains("missing or non-integer 'chosen_index'"));
    }

    #[test]
    fn llm_judge_parse_negative_chosen_index() {
        let err = parse_ballot(r#"{"chosen_index": -1, "reason": "better"}"#).unwrap_err();
        assert!(err
            .to_string()
            .contains("missing or non-integer 'chosen_index'"));
    }

    #[test]
    fn llm_judge_parse_out_of_bounds_index() {
        let ballot = parse_ballot(r#"{"chosen_index": 5, "reason": "best"}"#).unwrap();
        assert_eq!(clamp_chosen_index(ballot.chosen_index, 2), 1);
    }

    #[test]
    fn llm_judge_parse_missing_reason() {
        let err = parse_ballot(r#"{"chosen_index": 0}"#).unwrap_err();
        assert!(err.to_string().contains("missing or non-string 'reason'"));
    }

    #[test]
    fn llm_judge_parse_malformed_json() {
        let err = parse_ballot("not json").unwrap_err();
        assert!(err.to_string().contains("JSON parse error"));
    }

    #[test]
    fn llm_judge_parse_within_bounds_index_clamped() {
        assert_eq!(clamp_chosen_index(1, 3), 1);
    }

    #[test]
    fn llm_judge_parse_out_of_bounds_index_clamped() {
        assert_eq!(clamp_chosen_index(5, 3), 2);
    }

    #[test]
    fn llm_judge_parse_zero_candidates_index() {
        assert_eq!(clamp_chosen_index(0, 0), 0);
    }
}
