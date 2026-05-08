//! Artefact path resolution with traversal + symlink containment.
//!
//! `resolve_artefact` joins a run root, a run ID, and a user-supplied
//! relative path, then validates that the resolved file stays inside the
//! run directory and is not reached via a symlink.

use std::path::{Path, PathBuf};

/// Errors from artefact path resolution.
#[derive(Debug, thiserror::Error)]
pub enum ArtefactError {
    #[error("artefact path escapes run root")]
    Traversal,
    #[error("artefact not found")]
    NotFound,
    #[error("artefact path contains symlink")]
    Symlink,
    #[error("artefact io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolve an artefact path under `run_root/runs/<run_id>/`.
///
/// 1. Validate `run_id` (no path separators, no `..`).
/// 2. Percent-decode and split `rel` on `/` into components.
/// 3. Reject empty, `.`, `..`, or components containing `\` or NUL.
/// 4. Build the candidate path by joining components under the run directory.
/// 5. Walk the *unresolved* candidate path: if any component is a symlink, refuse.
/// 6. Canonicalise and prefix-check against the canonical run root.
/// 7. Return the canonical path only if it exists and is a file.
pub fn resolve_artefact(
    project_root: &Path,
    run_id: &str,
    rel: &str,
) -> Result<PathBuf, ArtefactError> {
    // Validate run_id
    if run_id.is_empty()
        || run_id.contains("..")
        || run_id.contains('/')
        || run_id.contains('\\')
        || run_id.contains('\0')
    {
        return Err(ArtefactError::Traversal);
    }

    let run_root = project_root.join("runs").join(run_id);

    // Percent-decode the entire relative path first, then split on '/'
    let decoded_rel = percent_decode(rel);
    let components: Vec<&str> = decoded_rel.split('/').collect();
    let mut validated = Vec::with_capacity(components.len());
    for comp in &components {
        if comp.is_empty() || *comp == "." || *comp == ".." {
            return Err(ArtefactError::Traversal);
        }
        if comp.contains('\0') || comp.contains('\\') {
            return Err(ArtefactError::Traversal);
        }
        validated.push(*comp);
    }

    // Build the candidate path
    let mut candidate = run_root.clone();
    for comp in validated {
        candidate = candidate.join(comp);
    }

    // Walk the unresolved path and reject any symlink along the way
    let mut current = run_root.clone();
    // Check run_root itself is not a symlink
    if is_symlink(&current)? {
        return Err(ArtefactError::Symlink);
    }

    for comp in &components {
        current = current.join(comp);
        if is_symlink(&current)? {
            return Err(ArtefactError::Symlink);
        }
    }

    // Canonicalise and prefix-check
    let canonical = match std::fs::canonicalize(&candidate) {
        Ok(p) => p,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ArtefactError::NotFound);
        }
        Err(e) => return Err(ArtefactError::Io(e)),
    };
    let canonical_root = match std::fs::canonicalize(&run_root) {
        Ok(p) => p,
        Err(e) => return Err(ArtefactError::Io(e)),
    };
    if !canonical.starts_with(&canonical_root) {
        return Err(ArtefactError::Traversal);
    }

    if !canonical.exists() {
        return Err(ArtefactError::NotFound);
    }

    Ok(canonical)
}

/// Simple percent-decoder for the artefact path segment.
fn percent_decode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(a), Some(b)) = (h1, h2) {
                if let Ok(byte) = u8::from_str_radix(&format!("{}{}", a, b), 16) {
                    result.push(byte as char);
                    continue;
                }
            }
            // Malformed percent encoding — pass through literally
            result.push(c);
            if let Some(a) = h1 {
                result.push(a);
            }
            if let Some(b) = h2 {
                result.push(b);
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Check whether `path` is a symlink.
fn is_symlink(path: &Path) -> Result<bool, ArtefactError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.file_type().is_symlink()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(ArtefactError::Io(e)),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_run(project_root: &Path, run_id: &str, rel: &str, content: &[u8]) -> PathBuf {
        let run_dir = project_root.join("runs").join(run_id);
        let artefact_dir = run_dir.join("artefacts");
        fs::create_dir_all(&artefact_dir).unwrap();
        let path = artefact_dir.join(rel);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn canonical_path_inside_root_is_allowed() {
        let tmp = tempfile::tempdir().unwrap();
        setup_run(tmp.path(), "run-001", "review.md", b"# Review");

        let result = resolve_artefact(tmp.path(), "run-001", "artefacts/review.md");
        assert!(result.is_ok(), "expected ok, got {:?}", result);
    }

    #[test]
    fn dotdot_segment_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        setup_run(tmp.path(), "run-002", "review.md", b"# Review");

        let result = resolve_artefact(tmp.path(), "run-002", "../evil.md");
        assert!(
            matches!(result, Err(ArtefactError::Traversal)),
            "expected Traversal, got {:?}",
            result
        );
    }

    #[test]
    fn percent_encoded_traversal_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        setup_run(tmp.path(), "run-003", "review.md", b"# Review");

        let result = resolve_artefact(tmp.path(), "run-003", "%2e%2e%2fetc%2fpasswd");
        assert!(
            matches!(result, Err(ArtefactError::Traversal)),
            "expected Traversal, got {:?}",
            result
        );
    }

    #[test]
    fn absolute_path_param_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        setup_run(tmp.path(), "run-004", "review.md", b"# Review");

        let result = resolve_artefact(tmp.path(), "run-004", "/etc/passwd");
        assert!(
            matches!(result, Err(ArtefactError::Traversal)),
            "expected Traversal, got {:?}",
            result
        );
    }

    #[test]
    fn nonexistent_path_returns_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("runs").join("run-005")).unwrap();

        let result = resolve_artefact(tmp.path(), "run-005", "missing.md");
        assert!(
            matches!(result, Err(ArtefactError::NotFound)),
            "expected NotFound, got {:?}",
            result
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escaping_root_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path().join("runs").join("run-006");
        fs::create_dir_all(&run_dir).unwrap();
        let escape = run_dir.join("escape");
        std::os::unix::fs::symlink("/etc/passwd", &escape).unwrap();

        let result = resolve_artefact(tmp.path(), "run-006", "escape");
        assert!(
            matches!(result, Err(ArtefactError::Symlink)),
            "expected Symlink, got {:?}",
            result
        );
    }

    #[cfg(unix)]
    #[test]
    fn intermediate_symlink_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path().join("runs").join("run-007");
        fs::create_dir_all(&run_dir).unwrap();
        let dir = run_dir.join("dir");
        let tmp_dir = tmp.path().join("tmp_target");
        fs::create_dir_all(&tmp_dir).unwrap();
        fs::write(tmp_dir.join("foo.txt"), b"bar").unwrap();
        std::os::unix::fs::symlink(&tmp_dir, &dir).unwrap();

        let result = resolve_artefact(tmp.path(), "run-007", "dir/foo.txt");
        assert!(
            matches!(result, Err(ArtefactError::Symlink)),
            "expected Symlink, got {:?}",
            result
        );
    }
}
