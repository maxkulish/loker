//! Trace file reader — tails the last N events from a trace.jsonl file.
//!
//! Provides `tail_trace_file` which reads the file and returns the last N
//! lines as `TraceEventDisplay` values.

use std::fs;
use std::path::Path;

use crate::ui::templates::TraceEventDisplay;

/// Tail the last N lines of a trace.jsonl file.
///
/// Returns an empty Vec if the file is missing, unreadable, or empty.
/// Each line is parsed as JSON; lines that do not parse are skipped with
/// a placeholder summary.
pub fn tail_trace_file(trace_path: &Path, n: usize) -> Vec<TraceEventDisplay> {
    let text = match fs::read_to_string(trace_path) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    parse_trace_lines(&text, n)
}

/// Parse the last N trace events from already-read file content.
///
/// Used when the caller has read the file content itself and wants to
/// avoid a metadata race between reading and computing the byte offset.
pub fn parse_trace_lines(content: &str, n: usize) -> Vec<TraceEventDisplay> {
    let lines: Vec<&str> = content.lines().collect();
    let start = if lines.len() > n { lines.len() - n } else { 0 };

    lines[start..]
        .iter()
        .filter_map(|line| parse_trace_line(line))
        .collect()
}

/// Read new lines from a trace.jsonl file starting at a given offset.
///
/// Returns the new lines and the updated offset.
pub fn read_from_offset(trace_path: &Path, offset: u64) -> (Vec<(u64, String)>, u64) {
    let file = match fs::File::open(trace_path) {
        Ok(f) => f,
        Err(_) => return (Vec::new(), offset),
    };

    let metadata = match file.metadata() {
        Ok(m) => m,
        Err(_) => return (Vec::new(), offset),
    };

    let len = metadata.len();
    if len <= offset {
        return (Vec::new(), offset);
    }

    use std::io::{Read, Seek, SeekFrom};
    let mut reader = std::io::BufReader::new(file);
    if reader.seek(SeekFrom::Start(offset)).is_err() {
        return (Vec::new(), offset);
    }

    let mut buffer = Vec::new();
    if reader.read_to_end(&mut buffer).is_err() {
        return (Vec::new(), offset);
    }

    if buffer.is_empty() {
        return (Vec::new(), offset);
    }

    // Find the last newline to avoid capturing a partial line
    if let Some(last_newline_pos) = buffer.iter().rposition(|&b| b == b'\n') {
        let complete_part = &buffer[..=last_newline_pos];
        let mut lines = Vec::new();
        let mut current_pos = offset;

        let mut start = 0;
        while start < complete_part.len() {
            if let Some(end) = complete_part[start..].iter().position(|&b| b == b'\n') {
                let line_bytes = &complete_part[start..start + end];
                let line_str = String::from_utf8_lossy(line_bytes).to_string();
                if !line_str.trim().is_empty() {
                    lines.push((current_pos, line_str));
                }
                current_pos += (end + 1) as u64;
                start += end + 1;
            } else {
                break;
            }
        }

        return (lines, current_pos);
    }

    (Vec::new(), offset)
}

fn parse_trace_line(line: &str) -> Option<TraceEventDisplay> {
    if line.trim().is_empty() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_str(line).ok()?;
    let timestamp = json
        .get("timestamp")
        .or_else(|| json.get("time"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let event_type = json
        .get("event_type")
        .or_else(|| json.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("event")
        .to_string();
    let summary = json
        .get("summary")
        .or_else(|| json.get("message"))
        .map(|v| v.as_str().unwrap_or("").to_string())
        .or_else(|| {
            // Build a one-line summary from the JSON itself
            let s = serde_json::to_string(&json).unwrap_or_default();
            if s.len() > 120 {
                Some(format!("{}…", &s[..120]))
            } else {
                Some(s)
            }
        })
        .unwrap_or_default();
    Some(TraceEventDisplay {
        timestamp,
        event_type,
        summary,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_tail_trace_file_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("trace.jsonl");
        let events = tail_trace_file(&path, 10);
        assert!(events.is_empty());
    }

    #[test]
    fn test_tail_trace_file_last_n() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("trace.jsonl");
        let mut content = String::new();
        for i in 0..100 {
            content.push_str(
                &serde_json::json!({
                    "timestamp": format!("2026-01-01T00:00:{:02}Z", i),
                    "event_type": "step",
                    "message": format!("event {}", i)
                })
                .to_string(),
            );
            content.push('\n');
        }
        fs::write(&path, &content).unwrap();

        let events = tail_trace_file(&path, 10);
        assert_eq!(events.len(), 10);
        // Last 10 events — event 90 to 99
        assert!(events[0].summary.contains("event 90"));
        assert!(events[9].summary.contains("event 99"));
    }

    #[test]
    fn test_tail_trace_file_truncated() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("trace.jsonl");
        let mut content = String::new();
        for i in 0..5 {
            content.push_str(
                &serde_json::json!({
                    "timestamp": "2026-01-01T00:00:00Z",
                    "event_type": "step",
                    "summarized": format!("event {}", i)
                })
                .to_string(),
            );
            content.push('\n');
        }
        fs::write(&path, &content).unwrap();

        let events = tail_trace_file(&path, 10);
        assert_eq!(events.len(), 5);
    }

    #[test]
    fn test_tail_trace_file_skips_invalid_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("trace.jsonl");
        let content = "not json\n{\"timestamp\": \"2026-01-01T00:00:00Z\", \"event_type\": \"step\", \"summarized\": \"valid\"}\nmore garbage\n";
        fs::write(&path, content).unwrap();

        let events = tail_trace_file(&path, 10);
        // Only the valid line should appear
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn test_read_from_offset() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("trace.jsonl");
        let content = "line1\nline2\nline3\n";
        fs::write(&path, content.as_bytes()).unwrap();

        // Read from 0
        let (lines1, offset1) = read_from_offset(&path, 0);
        let lines1_content: Vec<String> = lines1.into_iter().map(|(_, s)| s).collect();
        assert_eq!(lines1_content, vec!["line1", "line2", "line3"]);
        assert_eq!(offset1, content.len() as u64);

        // Read from mid-line (should still happen, but we'll just get the rest)
        // "line1\n" is 6 bytes.
        let (lines2, offset2) = read_from_offset(&path, 6);
        let lines2_content: Vec<String> = lines2.into_iter().map(|(_, s)| s).collect();
        assert_eq!(lines2_content, vec!["line2", "line3"]);
        assert_eq!(offset2, content.len() as u64);

        // Read from EOF
        let (lines3, offset3) = read_from_offset(&path, content.len() as u64);
        assert!(lines3.is_empty());
        assert_eq!(offset3, content.len() as u64);
    }
}
