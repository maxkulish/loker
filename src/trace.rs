//! Trace emission primitives for OpenTelemetry GenAI-compatible spans.
//!
//! Provides a `TraceSink` trait that PhaseRunner calls at lifecycle points,
//! plus two implementations:
//! - `TraceWriter`  → append-only JSONL file sink
//! - `InMemorySink` → test capture sink

use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;

pub mod memory;
pub mod writer;

pub use memory::InMemorySink;
pub use writer::TraceWriter;

/// Trait for emitting trace spans. Implementations may write to disk,
/// memory, or a downstream OTel pipeline.
///
/// All methods take `&self` so the sink can be shared across async tasks.
pub trait TraceSink: Send + Sync {
    fn phase_started(&self, ctx: &PhaseSpanContext);
    fn backend_call(
        &self,
        ctx: &PhaseSpanContext,
        attempt: &AttemptSpanContext,
        result: &BackendSpanResult,
    );
    fn aggregator_fold(&self, ctx: &PhaseSpanContext, kind: &str, input_count: usize);
    fn verify_result(&self, ctx: &PhaseSpanContext, hook_name: &str, result: &VerifySpanResult);
    fn phase_finished(&self, ctx: &PhaseSpanContext, outcome: &str, duration_ms: u64);
    fn error(&self, ctx: &PhaseSpanContext, kind: &str, message: &str, duration_ms: u64);
}

/// Context shared by all spans within a phase.
#[derive(Clone, Debug)]
pub struct PhaseSpanContext {
    pub trace_id: Uuid,
    pub span_id: String,
    pub phase: String,
    pub strategy: String,
    pub aggregator: String,
    pub verify_hook: Option<String>,
}

/// Per-attempt context for backend spans.
#[derive(Clone, Debug)]
pub struct AttemptSpanContext {
    pub span_id: String,
    pub attempt: usize,
    pub backend: String,
    pub model: Option<String>,
}

/// Result of a backend call, used to populate gen_ai.* fields.
pub struct BackendSpanResult {
    pub duration_ms: u64,
    pub usage_input_tokens: Option<u64>,
    pub usage_output_tokens: Option<u64>,
    pub finish_reasons: Vec<String>,
    pub error: Option<String>,
}

/// Result of a verify hook invocation.
pub struct VerifySpanResult {
    pub passed: bool,
    pub message: Option<String>,
    pub duration_ms: u64,
}

/// Generates a random 16-byte span ID as a 32-character hex string.
pub fn new_span_id() -> String {
    let bytes: [u8; 16] = rand::random();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Build the common JSON object skeleton shared by every span kind.
pub(crate) fn build_span_skeleton(
    trace_id: Uuid,
    span_id: &str,
    parent_span_id: Option<&str>,
    name: &str,
) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    map.insert("trace_id".to_string(), Value::String(trace_id.to_string()));
    map.insert("span_id".to_string(), Value::String(span_id.to_string()));
    if let Some(pid) = parent_span_id {
        map.insert("parent_span_id".to_string(), Value::String(pid.to_string()));
    }
    map.insert(
        "timestamp".to_string(),
        Value::String(Utc::now().to_rfc3339()),
    );
    map.insert("name".to_string(), Value::String(name.to_string()));
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_span_id_is_32_hex_chars() {
        let id = new_span_id();
        assert_eq!(id.len(), 32);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn new_span_id_produces_different_values() {
        let a = new_span_id();
        let b = new_span_id();
        assert_ne!(a, b);
    }

    #[test]
    fn build_span_skeleton_has_required_keys() {
        let trace_id = Uuid::new_v4();
        let span_id = new_span_id();
        let map = build_span_skeleton(trace_id, &span_id, None, "phase.test");
        assert!(map.contains_key("trace_id"));
        assert!(map.contains_key("span_id"));
        assert!(!map.contains_key("parent_span_id"));
        assert!(map.contains_key("timestamp"));
        assert!(map.contains_key("name"));
        assert_eq!(map["name"], "phase.test");
    }

    #[test]
    fn build_span_skeleton_with_parent() {
        let trace_id = Uuid::new_v4();
        let span_id = new_span_id();
        let parent = new_span_id();
        let map = build_span_skeleton(trace_id, &span_id, Some(&parent), "backend.mock");
        assert_eq!(map["parent_span_id"], parent);
    }
}
