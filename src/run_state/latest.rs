use std::io;
use std::path::{Path, PathBuf};

use crate::run_state::atomic_write;

/// Best-effort convenience pointer to the latest attempt.
///
/// - If the attempt directory still exists (in-progress or failed) and
///   symlinks are supported, creates `run_dir/<phase>/latest` pointing to
///   `../attempts/<phase>/<n>/`.
/// - If the attempt was promoted to canonical, writes `latest.json` with
///   the canonical path to avoid self-referential symlinks.
/// - Falls back to `latest.json` on all platforms where symlinks fail.
pub struct LatestPointer;

impl LatestPointer {
    /// Update the latest pointer for `phase` to point at `attempt`.
    ///
    /// This is best-effort: errors are logged (via `eprintln`) and returned
    /// but do **not** block the caller.  A phase can complete successfully
    /// even if the convenience pointer cannot be created.
    ///
    /// # Note
    /// If the attempt directory was promoted (no longer exists), the pointer
    /// resolves to the canonical phase directory (`./`).  Callers should
    /// not propagate errors with `?`; use `.ok()` or log them.
    pub fn update(run_dir: &Path, phase: &str, attempt: u32) -> io::Result<()> {
        let phase_dir = run_dir.join(phase);
        std::fs::create_dir_all(&phase_dir)?;

        // If the attempt dir still exists (in-progress or failed), point to it.
        // If it was promoted, write latest.json with the canonical path instead
        // of creating a self-referential symlink.
        let attempt_dir = run_dir
            .join("attempts")
            .join(phase)
            .join(attempt.to_string());

        if attempt_dir.exists() {
            let target = PathBuf::from(format!("../attempts/{phase}/{attempt}/"));
            // Best-effort symlink on Unix
            #[cfg(unix)]
            {
                let latest_link = phase_dir.join("latest");
                let _ = std::fs::remove_file(&latest_link);
                // Also remove any stale latest.json from a prior promoted attempt
                let _ = std::fs::remove_file(phase_dir.join("latest.json"));
                if std::os::unix::fs::symlink(&target, &latest_link).is_ok() {
                    return Ok(());
                }
            }
            // Symlink failed or non-Unix: write fallback JSON
            let pointer = phase_dir.join("latest.json");
            let body = serde_json::json!({
                "attempt": attempt,
                "path": format!("attempts/{phase}/{attempt}/"),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            });
            atomic_write(&pointer, body.to_string().as_bytes())?;
            Ok(())
        } else {
            // Promoted attempt: write latest.json pointing to canonical path.
            // Remove any stale symlink first so it doesn't shadow the json.
            let _ = std::fs::remove_file(phase_dir.join("latest"));
            let pointer = phase_dir.join("latest.json");
            let body = serde_json::json!({
                "attempt": attempt,
                "path": format!("{phase}/"),
                "updated_at": chrono::Utc::now().to_rfc3339(),
            });
            atomic_write(&pointer, body.to_string().as_bytes())?;
            Ok(())
        }
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
        // Create the attempt dir so latest points to it (not canonical)
        let attempt_dir = tmp.path().join("attempts").join("design").join("2");
        std::fs::create_dir_all(&attempt_dir).unwrap();

        LatestPointer::update(tmp.path(), "design", 2).unwrap();

        let resolved = LatestPointer::resolve(tmp.path(), "design").unwrap();
        assert!(resolved.to_string_lossy().contains("attempts/design/2"));
    }

    #[test]
    fn latest_pointer_overwrites_previous() {
        let tmp = tempfile::tempdir().unwrap();
        for (phase, attempt) in [("design", 0), ("design", 1)] {
            let dir = tmp
                .path()
                .join("attempts")
                .join(phase)
                .join(attempt.to_string());
            std::fs::create_dir_all(&dir).unwrap();
        }

        LatestPointer::update(tmp.path(), "design", 0).unwrap();
        LatestPointer::update(tmp.path(), "design", 1).unwrap();

        let resolved = LatestPointer::resolve(tmp.path(), "design").unwrap();
        assert!(resolved.to_string_lossy().contains("attempts/design/1"));
    }

    #[test]
    fn latest_pointer_cross_phase_isolation() {
        let tmp = tempfile::tempdir().unwrap();
        let d3 = tmp.path().join("attempts").join("design").join("3");
        std::fs::create_dir_all(&d3).unwrap();
        let r1 = tmp.path().join("attempts").join("review").join("1");
        std::fs::create_dir_all(&r1).unwrap();

        LatestPointer::update(tmp.path(), "design", 3).unwrap();
        LatestPointer::update(tmp.path(), "review", 1).unwrap();

        let d = LatestPointer::resolve(tmp.path(), "design").unwrap();
        let r = LatestPointer::resolve(tmp.path(), "review").unwrap();
        assert!(d.to_string_lossy().contains("attempts/design/3"));
        assert!(r.to_string_lossy().contains("attempts/review/1"));
    }

    #[test]
    fn latest_pointer_points_to_canonical_after_promotion() {
        let tmp = tempfile::tempdir().unwrap();
        // No attempt dir → should point to canonical
        LatestPointer::update(tmp.path(), "design", 2).unwrap();

        let resolved = LatestPointer::resolve(tmp.path(), "design").unwrap();
        // Should resolve to the canonical phase directory
        assert!(resolved.to_string_lossy().contains("design"));
    }

    #[test]
    fn latest_pointer_missing_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(LatestPointer::resolve(tmp.path(), "design").is_none());
    }
}
