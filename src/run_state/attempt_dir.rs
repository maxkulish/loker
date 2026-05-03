use std::io;
use std::path::{Path, PathBuf};

/// A scoped directory for one phase attempt.
///
/// The attempt directory lives at `run_dir/attempts/<phase>/<attempt>/`.
/// On success, the entire directory can be atomically promoted to the canonical
/// phase path (`run_dir/<phase>/`). On failure, the directory is left in place
/// as postmortem debris.
pub struct AttemptDir {
    path: PathBuf,
}

impl AttemptDir {
    /// Create a new handle for the given run, phase, and attempt number.
    pub fn new(run_dir: &Path, phase: &str, attempt: u32) -> Self {
        Self {
            path: run_dir
                .join("attempts")
                .join(phase)
                .join(attempt.to_string()),
        }
    }

    /// Return the attempt-scoped directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Idempotently create the attempt directory (and any parent dirs).
    pub fn create(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.path)
    }

    /// Atomically promote the attempt directory to the canonical phase path.
    ///
    /// On the same filesystem this is a single `rename(2)` — atomic and
    /// crash-safe.  If the source and destination are on different devices
    /// (should not happen inside a single run directory, but we guard against
    /// it) we fall back to a recursive copy + remove.
    pub fn promote_to_canonical(&self, canonical_dir: &Path) -> io::Result<()> {
        // Ensure canonical parent exists
        if let Some(parent) = canonical_dir.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Try atomic rename first
        match std::fs::rename(&self.path, canonical_dir) {
            Ok(()) => Ok(()),
            #[allow(clippy::incompatible_msrv)]
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                Self::copy_tree(&self.path, canonical_dir)?;
                std::fs::remove_dir_all(&self.path)?;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Recursively copy a directory tree.
    fn copy_tree(src: &Path, dst: &Path) -> io::Result<()> {
        std::fs::create_dir_all(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                Self::copy_tree(&src_path, &dst_path)?;
            } else {
                std::fs::copy(&src_path, &dst_path)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_dir_path_computed_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = AttemptDir::new(tmp.path(), "design", 3);
        assert_eq!(
            dir.path(),
            tmp.path().join("attempts").join("design").join("3")
        );
    }

    #[test]
    fn attempt_dir_create_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = AttemptDir::new(tmp.path(), "design", 0);
        dir.create().unwrap();
        assert!(dir.path().exists());
        // Second create should not fail
        dir.create().unwrap();
        assert!(dir.path().exists());
    }

    #[test]
    fn attempt_dir_promote_atomic() {
        let tmp = tempfile::tempdir().unwrap();
        let attempt_dir = AttemptDir::new(tmp.path(), "design", 0);
        attempt_dir.create().unwrap();

        // Write a file into the attempt dir
        let file_path = attempt_dir.path().join("design.md");
        std::fs::write(&file_path, b"hello design").unwrap();

        let canonical = tmp.path().join("design");
        attempt_dir.promote_to_canonical(&canonical).unwrap();

        // Canonical path should now have the file
        assert!(canonical.join("design.md").exists());
        // Attempt dir should be gone
        assert!(!attempt_dir.path().exists());
    }

    #[test]
    fn attempt_dir_promote_nested_files() {
        let tmp = tempfile::tempdir().unwrap();
        let attempt_dir = AttemptDir::new(tmp.path(), "review", 1);
        attempt_dir.create().unwrap();

        std::fs::create_dir_all(attempt_dir.path().join("sub")).unwrap();
        std::fs::write(attempt_dir.path().join("sub").join("file.txt"), b"nested").unwrap();

        let canonical = tmp.path().join("review");
        attempt_dir.promote_to_canonical(&canonical).unwrap();

        assert!(canonical.join("sub").join("file.txt").exists());
        assert!(!attempt_dir.path().exists());
    }
}
