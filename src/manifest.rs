use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::family::PhaseError;
use crate::run_state::atomic_write;

/// Completed marker — subset of the marker file schema, just enough for orphan sweep.
#[derive(Debug, Clone, Deserialize)]
struct CompletedMarker {
    manifest_entry_sha256: String,
}

#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Phase(#[from] PhaseError),
}

/// Artefact kind. Serialises to the bare strings defined in manifest.schema.json.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    #[serde(rename = "design.md")]
    DesignMd,
    #[serde(rename = "review.md")]
    ReviewMd,
    #[serde(rename = "verify.json")]
    VerifyJson,
    #[serde(rename = "phase_result.json")]
    PhaseResultJson,
    #[serde(rename = "pending.json")]
    PendingJson,
    #[serde(rename = "response.json")]
    ResponseJson,
    #[serde(rename = "summary.json")]
    SummaryJson,
    #[serde(rename = "changes/")]
    ChangesDir,
    #[serde(rename = "trace.jsonl")]
    TraceJsonl,
}

/// Producer backend that created the artefact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Producer {
    #[serde(rename = "single")]
    Single,
    #[serde(rename = "parallel")]
    Parallel,
    #[serde(rename = "escalating")]
    Escalating,
    #[serde(rename = "verify")]
    Verify,
    #[serde(rename = "hitl")]
    Hitl,
}

/// A single entry in the manifest — one artefact produced by one phase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestEntry {
    pub name: String,
    pub kind: Kind,
    pub schema_version: u32,
    pub sha256: String,
    pub producer: Producer,
    pub phase: Option<String>,
    pub attempt: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Manifest envelope — the artefact registry for a single run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(rename = "loker.run_id")]
    pub run_id: String,
    pub schema_version: u32,
    pub entries: Vec<ManifestEntry>,
}

/// sha256 hex string of arbitrary bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(hex, "{:02x}", b);
    }
    hex
}

/// Deterministic directory digest for `kind: changes/`.
/// Walks the directory recursively, collects (relative_path, sha256_hex(content))
/// for every regular file (flattened including subdirs), sorts by path,
/// produces "<path>\t<sha256>\n" per line, then sha256_hex of the whole
/// concatenation.
pub fn dir_digest(root: &Path) -> Result<String, std::io::Error> {
    fn walk<'a>(
        root: &'a Path,
        prefix: &'a Path,
        out: &mut Vec<(String, String)>,
    ) -> Result<(), std::io::Error> {
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_file() {
                let rel = path.strip_prefix(prefix).unwrap_or(&path);
                let content = std::fs::read(&path)?;
                let hash = sha256_hex(&content);
                out.push((rel.to_string_lossy().into_owned(), hash));
            } else if file_type.is_dir() {
                walk(&path, prefix, out)?;
            }
        }
        Ok(())
    }

    let mut entries = Vec::new();
    walk(root, root, &mut entries)?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (rel, hash) in entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\t");
        hasher.update(hash.as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(hex, "{:02x}", b);
    }
    Ok(hex)
}



impl ManifestEntry {
    /// Create a ManifestEntry from a byte payload (auto-computes sha256).
    pub fn from_payload(
        name: impl Into<String>,
        kind: Kind,
        schema_version: u32,
        producer: Producer,
        phase: Option<String>,
        attempt: Option<u32>,
        payload: &[u8],
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            schema_version,
            sha256: sha256_hex(payload),
            producer,
            phase,
            attempt,
            created_at: Some(chrono::Utc::now()),
        }
    }
}

impl Manifest {
    /// Create an empty manifest for a given run id.
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            schema_version: 1,
            entries: Vec::new(),
        }
    }

    /// Serialize to a JSON string (pretty-printed).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from a JSON string.
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Load a manifest file from disk, validate schema version, run orphan sweep.
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let content = std::fs::read_to_string(path)?;
        let mut manifest: Manifest = Self::from_json(&content)?;

        if manifest.schema_version != 1 {
            return Err(PhaseError::ArtefactSchemaMismatch {
                detail: format!(
                    "manifest schema_version is {}; only v1 is supported",
                    manifest.schema_version
                ),
            }
            .into());
        }

        // Orphan sweep: drop entries whose sha256 is not referenced by any
        // markers/<phase>.completed file under the run directory.
        // Only sweep when the markers directory exists; if it doesn't, this is
        // a normal load (not crash recovery) and all entries are valid.
        let run_dir = path.parent().unwrap_or(Path::new("."));
        let markers_dir = run_dir.join("markers");
        if markers_dir.is_dir() {
            let mut referenced = std::collections::HashSet::<String>::new();
            for entry in std::fs::read_dir(&markers_dir)? {
                let entry = entry?;
                let file_name = entry.file_name();
                let Some(name) = file_name.to_str() else {
                    continue;
                };
                if !name.ends_with(".completed") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };
                let Ok(marker) = serde_json::from_str::<CompletedMarker>(&text) else {
                    continue;
                };
                referenced.insert(marker.manifest_entry_sha256);
            }

            let original_len = manifest.entries.len();
            manifest.entries.retain(|e| {
                if referenced.contains(&e.sha256) {
                    true
                } else {
                    // TODO(T-029): replace eprintln with injected log sink once trace writer lands
                    eprintln!("orphan manifest entry dropped: {} ({})", e.name, e.sha256);
                    false
                }
            });
            let dropped = original_len - manifest.entries.len();
            if dropped > 0 {
                // TODO(T-029): replace eprintln with injected log sink
                eprintln!(
                    "orphan sweep dropped {} entries from manifest {}",
                    dropped, manifest.run_id
                );
            }
        }

        // Validate per-entry schema_version
        for entry in &manifest.entries {
            if entry.schema_version != 1 {
                return Err(PhaseError::ArtefactSchemaMismatch {
                    detail: format!(
                        "entry '{}' has schema_version {}; only v1 is supported",
                        entry.name, entry.schema_version
                    ),
                }
                .into());
            }
        }

        Ok(manifest)
    }

    /// Append an entry and atomically rewrite the manifest file on disk.
    pub fn append(&mut self, entry: ManifestEntry, path: &Path) -> Result<(), ManifestError> {
        self.entries.push(entry);
        let json = self.to_json();
        match json {
            Ok(json_str) => match atomic_write(path, json_str.as_bytes()) {
                Ok(()) => Ok(()),
                Err(e) => {
                    self.entries.pop(); // rollback on write failure
                    Err(e.into())
                }
            },
            Err(e) => {
                self.entries.pop(); // rollback on serialization failure
                Err(e.into())
            }
        }
    }

    /// Look up the sha256 of an entry by its name.  O(N) — fine for v0 sizes.
    pub fn sha256_for(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.sha256.as_str())
    }

    /// Content-addressed verification: does the payload match the recorded sha256?
    pub fn verify(&self, name: &str, payload: &[u8]) -> Result<(), PhaseError> {
        let recorded = self
            .sha256_for(name)
            .ok_or_else(|| PhaseError::ArtefactSchemaMismatch {
                detail: format!("entry '{}' not found in manifest", name),
            })?;
        let computed = sha256_hex(payload);
        if recorded != computed {
            return Err(PhaseError::ArtefactSchemaMismatch {
                detail: format!(
                    "sha256 mismatch for '{}': recorded {} vs computed {}",
                    name, recorded, computed
                ),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_manifest_roundtrips() {
        let manifest = Manifest::new("run-001");
        let json = manifest.to_json().unwrap();
        let loaded: Manifest = Manifest::from_json(&json).unwrap();
        assert_eq!(manifest.run_id, loaded.run_id);
        assert_eq!(manifest.schema_version, loaded.schema_version);
        assert_eq!(manifest.entries, loaded.entries);
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        let data = b"hello world";
        let got = sha256_hex(data);
        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert_eq!(got, expected);
    }

    #[test]
    fn atomic_write_and_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("manifest.json");
        let contents = b"{\"loker.run_id\":\"r1\",\"schema_version\":1,\"entries\":[]}";
        atomic_write(&path, contents).unwrap();
        let read = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read.as_bytes(), contents);
    }
}
