use std::fs::File;
use std::io::{self, Write as _};
use std::path::Path;

/// Atomic write helper: tmp → fsync → rename → parent-fsync.
///
/// Writes `contents` to a temporary file in the same directory as `path`,
/// calls `fsync` on the temp file, atomically renames it to `path`
/// (POSIX semantics: rename is atomic on the same filesystem), then
/// `fsync`s the parent directory to ensure the directory entry is durable.
///
/// On Windows the parent-directory fsync is skipped because directories
/// cannot be opened as regular files there.
pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(contents)?;
    tmp.as_file().sync_all()?;
    let _final_path = tmp.persist(path)?;

    // Parent-directory fsync on Unix ensures the directory entry update
    // is durable; on Windows this is a no-op (directories can't be opened
    // as regular files).
    #[cfg(unix)]
    {
        let parent_file = File::open(parent)?;
        parent_file.sync_all()?;
    }

    Ok(())
}
