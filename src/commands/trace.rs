use std::io::{BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result};
use colored::{control, Colorize};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Color output policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorChoice {
    #[default]
    Auto,
    Always,
    Never,
}

/// Pretty-printer for `trace.jsonl` spans.
pub struct TracePrinter<W: Write> {
    writer: W,
    color: ColorChoice,
}

impl<W: Write> TracePrinter<W> {
    pub fn new(writer: W, color: ColorChoice) -> Self {
        Self { writer, color }
    }

    /// Read each line of `trace.jsonl`, parse, format, and write one
    /// rendered line per span. Malformed lines emit `<malformed>` to
    /// stdout and a warning to stderr; processing continues.
    pub fn render_file(&mut self, path: &Path) -> Result<()> {
        use std::fs::File;
        use std::io::BufReader;

        let file = File::open(path)
            .with_context(|| format!("failed to open trace file: {}", path.display()))?;
        let reader = BufReader::new(file);

        for (line_no, line) in reader.lines().enumerate() {
            let line_no = line_no + 1;
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<serde_json::Value>(&line) {
                Ok(span) => self.render_span(&span)?,
                Err(e) => {
                    eprintln!("warning: malformed span on line {}: {}", line_no, e);
                    writeln!(self.writer, "<malformed>")?;
                }
            }
        }
        Ok(())
    }

    /// Render a single already-parsed JSON span.
    pub fn render_span(&mut self, span: &serde_json::Value) -> Result<()> {
        let (formatted, status) = format_span(span);
        let colored = if should_colorize(self.color) {
            colorize_line(&formatted, status)
        } else {
            formatted
        };
        writeln!(self.writer, "{colored}")?;
        Ok(())
    }
}

/// Stream `path` to `writer` byte-for-byte — used by `--json` mode.
pub fn passthrough<W: Write>(path: &Path, writer: &mut W) -> Result<()> {
    use std::fs::File;

    let mut file = File::open(path)
        .with_context(|| format!("failed to open trace file: {}", path.display()))?;
    std::io::copy(&mut file, writer)?;
    Ok(())
}

/// CLI entry point invoked from `main.rs`. Dispatches between pretty-print
/// and `--json` modes and writes to stdout.
pub fn run(path: &Path, json: bool, color: ColorChoice) -> Result<()> {
    if json {
        passthrough(path, &mut std::io::stdout())
    } else {
        TracePrinter::new(std::io::stdout(), color).render_file(path)
    }
}

// ---------------------------------------------------------------------------
// Internal formatting logic
// ---------------------------------------------------------------------------

fn should_colorize(choice: ColorChoice) -> bool {
    match choice {
        ColorChoice::Auto => control::SHOULD_COLORIZE.should_colorize(),
        ColorChoice::Always => true,
        ColorChoice::Never => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    Repair,
    Error,
    Shortfall,
    None,
}

/// Apply color to the formatted line based on its status.
fn colorize_line(line: &str, status: Status) -> String {
    match status {
        Status::Error => line.red().bold().to_string(),
        Status::Fail | Status::Shortfall => line.red().to_string(),
        Status::Repair => line.yellow().to_string(),
        Status::Pass => line.green().to_string(),
        Status::None => line.to_string(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpanKind {
    Phase,
    Backend,
    Aggregator,
    Verify,
    Finished,
    Error,
    Other,
}

impl SpanKind {
    fn from_name(name: &str) -> Self {
        if name.ends_with(".finished") {
            Self::Finished
        } else if name.ends_with(".error") {
            Self::Error
        } else if name.starts_with("phase.") {
            Self::Phase
        } else if name.starts_with("backend.") {
            Self::Backend
        } else if name.starts_with("aggregator.") {
            Self::Aggregator
        } else if name.starts_with("verify.") {
            Self::Verify
        } else {
            Self::Other
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Phase => "phase",
            Self::Backend => "backend",
            Self::Aggregator => "agg",
            Self::Verify => "verify",
            Self::Finished => "done",
            Self::Error => "error",
            Self::Other => "other",
        }
    }
}

/// Format a JSON span into a single (plain, ≤80 visible char) line,
/// together with its status classification.
fn format_span(span: &serde_json::Value) -> (String, Status) {
    let obj = match span.as_object() {
        Some(o) => o,
        None => return ("<not-an-object>".to_string(), Status::Error),
    };

    let stamp = obj
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| s.split('T').nth(1))
        .and_then(|s| s.split('.').next())
        .and_then(|s| s.split('Z').next())
        .unwrap_or("--:--:--");

    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let kind = SpanKind::from_name(name);

    let phase = obj
        .get("loker.phase")
        .and_then(|v| v.as_str())
        .unwrap_or("-");

    let backend = obj
        .get("gen_ai.system")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| {
            if let SpanKind::Verify = kind {
                name.strip_prefix("verify.").unwrap_or("verify")
            } else {
                ""
            }
        });

    let dur = obj
        .get("duration_ms")
        .and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|n| n as f64)))
        .map(|ms| {
            if ms >= 1000.0 {
                format!("{:.1}s", ms / 1000.0)
            } else {
                format!("{:.0}ms", ms)
            }
        })
        .unwrap_or_default();

    let tokens = match (
        obj.get("gen_ai.usage.input_tokens")
            .and_then(|v| v.as_u64()),
        obj.get("gen_ai.usage.output_tokens")
            .and_then(|v| v.as_u64()),
    ) {
        (Some(in_t), Some(out_t)) => format_token_pair(in_t, out_t),
        (Some(in_t), None) => format!("{}k", in_t),
        (None, Some(out_t)) => format!("+{}k", out_t),
        (None, None) => String::new(),
    };

    let outcome = obj
        .get("loker.outcome")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let error_kind = obj.get("error.kind").and_then(|v| v.as_str()).unwrap_or("");

    let error_msg = obj
        .get("error.message")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let min_responses_met = obj.get("loker.min_responses_met").and_then(|v| v.as_bool());

    let mut status_str = if !error_kind.is_empty() || !error_msg.is_empty() {
        format!("[{}] {}", error_kind, error_msg)
    } else if !outcome.is_empty() && outcome != "success" {
        format!("[{}]", outcome)
    } else {
        String::new()
    };
    if min_responses_met == Some(false) {
        status_str = if status_str.is_empty() {
            "[shortfall]".to_string()
        } else {
            format!("{} [shortfall]", status_str)
        };
    }

    let status = if min_responses_met == Some(false) {
        Status::Shortfall
    } else if !error_kind.is_empty() || !error_msg.is_empty() {
        Status::Error
    } else if !outcome.is_empty() {
        match outcome {
            "success" | "verify_passed" => Status::Pass,
            "verify_failed" => Status::Fail,
            "aggregator_failed" | "backend_error" | "io_failed" | "manifest_failed"
            | "marker_failed" | "phase_failed" | "strategy_failed" | "timeout" => Status::Error,
            _ => Status::Repair,
        }
    } else {
        Status::None
    };

    // Build the line, truncating variable fields as needed.
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("[{}]", stamp));
    parts.push(phase.to_string());
    parts.push(kind.label().to_string());
    if !backend.is_empty() {
        parts.push(truncate(backend, 12));
    }
    if !dur.is_empty() {
        parts.push(dur);
    }
    if !tokens.is_empty() {
        parts.push(tokens);
    }
    if !status_str.is_empty() {
        parts.push(truncate(&status_str, 30));
    }

    let mut line = parts.join(" ");
    if line.len() > 80 {
        line = fit(&line, 80);
    }
    (line, status)
}

/// Truncate `s` to at most `max` *bytes* of visible content, appending
/// `…` when truncation actually occurs. Never splits a multi-byte UTF-8
/// sequence.
fn fit(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    // Reserve 3 bytes for the ellipsis at the end.
    let max_inner = max.saturating_sub(3);
    let mut end = max_inner;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 {
        end = s.chars().next().map(|c| c.len_utf8()).unwrap_or(1);
    }
    format!("{}…", &s[..end])
}

/// Wrapper around `fit` that does nothing for non-truncated inputs.
fn truncate(s: &str, max: usize) -> String {
    fit(s, max)
}

/// Format token counts compactly (e.g. "1.2k+3.4k").
fn format_token_pair(input: u64, output: u64) -> String {
    let fmt = |n: u64| {
        if n >= 1000 {
            format!("{:.1}k", n as f64 / 1000.0)
        } else {
            n.to_string()
        }
    };
    format!("{}+{}", fmt(input), fmt(output))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_happy_path() {
        let span = serde_json::json!({
            "trace_id": "01f2a4e0-3b6f-7c8d-9e1a-2b3c4d5e6f70",
            "span_id": "abc123def4567890",
            "timestamp": "2026-04-25T20:45:00Z",
            "name": "phase.design",
            "loker.phase": "design",
            "loker.strategy": "single",
        });
        let mut buf = Vec::new();
        TracePrinter::new(&mut buf, ColorChoice::Never)
            .render_span(&span)
            .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("design"));
        assert!(out.contains("phase"));
        let line = out.lines().next().unwrap_or("").trim();
        assert!(line.len() <= 80, "line too long: {}", line.len());
    }

    #[test]
    fn renders_backend_call() {
        let span = serde_json::json!({
            "trace_id": "01f2a4e0-3b6f-7c8d-9e1a-2b3c4d5e6f70",
            "span_id": "def4567890abc123",
            "parent_span_id": "abc123def4567890",
            "timestamp": "2026-04-25T20:45:03Z",
            "name": "backend.claude",
            "loker.phase": "design",
            "loker.attempt": 0,
            "gen_ai.system": "anthropic",
            "gen_ai.request.model": "claude-opus-4-7",
            "gen_ai.usage.input_tokens": 1843,
            "gen_ai.usage.output_tokens": 712,
            "duration_ms": 3450,
        });
        let mut buf = Vec::new();
        TracePrinter::new(&mut buf, ColorChoice::Never)
            .render_span(&span)
            .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("anthropic"));
        assert!(out.contains("1.8k+712"));
        assert!(out.contains("3.5s"));
    }

    #[test]
    fn renders_error_span_in_red_bold() {
        colored::control::set_override(true);
        let span = serde_json::json!({
            "trace_id": "01f2a4e0-3b6f-7c8d-9e1a-2b3c4d5e6f70",
            "span_id": "err7890abc123def4",
            "timestamp": "2026-04-25T20:46:11Z",
            "name": "phase.error",
            "loker.phase": "implement",
            "error.kind": "backend_error",
            "error.message": "timeout",
        });
        let mut buf = Vec::new();
        TracePrinter::new(&mut buf, ColorChoice::Always)
            .render_span(&span)
            .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("error"));
        assert!(out.contains("backend_error"));
        assert!(out.contains("timeout"));
        // ANSI red + bold escapes
        assert!(
            out.contains("\u{1b}[31m") || out.contains("\u{1b}[1;31m"),
            "expected red ANSI escape in: {:?}",
            out
        );
        colored::control::set_override(false);
    }

    #[test]
    fn renders_min_responses_shortfall() {
        let span = serde_json::json!({
            "trace_id": "01f2a4e0-3b6f-7c8d-9e1a-2b3c4d5e6f70",
            "span_id": "shf123abc456def78",
            "timestamp": "2026-04-25T20:46:11Z",
            "name": "aggregator.concat",
            "loker.phase": "review",
            "loker.outcome": "success",
            "loker.min_responses_met": false,
        });
        let mut buf = Vec::new();
        TracePrinter::new(&mut buf, ColorChoice::Never)
            .render_span(&span)
            .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("[shortfall]"));
    }

    #[test]
    fn renders_verify_failure() {
        let span = serde_json::json!({
            "trace_id": "01f2a4e0-3b6f-7c8d-9e1a-2b3c4d5e6f70",
            "span_id": "vfy123abc456def78",
            "timestamp": "2026-04-25T20:46:11Z",
            "name": "verify.make_check",
            "loker.phase": "implement",
            "loker.outcome": "verify_failed",
        });
        let mut buf = Vec::new();
        TracePrinter::new(&mut buf, ColorChoice::Never)
            .render_span(&span)
            .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("verify_failed"));
    }

    #[test]
    fn truncates_overlong_backend() {
        let span = serde_json::json!({
            "trace_id": "t",
            "span_id": "s",
            "timestamp": "2026-04-25T20:45:00Z",
            "name": "backend.x",
            "gen_ai.system": "a".repeat(200),
        });
        let mut buf = Vec::new();
        TracePrinter::new(&mut buf, ColorChoice::Never)
            .render_span(&span)
            .unwrap();
        let line = String::from_utf8(buf).unwrap();
        let visible = line.trim_end();
        assert!(visible.len() <= 80, "line too long: {}", visible.len());
        assert!(visible.contains('…'));
    }

    #[test]
    fn fit_does_not_split_unicode() {
        assert_eq!(fit("αβγδ", 4), "α…");
        assert_eq!(fit("αβγδ", 5), "α…");
        assert_eq!(fit("αβγδ", 7), "αβ…");
        assert_eq!(fit("aαb", 2), "a…");
        assert_eq!(fit("hello", 8), "hello");
    }

    #[test]
    fn malformed_line_does_not_abort() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("trace.jsonl");
        std::fs::write(
            &path,
            r#"{"timestamp":"2026-04-25T20:45:00Z","name":"phase.x","trace_id":"t","span_id":"s1","loker.phase":"design"}
bad json
{"timestamp":"2026-04-25T20:45:01Z","name":"phase.y","trace_id":"t","span_id":"s2","loker.phase":"implement"}
"#,
        )
        .unwrap();

        let mut buf = Vec::new();
        TracePrinter::new(&mut buf, ColorChoice::Never)
            .render_file(&path)
            .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.contains("design"));
        assert!(out.contains("<malformed>"));
        assert!(out.contains("implement"));
    }

    #[test]
    fn span_kind_table() {
        assert_eq!(SpanKind::from_name("phase.design"), SpanKind::Phase);
        assert_eq!(SpanKind::from_name("backend.claude"), SpanKind::Backend);
        assert_eq!(
            SpanKind::from_name("aggregator.concat"),
            SpanKind::Aggregator
        );
        assert_eq!(SpanKind::from_name("verify.make_check"), SpanKind::Verify);
        assert_eq!(SpanKind::from_name("phase.finished"), SpanKind::Finished);
        assert_eq!(SpanKind::from_name("phase.error"), SpanKind::Error);
        assert_eq!(SpanKind::from_name("phase.x.error"), SpanKind::Error);
        assert_eq!(SpanKind::from_name("phase.x.finished"), SpanKind::Finished);
    }

    #[test]
    fn status_derivation_table() {
        let json = |o: &str, e: &str, mr: bool| {
            let mut m = serde_json::Map::new();
            if !o.is_empty() {
                m.insert("loker.outcome".into(), serde_json::Value::String(o.into()));
            }
            if !e.is_empty() {
                m.insert("error.kind".into(), serde_json::Value::String(e.into()));
            }
            m.insert(
                "loker.min_responses_met".into(),
                serde_json::Value::Bool(mr),
            );
            serde_json::Value::Object(m)
        };

        // error overrides everything
        assert_eq!(
            format_span(&json("success", "backend_error", true)).1,
            Status::Error
        );
        // min_responses_met = false → Shortfall even with success
        assert_eq!(
            format_span(&json("success", "", false)).1,
            Status::Shortfall
        );
        // normal outcomes
        assert_eq!(
            format_span(&json("verify_passed", "", true)).1,
            Status::Pass
        );
        assert_eq!(
            format_span(&json("verify_failed", "", true)).1,
            Status::Fail
        );
        assert_eq!(
            format_span(&json("strategy_failed", "", true)).1,
            Status::Error
        );
        assert_eq!(
            format_span(&json("backend_error", "", true)).1,
            Status::Error
        );
        // unknown outcome
        assert_eq!(format_span(&json("custom", "", true)).1, Status::Repair);
        // no outcome, no error, min_responses_met = true → None
        assert_eq!(format_span(&json("", "", true)).1, Status::None);
    }

    #[test]
    fn passthrough_verbatim() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("trace.jsonl");
        let data = r#"{"a":1}
{"b":2}
"#;
        std::fs::write(&path, data).unwrap();

        let mut buf = Vec::new();
        passthrough(&path, &mut buf).unwrap();
        assert_eq!(String::from_utf8(buf).unwrap(), data);
    }
}
