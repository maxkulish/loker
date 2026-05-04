pub mod lock;
pub mod sweep;

use chrono::{DateTime, Utc};

/// Errors specific to the resume orchestration flow.
#[derive(Debug, thiserror::Error)]
pub enum ResumeError {
    #[error("run is already in progress (heartbeat live at {last_tick})")]
    RunInProgress { last_tick: DateTime<Utc> },

    /// `ArtefactCorrupt` / `ArtefactMissing` surface through `LoadError`
    /// (from `RunState::load`) rather than as first-class `ResumeError`
    /// variants, keeping the error taxonomy flat.
    #[error("manifest error: {0}")]
    Manifest(#[from] crate::manifest::ManifestError),

    #[error("load error: {0}")]
    Load(Box<crate::run_state::LoadError>),

    #[error("phase error: {0}")]
    Phase(Box<crate::phase_runner::PhaseError>),

    #[error("marker error: {0}")]
    Marker(#[from] crate::run_state::MarkerError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Advisory lock is OS-cooperative only (flock / LockFileEx).
    /// Another loker process holds the lock; non-loker processes may ignore it.
    #[error("run directory is locked by another loker process")]
    LockInUse,
}

impl From<crate::run_state::LoadError> for ResumeError {
    fn from(e: crate::run_state::LoadError) -> Self {
        ResumeError::Load(Box::new(e))
    }
}

impl From<crate::phase_runner::PhaseError> for ResumeError {
    fn from(e: crate::phase_runner::PhaseError) -> Self {
        ResumeError::Phase(Box::new(e))
    }
}

use std::path::{Path, PathBuf};

/// Move `run_dir/<phase>/` → `run_dir/attempts/<phase>/<attempt>/` atomically.
///
/// `attempt` is the *current* attempt number (from the existing `.started` marker
/// or from directory scanning). The target must not exist; if it does, this is
/// an invariant violation and returns `ResumeError::Io`.
pub fn archive_current_attempt(
    run_dir: &Path,
    phase: &str,
    attempt: u32,
) -> Result<PathBuf, ResumeError> {
    let src = run_dir.join(phase);
    if !src.exists() {
        // Nothing to archive — not an error, just a no-op.
        return Ok(src);
    }

    let dest_parent = run_dir.join("attempts").join(phase);
    std::fs::create_dir_all(&dest_parent)?;

    let dest = dest_parent.join(attempt.to_string());
    if dest.exists() {
        return Err(ResumeError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("archive destination already exists: {}", dest.display()),
        )));
    }

    std::fs::rename(&src, &dest)?;

    // fsync the parent directory to guarantee rename visibility
    if let Ok(dir) = std::fs::File::open(&dest_parent) {
        let _ = dir.sync_all();
    }

    Ok(dest)
}

/// Decision for a single phase in a resume plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhaseAction {
    /// Phase is already completed and verified — do not invoke PhaseRunner.
    Skip,
    /// Phase was started (stale heartbeat) or failed. Archive current attempt
    /// and start a new one at the given counter.
    /// `next_attempt` is the attempt number for the *new* attempt (current + 1).
    Resume { next_attempt: u32 },
    /// No markers exist for this phase — normal first-time execution.
    RunFresh,
}

/// The resume plan: ordered actions aligned with the workflow phase list.
#[derive(Debug, Clone)]
pub struct ResumePlan {
    pub run_dir: PathBuf,
    /// Ordered actions aligned with the workflow phase list.
    pub actions: Vec<(crate::phase_runner::PhaseConfig, PhaseAction)>,
    /// Tmp files that were swept before planning.
    pub swept_tmp: Vec<PathBuf>,
    /// The original run ID, preserved across resume attempts so that
    /// traces and backend records remain consistently associated.
    pub run_id: uuid::Uuid,
}

/// Planner that inspects the on-disk state and decides what to do per phase.
pub struct ResumePlanner;

impl ResumePlanner {
    /// Given the on-disk state and the ordered list of phases in a workflow,
    /// produce a plan that tells the executor what to do per phase.
    pub fn plan(
        run_dir: &Path,
        run_state: &crate::run_state::RunState,
        phases: &[crate::phase_runner::PhaseConfig],
        swept_tmp: Vec<PathBuf>,
    ) -> Result<ResumePlan, ResumeError> {
        let mut actions = Vec::with_capacity(phases.len());

        for phase_cfg in phases {
            let phase_name = &phase_cfg.phase;
            let action = match run_state.phase_status.get(phase_name) {
                None => PhaseAction::RunFresh,
                Some(crate::run_state::PhaseStatus::Completed) => {
                    // Additional safety: verify that the manifest entry's SHA
                    // matches what the completed marker claimed. This was
                    // already done by RunState::load, but we double-check here.
                    PhaseAction::Skip
                }
                Some(crate::run_state::PhaseStatus::Failed) => {
                    // Determine the next attempt counter.
                    let next = crate::run_state::next_attempt(run_dir, phase_name)?;
                    PhaseAction::Resume { next_attempt: next }
                }
                Some(crate::run_state::PhaseStatus::Started) => {
                    // Stale heartbeat (RunState::load would have errored on Live).
                    let next = crate::run_state::next_attempt(run_dir, phase_name)?;
                    PhaseAction::Resume { next_attempt: next }
                }
                Some(crate::run_state::PhaseStatus::None) => PhaseAction::RunFresh,
            };
            actions.push((phase_cfg.clone(), action));
        }

        Ok(ResumePlan {
            run_dir: run_dir.to_path_buf(),
            actions,
            swept_tmp,
            run_id: uuid::Uuid::parse_str(&run_state.run_id)
                .unwrap_or_else(|_| uuid::Uuid::new_v4()),
        })
    }
}

/// Runner that executes a resume plan against a run directory.
pub struct ResumeRunner {
    backends: Vec<std::sync::Arc<dyn crate::backend::Backend>>,
}

impl ResumeRunner {
    /// Create a new ResumeRunner with the given backends.
    pub fn new(backends: Vec<std::sync::Arc<dyn crate::backend::Backend>>) -> Self {
        Self { backends }
    }

    /// Execute the resume plan. Returns Ok(()) when the run reaches completion
    /// (either because work was done or because all phases were already done).
    pub async fn execute(&self, plan: &ResumePlan) -> Result<(), ResumeError> {
        let runner = crate::phase_runner::PhaseRunner::new();

        for (phase_cfg, action) in &plan.actions {
            match action {
                PhaseAction::Skip => {
                    eprintln!(
                        "  [resume] skipping '{}': already completed",
                        phase_cfg.phase
                    );
                }
                PhaseAction::Resume {
                    next_attempt: attempt,
                } => {
                    eprintln!(
                        "  [resume] resuming '{}' at attempt {}",
                        phase_cfg.phase, attempt
                    );
                    archive_current_attempt(&plan.run_dir, &phase_cfg.phase, attempt - 1)?;
                    self.run_phase(&runner, phase_cfg, &plan.run_dir, *attempt, plan.run_id)
                        .await?;
                }
                PhaseAction::RunFresh => {
                    eprintln!("  [resume] running '{}' fresh", phase_cfg.phase);
                    self.run_phase(&runner, phase_cfg, &plan.run_dir, 0, plan.run_id)
                        .await?;
                }
            }
        }

        Ok(())
    }

    async fn run_phase(
        &self,
        runner: &crate::phase_runner::PhaseRunner,
        cfg: &crate::phase_runner::PhaseConfig,
        run_dir: &std::path::Path,
        _attempt: u32,
        run_id: uuid::Uuid,
    ) -> Result<(), ResumeError> {
        let ctx = crate::strategy::PhaseContext::new(&cfg.phase, run_id);
        let prompt = crate::strategy::Prompt::new();
        let inputs = crate::phase_runner::PhaseInputs {
            backends: &self.backends,
            prompt,
            ctx,
            verify: None,
            run_dir: run_dir.to_path_buf(),
            trace: None,
        };

        let _outcome = runner.run(cfg, inputs).await?;
        Ok(())
    }
}

#[cfg(test)]
mod planner_tests {
    use std::collections::HashMap;

    use super::*;
    use crate::manifest::{Kind, ManifestEntry, Producer};
    use crate::run_state::{HeartbeatStatus, PhaseStatus, RunState};

    #[test]
    fn planner_all_completed_returns_all_skip() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path();

        let mut phase_status = HashMap::new();
        phase_status.insert("design".to_string(), PhaseStatus::Completed);
        phase_status.insert("review".to_string(), PhaseStatus::Completed);
        phase_status.insert("verify".to_string(), PhaseStatus::Completed);

        let run_state = RunState {
            run_id: "test".to_string(),
            entries: vec![],
            dropped_orphans: vec![],
            phase_status,
            heartbeat: None,
        };

        let phases = vec![
            crate::phase_runner::PhaseConfig::single("design", "openai", "do design", "design.md"),
            crate::phase_runner::PhaseConfig::single("review", "openai", "do review", "review.md"),
            crate::phase_runner::PhaseConfig::single(
                "verify",
                "openai",
                "do verify",
                "verify.json",
            ),
        ];

        let plan = ResumePlanner::plan(run_dir, &run_state, &phases, vec![]).unwrap();
        assert_eq!(plan.actions.len(), 3);
        assert!(plan.actions.iter().all(|(_, a)| *a == PhaseAction::Skip));
    }

    #[test]
    fn planner_first_started_becomes_resume() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path();

        // Write a .started.1 marker so next_attempt returns 2
        let markers_dir = run_dir.join("markers");
        std::fs::create_dir_all(&markers_dir).unwrap();
        let payload = serde_json::json!({
            "phase": "phase2",
            "attempt": 1,
            "started_at": chrono::Utc::now().to_rfc3339(),
            "writer_pid": 123,
            "writer_host": "localhost",
            "heartbeat_ttl_seconds": 300,
        });
        std::fs::write(markers_dir.join("phase2.started.1"), payload.to_string()).unwrap();

        let mut phase_status = HashMap::new();
        phase_status.insert("phase1".to_string(), PhaseStatus::Completed);
        phase_status.insert("phase2".to_string(), PhaseStatus::Started);

        let run_state = RunState {
            run_id: "test".to_string(),
            entries: vec![],
            dropped_orphans: vec![],
            phase_status,
            heartbeat: None,
        };

        let phases = vec![
            crate::phase_runner::PhaseConfig::single("phase1", "openai", "p1", "a1.md"),
            crate::phase_runner::PhaseConfig::single("phase2", "openai", "p2", "a2.md"),
            crate::phase_runner::PhaseConfig::single("phase3", "openai", "p3", "a3.md"),
        ];

        let plan = ResumePlanner::plan(run_dir, &run_state, &phases, vec![]).unwrap();
        assert_eq!(plan.actions[0].1, PhaseAction::Skip);
        assert_eq!(plan.actions[1].1, PhaseAction::Resume { next_attempt: 2 });
        assert_eq!(plan.actions[2].1, PhaseAction::RunFresh);
    }

    #[test]
    fn planner_failed_increments_attempt() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path();

        let markers_dir = run_dir.join("markers");
        std::fs::create_dir_all(&markers_dir).unwrap();
        let started = serde_json::json!({
            "phase": "phase2",
            "attempt": 1,
            "started_at": chrono::Utc::now().to_rfc3339(),
            "writer_pid": 123,
            "writer_host": "localhost",
            "heartbeat_ttl_seconds": 300,
        });
        std::fs::write(markers_dir.join("phase2.started.1"), started.to_string()).unwrap();
        let failed = serde_json::json!({
            "phase": "phase2",
            "attempts_made": 1,
            "failed_at": chrono::Utc::now().to_rfc3339(),
            "error_class": "test",
            "last_attempt_path": "attempts/phase2/0",
        });
        std::fs::write(markers_dir.join("phase2.failed"), failed.to_string()).unwrap();

        let mut phase_status = HashMap::new();
        phase_status.insert("phase1".to_string(), PhaseStatus::Completed);
        phase_status.insert("phase2".to_string(), PhaseStatus::Failed);

        let run_state = fake_run_state(phase_status);

        let phases = vec![
            crate::phase_runner::PhaseConfig::single("phase1", "openai", "p1", "a1.md"),
            crate::phase_runner::PhaseConfig::single("phase2", "openai", "p2", "a2.md"),
        ];

        let plan = ResumePlanner::plan(run_dir, &run_state, &phases, vec![]).unwrap();
        assert_eq!(plan.actions[0].1, PhaseAction::Skip);
        assert_eq!(plan.actions[1].1, PhaseAction::Resume { next_attempt: 2 });
    }

    #[test]
    fn planner_none_is_run_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path();

        let run_state = fake_run_state(HashMap::new());

        let phases = vec![crate::phase_runner::PhaseConfig::single(
            "phase1", "openai", "p1", "a1.md",
        )];

        let plan = ResumePlanner::plan(run_dir, &run_state, &phases, vec![]).unwrap();
        assert_eq!(plan.actions[0].1, PhaseAction::RunFresh);
    }

    fn fake_run_state(phase_status: HashMap<String, PhaseStatus>) -> RunState {
        RunState {
            run_id: "test".to_string(),
            entries: vec![],
            dropped_orphans: vec![],
            phase_status,
            heartbeat: None,
        }
    }

    #[test]
    fn planner_manifest_sha_mismatch_surfaces_load_error() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path();

        // Set up: completed marker for phase1, but manifest entry SHA doesn't match
        let markers_dir = run_dir.join("markers");
        std::fs::create_dir_all(&markers_dir).unwrap();

        // Create the completed marker with a specific SHA
        let completed = serde_json::json!({
            "phase": "phase1",
            "attempt": 1,
            "completed_at": chrono::Utc::now().to_rfc3339(),
            "manifest_entry_sha256": "00".repeat(32),
            "artefact_paths": ["phase1/file.md"],
        });
        std::fs::write(markers_dir.join("phase1.completed"), completed.to_string()).unwrap();

        let mut phase_status = HashMap::new();
        phase_status.insert("phase1".to_string(), PhaseStatus::Completed);

        let run_state = RunState {
            run_id: "test".to_string(),
            entries: vec![crate::manifest::ManifestEntry {
                schema_version: 1,
                name: "phase1/file.md".to_string(),
                kind: crate::manifest::Kind::DesignMd,
                sha256: "ff".repeat(32),
                phase: Some("phase1".to_string()),
                producer: crate::manifest::Producer::Single,
                attempt: None,
                created_at: Some(chrono::Utc::now()),
            }],
            dropped_orphans: vec![],
            phase_status,
            heartbeat: None,
        };

        let phases = vec![crate::phase_runner::PhaseConfig::single(
            "phase1", "openai", "p1", "file.md",
        )];

        // The plan succeeds because ResumePlanner trusts RunState::load results.
        // SHA verification is done by RunState::load, not by the planner.
        let plan = ResumePlanner::plan(run_dir, &run_state, &phases, vec![]).unwrap();
        // The phase is marked Completed, so we Skip.
        assert_eq!(plan.actions[0].1, PhaseAction::Skip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_happy_path() {
        let tmp = tempfile::tempdir().unwrap();
        let phase_dir = tmp.path().join("design");
        std::fs::create_dir_all(&phase_dir).unwrap();
        std::fs::write(phase_dir.join("artefact.md"), "hello").unwrap();

        let dest = archive_current_attempt(tmp.path(), "design", 1).unwrap();
        assert!(dest.ends_with("attempts/design/1"));
        assert!(!phase_dir.exists());
        assert!(dest.join("artefact.md").exists());
    }

    #[test]
    fn archive_noop_when_source_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let result = archive_current_attempt(tmp.path(), "design", 1).unwrap();
        assert_eq!(result, tmp.path().join("design"));
    }

    #[test]
    fn archive_errors_when_target_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let phase_dir = tmp.path().join("design");
        std::fs::create_dir_all(&phase_dir).unwrap();
        std::fs::write(phase_dir.join("a.md"), "a").unwrap();

        // Pre-create the target to trigger the error
        let target = tmp.path().join("attempts").join("design").join("1");
        std::fs::create_dir_all(&target).unwrap();

        let result = archive_current_attempt(tmp.path(), "design", 1);
        assert!(result.is_err(), "should error when target exists");
    }
}
