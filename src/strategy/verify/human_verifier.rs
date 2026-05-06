//! Human verifier hook primitives for HITL scaffolding.
//!
//! `HumanVerifier` emits a pending JSON request under
//! `runs/<run_id>/pending/<phase>.json` and waits for a structured
//! human response at `runs/<run_id>/responses/<phase>.json`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::manifest::Kind;
use crate::run_state::atomic_write;
use crate::strategy::verify::{
    FailureReason, VerifyContext, VerifyError, VerifyHook, VerifyResult,
};

const SCHEMA_VERSION: u32 = 1;

/// Configuration passed to `VerifyHookName::HumanVerifier`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanVerifierConfig {
    pub run_dir: PathBuf,
    pub run_id: String,
    pub workflow: String,
    pub phase: String,
    pub artefact_name: String,
    pub artefact_kind: Kind,
    pub severity: HumanSeverity,
    pub decision_options: Vec<HumanDecision>,
    pub timeout_policy: HumanTimeoutPolicy,
}

impl HumanVerifierConfig {
    pub fn with_default_timeout_policy(mut self) -> Self {
        self.timeout_policy = HumanTimeoutPolicy::default();
        self
    }
}

/// Filesystem-backed HITL hook.
pub struct HumanVerifier {
    pub config: HumanVerifierConfig,
    clock: Arc<dyn HumanClock>,
}

impl HumanVerifier {
    pub fn new(config: HumanVerifierConfig) -> Self {
        Self::new_with_clock(config, Arc::new(SystemHumanClock))
    }

    pub fn new_with_clock(config: HumanVerifierConfig, clock: Arc<dyn HumanClock>) -> Self {
        assert!(
            !config.decision_options.is_empty(),
            "decision_options must not be empty"
        );
        Self { config, clock }
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

    fn pending_payload(
        &self,
        artefact_path: &str,
        artefact_kind: &str,
        prompt_summary: &str,
        preview_lines: u32,
    ) -> Result<PendingRequest, VerifyError> {
        let path = self.pending_path();
        if path.exists() {
            let text = std::fs::read_to_string(&path).map_err(|err| {
                VerifyError::new(format!(
                    "failed to read pending request {}: {err}",
                    path.display()
                ))
            })?;
            let payload: PendingRequest = serde_json::from_str(&text).map_err(|err| {
                VerifyError::new(format!(
                    "failed to parse pending request {}: {err}",
                    path.display()
                ))
            })?;
            if payload.schema_version != SCHEMA_VERSION {
                return Err(VerifyError::new(format!(
                    "unsupported pending schema version for phase {}: expected {}, got {}",
                    self.config.phase, SCHEMA_VERSION, payload.schema_version
                )));
            }
            return Ok(payload);
        }

        let now = self.clock.now();
        let rule = self.config.timeout_policy.rule_for(self.config.severity);
        Ok(PendingRequest {
            schema_version: SCHEMA_VERSION,
            run_id: self.config.run_id.clone(),
            workflow: self.config.workflow.clone(),
            phase: self.config.phase.clone(),
            severity: self.config.severity,
            opened_at: now.to_rfc3339(),
            timeout_at: rule.timeout.map(|timeout| (now + timeout).to_rfc3339()),
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
        })
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

    fn existing_pending_timeout_at(&self) -> Option<String> {
        let path = self.pending_path();
        let text = std::fs::read_to_string(path).ok()?;
        let payload: PendingRequest = serde_json::from_str(&text).ok()?;
        (payload.schema_version == SCHEMA_VERSION)
            .then_some(payload.timeout_at)
            .flatten()
    }

    fn parse_response(&self) -> Result<Option<HumanResponse>, String> {
        let path = self.response_path();
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|err| format!("failed to read response file {}: {err}", path.display()))?;
        let response: HumanResponse = serde_json::from_str(&text)
            .map_err(|err| format!("failed to parse response file {}: {err}", path.display()))?;
        if response.schema_version != SCHEMA_VERSION {
            return Err(format!(
                "unsupported schema version: expected {}, got {}",
                SCHEMA_VERSION, response.schema_version
            ));
        }
        Ok(Some(response))
    }

    fn consume_response(&self, suffix: &str) -> Result<(), VerifyError> {
        let source = self.response_path();
        if !source.exists() {
            return Ok(());
        }
        let destination = source.with_file_name(format!(
            "{}.json.{}.{}",
            self.config.phase,
            suffix,
            self.clock.now().timestamp_millis()
        ));

        if let Some(dir) = destination.parent() {
            std::fs::create_dir_all(dir).map_err(|err| {
                VerifyError::new(format!(
                    "failed to prepare response archive dir {}: {err}",
                    dir.display()
                ))
            })?;
        }

        std::fs::rename(&source, &destination).map_err(|err| {
            VerifyError::new(format!(
                "failed to consume response for {}: {err}",
                self.config.phase
            ))
        })
    }

    pub async fn verify_with_report(
        &self,
        ctx: &VerifyContext,
    ) -> Result<(VerifyResult, HumanVerifyReport), VerifyError> {
        let preview_lines = u32::try_from(ctx.stdout.lines().count()).unwrap_or(u32::MAX);
        let summary = ctx.stdout.chars().take(160).collect::<String>();
        let default_report = |timeout_at: Option<String>| {
            HumanVerifyReport::from_policy(
                self.config.severity,
                timeout_at,
                self.config
                    .timeout_policy
                    .rule_for(self.config.severity)
                    .on_timeout,
                HumanTimeoutOutcome::NotTimedOut,
            )
        };

        let decision_or_pending = match self.parse_response() {
            Ok(Some(response)) => Some(response),
            Ok(None) => None,
            Err(err) => {
                let payload = self.pending_payload(
                    &self.config.artefact_name,
                    kind_str(&self.config.artefact_kind),
                    &summary,
                    preview_lines,
                )?;
                self.ensure_pending_file(&payload)?;
                let reason = FailureReason::new(format!(
                    "malformed or missing human response for {}: {}",
                    self.config.phase, err
                ));
                return Ok((
                    VerifyResult::Fail { reason },
                    default_report(payload.timeout_at.clone()),
                ));
            }
        };

        match decision_or_pending {
            Some(response) => {
                if response.phase != self.config.phase {
                    let payload = self.pending_payload(
                        &self.config.artefact_name,
                        kind_str(&self.config.artefact_kind),
                        &summary,
                        preview_lines,
                    )?;
                    self.ensure_pending_file(&payload)?;
                    let reason = FailureReason::new(format!(
                        "response phase mismatch for {}: expected {}, got {}",
                        self.config.phase, self.config.phase, response.phase
                    ));
                    return Ok((
                        VerifyResult::Fail { reason },
                        default_report(payload.timeout_at.clone()),
                    ));
                }

                self.consume_response("handled")?;

                let timeout_at = self.existing_pending_timeout_at();
                let result = match response.decision {
                    HumanDecision::Approve => VerifyResult::pass(),
                    HumanDecision::Reject => VerifyResult::fail(format!(
                        "human rejected phase {}: {}",
                        self.config.phase,
                        response.global_comment.unwrap_or_default()
                    )),
                    HumanDecision::CommentOnly => {
                        VerifyResult::fail("human comment_only is not treated as approval")
                    }
                };
                Ok((result, default_report(timeout_at)))
            }
            None => {
                let payload = self.pending_payload(
                    &self.config.artefact_name,
                    kind_str(&self.config.artefact_kind),
                    &summary,
                    preview_lines,
                )?;
                self.ensure_pending_file(&payload)?;
                let rule = self.config.timeout_policy.rule_for(payload.severity);
                let timed_out = match payload.timeout_at.as_deref() {
                    Some(raw) => match DateTime::parse_from_rfc3339(raw) {
                        Ok(deadline) => self.clock.now() >= deadline.with_timezone(&Utc),
                        Err(err) => {
                            let report = HumanVerifyReport::from_policy(
                                payload.severity,
                                payload.timeout_at.clone(),
                                rule.on_timeout,
                                HumanTimeoutOutcome::NotTimedOut,
                            );
                            return Ok((
                                VerifyResult::fail(format!(
                                    "invalid human review timeout_at for {}: {}",
                                    self.config.phase, err
                                )),
                                report,
                            ));
                        }
                    },
                    None => false,
                };

                let outcome = if timed_out {
                    match rule.on_timeout {
                        HumanTimeoutAction::AutoApprove => HumanTimeoutOutcome::AutoApproved,
                        HumanTimeoutAction::AutoFail => HumanTimeoutOutcome::AutoFailed,
                        HumanTimeoutAction::Block => HumanTimeoutOutcome::Blocking,
                    }
                } else {
                    HumanTimeoutOutcome::NotTimedOut
                };
                let report = HumanVerifyReport::from_policy(
                    payload.severity,
                    payload.timeout_at.clone(),
                    rule.on_timeout,
                    outcome,
                );

                let result = match outcome {
                    HumanTimeoutOutcome::AutoApproved => VerifyResult::pass(),
                    HumanTimeoutOutcome::AutoFailed => VerifyResult::fail(format!(
                        "human review timed out for {} with severity {}",
                        self.config.phase,
                        payload.severity.as_str()
                    )),
                    HumanTimeoutOutcome::NotTimedOut | HumanTimeoutOutcome::Blocking => {
                        VerifyResult::fail(format!(
                            "waiting for human review on {}",
                            self.config.phase
                        ))
                    }
                };
                Ok((result, report))
            }
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

impl HumanSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanTimeoutAction {
    AutoApprove,
    AutoFail,
    Block,
}

impl HumanTimeoutAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AutoApprove => "auto_approve",
            Self::AutoFail => "auto_fail",
            Self::Block => "block",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanTimeoutRule {
    pub timeout: Option<Duration>,
    pub on_timeout: HumanTimeoutAction,
}

impl HumanTimeoutRule {
    pub fn low_default() -> Self {
        Self {
            timeout: Some(Duration::hours(1)),
            on_timeout: HumanTimeoutAction::AutoApprove,
        }
    }

    pub fn medium_default() -> Self {
        Self {
            timeout: Some(Duration::hours(24)),
            on_timeout: HumanTimeoutAction::AutoFail,
        }
    }

    pub fn high_default() -> Self {
        Self {
            timeout: None,
            on_timeout: HumanTimeoutAction::Block,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanTimeoutPolicy {
    pub low: HumanTimeoutRule,
    pub medium: HumanTimeoutRule,
    pub high: HumanTimeoutRule,
}

impl Default for HumanTimeoutPolicy {
    fn default() -> Self {
        Self {
            low: HumanTimeoutRule::low_default(),
            medium: HumanTimeoutRule::medium_default(),
            high: HumanTimeoutRule::high_default(),
        }
    }
}

impl HumanTimeoutPolicy {
    pub fn rule_for(&self, severity: HumanSeverity) -> &HumanTimeoutRule {
        match severity {
            HumanSeverity::Low => &self.low,
            HumanSeverity::Medium => &self.medium,
            HumanSeverity::High => &self.high,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanTimeoutOutcome {
    NotTimedOut,
    AutoApproved,
    AutoFailed,
    Blocking,
}

impl HumanTimeoutOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotTimedOut => "not_timed_out",
            Self::AutoApproved => "auto_approved",
            Self::AutoFailed => "auto_failed",
            Self::Blocking => "blocking",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanVerifyReport {
    pub severity: HumanSeverity,
    pub timeout_at: Option<String>,
    pub timeout_action: HumanTimeoutAction,
    pub timeout_outcome: HumanTimeoutOutcome,
}

impl HumanVerifyReport {
    fn from_policy(
        severity: HumanSeverity,
        timeout_at: Option<String>,
        timeout_action: HumanTimeoutAction,
        timeout_outcome: HumanTimeoutOutcome,
    ) -> Self {
        Self {
            severity,
            timeout_at,
            timeout_action,
            timeout_outcome,
        }
    }
}

pub trait HumanClock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemHumanClock;

impl HumanClock for SystemHumanClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
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
        let (result, _) = self.verify_with_report(ctx).await?;
        Ok(result)
    }
}

fn kind_str(kind: &Kind) -> &'static str {
    match kind {
        Kind::DesignMd | Kind::ReviewMd => "text/markdown",
        Kind::VerifyJson
        | Kind::PhaseResultJson
        | Kind::PendingJson
        | Kind::ResponseJson
        | Kind::SummaryJson
        | Kind::TraceJsonl => "application/json",
        Kind::ChangesDir => "application/directory",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use std::fs;

    struct FixedClock(DateTime<Utc>);

    impl HumanClock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }
    fn hook(tmp: &tempfile::TempDir, severity: HumanSeverity) -> HumanVerifier {
        HumanVerifier::new(HumanVerifierConfig {
            run_dir: tmp.path().to_path_buf(),
            run_id: Utc::now().to_rfc3339(),
            workflow: "design-doc-tdd".into(),
            phase: "review".into(),
            artefact_name: "review.md".into(),
            artefact_kind: Kind::ReviewMd,
            severity,
            decision_options: vec![
                HumanDecision::Approve,
                HumanDecision::Reject,
                HumanDecision::CommentOnly,
            ],
            timeout_policy: HumanTimeoutPolicy::default(),
        })
    }

    fn hook_at(
        tmp: &tempfile::TempDir,
        severity: HumanSeverity,
        now: DateTime<Utc>,
        policy: HumanTimeoutPolicy,
    ) -> HumanVerifier {
        HumanVerifier::new_with_clock(
            HumanVerifierConfig {
                run_dir: tmp.path().to_path_buf(),
                run_id: "run-1".into(),
                workflow: "design-doc-tdd".into(),
                phase: "review".into(),
                artefact_name: "review.md".into(),
                artefact_kind: Kind::ReviewMd,
                severity,
                decision_options: vec![
                    HumanDecision::Approve,
                    HumanDecision::Reject,
                    HumanDecision::CommentOnly,
                ],
                timeout_policy: policy,
            },
            Arc::new(FixedClock(now)),
        )
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

    fn write_response(path: &PathBuf, phase: &str, decision: HumanDecision, comment: Option<&str>) {
        let response = HumanResponse {
            schema_version: SCHEMA_VERSION,
            phase: phase.into(),
            claimed_by: "human".into(),
            decided_at: "2026-05-04T00:00:00Z".into(),
            decision,
            global_comment: comment.map(ToString::to_string),
            inline_comments_path: None,
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, serde_json::to_vec_pretty(&response).unwrap()).unwrap();
    }

    #[test]
    fn default_policy_matches_severity_ladder() {
        let policy = HumanTimeoutPolicy::default();
        assert_eq!(
            policy.rule_for(HumanSeverity::Low),
            &HumanTimeoutRule {
                timeout: Some(Duration::hours(1)),
                on_timeout: HumanTimeoutAction::AutoApprove,
            }
        );
        assert_eq!(
            policy.rule_for(HumanSeverity::Medium),
            &HumanTimeoutRule {
                timeout: Some(Duration::hours(24)),
                on_timeout: HumanTimeoutAction::AutoFail,
            }
        );
        assert_eq!(
            policy.rule_for(HumanSeverity::High),
            &HumanTimeoutRule {
                timeout: None,
                on_timeout: HumanTimeoutAction::Block,
            }
        );
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
        assert_eq!(
            serde_json::from_value::<HumanResponse>(response_json).unwrap(),
            response
        );
    }

    #[test]
    fn config_builds_paths() {
        let hook = HumanVerifier::new(HumanVerifierConfig {
            run_dir: PathBuf::from("/tmp/run"),
            run_id: "run-1".into(),
            workflow: "wf".into(),
            phase: "review".into(),
            artefact_name: "review.md".into(),
            artefact_kind: Kind::ReviewMd,
            severity: HumanSeverity::Medium,
            decision_options: vec![HumanDecision::Approve],
            timeout_policy: HumanTimeoutPolicy::default(),
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
        let result = hook
            .verify(&context_with_output("candidate output"))
            .await
            .unwrap();

        assert!(matches!(result, VerifyResult::Fail { .. }));
        assert!(tmp.path().join("pending").join("review.json").is_file());

        let text = fs::read_to_string(tmp.path().join("pending/review.json")).unwrap();
        let request: PendingRequest = serde_json::from_str(&text).unwrap();
        assert_eq!(request.phase, "review");
        assert_eq!(request.artefact.path, "review.md");
        assert_eq!(request.artefact.kind, "text/markdown");
    }

    #[tokio::test]
    async fn missing_low_response_after_timeout_auto_approves() {
        let tmp = tempfile::tempdir().unwrap();
        let t0 = Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap();
        let hook = hook_at(&tmp, HumanSeverity::Low, t0, HumanTimeoutPolicy::default());
        let first = hook
            .verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(first.0.is_fail());
        assert_eq!(first.1.timeout_outcome, HumanTimeoutOutcome::NotTimedOut);

        let later = hook_at(
            &tmp,
            HumanSeverity::Low,
            t0 + Duration::minutes(61),
            HumanTimeoutPolicy::default(),
        );
        let (result, report) = later
            .verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(result.is_pass());
        assert_eq!(report.timeout_outcome, HumanTimeoutOutcome::AutoApproved);
        assert_eq!(report.timeout_action, HumanTimeoutAction::AutoApprove);
    }

    #[tokio::test]
    async fn missing_medium_response_after_timeout_auto_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let t0 = Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap();
        let hook = hook_at(
            &tmp,
            HumanSeverity::Medium,
            t0,
            HumanTimeoutPolicy::default(),
        );
        hook.verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();

        let later = hook_at(
            &tmp,
            HumanSeverity::Medium,
            t0 + Duration::hours(25),
            HumanTimeoutPolicy::default(),
        );
        let (result, report) = later
            .verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(result.is_fail());
        assert_eq!(report.timeout_outcome, HumanTimeoutOutcome::AutoFailed);
        if let VerifyResult::Fail { reason } = result {
            assert!(reason.summary.contains("timed out"));
        }
    }

    #[tokio::test]
    async fn high_response_missing_blocks_indefinitely() {
        let tmp = tempfile::tempdir().unwrap();
        let t0 = Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap();
        let hook = hook_at(&tmp, HumanSeverity::High, t0, HumanTimeoutPolicy::default());
        let (result, report) = hook
            .verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(result.is_fail());
        assert_eq!(report.timeout_at, None);

        let later = hook_at(
            &tmp,
            HumanSeverity::High,
            t0 + Duration::days(365),
            HumanTimeoutPolicy::default(),
        );
        let (result, report) = later
            .verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(result.is_fail());
        assert_eq!(report.timeout_outcome, HumanTimeoutOutcome::NotTimedOut);
    }

    #[tokio::test]
    async fn existing_pending_deadline_is_stable() {
        let tmp = tempfile::tempdir().unwrap();
        let t0 = Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap();
        let hook = hook_at(&tmp, HumanSeverity::Low, t0, HumanTimeoutPolicy::default());
        hook.verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();
        let original = fs::read_to_string(tmp.path().join("pending/review.json")).unwrap();
        let original: PendingRequest = serde_json::from_str(&original).unwrap();

        let later = hook_at(
            &tmp,
            HumanSeverity::Low,
            t0 + Duration::minutes(30),
            HumanTimeoutPolicy::default(),
        );
        later
            .verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();
        let rewritten = fs::read_to_string(tmp.path().join("pending/review.json")).unwrap();
        let rewritten: PendingRequest = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(rewritten.opened_at, original.opened_at);
        assert_eq!(rewritten.timeout_at, original.timeout_at);
    }

    #[tokio::test]
    async fn maps_approve_reject_comment_only() {
        let tmp = tempfile::tempdir().unwrap();
        let hook = hook(&tmp, HumanSeverity::High);

        let response_path = hook.response_path();

        write_response(&response_path, "review", HumanDecision::Approve, None);
        let ok = hook
            .verify(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(ok.is_pass());

        write_response(
            &response_path,
            "review",
            HumanDecision::Reject,
            Some("Needs additional context around security section"),
        );
        let fail = hook
            .verify(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(fail.is_fail());

        if let VerifyResult::Fail { reason } = fail {
            assert!(reason.summary.contains("human rejected"));
        } else {
            panic!("expected fail");
        }

        write_response(
            &response_path,
            "review",
            HumanDecision::CommentOnly,
            Some("Looks okay as next step"),
        );
        let pending = hook
            .verify(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(pending.is_fail());
    }

    #[tokio::test]
    async fn phase_mismatch_keeps_response_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let hook = hook(&tmp, HumanSeverity::Medium);
        let response_path = hook.response_path();

        write_response(&response_path, "design", HumanDecision::Approve, None);
        let mismatch = hook
            .verify(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(mismatch.is_fail());

        // original response file should still exist (not consumed on mismatch)
        assert!(response_path.exists());
    }

    #[tokio::test]
    async fn consumes_response_after_successful_parse() {
        let tmp = tempfile::tempdir().unwrap();
        let hook = hook(&tmp, HumanSeverity::Medium);
        let response_path = hook.response_path();

        write_response(&response_path, "review", HumanDecision::Approve, None);
        let ok = hook
            .verify(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(ok.is_pass());

        assert!(!response_path.exists());
        let response_dir = tmp.path().join("responses");
        let consumed = response_dir
            .read_dir()
            .unwrap()
            .filter_map(|entry| entry.ok())
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .to_string()
                    .contains("review.json.handled.")
            });
        assert!(consumed);
    }

    #[tokio::test]
    async fn existing_pending_schema_version_is_validated() {
        let tmp = tempfile::tempdir().unwrap();
        let t0 = Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap();
        let hook = hook_at(&tmp, HumanSeverity::Low, t0, HumanTimeoutPolicy::default());
        let path = tmp.path().join("pending/review.json");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            serde_json::json!({
                "schema_version": 99,
                "run_id": "run-1",
                "workflow": "wf",
                "phase": "review",
                "severity": "low",
                "opened_at": "2026-05-06T12:00:00Z",
                "timeout_at": "2026-05-06T13:00:00Z",
                "artefact": {"path":"review.md", "kind":"text/markdown", "preview_lines":1},
                "context": {"preceded_by": [], "next_phase": null, "prompt_summary":"x"},
                "decision_options": ["approve"]
            })
            .to_string(),
        )
        .unwrap();

        let err = hook
            .verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap_err();
        assert!(err
            .to_string()
            .contains("unsupported pending schema version"));
    }

    #[tokio::test]
    async fn invalid_pending_timeout_at_fails_clearly() {
        let tmp = tempfile::tempdir().unwrap();
        let t0 = Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap();
        let hook = hook_at(&tmp, HumanSeverity::Low, t0, HumanTimeoutPolicy::default());
        hook.verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();
        let path = tmp.path().join("pending/review.json");
        let mut pending: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        pending["timeout_at"] = serde_json::Value::String("not-a-date".into());
        fs::write(&path, serde_json::to_vec_pretty(&pending).unwrap()).unwrap();

        let (result, report) = hook
            .verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(result.is_fail());
        assert_eq!(report.timeout_at.as_deref(), Some("not-a-date"));
        if let VerifyResult::Fail { reason } = result {
            assert!(reason.summary.contains("invalid human review timeout_at"));
        }
    }

    #[tokio::test]
    async fn explicit_response_report_preserves_existing_timeout_at() {
        let tmp = tempfile::tempdir().unwrap();
        let t0 = Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap();
        let hook = hook_at(&tmp, HumanSeverity::Low, t0, HumanTimeoutPolicy::default());
        hook.verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();
        let pending: PendingRequest = serde_json::from_str(
            &fs::read_to_string(tmp.path().join("pending/review.json")).unwrap(),
        )
        .unwrap();
        write_response(
            &hook.response_path(),
            "review",
            HumanDecision::Approve,
            None,
        );

        let (result, report) = hook
            .verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(result.is_pass());
        assert_eq!(report.timeout_at, pending.timeout_at);
    }

    #[tokio::test]
    async fn malformed_response_report_preserves_timeout_at() {
        let tmp = tempfile::tempdir().unwrap();
        let t0 = Utc.with_ymd_and_hms(2026, 5, 6, 12, 0, 0).unwrap();
        let hook = hook_at(&tmp, HumanSeverity::Low, t0, HumanTimeoutPolicy::default());
        hook.verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();
        let expected: PendingRequest = serde_json::from_str(
            &fs::read_to_string(tmp.path().join("pending/review.json")).unwrap(),
        )
        .unwrap();

        let response_path = hook.response_path();
        fs::create_dir_all(response_path.parent().unwrap()).unwrap();
        fs::write(&response_path, b"{not valid json}").unwrap();

        let (_result, report) = hook
            .verify_with_report(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert_eq!(report.timeout_at, expected.timeout_at);
    }

    #[tokio::test]
    async fn keeps_pending_on_malformed_response() {
        let tmp = tempfile::tempdir().unwrap();
        let hook = hook(&tmp, HumanSeverity::Medium);
        let response_path = hook.response_path();

        if let Some(parent) = response_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&response_path, b"{not valid json}").unwrap();

        let pending = hook
            .verify(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(pending.is_fail());
        assert!(tmp.path().join("pending/review.json").is_file());
    }

    #[tokio::test]
    async fn rejects_response_with_unexpected_decision() {
        let tmp = tempfile::tempdir().unwrap();
        let hook = HumanVerifier::new(HumanVerifierConfig {
            run_dir: tmp.path().to_path_buf(),
            run_id: "run-1".into(),
            workflow: "wf".into(),
            phase: "review".into(),
            artefact_name: "review.md".into(),
            artefact_kind: Kind::ReviewMd,
            severity: HumanSeverity::Medium,
            decision_options: vec![HumanDecision::Approve],
            timeout_policy: HumanTimeoutPolicy::default(),
        });
        let response_path = hook.response_path();

        write_response(
            &response_path,
            "review",
            HumanDecision::Reject,
            Some("nope"),
        );
        let fail = hook
            .verify(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(fail.is_fail());
    }

    #[tokio::test]
    async fn rejects_response_with_unknown_schema_version() {
        let tmp = tempfile::tempdir().unwrap();
        let hook = hook(&tmp, HumanSeverity::Medium);
        let response_path = hook.response_path();

        let response = HumanResponse {
            schema_version: 99,
            phase: "review".into(),
            claimed_by: "human".into(),
            decided_at: "2026-05-04T00:00:00Z".into(),
            decision: HumanDecision::Approve,
            global_comment: None,
            inline_comments_path: None,
        };
        if let Some(parent) = response_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(
            &response_path,
            serde_json::to_vec_pretty(&response).unwrap(),
        )
        .unwrap();

        let pending = hook
            .verify(&context_with_output("candidate output"))
            .await
            .unwrap();
        assert!(pending.is_fail());
        assert!(tmp.path().join("pending/review.json").is_file());
    }
}
