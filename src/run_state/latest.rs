use std::io;
use std::path::{Path, PathBuf};

use crate::run_state::atomic_write;

/// Best-effort convenience pointer to the latest completed attempt.
///
/// On Unix, creates a symlink `run_dir/<phase>/latest → ../attempts/<phase>/<n>/`.
/// If symlinks are unavailable (Windows without privileges, or the call fails),
/// falls back to writing a `latest.json` pointer file with the same metadata.
pub struct LatestPointer;

impl LatestPointer {
    /// Update the latest pointer for `phase` to point at `attempt`.
    ///
    /// This is best-effort: errors are logged (via `eprintln`) and returned
    /// but do **not** block the caller.  A phase can complete successfully
    /// even if the convenience pointer cannot be created.
    pub fn update(run_dir: &Path, phase: &str, attempt: u32) -> io::Result<()> {
        let phase_dir = run_dir.join(phase);
        std::fs::create_dir_all(&phase_dir)?;

        let target = PathBuf::from(format!("../attempts/{phase}/{attempt}/"));

        // Best-effort symlink on Unix
        #[cfg(unix)]
        {
            let latest_link = phase_dir.join("latest");
            let _ = std::fs::remove_file(&latest_link);
            match std::os::unix::fs::symlink(&target, &latest_link) {
                Ok(()) => return Ok(()),
                Err(e) => {
                    eprintln!(
                        "warn: symlink latest for {phase} attempt {attempt} failed: {e}, \
                         falling back to latest.json"
                    );
                }
            }
        }

        // Fallback (non-Unix or symlink failed): write latest.json
        let pointer = phase_dir.join("latest.json");
        let body = serde_json::json!({
            "attempt": attempt,
            "path": format!("attempts/{phase}/{attempt}/"),
            "updated_at": chrono::Utc::now().to_rfc3339(),
        });
        atomic_write(&pointer, body.to_string().as_bytes())?;
        Ok(())
    }

    /// Resolve the latest pointer for `phase`.
    ///
    /// Returns `Some(path)` if a symlink or `latest.json` exists.
    /// Returns `None` if neither exists.
    pub fn resolve(run_dir: &Path, phase: &str) -> Option<PathBuf> {
        let phase_dir = run_dir.join(phase);

        // Try symlink first — use symlink_metadata so broken symlinks are
        // still detected (the symlink itself is the pointer).
        let symlink = phase_dir.join("latest");
        if std::fs::symlink_metadata(&symlink).is_ok() {
            // On Unix, read the symlink target and resolve it relative to phase_dir
            #[cfg(unix)]
            {
                if let Ok(target) = std::fs::read_link(&symlink) {
                    return Some(phase_dir.join(target));
                }
            }
            // On non-Unix, a regular file named "latest" might be the fallback
            // json or something else; fall through to json check.
        }

        // Try latest.json
        let json_path = phase_dir.join("latest.json");
        if json_path.exists() {
            if let Ok(text) = std::fs::read_to_string(&json_path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(path) = json.get("path").and_then(|v| v.as_str()) {
                        return Some(run_dir.join(path));
                    }
                }
            }
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_pointer_updates_and_resolves() {
        let tmp = tempfile::tempdir().unwrap();
        LatestPointer::update(tmp.path(), "design", 2).unwrap();

        let resolved = LatestPointer::resolve(tmp.path(), "design").unwrap();
        assert!(resolved.to_string_lossy().contains("attempts/design/2"));
    }

    #[test]
    fn latest_pointer_overwrites_previous() {
        let tmp = tempfile::tempdir().unwrap();
        LatestPointer::update(tmp.path(), "design", 0).unwrap();
        LatestPointer::update(tmp.path(), "design", 1).unwrap();

        let resolved = LatestPointer::resolve(tmp.path(), "design").unwrap();
        assert!(resolved.to_string_lossy().contains("attempts/design/1"));
    }

    #[test]
    fn latest_pointer_cross_phase_isolation() {
        let tmp = tempfile::tempdir().unwrap();
        LatestPointer::update(tmp.path(), "design", 3).unwrap();
        LatestPointer::update(tmp.path(), "review", 1).unwrap();

        let d = LatestPointer::resolve(tmp.path(), "design").unwrap();
        let r = LatestPointer::resolve(tmp.path(), "review").unwrap();
        assert!(d.to_string_lossy().contains("attempts/design/3"));
        assert!(r.to_string_lossy().contains("attempts/review/1"));
    }

    #[test]
    fn latest_pointer_missing_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(LatestPointer::resolve(tmp.path(), "design").is_none());
    }
}
