use serde::{Deserialize, Serialize};

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
pub struct Manifest {
    #[serde(rename = "loker.run_id")]
    pub run_id: String,
    pub schema_version: u32,
    pub entries: Vec<ManifestEntry>,
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
}
