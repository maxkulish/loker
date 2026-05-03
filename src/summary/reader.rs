//! Trace reader: parses trace.jsonl and aggregates token usage per (backend, model).
//!
//! Reads every line of the append-only trace file, filters to backend_call
//! spans (those with `gen_ai.usage.input_tokens` / `gen_ai.usage.output_tokens`),
//! and groups by `gen_ai.system` + `gen_ai.request.model`.

use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// Aggregated token usage for a single (backend, model) pair.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackendUsage {
    pub name: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Cost in USD, if the price table had an entry. Set externally.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// Error type for trace reader operations.
#[derive(Debug, thiserror::Error)]
pub enum TraceReaderError {
    #[error("IO error: {0}")]
    Io(String),
    #[error("Parse error at line {line}: {message}")]
    Parse { line: usize, message: String },
}

/// Parses trace.jsonl and aggregates token usage.
pub struct TraceReader;

impl TraceReader {
    /// Read all lines from `trace.jsonl`, parse gen_ai.* backend_call spans,
    /// and aggregate token usage per (backend, model).
    ///
    /// Only processes lines that have all three of `gen_ai.system`,
    /// `gen_ai.usage.input_tokens`, and `gen_ai.usage.output_tokens`.
    /// Lines without these fields (aggregator, verify, phase marker spans)
    /// are skipped silently.
    pub fn aggregate(path: &Path) -> Result<Vec<BackendUsage>, TraceReaderError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| TraceReaderError::Io(format!("cannot read trace file: {e}")))?;

        if content.trim().is_empty() {
            return Ok(Vec::new());
        }

        // Group by (backend, model) → (input_tokens, output_tokens)
        let mut groups: HashMap<(String, String), (u64, u64)> = HashMap::new();

        for (idx, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let value: Value = serde_json::from_str(line)
                .map_err(|e| TraceReaderError::Parse {
                    line: idx + 1,
                    message: format!("invalid JSON: {e}"),
                })?;

            let obj = match value.as_object() {
                Some(o) => o,
                None => continue,
            };

            // Extract system (backend name)
            let system = match obj.get("gen_ai.system").and_then(|v| v.as_str()) {
                Some(s) if !s.is_empty() => s.to_string(),
                _ => continue, // not a backend call span
            };

            // Extract model
            let model = obj
                .get("gen_ai.request.model")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();

            // Extract token counts — both must be present
            let input_tokens = match obj
                .get("gen_ai.usage.input_tokens")
                .and_then(|v| v.as_u64())
            {
                Some(n) => n,
                None => continue,
            };
            let output_tokens = match obj
                .get("gen_ai.usage.output_tokens")
                .and_then(|v| v.as_u64())
            {
                Some(n) => n,
                None => continue,
            };

            let entry = groups.entry((system, model)).or_insert((0, 0));
            entry.0 = entry.0.saturating_add(input_tokens);
            entry.1 = entry.1.saturating_add(output_tokens);
        }

        let mut backends: Vec<BackendUsage> = groups
            .into_iter()
            .map(|((backend, model), (input_tokens, output_tokens))| BackendUsage {
                name: backend,
                model,
                input_tokens,
                output_tokens,
                cost_usd: None,
            })
            .collect();

        // Sort for deterministic output
        backends.sort_by(|a, b| a.name.cmp(&b.name).then(a.model.cmp(&b.model)));

        Ok(backends)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: write a trace.jsonl fixture and return its path.
    fn write_trace(tmp: &tempfile::TempDir, lines: &[&str]) -> std::path::PathBuf {
        let path = tmp.path().join("trace.jsonl");
        std::fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    /// Helper: find a BackendUsage by name in the result.
    fn find<'a>(backends: &'a [BackendUsage], name: &str, model: &str) -> Option<&'a BackendUsage> {
        backends.iter().find(|b| b.name == name && b.model == model)
    }

    #[test]
    fn empty_trace_no_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_trace(&tmp, &[]);
        let result = TraceReader::aggregate(&path).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn only_whitespace_no_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_trace(&tmp, &["", "  ", ""]);
        let result = TraceReader::aggregate(&path).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn single_backend_call() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_trace(
            &tmp,
            &[r#"{"name":"backend.mock","gen_ai.system":"mock","gen_ai.request.model":"m1","gen_ai.usage.input_tokens":10,"gen_ai.usage.output_tokens":20}"#],
        );
        let result = TraceReader::aggregate(&path).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "mock");
        assert_eq!(result[0].model, "m1");
        assert_eq!(result[0].input_tokens, 10);
        assert_eq!(result[0].output_tokens, 20);
    }

    #[test]
    fn multiple_backends_separated() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_trace(
            &tmp,
            &[
                r#"{"name":"backend.anthropic","gen_ai.system":"anthropic","gen_ai.request.model":"opus","gen_ai.usage.input_tokens":100,"gen_ai.usage.output_tokens":200}"#,
                r#"{"name":"backend.openai","gen_ai.system":"openai","gen_ai.request.model":"gpt5","gen_ai.usage.input_tokens":50,"gen_ai.usage.output_tokens":80}"#,
            ],
        );
        let result = TraceReader::aggregate(&path).unwrap();
        assert_eq!(result.len(), 2);
        let anthropic = find(&result, "anthropic", "opus").unwrap();
        assert_eq!(anthropic.input_tokens, 100);
        assert_eq!(anthropic.output_tokens, 200);
        let openai = find(&result, "openai", "gpt5").unwrap();
        assert_eq!(openai.input_tokens, 50);
        assert_eq!(openai.output_tokens, 80);
    }

    #[test]
    fn same_backend_aggregated() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_trace(
            &tmp,
            &[
                r#"{"name":"backend.mock","gen_ai.system":"mock","gen_ai.request.model":"m1","gen_ai.usage.input_tokens":10,"gen_ai.usage.output_tokens":20}"#,
                r#"{"name":"backend.mock","gen_ai.system":"mock","gen_ai.request.model":"m1","gen_ai.usage.input_tokens":30,"gen_ai.usage.output_tokens":40}"#,
            ],
        );
        let result = TraceReader::aggregate(&path).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].input_tokens, 40);
        assert_eq!(result[0].output_tokens, 60);
    }

    #[test]
    fn different_model_same_backend() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_trace(
            &tmp,
            &[
                r#"{"name":"backend.m","gen_ai.system":"m","gen_ai.request.model":"cheap","gen_ai.usage.input_tokens":5,"gen_ai.usage.output_tokens":5}"#,
                r#"{"name":"backend.m","gen_ai.system":"m","gen_ai.request.model":"strong","gen_ai.usage.input_tokens":50,"gen_ai.usage.output_tokens":100}"#,
            ],
        );
        let result = TraceReader::aggregate(&path).unwrap();
        assert_eq!(result.len(), 2);
        let cheap = find(&result, "m", "cheap").unwrap();
        assert_eq!(cheap.input_tokens, 5);
        let strong = find(&result, "m", "strong").unwrap();
        assert_eq!(strong.input_tokens, 50);
    }

    #[test]
    fn skips_non_backend_spans() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_trace(
            &tmp,
            &[
                r#"{"name":"backend.valid","gen_ai.system":"valid","gen_ai.request.model":"m","gen_ai.usage.input_tokens":10,"gen_ai.usage.output_tokens":20}"#,
                r#"{"name":"aggregator.concat","loker.phase":"design","loker.aggregator":"concat"}"#,
                r#"{"name":"verify.test","loker.phase":"design","loker.outcome":"verify_passed"}"#,
            ],
        );
        let result = TraceReader::aggregate(&path).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].input_tokens, 10);
    }

    #[test]
    fn missing_token_fields_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_trace(
            &tmp,
            &[
                // Has system but no usage — skip
                r#"{"name":"backend.nousage","gen_ai.system":"no","gen_ai.request.model":"m1"}"#,
                // Has system and input but no output — skip
                r#"{"name":"backend.partial","gen_ai.system":"partial","gen_ai.request.model":"m1","gen_ai.usage.input_tokens":5}"#,
            ],
        );
        let result = TraceReader::aggregate(&path).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn error_span_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_trace(
            &tmp,
            &[r#"{"name":"backend.mock","gen_ai.system":"mock","gen_ai.request.model":"m1","gen_ai.usage.input_tokens":10,"gen_ai.usage.output_tokens":20,"error.message":"rate limited"}"#],
        );
        // Error spans with token usage SHOULD still be counted
        let result = TraceReader::aggregate(&path).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].input_tokens, 10);
    }

    #[test]
    fn missing_trace_file() {
        let result = TraceReader::aggregate(Path::new("/nonexistent/trace.jsonl"));
        assert!(result.is_err());
    }

    #[test]
    fn malformed_json_line() {
        let tmp = tempfile::tempdir().unwrap();
        let path = write_trace(&tmp, &["{this is not json}"]);
        let result = TraceReader::aggregate(&path);
        assert!(result.is_err());
        match result {
            Err(TraceReaderError::Parse { line: 1, ..}) => {} // expected
            _ => panic!("expected Parse error on line 1"),
        }
    }
}
