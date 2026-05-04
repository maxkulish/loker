use std::fs;
use std::path::Path;

use chrono::{Duration, Utc};
use loker::manifest::{Kind, Manifest, ManifestEntry, Producer};
use loker::phase_runner::PhaseConfig;
use loker::resume::{PhaseAction, ResumePlanner};
use loker::run_state::{CompletedMarker, HeartbeatBody, PhaseStatus, RunState};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup_run_dir_with_manifest(tmp: &Path, entries: Vec<ManifestEntry>) {
    fs::create_dir_all(tmp.join("markers")).unwrap();
    fs::create_dir_all(tmp.join("attempts")).unwrap();

    let manifest = Manifest {
        run_id: "test-run-id".to_string(),
        schema_version: 1,
        workflow_name: None,
        entries,
    };
    let manifest_json = manifest.to_json().unwrap();
    fs::write(tmp.join("manifest.json"), manifest_json).unwrap();
}

fn setup_run_dir(tmp: &Path) {
    setup_run_dir_with_manifest(tmp, vec![]);
}

fn write_heartbeat(tmp: &Path, tick_at: chrono::DateTime<Utc>, ttl: u64) {
    let body = HeartbeatBody {
        writer_pid: std::process::id(),
        writer_host: "localhost".to_string(),
        tick_at,
        ttl_seconds: Some(ttl),
    };
    fs::write(
        tmp.join("heartbeat.json"),
        serde_json::to_string_pretty(&body).unwrap(),
    )
    .unwrap();
}

fn write_completed_marker(tmp: &Path, phase: &str, attempt: u32, sha: &str) {
    let marker = CompletedMarker {
        phase: phase.to_string(),
        attempt,
        completed_at: Utc::now(),
        manifest_entry_sha256: sha.to_string(),
        artefact_paths: vec![format!("{phase}/out.md")],
    };
    fs::write(
        tmp.join("markers").join(format!("{phase}.completed")),
        serde_json::to_string_pretty(&marker).unwrap(),
    )
    .unwrap();
}

fn write_started_marker(tmp: &Path, phase: &str, attempt: u32) {
    let marker = loker::run_state::StartedMarker {
        phase: phase.to_string(),
        attempt,
        started_at: Utc::now(),
        writer_pid: std::process::id(),
        writer_host: "localhost".to_string(),
        heartbeat_ttl_seconds: 300,
    };
    fs::write(
        tmp.join("markers")
            .join(format!("{phase}.started.{attempt}")),
        serde_json::to_string_pretty(&marker).unwrap(),
    )
    .unwrap();
}

fn make_phase_cfgs() -> Vec<PhaseConfig> {
    vec![
        PhaseConfig::single("design", "openai", "do design", "design.md"),
        PhaseConfig::single("review", "openai", "do review", "review.md"),
        PhaseConfig::single("verify", "openai", "do verify", "verify.json"),
    ]
}

// ---------------------------------------------------------------------------
// TDD Test 1: Kill mid-phase-2
// ---------------------------------------------------------------------------
#[test]
fn resume_kill_mid_phase_2() {
    let tmp = tempfile::tempdir().unwrap();
    setup_run_dir(tmp.path());

    // Phase 1 completed
    let sha1 = "00".repeat(32);
    write_completed_marker(tmp.path(), "design", 1, &sha1);

    // Phase 2 started (simulating kill)
    write_started_marker(tmp.path(), "review", 1);

    // Write a stale heartbeat (older than TTL)
    let old_tick = Utc::now() - Duration::seconds(600);
    write_heartbeat(tmp.path(), old_tick, 300);

    let run_state = RunState::load(tmp.path(), 300).unwrap();
    assert_eq!(
        run_state.phase_status.get("design"),
        Some(&PhaseStatus::Completed)
    );
    assert_eq!(
        run_state.phase_status.get("review"),
        Some(&PhaseStatus::Started)
    );

    let swept = loker::resume::sweep::sweep_stale_tmp(tmp.path(), 300).unwrap();

    let phases = make_phase_cfgs();
    let plan = ResumePlanner::plan(tmp.path(), &run_state, &phases, swept).unwrap();

    assert_eq!(plan.actions[0].1, PhaseAction::Skip);
    assert_eq!(plan.actions[1].1, PhaseAction::Resume { next_attempt: 2 });
    assert_eq!(plan.actions[2].1, PhaseAction::RunFresh);
}

// ---------------------------------------------------------------------------
// TDD Test 2: Already complete
// ---------------------------------------------------------------------------
#[test]
fn resume_already_complete() {
    let tmp = tempfile::tempdir().unwrap();
    setup_run_dir(tmp.path());

    let sha = "00".repeat(32);
    write_completed_marker(tmp.path(), "design", 1, &sha);
    write_completed_marker(tmp.path(), "review", 1, &sha);
    write_completed_marker(tmp.path(), "verify", 1, &sha);

    // Stale heartbeat (writer is dead)
    let old_tick = Utc::now() - Duration::seconds(600);
    write_heartbeat(tmp.path(), old_tick, 300);

    let run_state = RunState::load(tmp.path(), 300).unwrap();
    let phases = make_phase_cfgs();
    let swept = loker::resume::sweep::sweep_stale_tmp(tmp.path(), 300).unwrap();
    let plan = ResumePlanner::plan(tmp.path(), &run_state, &phases, swept).unwrap();

    assert!(plan.actions.iter().all(|(_, a)| *a == PhaseAction::Skip));
}

// ---------------------------------------------------------------------------
// TDD Test 3: Corrupt manifest entry
// ---------------------------------------------------------------------------
#[test]
fn resume_corrupt_manifest_entry() {
    let tmp = tempfile::tempdir().unwrap();
    let entry = ManifestEntry {
        schema_version: 1,
        name: "design/out.md".to_string(),
        kind: Kind::DesignMd,
        sha256: "00".repeat(32),
        phase: Some("design".to_string()),
        producer: Producer::Single,
        attempt: None,
        created_at: Some(Utc::now()),
    };
    setup_run_dir_with_manifest(tmp.path(), vec![entry]);

    // Write a completed marker claiming sha "00..."
    write_completed_marker(tmp.path(), "design", 1, &"00".repeat(32));

    // But create the artefact with different content
    let design_dir = tmp.path().join("design");
    fs::create_dir_all(&design_dir).unwrap();
    fs::write(design_dir.join("out.md"), "different content").unwrap();

    // Stale heartbeat
    let old_tick = Utc::now() - Duration::seconds(600);
    write_heartbeat(tmp.path(), old_tick, 300);

    // RunState::load should detect the SHA mismatch
    let result = RunState::load(tmp.path(), 300);
    assert!(
        result.is_err(),
        "expected corrupt artefact error, got {:?}",
        result
    );
}

// ---------------------------------------------------------------------------
// TDD Test 4: Live writer
// ---------------------------------------------------------------------------
#[test]
fn resume_live_writer() {
    let tmp = tempfile::tempdir().unwrap();
    setup_run_dir(tmp.path());

    // Write a fresh heartbeat (within TTL)
    let recent_tick = Utc::now() - Duration::seconds(10);
    write_heartbeat(tmp.path(), recent_tick, 300);

    let run_state = RunState::load(tmp.path(), 300).unwrap();

    // Heartbeat should be classified as Live
    assert!(matches!(
        run_state.heartbeat,
        Some(loker::run_state::HeartbeatStatus::Live(_))
    ));
}

// ---------------------------------------------------------------------------
// TDD Test 5: Stale writer
// ---------------------------------------------------------------------------
#[test]
fn resume_stale_writer() {
    let tmp = tempfile::tempdir().unwrap();
    setup_run_dir(tmp.path());

    // Phase 1 completed
    let sha1 = "00".repeat(32);
    write_completed_marker(tmp.path(), "design", 1, &sha1);

    // Phase 2 started by a stale writer
    write_started_marker(tmp.path(), "review", 1);

    // Old heartbeat
    let old_tick = Utc::now() - Duration::seconds(600);
    write_heartbeat(tmp.path(), old_tick, 300);

    let run_state = RunState::load(tmp.path(), 300).unwrap();
    let phases = make_phase_cfgs();
    let swept = loker::resume::sweep::sweep_stale_tmp(tmp.path(), 300).unwrap();
    let plan = ResumePlanner::plan(tmp.path(), &run_state, &phases, swept).unwrap();

    // Lock acquired (tested separately)
    // Stale tmp swept (tested)
    // Phase 2 resumed at attempt 2
    assert_eq!(plan.actions[0].1, PhaseAction::Skip);
    assert_eq!(plan.actions[1].1, PhaseAction::Resume { next_attempt: 2 });
    assert_eq!(plan.actions[2].1, PhaseAction::RunFresh);
}
