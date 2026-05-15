//! Round-trip integration tests for `loker resume`.
//!
//! These tests exercise the full resume cycle:
//!   1. Create a run directory with markers and manifest entries.
//!   2. Load [`RunState`] from the directory.
//!   3. Plan via [`ResumePlanner`].
//!   4. Execute via [`ResumeRunner`].
//!   5. Assert sentinel mtime invariance (completed phases are not re-run).
//!
//! Three scenarios:
//!   - All phases complete → resume is a no-op.
//!   - Phase 2 interrupted mid-execution (started marker only) → resume
//!     skips phase 1 and resumes phase 2.
//!   - Phase 2 failed → resume archives the failed attempt and retries.

use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use loker::backend::{Backend, BackendCapabilities, BackendError, QueryOutput};
use loker::manifest::{Kind, Manifest, ManifestEntry, Producer, sha256_hex};
use loker::phase_runner::PhaseConfig;
use loker::resume::{PhaseAction, ResumePlanner, ResumeRunner};
use loker::resume::sweep::sweep_stale_tmp;
use loker::run_state::{
    CompletedMarker, HeartbeatBody, PhaseStatus, RunDir, RunState, StartedMarker,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A simple mock backend that returns canned text output.
#[derive(Debug)]
struct MockBackend {
    name: &'static str,
}

#[async_trait]
impl Backend for MockBackend {
    fn name(&self) -> &str {
        self.name
    }

    async fn query(
        &self,
        _prompt: &str,
        _cwd: &std::path::Path,
        _model: Option<&str>,
    ) -> Result<QueryOutput, BackendError> {
        Ok(QueryOutput::from_text(
            format!("output from {}", self.name),
            self.name,
            std::time::Duration::from_millis(1),
        ))
    }

    fn is_available(&self) -> bool {
        true
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::none()
    }
}

/// Initial marker state to create for the round-trip fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InitialState {
    /// Both phases have completed markers and manifest entries.
    AllComplete,
    /// Phase 1 completed, phase 2 has only a started marker (simulating SIGTERM).
    InterruptedPhase,
    /// Phase 1 completed, phase 2 has a failed marker.
    FailedPhase,
}

/// Create a RunDir with marker state matching `initial_state`.
///
/// Returns (RunDir, Vec<PhaseConfig>).  The TempDir must stay alive for the
/// duration of the test (it is dropped at end of scope).
fn build_roundtrip_run_dir(
    base: &tempfile::TempDir,
    initial_state: InitialState,
) -> (RunDir, Vec<PhaseConfig>) {
    let run_dir = RunDir::create(base.path(), "test-roundtrip").unwrap();

    // Create markers directory (RunDir::create does not create this).
    let markers_dir = run_dir.path().join("markers");
    std::fs::create_dir_all(&markers_dir).unwrap();

    // Phase configs (two phases, single strategy, same backend name "mock").
    let phases = vec![
        PhaseConfig::single("phase1", "mock", "do phase one", "phase1.md"),
        PhaseConfig::single("phase2", "mock", "do phase two", "phase2.md"),
    ];

    // Write a stale heartbeat so RunState::load does not see a live writer.
    write_stale_heartbeat(run_dir.path());

    match initial_state {
        InitialState::AllComplete => {
            // Phase 1: artefact + manifest entry + completed marker.
            let content1 = b"sentinel phase1";
            let sha1 = sha256_hex(content1);
            write_artefact(run_dir.path(), "phase1.md", content1);
            write_manifest_entry(&run_dir, "phase1.md", &sha1, 0);
            write_completed_marker(run_dir.path(), "phase1", 0, &sha1, &["phase1.md"]);

            // Phase 2: same.
            let content2 = b"sentinel phase2";
            let sha2 = sha256_hex(content2);
            write_artefact(run_dir.path(), "phase2.md", content2);
            write_manifest_entry(&run_dir, "phase2.md", &sha2, 0);
            write_completed_marker(run_dir.path(), "phase2", 0, &sha2, &["phase2.md"]);
        }

        InitialState::InterruptedPhase => {
            // Phase 1: completed.
            let content1 = b"sentinel phase1";
            let sha1 = sha256_hex(content1);
            write_artefact(run_dir.path(), "phase1.md", content1);
            write_manifest_entry(&run_dir, "phase1.md", &sha1, 0);
            write_completed_marker(run_dir.path(), "phase1", 0, &sha1, &["phase1.md"]);

            // Phase 2: started-only (no completed/failed — simulates SIGTERM).
            let started = StartedMarker {
                phase: "phase2".to_string(),
                attempt: 0,
                started_at: Utc::now(),
                writer_pid: std::process::id(),
                writer_host: "localhost".to_string(),
                heartbeat_ttl_seconds: 300,
            };
            std::fs::write(
                markers_dir.join("phase2.started.0"),
                serde_json::to_string_pretty(&started).unwrap(),
            )
            .unwrap();
        }

        InitialState::FailedPhase => {
            // Phase 1: completed.
            let content1 = b"sentinel phase1";
            let sha1 = sha256_hex(content1);
            write_artefact(run_dir.path(), "phase1.md", content1);
            write_manifest_entry(&run_dir, "phase1.md", &sha1, 0);
            write_completed_marker(run_dir.path(), "phase1", 0, &sha1, &["phase1.md"]);

            // Phase 2: started + failed markers.
            let started = StartedMarker {
                phase: "phase2".to_string(),
                attempt: 0,
                started_at: Utc::now(),
                writer_pid: std::process::id(),
                writer_host: "localhost".to_string(),
                heartbeat_ttl_seconds: 300,
            };
            std::fs::write(
                markers_dir.join("phase2.started.0"),
                serde_json::to_string_pretty(&started).unwrap(),
            )
            .unwrap();

            let failed = serde_json::json!({
                "phase": "phase2",
                "attempts_made": 1,
                "failed_at": Utc::now().to_rfc3339(),
                "error_class": "test_failure",
                "last_attempt_path": "attempts/phase2/0",
            });
            std::fs::write(
                markers_dir.join("phase2.failed"),
                serde_json::to_string_pretty(&failed).unwrap(),
            )
            .unwrap();
        }
    }

    (run_dir, phases)
}

fn write_stale_heartbeat(run_dir: &std::path::Path) {
    let old_tick = Utc::now() - Duration::seconds(600);
    let body = HeartbeatBody {
        writer_pid: 99999,
        writer_host: "localhost".to_string(),
        tick_at: old_tick,
        ttl_seconds: Some(300),
    };
    std::fs::write(
        run_dir.join("heartbeat.json"),
        serde_json::to_string_pretty(&body).unwrap(),
    )
    .unwrap();
}

fn write_artefact(run_dir: &std::path::Path, artefact_name: &str, content: &[u8]) {
    std::fs::write(run_dir.join(artefact_name), content).unwrap();
}

fn write_completed_marker(
    run_dir: &std::path::Path,
    phase: &str,
    attempt: u32,
    sha: &str,
    artefact_paths: &[&str],
) {
    let marker = CompletedMarker {
        phase: phase.to_string(),
        attempt,
        completed_at: Utc::now(),
        manifest_entry_sha256: sha.to_string(),
        artefact_paths: artefact_paths.iter().map(|s| s.to_string()).collect(),
        hitl: None,
    };
    std::fs::write(
        run_dir
            .join("markers")
            .join(format!("{phase}.completed")),
        serde_json::to_string_pretty(&marker).unwrap(),
    )
    .unwrap();
}

fn write_manifest_entry(run_dir: &RunDir, name: &str, sha: &str, attempt: u32) {
    let manifest_path = run_dir.manifest_path();
    let text = std::fs::read_to_string(&manifest_path).unwrap();
    let mut manifest: Manifest = Manifest::from_json(&text).unwrap();

    // Extract phase name from artefact name (e.g. "phase1.md" → "phase1").
    let phase_name = name.strip_suffix(".md").unwrap_or(name).to_string();

    manifest.entries.push(ManifestEntry {
        schema_version: 1,
        name: name.to_string(),
        kind: Kind::DesignMd,
        sha256: sha.to_string(),
        producer: Producer::Single,
        phase: Some(phase_name),
        attempt: Some(attempt),
        created_at: Some(Utc::now()),
    });

    std::fs::write(&manifest_path, manifest.to_json().unwrap()).unwrap();
}

/// Capture the mtime of a phase's sentinel artefact file.
fn capture_sentinel_mtime(run_dir: &RunDir, phase_name: &str) -> Option<SystemTime> {
    let path = run_dir.path().join(format!("{phase_name}.md"));
    std::fs::metadata(&path).ok().map(|m| m.modified().unwrap())
}

/// Assert that a sentinel artefact's mtime has not changed.
fn assert_sentinel_unchanged(
    _run_dir: &RunDir,
    phase_name: &str,
    before: Option<SystemTime>,
    after: Option<SystemTime>,
) {
    assert_eq!(
        before, after,
        "sentinel mtime for phase '{phase_name}' changed — phase was re-run"
    );
}

// ---------------------------------------------------------------------------
// ST2: test_resume_roundtrip_all_complete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_resume_roundtrip_all_complete() {
    let base = tempfile::tempdir().unwrap();
    let (run_dir, phases) = build_roundtrip_run_dir(&base, InitialState::AllComplete);

    // Capture sentinel mtimes before resume.
    let mtime_p1_before = capture_sentinel_mtime(&run_dir, "phase1");
    let mtime_p2_before = capture_sentinel_mtime(&run_dir, "phase2");
    assert!(mtime_p1_before.is_some(), "phase1 sentinel should exist");
    assert!(mtime_p2_before.is_some(), "phase2 sentinel should exist");

    // Load run state → all phases completed.
    let run_state = RunState::load(run_dir.path(), 300).unwrap();
    assert_eq!(
        run_state.phase_status.get("phase1"),
        Some(&PhaseStatus::Completed)
    );
    assert_eq!(
        run_state.phase_status.get("phase2"),
        Some(&PhaseStatus::Completed)
    );

    // Sweep stale tmp.
    let swept = sweep_stale_tmp(run_dir.path(), 300).unwrap();

    // Plan → both Skip.
    let plan = ResumePlanner::plan(run_dir.path(), &run_state, &phases, swept).unwrap();
    assert_eq!(plan.actions.len(), 2);
    assert_eq!(plan.actions[0].1, PhaseAction::Skip);
    assert_eq!(plan.actions[1].1, PhaseAction::Skip);

    // Execute → no-op.
    let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(MockBackend { name: "mock" })];
    let runner = ResumeRunner::new(backends);
    let result = runner.execute(&plan).await;
    assert!(result.is_ok(), "resume should succeed: {:?}", result.err());

    // Sentinels unchanged.
    let mtime_p1_after = capture_sentinel_mtime(&run_dir, "phase1");
    let mtime_p2_after = capture_sentinel_mtime(&run_dir, "phase2");
    assert_sentinel_unchanged(&run_dir, "phase1", mtime_p1_before, mtime_p1_after);
    assert_sentinel_unchanged(&run_dir, "phase2", mtime_p2_before, mtime_p2_after);

    // Manifest still has both entries.
    let manifest_text = std::fs::read_to_string(run_dir.manifest_path()).unwrap();
    let manifest = Manifest::from_json(&manifest_text).unwrap();
    assert_eq!(manifest.entries.len(), 2);
}

// ---------------------------------------------------------------------------
// ST3: test_resume_roundtrip_kill_phase2
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_resume_roundtrip_kill_phase2() {
    let base = tempfile::tempdir().unwrap();
    let (run_dir, phases) = build_roundtrip_run_dir(&base, InitialState::InterruptedPhase);

    // Capture sentinel mtime for phase 1 before resume.
    let mtime_p1_before = capture_sentinel_mtime(&run_dir, "phase1");
    assert!(mtime_p1_before.is_some(), "phase1 sentinel should exist");

    // Load run state.
    let run_state = RunState::load(run_dir.path(), 300).unwrap();
    assert_eq!(
        run_state.phase_status.get("phase1"),
        Some(&PhaseStatus::Completed)
    );
    assert_eq!(
        run_state.phase_status.get("phase2"),
        Some(&PhaseStatus::Started)
    );

    // Sweep.
    let swept = sweep_stale_tmp(run_dir.path(), 300).unwrap();

    // Plan.
    let plan = ResumePlanner::plan(run_dir.path(), &run_state, &phases, swept).unwrap();
    assert_eq!(plan.actions.len(), 2);
    assert_eq!(plan.actions[0].1, PhaseAction::Skip);
    assert_eq!(
        plan.actions[1].1,
        PhaseAction::Resume { next_attempt: 1 }
    );

    // Execute.
    let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(MockBackend { name: "mock" })];
    let runner = ResumeRunner::new(backends);
    let result = runner.execute(&plan).await;
    assert!(result.is_ok(), "resume should succeed: {:?}", result.err());

    // Sentinel mtime for phase 1 unchanged.
    let mtime_p1_after = capture_sentinel_mtime(&run_dir, "phase1");
    assert_sentinel_unchanged(&run_dir, "phase1", mtime_p1_before, mtime_p1_after);

    // Phase 2 should now be completed.
    let run_state_after = RunState::load(run_dir.path(), 300).unwrap();
    assert_eq!(
        run_state_after.phase_status.get("phase2"),
        Some(&PhaseStatus::Completed)
    );
}

// ---------------------------------------------------------------------------
// ST4: test_resume_roundtrip_phase2_failed_then_retry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_resume_roundtrip_phase2_failed_then_retry() {
    let base = tempfile::tempdir().unwrap();
    let (run_dir, phases) = build_roundtrip_run_dir(&base, InitialState::FailedPhase);

    // Capture sentinel mtime for phase 1 before resume.
    let mtime_p1_before = capture_sentinel_mtime(&run_dir, "phase1");
    assert!(mtime_p1_before.is_some(), "phase1 sentinel should exist");

    // Load run state.
    let run_state = RunState::load(run_dir.path(), 300).unwrap();
    assert_eq!(
        run_state.phase_status.get("phase1"),
        Some(&PhaseStatus::Completed)
    );
    assert_eq!(
        run_state.phase_status.get("phase2"),
        Some(&PhaseStatus::Failed)
    );

    // Sweep.
    let swept = sweep_stale_tmp(run_dir.path(), 300).unwrap();

    // Plan.
    let plan = ResumePlanner::plan(run_dir.path(), &run_state, &phases, swept).unwrap();
    assert_eq!(plan.actions.len(), 2);
    assert_eq!(plan.actions[0].1, PhaseAction::Skip);
    assert_eq!(
        plan.actions[1].1,
        PhaseAction::Resume { next_attempt: 1 }
    );

    // Execute with a working mock backend so the retry succeeds.
    let backends: Vec<Arc<dyn Backend>> = vec![Arc::new(MockBackend { name: "mock" })];
    let runner = ResumeRunner::new(backends);
    let result = runner.execute(&plan).await;
    assert!(result.is_ok(), "resume should succeed: {:?}", result.err());

    // Sentinel mtime for phase 1 unchanged.
    let mtime_p1_after = capture_sentinel_mtime(&run_dir, "phase1");
    assert_sentinel_unchanged(&run_dir, "phase1", mtime_p1_before, mtime_p1_after);

    // Phase 2 should now be completed (not failed).
    let run_state_after = RunState::load(run_dir.path(), 300).unwrap();
    assert_eq!(
        run_state_after.phase_status.get("phase2"),
        Some(&PhaseStatus::Completed)
    );
}
