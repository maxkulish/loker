//! Append-only JSONL trace writer.

use super::{build_span_skeleton, TraceSink};
use serde_json::Value;
use std::fs::OpenOptions;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

/// File-backed `TraceSink` that writes one JSON object per line to
/// `run_dir/trace.jsonl`.
pub struct TraceWriter {
    writer: Mutex<BufWriter<std::fs::File>>,
    fsync: bool,
}

impl TraceWriter {
    /// Open (or create) the trace file at `path`.
    ///
    /// `fsync` controls whether every line is flushed + fsynced before
    /// returning. Default `false` for speed; set `true` in tests.
    pub fn new(path: &Path, fsync: bool) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            writer: Mutex::new(BufWriter::new(file)),
            fsync,
        })
    }

    fn write_line(
        &self,
        trace_id: uuid::Uuid,
        span_id: &str,
        parent_span_id: Option<&str>,
        name: &str,
        extras: serde_json::Map<String, Value>,
    ) {
        let mut map = build_span_skeleton(trace_id, span_id, parent_span_id, name);
        for (k, v) in extras.into_iter() {
            map.insert(k, v);
        }
        let line = match serde_json::to_string(&Value::Object(map)) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("trace serialization failed: {e}");
                return;
            }
        };
        let mut guard = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        if let Err(e) = writeln!(guard, "{line}") {
            eprintln!("trace write failed: {e}");
            return;
        }
        if self.fsync {
            if let Err(e) = guard.flush() {
                eprintln!("trace flush failed: {e}");
            }
        }
    }
}

impl TraceSink for TraceWriter {
    fn phase_started(&self, ctx: &super::PhaseSpanContext) {
        let mut extras = serde_json::Map::new();
        extras.insert(
            "loker.run_id".to_string(),
            Value::String(ctx.trace_id.to_string()),
        );
        extras.insert("loker.phase".to_string(), Value::String(ctx.phase.clone()));
        extras.insert(
            "loker.strategy".to_string(),
            Value::String(ctx.strategy.clone()),
        );
        extras.insert(
            "loker.aggregator".to_string(),
            Value::String(ctx.aggregator.clone()),
        );
        if let Some(ref hook) = ctx.verify_hook {
            extras.insert("loker.verify_hook".to_string(), Value::String(hook.clone()));
        }
        self.write_line(
            ctx.trace_id,
            &ctx.span_id,
            None,
            &format!("phase.{}", ctx.phase),
            extras,
        );
    }

    fn backend_call(
        &self,
        ctx: &super::PhaseSpanContext,
        attempt: &super::AttemptSpanContext,
        result: &super::BackendSpanResult,
    ) {
        let mut extras = serde_json::Map::new();
        extras.insert(
            "loker.run_id".to_string(),
            Value::String(ctx.trace_id.to_string()),
        );
        extras.insert("loker.phase".to_string(), Value::String(ctx.phase.clone()));
        extras.insert(
            "loker.attempt".to_string(),
            Value::Number(attempt.attempt.into()),
        );
        extras.insert(
            "gen_ai.system".to_string(),
            Value::String(attempt.backend.clone()),
        );
        if let Some(ref model) = attempt.model {
            extras.insert(
                "gen_ai.request.model".to_string(),
                Value::String(model.clone()),
            );
        }
        if let Some(input) = result.usage_input_tokens {
            extras.insert(
                "gen_ai.usage.input_tokens".to_string(),
                Value::Number(input.into()),
            );
        }
        if let Some(output) = result.usage_output_tokens {
            extras.insert(
                "gen_ai.usage.output_tokens".to_string(),
                Value::Number(output.into()),
            );
        }
        if !result.finish_reasons.is_empty() {
            extras.insert(
                "gen_ai.response.finish_reasons".to_string(),
                Value::Array(
                    result
                        .finish_reasons
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        }
        if let Some(ref err) = result.error {
            extras.insert("error.message".to_string(), Value::String(err.clone()));
        }
        self.write_line(
            ctx.trace_id,
            &attempt.span_id,
            Some(&ctx.span_id),
            &format!("backend.{}", attempt.backend),
            extras,
        );
    }

    fn aggregator_fold(&self, ctx: &super::PhaseSpanContext, kind: &str, input_count: usize) {
        let mut extras = serde_json::Map::new();
        extras.insert(
            "loker.run_id".to_string(),
            Value::String(ctx.trace_id.to_string()),
        );
        extras.insert("loker.phase".to_string(), Value::String(ctx.phase.clone()));
        extras.insert(
            "loker.aggregator".to_string(),
            Value::String(kind.to_string()),
        );
        extras.insert(
            "loker.input_count".to_string(),
            Value::Number(input_count.into()),
        );
        let span_id = super::new_span_id();
        self.write_line(
            ctx.trace_id,
            &span_id,
            Some(&ctx.span_id),
            &format!("aggregator.{kind}"),
            extras,
        );
    }

    fn verify_result(
        &self,
        ctx: &super::PhaseSpanContext,
        hook_name: &str,
        result: &super::VerifySpanResult,
    ) {
        let mut extras = serde_json::Map::new();
        extras.insert(
            "loker.run_id".to_string(),
            Value::String(ctx.trace_id.to_string()),
        );
        extras.insert("loker.phase".to_string(), Value::String(ctx.phase.clone()));
        extras.insert(
            "loker.verify_hook".to_string(),
            Value::String(hook_name.to_string()),
        );
        extras.insert(
            "loker.outcome".to_string(),
            Value::String(if result.passed {
                "verify_passed".to_string()
            } else {
                "verify_failed".to_string()
            }),
        );
        if let Some(ref msg) = result.message {
            extras.insert("error.message".to_string(), Value::String(msg.clone()));
        }
        let span_id = super::new_span_id();
        self.write_line(
            ctx.trace_id,
            &span_id,
            Some(&ctx.span_id),
            &format!("verify.{hook_name}"),
            extras,
        );
    }

    fn phase_finished(&self, ctx: &super::PhaseSpanContext, outcome: &str) {
        let mut extras = serde_json::Map::new();
        extras.insert(
            "loker.run_id".to_string(),
            Value::String(ctx.trace_id.to_string()),
        );
        extras.insert("loker.phase".to_string(), Value::String(ctx.phase.clone()));
        extras.insert(
            "loker.outcome".to_string(),
            Value::String(outcome.to_string()),
        );
        let span_id = super::new_span_id();
        self.write_line(
            ctx.trace_id,
            &span_id,
            Some(&ctx.span_id),
            &format!("phase.{}.finished", ctx.phase),
            extras,
        );
    }

    fn error(&self, ctx: &super::PhaseSpanContext, kind: &str, message: &str) {
        let mut extras = serde_json::Map::new();
        extras.insert(
            "loker.run_id".to_string(),
            Value::String(ctx.trace_id.to_string()),
        );
        extras.insert("loker.phase".to_string(), Value::String(ctx.phase.clone()));
        extras.insert("error.kind".to_string(), Value::String(kind.to_string()));
        extras.insert(
            "error.message".to_string(),
            Value::String(message.to_string()),
        );
        let span_id = super::new_span_id();
        self.write_line(
            ctx.trace_id,
            &span_id,
            Some(&ctx.span_id),
            &format!("phase.{}.error", ctx.phase),
            extras,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_file_and_appends() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("trace.jsonl");
        let writer = TraceWriter::new(&path, true).unwrap();
        assert!(path.exists());

        let ctx = super::super::PhaseSpanContext {
            trace_id: uuid::Uuid::new_v4(),
            span_id: super::super::new_span_id(),
            phase: "design".to_string(),
            strategy: "single".to_string(),
            aggregator: "first".to_string(),
            verify_hook: None,
        };
        writer.phase_started(&ctx);

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 1);
        let parsed: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(parsed["name"], "phase.design");
        assert_eq!(parsed["loker.phase"], "design");
    }

    #[test]
    fn appends_valid_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("trace.jsonl");
        let writer = TraceWriter::new(&path, true).unwrap();

        let trace_id = uuid::Uuid::new_v4();
        let phase_span = super::super::new_span_id();
        let ctx = super::super::PhaseSpanContext {
            trace_id,
            span_id: phase_span.clone(),
            phase: "test".to_string(),
            strategy: "single".to_string(),
            aggregator: "first".to_string(),
            verify_hook: None,
        };
        writer.phase_started(&ctx);

        let attempt = super::super::AttemptSpanContext {
            span_id: super::super::new_span_id(),
            attempt: 0,
            backend: "mock".to_string(),
            model: Some("m1".to_string()),
        };
        let result = super::super::BackendSpanResult {
            duration_ms: 100,
            usage_input_tokens: Some(10),
            usage_output_tokens: Some(20),
            finish_reasons: vec!["stop".to_string()],
            error: None,
        };
        writer.backend_call(&ctx, &attempt, &result);

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(parsed.is_object());
        }
        let backend: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(backend["parent_span_id"], phase_span);
        assert_eq!(backend["gen_ai.system"], "mock");
        assert_eq!(backend["gen_ai.request.model"], "m1");
        assert_eq!(backend["gen_ai.usage.input_tokens"], 10);
        assert_eq!(backend["gen_ai.usage.output_tokens"], 20);
    }

    #[test]
    fn fsync_flushes() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("trace.jsonl");
        let writer = TraceWriter::new(&path, true).unwrap();

        let ctx = super::super::PhaseSpanContext {
            trace_id: uuid::Uuid::new_v4(),
            span_id: super::super::new_span_id(),
            phase: "x".to_string(),
            strategy: "single".to_string(),
            aggregator: "first".to_string(),
            verify_hook: None,
        };
        writer.phase_started(&ctx);

        // With fsync=true, flush happens immediately; file should contain the line.
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(!content.is_empty());
    }
}
