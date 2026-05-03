use std::path::{Path, PathBuf};

use crate::manifest::{Manifest, ManifestEntry};
use crate::run_state::atomic_write;
use crate::run_state::markers::{MarkerError, MarkerWriter};

use super::{PhaseConfig, PhaseError};

pub fn start_attempt(markers: &MarkerWriter, phase: &str, attempt: u32) -> Result<(), MarkerError> {
    markers.write_started(phase, attempt)?;
    Ok(())
}

pub fn archive_failed_attempt(
    run_dir: &Path,
    phase: &str,
    attempt: u32,
) -> Result<PathBuf, std::io::Error> {
    let dir = run_dir
        .join("attempts")
        .join(phase)
        .join(attempt.to_string());
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn commit_success(
    run_dir: &Path,
    cfg: &PhaseConfig,
    bytes: &[u8],
    attempt: u32,
    run_id: uuid::Uuid,
) -> Result<(PathBuf, ManifestEntry), PhaseError> {
    std::fs::create_dir_all(run_dir)?;
    let artefact_path = run_dir.join(&cfg.artefact_name);
    if let Some(parent) = artefact_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    atomic_write(&artefact_path, bytes)?;

    let entry = ManifestEntry::from_payload(
        cfg.artefact_name.clone(),
        cfg.artefact_kind.clone(),
        1,
        cfg.producer.clone(),
        Some(cfg.phase.clone()),
        Some(attempt),
        bytes,
    );

    let manifest_path = run_dir.join("manifest.json");
    let mut manifest = if manifest_path.exists() {
        Manifest::load(&manifest_path)?
    } else {
        Manifest::new(run_id.to_string())
    };
    manifest.append(entry.clone(), &manifest_path)?;
    Ok((artefact_path, entry))
}

pub fn record_terminal_failure(
    markers: &MarkerWriter,
    run_dir: &Path,
    cfg: &PhaseConfig,
    attempts_made: u32,
    error_class: &str,
) -> Result<(), PhaseError> {
    let last_attempt = attempts_made.saturating_sub(1);
    let archive = archive_failed_attempt(run_dir, &cfg.phase, last_attempt)?;
    let summary = serde_json::json!({
        "phase": cfg.phase,
        "attempt": last_attempt,
        "error_class": error_class,
    });
    atomic_write(
        &archive.join("failure-summary.json"),
        serde_json::to_string_pretty(&summary)?.as_bytes(),
    )?;
    markers.write_failed(
        &cfg.phase,
        attempts_made,
        error_class,
        &archive.to_string_lossy(),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{Kind, Producer};
    use crate::phase_runner::{AggregatorName, PhaseConfig, StrategyName, VerifyHookName};

    fn cfg() -> PhaseConfig {
        PhaseConfig {
            phase: "design".into(),
            strategy: StrategyName::Single,
            aggregator: AggregatorName::First,
            verify: VerifyHookName::None,
            artefact_name: "design.md".into(),
            artefact_kind: Kind::DesignMd,
            producer: Producer::Single,
            prompt_template: "prompt".into(),
            backend: Some("mock".into()),
            targets: Vec::new(),
            min_responses: 1,
            rungs: Vec::new(),
            pass_failure_context: false,
        }
    }

    #[test]
    fn phase_runner_persist_commit_success_writes_manifest_and_marker_inputs() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg();
        let (path, entry) =
            commit_success(tmp.path(), &cfg, b"hello", 0, uuid::Uuid::nil()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        assert_eq!(entry.name, "design.md");
        assert!(tmp.path().join("manifest.json").is_file());
    }

    #[test]
    fn phase_runner_persist_archive_failed_attempt_writes_summary_and_failed_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = cfg();
        let markers = MarkerWriter::new(tmp.path());
        record_terminal_failure(&markers, tmp.path(), &cfg, 1, "verify_failed").unwrap();
        assert!(tmp
            .path()
            .join("attempts/design/0/failure-summary.json")
            .is_file());
        assert!(tmp.path().join("markers/design.failed").is_file());
    }
}
