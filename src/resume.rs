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
