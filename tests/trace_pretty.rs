#![cfg(test)]

use loker::commands::trace::{passthrough, ColorChoice, TracePrinter};
use std::path::Path;

#[test]
fn snapshot_default_render_against_fixture() {
    let path = Path::new("tests/fixtures/trace/happy_path_and_errors.jsonl");
    let mut buf = Vec::new();
    TracePrinter::new(&mut buf, ColorChoice::Never)
        .render_file(path)
        .unwrap();
    let out = String::from_utf8(buf).unwrap();
    // Ensure output fits within 80 cols
    for line in out.lines() {
        assert!(line.len() <= 80, "line exceeds 80 columns: '{}'", line);
    }
    insta::assert_snapshot!(&out);
}

#[test]
fn json_passthrough_streams_file_verbatim() {
    let path = Path::new("tests/fixtures/trace/happy_path_and_errors.jsonl");
    let mut buf = Vec::new();
    passthrough(path, &mut buf).unwrap();
    let expected = std::fs::read_to_string(path).unwrap();
    assert_eq!(String::from_utf8(buf).unwrap(), expected);
}

#[test]
fn json_passthrough_errors_when_file_missing() {
    let path = Path::new("/nonexistent/trace.jsonl");
    let mut buf: Vec<u8> = Vec::new();
    let result = passthrough(path, &mut buf);
    assert!(result.is_err());
    let msg = format!("{}", result.unwrap_err());
    assert!(msg.contains("failed to open trace file"));
}

#[test]
fn traceprinter_rundir_roundtrip() {
    use loker::run_state::RunDir;
    let tmp = tempfile::tempdir().unwrap();
    let run_dir = RunDir::create(tmp.path(), "test-workflow").unwrap();

    // Copy fixture into the run dir
    let fixture = "tests/fixtures/trace/happy_path_and_errors.jsonl";
    std::fs::copy(fixture, run_dir.trace_path()).unwrap();

    // Pretty-print
    let mut buf = Vec::new();
    TracePrinter::new(&mut buf, ColorChoice::Never)
        .render_file(&run_dir.trace_path())
        .unwrap();
    let out = String::from_utf8(buf).unwrap();
    assert!(out.contains("design"));
    assert!(out.contains("implement"));

    // Passthrough
    let mut buf = Vec::new();
    passthrough(&run_dir.trace_path(), &mut buf).unwrap();
    let raw = String::from_utf8(buf).unwrap();
    assert!(raw.contains("\"name\":\"phase.design\""));
}

#[test]
fn render_file_malformed_warns_and_continues() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("trace.jsonl");
    std::fs::write(
        &path,
        r#"{"timestamp":"2026-04-25T20:45:00.000Z","name":"phase.x","trace_id":"t","span_id":"s1","loker.phase":"design"}
this is not json
{"timestamp":"2026-04-25T20:45:01.000Z","name":"phase.y","trace_id":"t","span_id":"s2","loker.phase":"implement"}
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
