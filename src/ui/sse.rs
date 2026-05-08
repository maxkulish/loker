//! SSE streaming for trace events.
//!
//! Provides types and utilities for formatting trace events as Server-Sent Events.

use serde::{Deserialize, Serialize};

/// A trace event as it appears in `trace.jsonl`.
/// We use `serde_json::Value` for the data to preserve the full event structure,
/// but we have a typed wrapper for SSE metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TraceSseEvent {
    pub event_id: String,
    pub data: serde_json::Value,
}

impl TraceSseEvent {
    /// Formats the event as a standard SSE message.
    ///
    /// The format is:
    /// event: trace_event
    /// data: <json_payload>
    ///
    /// followed by two newlines.
    pub fn to_sse_format(&self) -> String {
        let data_json = serde_json::to_string(&self.data).unwrap_or_else(|_| "{}".to_string());
        format!("event: trace_event\nid: {}\ndata: {}\n\n", self.event_id, data_json)
    }
}

/// Formats a raw JSON line from `trace.jsonl` as an SSE event.
pub fn format_line_as_sse(event_id: &str, line: &str) -> Option<String> {
    if line.trim().is_empty() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_str(line).ok()?;
    let event = TraceSseEvent {
        event_id: event_id.to_string(),
        data: json,
    };
    Some(event.to_sse_format())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_sse_event_formatting() {
        let data = json!({
            "timestamp": "2026-01-01T00:00:00Z",
            "event_type": "step",
            "summary": "hello world"
        });
        let event = TraceSseEvent {
            event_id: "123".to_string(),
            data,
        };

        let formatted = event.to_sse_format();
        assert!(formatted.contains("event: trace_event"));
        assert!(formatted.contains("id: 123"));
        assert!(formatted.contains("data: {"));
        assert!(formatted.contains("\"summary\":\"hello world\""));
        assert!(formatted.ends_with("\n\n"));
    }

    #[test]
    fn test_format_line_as_sse_valid() {
        let line = r#"{"timestamp": "2026-01-01T00:00:00Z", "event_type": "step", "summary": "line test"}"#;
        let result = format_line_as_sse("abc", line).unwrap();
        assert!(result.contains("id: abc"));
        assert!(result.contains("data: {"));
        assert!(result.contains("\"summary\":\"line test\""));
    }

    #[test]
    fn test_format_line_as_sse_invalid() {
        let line = "not json";
        let result = format_line_as_sse("abc", line);
        assert!(result.is_none());
    }

    #[test]
    fn test_format_line_as_sse_empty() {
        let line = "   \n  ";
        let result = format_line_as_sse("abc", line);
        assert!(result.is_none());
    }
}
