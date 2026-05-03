use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Walk `run_dir` recursively, collect every `*.tmp` file whose mtime is
/// older than `ttl_seconds`, move them into `run_dir/attempts/_orphan_tmp/<timestamp>/`.
/// Returns the list of swept paths (for logging / test assertions).
///
/// # Errors
///
/// Returns `Err` on any IO failure (disk-full, permission denied, etc.).
/// Callers should abort rather than proceed with a dirty run directory.
pub fn sweep_stale_tmp(run_dir: &Path, ttl_seconds: u64) -> Result<Vec<PathBuf>, io::Error> {
    let mut swept = Vec::new();
    let now = SystemTime::now();
    let cutoff = match now.duration_since(UNIX_EPOCH) {
        Ok(dur) => dur.as_secs().saturating_sub(ttl_seconds),
        Err(_) => return Ok(swept), // clock before epoch → nothing to sweep
    };

    let mut tmp_files = Vec::new();
    collect_tmp_files(run_dir, &mut tmp_files)?;

    if tmp_files.is_empty() {
        return Ok(swept);
    }

    // Build destination directory
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S-%3f").to_string();
    let orphan_dir = run_dir.join("attempts").join("_orphan_tmp").join(&ts);
    fs::create_dir_all(&orphan_dir)?;

    for path in tmp_files {
        let mtime = match fs::metadata(&path) {
            Ok(meta) => match meta.modified() {
                Ok(t) => match t.duration_since(UNIX_EPOCH) {
                    Ok(dur) => dur.as_secs(),
                    Err(_) => continue,
                },
                Err(_) => continue,
            },
            Err(e) => return Err(e),
        };

        if mtime <= cutoff {
            // Move relative to run_dir to preserve tree structure
            let rel = path.strip_prefix(run_dir).unwrap_or(&path);
            let dest = orphan_dir.join(rel);
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&path, &dest)?;
            swept.push(dest);
        }
    }

    // Sync the orphan directory parent to guarantee rename visibility
    if let Some(parent) = orphan_dir.parent() {
        if let Ok(dir) = fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }

    Ok(swept)
}

fn collect_tmp_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), io::Error> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            // Skip the attempts/_orphan_tmp subtree to avoid re-sweeping
            let rel = path.strip_prefix(dir).ok();
            if rel
                .as_ref()
                .map(|p| {
                    p.components()
                        .next()
                        .map(|c| c.as_os_str() == "_orphan_tmp")
                        == Some(true)
                })
                .unwrap_or(false)
            {
                continue;
            }
            collect_tmp_files(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("tmp") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweep_no_match_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let result = sweep_stale_tmp(tmp.path(), 300).unwrap();
        assert!(result.is_empty(), "nothing to sweep in empty dir");
    }

    #[test]
    fn sweep_moves_stale_tmp() {
        let tmp = tempfile::tempdir().unwrap();

        // Create a stale tmp file (mtime = epoch)
        let stale = tmp.path().join("foo.tmp");
        fs::write(&stale, "data").unwrap();
        let epoch = SystemTime::UNIX_EPOCH;
        let file = fs::File::open(&stale).unwrap();
        file.set_modified(epoch).unwrap();

        let swept = sweep_stale_tmp(tmp.path(), 1).unwrap(); // 1s TTL
        assert_eq!(swept.len(), 1, "exactly one file swept");
        assert!(!stale.exists(), "original file removed");

        // Verify it ended up in orphan dir
        let orphaned = fs::read_dir(&swept[0].parent().unwrap()).unwrap().next();
        assert!(orphaned.is_some(), "orphan dir has file");
    }

    #[test]
    fn sweep_skips_recent_tmp() {
        let tmp = tempfile::tempdir().unwrap();
        let recent = tmp.path().join("recent.tmp");
        fs::write(&recent, "data").unwrap();

        let swept = sweep_stale_tmp(tmp.path(), 300).unwrap();
        assert!(swept.is_empty(), "recent tmp should not be swept");
        assert!(recent.exists(), "recent tmp stays in place");
    }
}
