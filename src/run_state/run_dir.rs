use std::cell::Cell;
use std::path::{Path, PathBuf};

use crate::manifest::Manifest;
use crate::run_state::atomic_write;
use crate::run_state::attempt_dir::AttemptDir;

/// A run directory: `runs/<workflow_slug>-<YYYYMMDD>-<HHMMSS>-<short_uuid>/`.
///
/// Created via [`RunDir::create`] and is the canonical filesystem root
/// for all run artefacts: manifest, markers, attempt directories, and trace.
///
/// # Invariants
///
/// - `path()` exists on disk after [`RunDir::create`].
/// - `manifest.json` exists at `manifest_path()` with the initial schema.
/// - `attempts/` exists at `path().join("attempts")`.
/// - No other code constructs these paths manually — [`RunDir::create`] is
///   the single producer.
#[derive(Debug, Clone)]
pub struct RunDir {
    path: PathBuf,
    run_id: uuid::Uuid,
    workflow_slug: String,
}

impl RunDir {
    /// Create a new run directory under `base_dir/runs/`.
    ///
    /// 1. Generates a UTC timestamp and short UUID.
    /// 2. Derives the workflow slug from `workflow_name` via `slug::slugify`.
    /// 3. Constructs `base_dir/runs/<slug>-<YYYYMMDD>-<HHMMSS>-<short_uuid>/`.
    /// 4. Creates `base_dir/runs/` if absent (mkdir -p).
    /// 5. Creates the leaf directory with `mkdir` (fails atomically if collides).
    /// 6. On collision (extremely rare): retries once with a new UUID.
    /// 7. Writes the initial `manifest.json` and creates `attempts/`.
    ///    On failure during step 7, the leaf directory is removed to prevent
    ///    orphaned empty directories.
    ///
    /// Returns `RunDirError::Collision` if both attempts collide (retry-exhausted).
    pub fn create(base_dir: &Path, workflow_name: &str) -> Result<Self, RunDirError> {
        let now = chrono::Utc::now();
        let mut run_id = uuid::Uuid::new_v4();
        let slug = slug::slugify(workflow_name);

        let dir_name = generate_dir_name(&slug, &now, &run_id);
        let runs_dir = base_dir.join("runs");
        let mut path = runs_dir.join(&dir_name);

        // Ensure the parent runs/ directory exists
        std::fs::create_dir_all(&runs_dir)?;

        // Atomic leaf directory creation with retry-once
        match std::fs::create_dir(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // Retry once with a fresh UUID
                run_id = uuid::Uuid::new_v4();
                let dir_name = generate_dir_name(&slug, &now, &run_id);
                path = runs_dir.join(&dir_name);
                std::fs::create_dir(&path).map_err(|_| RunDirError::Collision(path.clone()))?;
            }
            Err(e) => return Err(e.into()),
        }

        // Cleanup guard: remove the leaf directory if subsequent steps fail.
        // We use Cell<Option<PathBuf>> so disarm() can take &self (not &mut self).
        let guard = CleanupGuard::new(path.clone());

        // Write initial manifest.json
        let manifest = Manifest::new(run_id.to_string());
        let manifest_json = manifest.to_json()?;
        atomic_write(&path.join("manifest.json"), manifest_json.as_bytes())?;

        // Create attempts/ subdirectory
        std::fs::create_dir_all(path.join("attempts"))?;

        // All steps succeeded — disarm the guard
        guard.disarm();

        Ok(RunDir {
            path,
            run_id,
            workflow_slug: slug,
        })
    }

    /// Convenience wrapper: creates a run directory in the current working directory.
    /// Equivalent to `RunDir::create(&std::env::current_dir()?, workflow_name)`.
    pub fn create_in_cwd(workflow_name: &str) -> Result<Self, RunDirError> {
        let cwd = std::env::current_dir()?;
        Self::create(&cwd, workflow_name)
    }

    /// The run root directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Path to `manifest.json` within this run directory.
    pub fn manifest_path(&self) -> PathBuf {
        self.path.join("manifest.json")
    }

    /// Path to `trace.jsonl` within this run directory.
    ///
    /// The file is NOT created by `RunDir::create` — the trace writer
    /// (T-029) creates it on first append. This accessor provides the
    /// canonical path for the trace writer to use.
    pub fn trace_path(&self) -> PathBuf {
        self.path.join("trace.jsonl")
    }

    /// Create an `AttemptDir` handle scoped to this run directory.
    ///
    /// This only creates the handle — call [`AttemptDir::create`] separately
    /// to materialize the directory on disk.
    pub fn attempt_dir(&self, phase: &str, attempt: u32) -> AttemptDir {
        AttemptDir::new(self.path(), phase, attempt)
    }

    /// The UUID that identifies this run (also embedded in `manifest.json`).
    pub fn run_id(&self) -> uuid::Uuid {
        self.run_id
    }

    /// The workflow slug used in the directory name.
    pub fn workflow_slug(&self) -> &str {
        &self.workflow_slug
    }
}

/// Generate the run directory name.
///
/// Format: `<slug>-<YYYYMMDD>-<HHMMSS>-<short_uuid>`
///
/// Example: `design-review-20260503-104215-a1b2c3d4`
fn generate_dir_name(slug: &str, now: &chrono::DateTime<chrono::Utc>, run_id: &uuid::Uuid) -> String {
    let uuid_short = &run_id.to_string()[..8];
    format!(
        "{}-{}-{}",
        slug,
        now.format("%Y%m%d-%H%M%S"),
        uuid_short
    )
}

/// A drop-guard that removes the tracked directory on drop unless disarmed.
///
/// Used to prevent orphaned empty directories when [`RunDir::create`] fails
/// partway through (after `mkdir` succeeds but before manifest/attempts are written).
struct CleanupGuard {
    path: Cell<Option<PathBuf>>,
}

impl CleanupGuard {
    fn new(path: PathBuf) -> Self {
        Self {
            path: Cell::new(Some(path)),
        }
    }

    /// Disarm the guard — the directory will be kept on drop.
    fn disarm(&self) {
        self.path.set(None);
    }
}

impl Drop for CleanupGuard {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// Errors that can occur during run directory creation.
#[derive(Debug, thiserror::Error)]
pub enum RunDirError {
    /// Run directory collided with an existing directory after the retry attempt.
    #[error("run directory collision after retry: {0}")]
    Collision(PathBuf),

    /// I/O error during directory creation or file writing.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// JSON serialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// Manifest serialization or write error.
    #[error("manifest write failed: {0}")]
    Manifest(#[from] crate::manifest::ManifestError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_produces_path_matching_format() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = RunDir::create(tmp.path(), "design-doc-tdd").unwrap();
        let path_str = run_dir.path().to_string_lossy().to_string();

        // Path should be: <tmp>/runs/design-doc-tdd-<8digits>-<6digits>-<8hex>
        let runs_prefix = tmp.path().join("runs").to_string_lossy().to_string();
        assert!(
            path_str.starts_with(&runs_prefix),
            "path should start with runs dir"
        );

        let dir_name = run_dir.path().file_name().unwrap().to_string_lossy();

        // Verify format: <slug>-<YYYYMMDD>-<HHMMSS>-<short_uuid>
        let re =
            regex::Regex::new(r"^design-doc-tdd-\d{8}-\d{6}-[0-9a-f]{8}$").unwrap();
        assert!(
            re.is_match(&dir_name),
            "directory name '{}' does not match expected pattern",
            dir_name
        );
    }

    #[test]
    fn two_creates_produce_distinct_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let a = RunDir::create(tmp.path(), "test-workflow").unwrap();
        let b = RunDir::create(tmp.path(), "test-workflow").unwrap();
        assert_ne!(
            a.path(),
            b.path(),
            "two creates should produce different paths"
        );
    }

    #[test]
    fn manifest_json_exists_with_correct_shape() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = RunDir::create(tmp.path(), "manifest-test").unwrap();

        let manifest_path = run_dir.manifest_path();
        assert!(manifest_path.exists(), "manifest.json should exist");

        let content = std::fs::read_to_string(&manifest_path).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(manifest["loker.run_id"], run_dir.run_id().to_string());
        assert_eq!(manifest["schema_version"], 1);
        assert!(manifest["entries"].is_array());
        assert!(manifest["entries"].as_array().unwrap().is_empty());
    }

    #[test]
    fn attempts_subdirectory_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = RunDir::create(tmp.path(), "attempts-test").unwrap();

        let attempts_path = run_dir.path().join("attempts");
        assert!(attempts_path.exists(), "attempts/ should exist");
        assert!(attempts_path.is_dir(), "attempts/ should be a directory");
    }

    #[test]
    fn accessors_return_paths_under_run_root() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = RunDir::create(tmp.path(), "accessors-test").unwrap();

        let root = run_dir.path();
        assert!(run_dir.manifest_path().starts_with(root));
        assert!(run_dir.trace_path().starts_with(root));
    }

    #[test]
    fn run_id_matches_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = RunDir::create(tmp.path(), "id-test").unwrap();

        let manifest_path = run_dir.manifest_path();
        let content = std::fs::read_to_string(&manifest_path).unwrap();
        let manifest: serde_json::Value = serde_json::from_str(&content).unwrap();

        let manifest_run_id = manifest["loker.run_id"].as_str().unwrap();
        assert_eq!(manifest_run_id, run_dir.run_id().to_string());
    }

    #[test]
    fn workflow_slug_matches_expected_format() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = RunDir::create(tmp.path(), "My Workflow!").unwrap();

        assert_eq!(run_dir.workflow_slug(), "my-workflow");

        let dir_name = run_dir.path().file_name().unwrap().to_string_lossy();
        assert!(
            dir_name.starts_with("my-workflow-"),
            "directory name should start with slugified workflow name"
        );
    }

    #[test]
    fn attempt_dir_returns_correct_handle() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = RunDir::create(tmp.path(), "attempt-dir-test").unwrap();

        let attempt = run_dir.attempt_dir("design", 0);
        let expected = AttemptDir::new(run_dir.path(), "design", 0);
        assert_eq!(attempt.path(), expected.path());
    }
}
