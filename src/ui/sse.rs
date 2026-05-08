//! SSE streaming for trace events.
//!
//! Provides types and utilities for formatting trace events as Server-Sent Events.

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::sync::mpsc;

use crate::ui::trace_reader::read_from_offset;

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
    /// id: <offset>
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

/// Watches a trace file for updates and streams new lines.
pub struct TraceWatcher {
    path: PathBuf,
    offset: u64,
}

impl TraceWatcher {
    pub fn new(path: PathBuf, offset: u64) -> Self {
        Self { path, offset }
    }

    /// Start watching the file. Sends new lines through the provided channel.
    /// Returns when the channel is closed or an unrecoverable error occurs.
    pub async fn watch(mut self, tx: mpsc::Sender<(u64, String)>) -> anyhow::Result<()> {
        let (event_tx, mut event_rx) = mpsc::channel(100);
        
        // The watcher needs a sync callback. We use a channel to bridge to async.
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<notify::Event>| {
                if let Ok(event) = res {
                    if event.kind.is_modify() {
                        let _ = event_tx.blocking_send(());
                    }
                }
            },
            Config::default(),
        )?;

        watcher.watch(&self.path, RecursiveMode::NonRecursive)?;

        // Initial read to catch any events that happened between connection and watch start
        let (initial_lines, initial_offset) = read_from_offset(self.path.as_path(), self.offset);
        self.offset = initial_offset;
        
        // Note: for initial lines, we don't have an accurate per-line offset without more work,
        // but we can use the starting offset as a base. 
        // For simplicity and since we just want a unique ID, we'll use the current state.
        for line in initial_lines {
            if tx.send((self.offset, line)).await.is_err() {
                return Ok(());
            }
        }

        while let Some(_) = event_rx.recv().await {
            let (lines, new_offset) = read_from_offset(self.path.as_path(), self.offset);
            let mut current_pos = self.offset;
            for line in lines {
                let line_bytes = line.as_bytes().len() as u64;
                if tx.send((current_pos, line)).await.is_err() {
                    return Ok(());
                }
                current_pos += line_bytes + 1; // +1 for newline
            }
            self.offset = new_offset;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::io::Write;
    use tokio::time::{sleep, Duration};

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

    #[tokio::test]
    async fn test_notify_watcher_triggers_event() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("trace.jsonl");
        fs::write(&path, b"line1\n").unwrap();

        let (tx, mut rx) = mpsc::channel(10);
        let watcher = TraceWatcher::new(path.clone(), 0);

        let handle = tokio::spawn(async move {
            watcher.watch(tx).await
        });

        // Initial line should be received
        let first = rx.recv().await.expect("Should receive initial line");
        assert_eq!(first, "line1");

        // Write second line
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"line2\n")
            .unwrap();

        // Notify can be asynchronous, so we poll briefly
        let mut received = false;
        for _ in 0..20 {
            if let Ok(line) = rx.try_recv() {
                if line == "line2" {
                    received = true;
                    break;
                }
            }
            sleep(Duration::from_millis(100)).await;
        }
        assert!(received, "Should have received line2 from watcher");

        handle.abort();
    }
}
