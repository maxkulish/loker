use std::fs::File;
use std::path::Path;

use fs2::FileExt;

use crate::resume::ResumeError;

/// An advisory file lock scoped to a run directory.
///
/// The lock is acquired via `flock` (Unix) or `LockFileEx` (Windows) and is
/// released when this value is dropped (file descriptor close).
#[derive(Debug)]
pub struct RunLock {
    #[allow(dead_code)]
    file: File,
}

impl RunLock {
    /// Acquire an exclusive non-blocking advisory lock on `run_dir/.lock`.
    ///
    /// If the lock file does not exist, it is created empty. If the lock is
    /// already held by another process, returns `ResumeError::LockInUse`.
    pub fn acquire(run_dir: &Path) -> Result<Self, ResumeError> {
        let lock_path = run_dir.join(".lock");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(ResumeError::Io)?;

        file.try_lock_exclusive()
            .map_err(|_| ResumeError::LockInUse)?;

        Ok(Self { file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_acquires_and_releases() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = RunLock::acquire(tmp.path()).unwrap();
        drop(lock);
        // After drop, the lock should be released — we can acquire it again.
        let lock2 = RunLock::acquire(tmp.path()).unwrap();
        drop(lock2);
    }

    #[test]
    fn lock_contentious_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let lock = RunLock::acquire(tmp.path()).unwrap();
        // While `lock` lives, a second acquire must fail.
        let result = RunLock::acquire(tmp.path());
        assert!(
            matches!(result, Err(ResumeError::LockInUse)),
            "expected LockInUse, got {:?}",
            result
        );
        drop(lock);
    }
}
