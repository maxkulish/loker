use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

use crate::manifest::{dir_digest, Kind, Manifest, ManifestEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PhaseStatus {
    Started,
    Completed,
    Failed,
    None,
}

#[derive(Debug, Clone)]
pub enum HeartbeatStatus {
    Live(Heartbeat),
    Stale {
        last_tick: DateTime<Utc>,
        ttl_seconds: u64,
    },
    Missing,
}

#[derive(Debug, Clone)]
pub struct Heartbeat {
    pub writer_pid: i64,
    pub writer_host: String,
    pub tick_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("manifest schema mismatch: expected {expected}, found {found} at {path}")]
    ArtefactSchemaMismatch {
        expected: u32,
        found: u32,
        path: String,
    },

    #[error("artefact missing: {path}")]
    ArtefactMissing { path: String },

    #[error("artefact corrupt: {path} (expected {expected}, found {found})")]
    ArtefactCorrupt {
        path: String,
        expected: String,
        found: String,
    },

    #[error("stale writer heartbeat: last tick {last_tick} older than ttl {ttl_seconds}s")]
    StaleWriter {
        last_tick: DateTime<Utc>,
        ttl_seconds: u64,
    },

    #[error("live writer heartbeat: pid={writer_pid}, host={writer_host}")]
    LiveWriter {
        writer_pid: i64,
        writer_host: String,
    },

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug)]
pub struct RunState {
    pub run_id: String,
    pub entries: Vec<ManifestEntry>,
    pub dropped_orphans: Vec<ManifestEntry>,
    pub phase_status: HashMap<String, PhaseStatus>,
}

#[derive(Debug, Deserialize)]
struct CompletedMarker {
    manifest_entry_sha256: String,
    #[serde(default)]
    phase: Option<String>,
}

#[derive(Debug, Deserialize)]
struct HeartbeatMarker {
    writer_pid: i64,
    writer_host: String,
    tick_at: DateTime<Utc>,
}

impl RunState {
    /// Load and validate run state from `<run_dir>/manifest.json`.
    ///
    /// Resume path contract:
    /// - Drop entries not referenced by any `*.completed` marker.
    /// - Verify referenced artefact hashes against on-disk data before returning state.
    /// - Return a typed `LoadError` for schema/artefact/heartbeat issues.
    pub fn load(run_dir: &Path, heartbeat_ttl_seconds: u64) -> Result<Self, LoadError> {
        let manifest = Self::load_manifest(run_dir)?;
        let (phase_status, completed_hashes) = Self::load_markers(run_dir)?;

        let (entries, dropped_orphans) = Self::orphan_sweep(manifest.entries, &completed_hashes);

        // Fail fast on heartbeat state if present.
        if let Some(heartbeat) = Self::read_heartbeat(run_dir)? {
            let now = Utc::now();
            let age = now.signed_duration_since(heartbeat.tick_at);
            if age > Duration::seconds(heartbeat_ttl_seconds as i64) {
                return Err(LoadError::StaleWriter {
                    last_tick: heartbeat.tick_at,
                    ttl_seconds: heartbeat_ttl_seconds,
                });
            }

            return Err(LoadError::LiveWriter {
                writer_pid: heartbeat.writer_pid,
                writer_host: heartbeat.writer_host,
            });
        }

        Self::verify_entries(run_dir, &entries)?;

        Ok(Self {
            run_id: manifest.run_id,
            entries,
            dropped_orphans,
            phase_status,
        })
    }

    fn load_manifest(run_dir: &Path) -> Result<Manifest, LoadError> {
        let manifest_path = manifest_path(run_dir);
        let text = std::fs::read_to_string(&manifest_path)?;
        let manifest: Manifest = Manifest::from_json(&text)?;

        if manifest.schema_version != 1 {
            return Err(LoadError::ArtefactSchemaMismatch {
                expected: 1,
                found: manifest.schema_version,
                path: manifest_path.display().to_string(),
            });
        }

        for entry in &manifest.entries {
            if entry.schema_version != 1 {
                return Err(LoadError::ArtefactSchemaMismatch {
                    expected: 1,
                    found: entry.schema_version,
                    path: entry.name.clone(),
                });
            }
        }

        Ok(manifest)
    }

    fn load_markers(
        run_dir: &Path,
    ) -> Result<(HashMap<String, PhaseStatus>, HashSet<String>), LoadError> {
        let mut status = HashMap::new();
        let mut completed = HashSet::new();
        let markers_dir = run_dir.join("markers");

        if !markers_dir.exists() {
            return Ok((status, completed));
        }

        for dir_entry in std::fs::read_dir(&markers_dir)? {
            let dir_entry = dir_entry?;
            let path = dir_entry.path();
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            let Some(phase) = marker_phase(file_name) else {
                continue;
            };

            if file_name.ends_with(".completed") {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    if let Ok(marker) = serde_json::from_str::<CompletedMarker>(&text) {
                        completed.insert(marker.manifest_entry_sha256);
                    }
                }
                let _ = update_phase_status(&mut status, phase.clone(), PhaseStatus::Completed);
                continue;
            }

            if file_name.ends_with(".failed") {
                let _ = update_phase_status(&mut status, phase, PhaseStatus::Failed);
                continue;
            }

            if file_name.ends_with(".started") {
                let _ = update_phase_status(&mut status, phase, PhaseStatus::Started);
            }
        }

        Ok((status, completed))
    }

    fn verify_entries(run_dir: &Path, entries: &[ManifestEntry]) -> Result<(), LoadError> {
        for entry in entries {
            let path = run_dir.join(&entry.name);
            match entry.kind {
                Kind::ChangesDir => {
                    let computed = dir_digest(&path).map_err(LoadError::Io)?;
                    if computed != entry.sha256 {
                        return Err(LoadError::ArtefactCorrupt {
                            path: entry.name.clone(),
                            expected: entry.sha256.clone(),
                            found: computed,
                        });
                    }
                }
                Kind::DesignMd
                | Kind::ReviewMd
                | Kind::VerifyJson
                | Kind::PhaseResultJson
                | Kind::PendingJson
                | Kind::ResponseJson
                | Kind::SummaryJson
                | Kind::TraceJsonl => {
                    if !path.exists() {
                        return Err(LoadError::ArtefactMissing {
                            path: path.display().to_string(),
                        });
                    }

                    let bytes = std::fs::read(&path)?;
                    let computed = crate::manifest::sha256_hex(&bytes);
                    if computed != entry.sha256 {
                        return Err(LoadError::ArtefactCorrupt {
                            path: path.display().to_string(),
                            expected: entry.sha256.clone(),
                            found: computed,
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn orphan_sweep(
        entries: Vec<ManifestEntry>,
        completed_hashes: &HashSet<String>,
    ) -> (Vec<ManifestEntry>, Vec<ManifestEntry>) {
        let mut kept = Vec::new();
        let mut dropped = Vec::new();

        for entry in entries {
            if completed_hashes.contains(&entry.sha256) {
                kept.push(entry);
            } else {
                let phase = entry
                    .phase
                    .clone()
                    .or_else(|| entry.name.split('/').next().map(ToString::to_string));
                let phase = phase.unwrap_or_else(|| "unknown".to_string());
                let kind = kind_name(&entry.kind);
                eprintln!(
                    "orphan manifest entry dropped: phase={phase}, kind={kind}, sha256={}",
                    entry.sha256
                );
                dropped.push(entry);
            }
        }

        (kept, dropped)
    }

    fn read_heartbeat(run_dir: &Path) -> Result<Option<Heartbeat>, LoadError> {
        let heartbeat_path = run_dir.join("heartbeat.json");
        if !heartbeat_path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&heartbeat_path)?;
        let heartbeat: HeartbeatMarker = serde_json::from_str(&text)?;
        Ok(Some(Heartbeat {
            writer_pid: heartbeat.writer_pid,
            writer_host: heartbeat.writer_host,
            tick_at: heartbeat.tick_at,
        }))
    }

    #[allow(dead_code)]
    pub fn status_from_heartbeat(heartbeat: &Heartbeat, ttl_seconds: u64) -> HeartbeatStatus {
        let age = Utc::now().signed_duration_since(heartbeat.tick_at);
        if age > Duration::seconds(ttl_seconds as i64) {
            HeartbeatStatus::Stale {
                last_tick: heartbeat.tick_at,
                ttl_seconds,
            }
        } else {
            HeartbeatStatus::Live(heartbeat.clone())
        }
    }
}

fn manifest_path(run_dir: &Path) -> PathBuf {
    run_dir.join("manifest.json")
}

fn marker_phase(file_name: &str) -> Option<String> {
    file_name
        .strip_suffix(".completed")
        .or_else(|| file_name.strip_suffix(".failed"))
        .or_else(|| file_name.strip_suffix(".started"))
        .map(ToString::to_string)
}

fn update_phase_status(
    map: &mut HashMap<String, PhaseStatus>,
    phase: String,
    status: PhaseStatus,
) -> bool {
    let next_rank = status_rank(status);
    match map.get(&phase).copied() {
        Some(current) => {
            if status_rank(current) < next_rank {
                map.insert(phase, status);
                true
            } else {
                false
            }
        }
        None => {
            map.insert(phase, status);
            true
        }
    }
}

fn status_rank(status: PhaseStatus) -> u8 {
    match status {
        PhaseStatus::None => 0,
        PhaseStatus::Started => 1,
        PhaseStatus::Failed => 2,
        PhaseStatus::Completed => 3,
    }
}

fn kind_name(kind: &Kind) -> &'static str {
    match kind {
        Kind::DesignMd => "design.md",
        Kind::ReviewMd => "review.md",
        Kind::VerifyJson => "verify.json",
        Kind::PhaseResultJson => "phase_result.json",
        Kind::PendingJson => "pending.json",
        Kind::ResponseJson => "response.json",
        Kind::SummaryJson => "summary.json",
        Kind::ChangesDir => "changes/",
        Kind::TraceJsonl => "trace.jsonl",
    }
}
