//! Human verifier hook primitives for HITL scaffolding.
//!
//! `HumanVerifier` emits a pending JSON request under
//! `runs/<run_id>/pending/<phase>.json` and waits for a structured
//! human response at `runs/<run_id>/responses/<phase>.json`.
//!
//! This file contains the scaffolding and payload models for the hook.
//! Verification behavior is implemented in later sub-tasks.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::strategy::verify::{VerifyContext, VerifyError, VerifyHook, VerifyResult};

/// Configuration passed to `VerifyHookName::HumanVerifier`.
#[derive(Debug, Clone)]
pub struct HumanVerifierConfig {
    pub run_dir: PathBuf,
    pub run_id: String,
    pub workflow: String,
    pub phase: String,
    pub severity: HumanSeverity,
    pub decision_options: Vec<HumanDecision>,
}

/// Filesystem-backed HITL hook.
#[derive(Debug, Clone)]
pub struct HumanVerifier {
    pub config: HumanVerifierConfig,
}

impl HumanVerifier {
    pub fn new(config: HumanVerifierConfig) -> Self {
        Self { config }
    }

    pub fn pending_path(&self) -> PathBuf {
        self.config.run_dir.join("pending").join(format!("{}.json", self.config.phase))
    }

    pub fn response_path(&self) -> PathBuf {
        self.config.run_dir.join("responses").join(format!("{}.json", self.config.phase))
    }

    fn ensure_pending_payload(
        &self,
        artefact_path: &str,
        artefact_kind: &str,
        prompt_summary: &str,
        preview_lines: u32,
    ) -> PendingRequest {
        PendingRequest {
            schema_version: 1,
            run_id: self.config.run_id.clone(),
            workflow: self.config.workflow.clone(),
            phase: self.config.phase.clone(),
            severity: self.config.severity,
            opened_at: String::new(),
            timeout_at: None,
            artefact: PendingArtefact {
                path: artefact_path.to_string(),
                kind: artefact_kind.to_string(),
                preview_lines,
            },
            context: PendingContext {
                preceded_by: Vec::new(),
                next_phase: None,
                prompt_summary: prompt_summary.to_string(),
            },
            decision_options: self.config.decision_options.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HumanSeverity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HumanDecision {
    Approve,
    Reject,
    #[serde(rename = "comment_only")]
    CommentOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRequest {
    pub schema_version: u32,
    pub run_id: String,
    pub workflow: String,
    pub phase: String,
    pub severity: HumanSeverity,
    pub opened_at: String,
    pub timeout_at: Option<String>,
    pub artefact: PendingArtefact,
    pub context: PendingContext,
    pub decision_options: Vec<HumanDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingContext {
    pub preceded_by: Vec<String>,
    pub next_phase: Option<String>,
    pub prompt_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingArtefact {
    pub path: String,
    pub kind: String,
    pub preview_lines: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanResponse {
    pub schema_version: u32,
    pub phase: String,
    pub claimed_by: String,
    pub decided_at: String,
    pub decision: HumanDecision,
    pub global_comment: Option<String>,
    pub inline_comments_path: Option<String>,
}

#[async_trait]
impl VerifyHook for HumanVerifier {
    fn name(&self) -> &str {
        "HumanVerifier"
    }

    async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
        Err(VerifyError::new(
            "human verifier behavior is scaffolding-only before ST2 implementation",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn types_roundtrip_and_defaults() {
        let payload = PendingRequest {
            schema_version: 1,
            run_id: "design-doc-tdd-2026-04-25-1437-a7c3".into(),
            workflow: "design-doc-tdd".into(),
            phase: "review".into(),
            severity: HumanSeverity::High,
            opened_at: "2026-04-25T14:42:11Z".into(),
            timeout_at: None,
            artefact: PendingArtefact {
                path: "review.md".into(),
                kind: "text/markdown".into(),
                preview_lines: 17,
            },
            context: PendingContext {
                preceded_by: vec!["design".into()],
                next_phase: Some("implement".into()),
                prompt_summary: "candidate output preview".into(),
            },
            decision_options: vec![HumanDecision::Approve, HumanDecision::Reject],
        };

        let response = HumanResponse {
            schema_version: 1,
            phase: "review".into(),
            claimed_by: "anon".into(),
            decided_at: "2026-04-25T14:42:11Z".into(),
            decision: HumanDecision::Approve,
            global_comment: None,
            inline_comments_path: None,
        };

        assert_eq!(serde_json::to_value(payload).unwrap(), json!({
            "schema_version":1,
            "run_id":"design-doc-tdd-2026-04-25-1437-a7c3",
            "workflow":"design-doc-tdd",
            "phase":"review",
            "severity":"high",
            "opened_at":"2026-04-25T14:42:11Z",
            "timeout_at":null,
            "artefact": {"path":"review.md","kind":"text/markdown","preview_lines":17},
            "context":{"preceded_by":["design"],"next_phase":"implement","prompt_summary":"candidate output preview"},
            "decision_options":["approve","reject"]
        }));

        let response_json = serde_json::to_value(response.clone()).unwrap();
        assert_eq!(response_json["decision"], "approve");
        assert_eq!(serde_json::from_value::<HumanResponse>(response_json).unwrap(), response);
    }

    #[test]
    fn config_builds_paths() {
        let hook = HumanVerifier::new(HumanVerifierConfig {
            run_dir: PathBuf::from("/tmp/run"),
            run_id: "run-1".into(),
            workflow: "wf".into(),
            phase: "review".into(),
            severity: HumanSeverity::Medium,
            decision_options: vec![HumanDecision::Approve],
        });

        assert_eq!(
            hook.pending_path(),
            PathBuf::from("/tmp/run")
                .join("pending")
                .join("review.json")
        );
        assert_eq!(
            hook.response_path(),
            PathBuf::from("/tmp/run")
                .join("responses")
                .join("review.json")
        );
    }
}
