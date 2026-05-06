//! In-memory trace sink for test capture.

use super::TraceSink;
use serde_json::Value;
use std::sync::Mutex;

fn inject_common(map: &mut serde_json::Map<String, Value>, ctx: &super::PhaseSpanContext) {
    map.insert(
        "loker.run_id".to_string(),
        Value::String(ctx.trace_id.to_string()),
    );
    map.insert("loker.phase".to_string(), Value::String(ctx.phase.clone()));
}

/// Captures all emitted spans into a `Vec<Value>` behind a mutex.
///
/// Safe to share across threads / async tasks. Each `TraceSink` method
/// appends one `serde_json::Value` to the internal buffer.
pub struct InMemorySink {
    spans: Mutex<Vec<Value>>,
}

impl InMemorySink {
    pub fn new() -> Self {
        Self {
            spans: Mutex::new(Vec::new()),
        }
    }

    /// Returns a clone of all captured spans.
    pub fn spans(&self) -> Vec<Value> {
        self.spans.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Clears the captured spans.
    pub fn clear(&self) {
        self.spans.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    fn push(&self, value: Value) {
        self.spans
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(value);
    }
}

impl Default for InMemorySink {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceSink for InMemorySink {
    fn phase_started(&self, ctx: &super::PhaseSpanContext) {
        let mut map = super::build_span_skeleton(
            ctx.trace_id,
            &ctx.span_id,
            None,
            &format!("phase.{}", ctx.phase),
        );
        inject_common(&mut map, ctx);
        map.insert(
            "loker.strategy".to_string(),
            Value::String(ctx.strategy.clone()),
        );
        map.insert(
            "loker.aggregator".to_string(),
            Value::String(ctx.aggregator.clone()),
        );
        if let Some(ref hook) = ctx.verify_hook {
            map.insert("loker.verify_hook".to_string(), Value::String(hook.clone()));
        }
        self.push(Value::Object(map));
    }

    fn backend_call(
        &self,
        ctx: &super::PhaseSpanContext,
        attempt: &super::AttemptSpanContext,
        result: &super::BackendSpanResult,
    ) {
        let mut map = super::build_span_skeleton(
            ctx.trace_id,
            &attempt.span_id,
            Some(&ctx.span_id),
            &format!("backend.{}", attempt.backend),
        );
        inject_common(&mut map, ctx);
        map.insert(
            "loker.attempt".to_string(),
            Value::Number(attempt.attempt.into()),
        );
        map.insert(
            "gen_ai.system".to_string(),
            Value::String(attempt.backend.clone()),
        );
        if let Some(ref model) = attempt.model {
            map.insert(
                "gen_ai.request.model".to_string(),
                Value::String(model.clone()),
            );
        }
        map.insert(
            "duration_ms".to_string(),
            Value::Number(result.duration_ms.into()),
        );
        if let Some(input) = result.usage_input_tokens {
            map.insert(
                "gen_ai.usage.input_tokens".to_string(),
                Value::Number(input.into()),
            );
        }
        if let Some(output) = result.usage_output_tokens {
            map.insert(
                "gen_ai.usage.output_tokens".to_string(),
                Value::Number(output.into()),
            );
        }
        if !result.finish_reasons.is_empty() {
            map.insert(
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
            map.insert("error.message".to_string(), Value::String(err.clone()));
        }
        self.push(Value::Object(map));
    }

    fn aggregator_fold(&self, ctx: &super::PhaseSpanContext, kind: &str, _input_count: usize) {
        let mut map = super::build_span_skeleton(
            ctx.trace_id,
            &super::new_span_id(),
            Some(&ctx.span_id),
            &format!("aggregator.{kind}"),
        );
        inject_common(&mut map, ctx);
        map.insert(
            "loker.aggregator".to_string(),
            Value::String(kind.to_string()),
        );
        self.push(Value::Object(map));
    }

    fn verify_result(
        &self,
        ctx: &super::PhaseSpanContext,
        hook_name: &str,
        result: &super::VerifySpanResult,
    ) {
        let mut map = super::build_span_skeleton(
            ctx.trace_id,
            &super::new_span_id(),
            Some(&ctx.span_id),
            &format!("verify.{hook_name}"),
        );
        inject_common(&mut map, ctx);
        map.insert(
            "loker.outcome".to_string(),
            Value::String(if result.passed {
                "verify_passed".to_string()
            } else {
                "verify_failed".to_string()
            }),
        );
        map.insert(
            "duration_ms".to_string(),
            Value::Number(result.duration_ms.into()),
        );
        if let Some(ref msg) = result.message {
            map.insert("error.message".to_string(), Value::String(msg.clone()));
        }
        if let Some(ref hitl) = result.hitl {
            map.insert(
                "loker.hitl.severity".to_string(),
                Value::String(hitl.severity.clone()),
            );
            if let Some(ref timeout_at) = hitl.timeout_at {
                map.insert(
                    "loker.hitl.timeout_at".to_string(),
                    Value::String(timeout_at.clone()),
                );
            }
            map.insert(
                "loker.hitl.timeout_action".to_string(),
                Value::String(hitl.timeout_action.clone()),
            );
            map.insert(
                "loker.hitl.timeout_outcome".to_string(),
                Value::String(hitl.timeout_outcome.clone()),
            );
        }
        self.push(Value::Object(map));
    }

    fn phase_finished(&self, ctx: &super::PhaseSpanContext, outcome: &str, duration_ms: u64) {
        let mut map = super::build_span_skeleton(
            ctx.trace_id,
            &super::new_span_id(),
            Some(&ctx.span_id),
            &format!("phase.{}.finished", ctx.phase),
        );
        inject_common(&mut map, ctx);
        map.insert(
            "loker.outcome".to_string(),
            Value::String(outcome.to_string()),
        );
        map.insert("duration_ms".to_string(), Value::Number(duration_ms.into()));
        self.push(Value::Object(map));
    }

    fn error(&self, ctx: &super::PhaseSpanContext, kind: &str, message: &str, duration_ms: u64) {
        let mut map = super::build_span_skeleton(
            ctx.trace_id,
            &super::new_span_id(),
            Some(&ctx.span_id),
            &format!("phase.{}.error", ctx.phase),
        );
        inject_common(&mut map, ctx);
        map.insert("error.kind".to_string(), Value::String(kind.to_string()));
        map.insert(
            "error.message".to_string(),
            Value::String(message.to_string()),
        );
        map.insert("duration_ms".to_string(), Value::Number(duration_ms.into()));
        self.push(Value::Object(map));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_phase_span() {
        let sink = InMemorySink::new();
        let ctx = super::super::PhaseSpanContext {
            trace_id: uuid::Uuid::new_v4(),
            span_id: super::super::new_span_id(),
            phase: "design".to_string(),
            strategy: "single".to_string(),
            aggregator: "first".to_string(),
            verify_hook: None,
        };
        sink.phase_started(&ctx);
        let spans = sink.spans();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0]["name"], "phase.design");
        assert!(spans[0].as_object().unwrap().contains_key("timestamp"));
    }

    #[test]
    fn capture_backend_span() {
        let sink = InMemorySink::new();
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
        let attempt = super::super::AttemptSpanContext {
            span_id: super::super::new_span_id(),
            attempt: 0,
            backend: "mock".to_string(),
            model: Some("m1".to_string()),
        };
        let result = super::super::BackendSpanResult {
            duration_ms: 50,
            usage_input_tokens: Some(5),
            usage_output_tokens: Some(10),
            finish_reasons: vec!["stop".to_string()],
            error: None,
        };
        sink.backend_call(&ctx, &attempt, &result);
        let spans = sink.spans();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0]["parent_span_id"], phase_span);
        assert_eq!(spans[0]["gen_ai.system"], "mock");
        assert_eq!(spans[0]["gen_ai.usage.input_tokens"], 5);
    }

    #[test]
    fn capture_all_kinds() {
        let sink = InMemorySink::new();
        let trace_id = uuid::Uuid::new_v4();
        let phase_span = super::super::new_span_id();
        let ctx = super::super::PhaseSpanContext {
            trace_id,
            span_id: phase_span.clone(),
            phase: "x".to_string(),
            strategy: "single".to_string(),
            aggregator: "first".to_string(),
            verify_hook: Some("run_command".to_string()),
        };
        sink.phase_started(&ctx);
        let attempt = super::super::AttemptSpanContext {
            span_id: super::super::new_span_id(),
            attempt: 0,
            backend: "mock".to_string(),
            model: None,
        };
        sink.backend_call(
            &ctx,
            &attempt,
            &super::super::BackendSpanResult {
                duration_ms: 0,
                usage_input_tokens: None,
                usage_output_tokens: None,
                finish_reasons: vec![],
                error: None,
            },
        );
        sink.aggregator_fold(&ctx, "first", 1);
        sink.verify_result(
            &ctx,
            "run_command",
            &super::super::VerifySpanResult {
                passed: true,
                message: None,
                duration_ms: 0,
                hitl: None,
            },
        );
        sink.phase_finished(&ctx, "success", 42);
        sink.error(&ctx, "test", "message", 7);

        let spans = sink.spans();
        assert_eq!(spans.len(), 6);
        assert_eq!(spans[0]["name"], "phase.x");
        assert_eq!(spans[1]["name"], "backend.mock");
        assert_eq!(spans[2]["name"], "aggregator.first");
        assert_eq!(spans[3]["name"], "verify.run_command");
        assert_eq!(spans[4]["name"], "phase.x.finished");
        assert_eq!(spans[5]["name"], "phase.x.error");
    }

    #[test]
    fn clear_empties_buffer() {
        let sink = InMemorySink::new();
        let ctx = super::super::PhaseSpanContext {
            trace_id: uuid::Uuid::new_v4(),
            span_id: super::super::new_span_id(),
            phase: "a".to_string(),
            strategy: "single".to_string(),
            aggregator: "first".to_string(),
            verify_hook: None,
        };
        sink.phase_started(&ctx);
        assert_eq!(sink.spans().len(), 1);
        sink.clear();
        assert!(sink.spans().is_empty());
    }
}
