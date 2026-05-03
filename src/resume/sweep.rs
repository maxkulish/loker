use std::path::{Path, PathBuf};

/// Walk `run_dir` recursively, collect every `*.tmp` file whose mtime is
/// older than `ttl_seconds`, move them into `run_dir/attempts/_orphan_tmp/<timestamp>/`.
/// Returns the list of swept paths (for logging / test assertions).
pub fn sweep_stale_tmp(
    _run_dir: &Path,
    _ttl_seconds: u64,
) -> Result<Vec<PathBuf>, std::io::Error> {
    // STUB: will be implemented in ST4
    Ok(Vec::new())
}
