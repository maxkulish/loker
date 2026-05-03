use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::run_state::atomic_write;

// ---------------------------------------------------------------------------
// Marker types
// ---------------------------------------------------------------------------

/// Marker written when a phase starts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StartedMarker {
    pub phase: String,
    pub attempt: u32,
    pub started_at: DateTime<Utc>,
    pub writer_pid: u32,
    pub writer_host: String,
    pub heartbeat_ttl_seconds: u32,
}

/// Marker written when a phase completes successfully.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CompletedMarker {
    pub phase: String,
    pub attempt: u32,
    pub completed_at: DateTime<Utc>,
    pub manifest_entry_sha256: String,
    pub artefact_paths: Vec<String>,
}

/// Marker written when a phase fails.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FailedMarker {
    pub phase: String,
    pub attempts_made: u32,
    pub failed_at: DateTime<Utc>,
    pub error_class: String,
    pub last_attempt_path: String,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum MarkerError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// MarkerWriter
// ---------------------------------------------------------------------------

/// Crash-safe writer for phase status markers (started / completed / failed).
///
/// All writes use the atomic tmp→fsync→rename→parent-fsync protocol via
/// [`atomic_write`] so that a crash at any point either produces a
/// completely written marker or no marker at all (no partial writes).
pub struct MarkerWriter {
    markers_dir: PathBuf,
}

impl MarkerWriter {
    /// Create a new writer that places markers under `run_dir / markers /`.
    pub fn new(run_dir: &Path) -> Self {
        Self {
            markers_dir: run_dir.join("markers"),
        }
    }

    /// Return the markers directory path.
    pub fn markers_dir(&self) -> &Path {
        &self.markers_dir
    }

    /// Write a "started" marker for the given phase and attempt.
    ///
    /// The filename includes the attempt number so multiple attempts
    /// can coexist: `<phase>.started.<attempt>`.
    ///
    /// Returns the written marker for inspection.
    pub fn write_started(&self, phase: &str, attempt: u32) -> Result<StartedMarker, MarkerError> {
        let marker = StartedMarker {
            phase: phase.to_owned(),
            attempt,
            started_at: Utc::now(),
            writer_pid: std::process::id(),
            writer_host: hostname(),
            heartbeat_ttl_seconds: 300,
        };
        self.write_marker_with_attempt(phase, "started", attempt, &marker)?;
        Ok(marker)
    }

    /// Write a "completed" marker for the given phase and attempt.
    ///
    /// The filename does NOT include the attempt number so only the
    /// latest completed marker is stored: `<phase>.completed`.
    ///
    /// Returns the written marker for inspection.
    pub fn write_completed(
        &self,
        phase: &str,
        attempt: u32,
        manifest_entry_sha256: &str,
        artefact_paths: &[String],
    ) -> Result<CompletedMarker, MarkerError> {
        let marker = CompletedMarker {
            phase: phase.to_owned(),
            attempt,
            completed_at: Utc::now(),
            manifest_entry_sha256: manifest_entry_sha256.to_owned(),
            artefact_paths: artefact_paths.to_owned(),
        };
        self.write_marker(phase, "completed", &marker)?;
        Ok(marker)
    }

    /// Write a "failed" marker for the given phase.
    ///
    /// The filename does NOT include the attempt number:
    /// `<phase>.failed`.
    ///
    /// Returns the written marker for inspection.
    pub fn write_failed(
        &self,
        phase: &str,
        attempts_made: u32,
        error_class: &str,
        last_attempt_path: &str,
    ) -> Result<FailedMarker, MarkerError> {
        let marker = FailedMarker {
            phase: phase.to_owned(),
            attempts_made,
            failed_at: Utc::now(),
            error_class: error_class.to_owned(),
            last_attempt_path: last_attempt_path.to_owned(),
        };
        self.write_marker(phase, "failed", &marker)?;
        Ok(marker)
    }

    /// Build the marker file path:
    /// - For started markers: `<markers_dir>/<phase>.<state>.<attempt>`
    ///   (attempt-numbered so multiple can coexist)
    /// - For completed/failed: `<markers_dir>/<phase>.<state>`
    ///   (terminal, only the latest is kept)
    fn marker_path(&self, phase: &str, state: &str, attempt: Option<u32>) -> PathBuf {
        match attempt {
            Some(a) => self.markers_dir.join(format!("{}.{}.{}", phase, state, a)),
            None => self.markers_dir.join(format!("{}.{}", phase, state)),
        }
    }

    /// Serialise `body` as JSON and atomically write it to the marker file.
    fn write_marker<T: Serialize>(
        &self,
        phase: &str,
        state: &str,
        body: &T,
    ) -> Result<(), MarkerError> {
        // Terminal markers (completed, failed) have no attempt in filename.
        std::fs::create_dir_all(&self.markers_dir)?;
        let path = self.marker_path(phase, state, None);
        let json = serde_json::to_string_pretty(body)?;
        atomic_write(&path, json.as_bytes())?;
        Ok(())
    }

    /// Like `write_marker` but includes the attempt number in the filename.
    fn write_marker_with_attempt<T: Serialize>(
        &self,
        phase: &str,
        state: &str,
        attempt: u32,
        body: &T,
    ) -> Result<(), MarkerError> {
        std::fs::create_dir_all(&self.markers_dir)?;
        let path = self.marker_path(phase, state, Some(attempt));
        let json = serde_json::to_string_pretty(body)?;
        atomic_write(&path, json.as_bytes())?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helper: hostname
// ---------------------------------------------------------------------------

/// Best-effort hostname. Falls back to "unknown" on error.
fn hostname() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("HOST"))
        .unwrap_or_else(|_| "unknown".to_string())
}

// ---------------------------------------------------------------------------
// Helper: next_attempt
// ---------------------------------------------------------------------------

/// Derive the next attempt number for a phase by scanning both started
/// markers and existing attempt directories.
///
/// Returns `0` if no started markers or attempt dirs exist for the phase.
/// Returns `max(attempt_numbers) + 1` otherwise (gaps in attempt numbering
/// do not reduce the counter).
///
/// Dual-source scanning ensures correctness after a crash between marker
/// write and attempt directory creation (or vice versa).
pub fn next_attempt(run_dir: &Path, phase: &str) -> Result<u32, MarkerError> {
    let markers_dir = run_dir.join("markers");
    let attempts_dir = run_dir.join("attempts").join(phase);

    let marker_max = next_attempt_from_markers(&markers_dir, phase)?;
    let dir_max = next_attempt_from_dirs(&attempts_dir)?;

    Ok(std::cmp::max(marker_max, dir_max))
}

// ---------------------------------------------------------------------------
// Internal helpers for next_attempt
// ---------------------------------------------------------------------------

fn next_attempt_from_markers(markers_dir: &Path, phase: &str) -> Result<u32, MarkerError> {
    let started_prefix = format!("{}.started.", phase);
    let dir = match std::fs::read_dir(markers_dir) {
        Ok(d) => d,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };

    let mut max_attempt: Option<u32> = None;
    for entry in dir {
        let entry = entry?;
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if !name.starts_with(&started_prefix) {
            continue;
        }
        let content = std::fs::read_to_string(entry.path())?;
        let marker: StartedMarker = serde_json::from_str(&content)?;
        if max_attempt.map_or(true, |m| marker.attempt > m) {
            max_attempt = Some(marker.attempt);
        }
    }

    Ok(max_attempt.map_or(0, |m| m + 1))
}

fn next_attempt_from_dirs(attempts_dir: &Path) -> Result<u32, MarkerError> {
    let dir = match std::fs::read_dir(attempts_dir) {
        Ok(d) => d,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.into()),
    };

    let mut max_attempt: Option<u32> = None;
    for entry in dir {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Ok(n) = name.parse::<u32>() {
            if max_attempt.map_or(true, |m| n > m) {
                max_attempt = Some(n);
            }
        }
    }

    // Same +1 logic as next_attempt_from_markers
    Ok(max_attempt.map_or(0, |m| m + 1))
}
