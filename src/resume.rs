pub(crate) mod lock;
pub(crate) mod sweep;

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
    Load(#[from] crate::run_state::LoadError),

    #[error("phase error: {0}")]
    Phase(#[from] crate::phase_runner::PhaseError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Advisory lock is OS-cooperative only (flock / LockFileEx).
    /// Another loker process holds the lock; non-loker processes may ignore it.
    #[error("run directory is locked by another loker process")]
    LockInUse,
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
