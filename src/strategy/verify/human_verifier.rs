//! Human verifier hook primitives for HITL scaffolding.
//!
//! `HumanVerifier` emits a pending JSON request under
//! `runs/<run_id>/pending/<phase>.json` and waits for a structured
//! human response at `runs/<run_id>/responses/<phase>.json`.

use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::run_state::atomic_write;
use crate::strategy::verify::{FailureReason, VerifyContext, VerifyError, VerifyHook, VerifyResult};

const SCHEMA_VERSION: u32 = 1;

/// Configuration passed to `VerifyHookName::HumanVerifier`.
pub struct HumanVerifierConfig {
    pub run_dir: PathBuf,
    pub run_id: String,
    pub workflow: String,
    pub phase: String,
    pub severity: HumanSeverity,
    pub decision_options: Vec<HumanDecision>,
}

/// Filesystem-backed HITL hook.
pub struct HumanVerifier {
    pub config: HumanVerifierConfig,
}

impl HumanVerifier {
    pub fn new(config: HumanVerifierConfig) -> Self {
        Self { config }
    }

    pub fn pending_path(&self) -> PathBuf {
        self.config
            .run_dir
            .join("pending")
            .join(format!("{}.json", self.config.phase))
    }

    pub fn response_path(&self) -> PathBuf {
        self.config
            .run_dir
            .join("responses")
            .join(format!("{}.json", self.config.phase))
    }

    fn ensure_pending_payload(
        &self,
        artefact_path: &str,
        artefact_kind: &str,
        prompt_summary: &str,
        preview_lines: u32,
    ) -> PendingRequest {
        let now = Utc::now();
        PendingRequest {
            schema_version: SCHEMA_VERSION,
            run_id: self.config.run_id.clone(),
            workflow: self.config.workflow.clone(),
            phase: self.config.phase.clone(),
            severity: self.config.severity,
            opened_at: now.to_rfc3339(),
            timeout_at: timeout_from(now, self.config.severity),
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

    fn ensure_pending_file(&self, payload: &PendingRequest) -> Result<(), VerifyError> {
        let path = self.pending_path();
        if path.exists() {
            return Ok(());
        }

        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|err| {
                VerifyError::new(format!(
                    "failed to create pending directory for phase {}: {err}",
                    self.config.phase
                ))
            })?;
        }

        let payload = serde_json::to_string_pretty(payload).map_err(|err| {
            VerifyError::new(format!(
                "failed to serialize pending request for phase {}: {err}",
                self.config.phase
            ))
        })?;

        atomic_write(&path, payload.as_bytes()).map_err(|err| {
            VerifyError::new(format!(
                "failed to write pending request for phase {}: {err}",
                self.config.phase
            ))
        })
    }

    fn parse_response(&self) -> Result<Option<HumanResponse>, VerifyError> {
        let path = self.response_path();
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path).map_err(|err| {
            VerifyError::new(format!("failed to read response file {}: {err}", path.display()))
        })?;
        let response: HumanResponse = serde_json::from_str(&text).map_err(|err| {
            VerifyError::new(format!("failed to parse response file {}: {err}", path.display()))
        })?;
        Ok(Some(response))
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

    async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
        let preview_lines = u32::try_from(ctx.stdout.lines().count()).unwrap_or(u32::MAX);
        let summary = ctx.stdout.chars().take(160).collect::<String>();

        match self.parse_response()? {
            Some(response) => {
                if response.phase != self.config.phase {
                    return Err(VerifyError::new(format!(
                        "response phase mismatch for {}: expected {}",
                        self.config.phase, response.phase
                    )));
                }

                match response.decision {
                    HumanDecision::Approve => Ok(VerifyResult::pass()),
                    HumanDecision::Reject => Ok(VerifyResult::fail(format!(
                        "human rejected phase {}: {}",
                        self.config.phase,
                        response.global_comment.unwrap_or_default()
                    ))),
                    HumanDecision::CommentOnly => {
                        Ok(VerifyResult::fail("human comment_only is not treated as approval"))
                    }
                }
            }
            None => {
                let payload = self.ensure_pending_payload(
                    &self.config.phase,
                    "text/plain",
                    &summary,
                    preview_lines,
                );
                self.ensure_pending_file(&payload)?;
                let reason = FailureReason::new(format!(
                    "waiting for human review on {}",
                    self.config.phase
                ));
                Ok(VerifyResult::Fail { reason })
            }
        }
    }
}

fn timeout_from(now: DateTime<Utc>, severity: HumanSeverity) -> Option<String> {
    let timeout = match severity {
        HumanSeverity::Low => Some(now + Duration::hours(1)),
        HumanSeverity::Medium => Some(now + Duration::hours(24)),
        HumanSeverity::High => None,
    };
    timeout.map(|t| t.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::fs;
    use std::io::Write;

    fn hook(tmp: &tempfile::TempDir, severity: HumanSeverity) -> HumanVerifier {
        HumanVerifier::new(HumanVerifierConfig {
            run_dir: tmp.path().to_path_buf(),
            run_id: Utc::now().to_rfc3339(),
            workflow: "design-doc-tdd".into(),
            phase: "review".into(),
            severity,
            decision_options: vec![
                HumanDecision::Approve,
                HumanDecision::Reject,
                HumanDecision::CommentOnly,
            ],
        })
    }

    fn context_with_output(output: &str) -> VerifyContext {
        VerifyContext {
            stdout: output.to_string(),
            stderr: None,
            exit_code: None,
            backend_name: "mock".into(),
            model: None,
            structured: None,
            duration: std::time::Duration::ZERO,
        }
    }

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

        let payload_json = serde_json::to_value(payload.clone()).unwrap();
        assert_eq!(payload_json["severity"], "high");

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

    #[tokio::test]
    async fn returns_fail_when_response_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let hook = hook(&tmp, HumanSeverity::High);
        let result = hook.verify(&context_with_output("candidate output")).await.unwrap();

        assert!(matches!(result, VerifyResult::Fail { .. }));
        assert!(tmp
            .path()
            .join("pending")
            .join("review.json")
            .is_file());

        let text = fs::read_to_string(tmp.path().join("pending/review.json")).unwrap();
        let request: PendingRequest = serde_json::from_str(&text).unwrap();
        assert_eq!(request.phase, "review");
    }

    #[tokio::test]
    async fn maps_approve_reject_comment_only() {
        let tmp = tempfile::tempdir().unwrap();
        let hook = hook(&tmp, HumanSeverity::High);

        let response_path = hook.response_path();
        if let Some(parent) = response_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }

        let write_response = |decision: HumanDecision, comment: Option<&str>| {
            let response = HumanResponse {
                schema_version: SCHEMA_VERSION,
                phase: "review".into(),
                claimed_by: "human".into(),
                decided_at: "2026-05-04T00:00:00Z".into(),
                decision,
                global_comment: comment.map(ToString::to_string),
                inline_comments_path: None,
            };
            let mut f = fs::File::create(&response_path).unwrap();
            f.write_all(serde_json::to_string_pretty(&response).unwrap().as_bytes())
                .unwrap();
        };

        write_response(HumanDecision::Approve, None);
        let ok = hook.verify(&context_with_output("candidate output")).await.unwrap();
        assert!(ok.is_pass());

        write_response(
            HumanDecision::Reject,
            Some("Needs additional context around security section"),
        );
        let fail = hook.verify(&context_with_output("candidate output")).await.unwrap();
        assert!(fail.is_fail());

        if let VerifyResult::Fail { reason } = fail {
            assert!(reason.summary.contains("human rejected"));
        } else {
            panic!("expected fail");
        }

        write_response(HumanDecision::CommentOnly, Some("Looks okay as next step"));
        let pending = hook.verify(&context_with_output("candidate output")).await.unwrap();
        assert!(pending.is_fail());
    }
}
