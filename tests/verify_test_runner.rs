//! Integration tests for the `TestRunner` verify hook.
//!
//! Drives the JSON parsers from canned fixture files — no actual `cargo`
//! or `pytest` invocation in unit tests. Parser output is then fed
//! through `TestRunner::to_verify_result` to exercise the
//! `VerifyHook::verify` logic without subprocess overhead.

use std::path::Path;

use loker::strategy::verify::run_command::{CapturedOutput, CommandRun};
use loker::strategy::verify::{TestRunner, VerifyResult};

// ── helpers ──────────────────────────────────────────────────

/// Read a fixture file as a string.
fn read_fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("test_runner")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("failed to read fixture {name}: {e}");
    })
}

/// Convert fixture content to a JSON‑lines string that the cargo parser
/// would see on stdout. `.jsonl` files contain a JSON array — we flatten
/// each element to one JSON-per-line.
fn fixture_to_cargo_stdout(fixture: &str) -> String {
    // Cargo JSON-lines fixtures are raw JSON-per-line content.
    fixture.to_string()
}

/// Read a pytest fixture file as a string.
fn fake_captured_output(data: &str) -> CapturedOutput {
    CapturedOutput {
        data: data.as_bytes().to_vec(),
        truncated: false,
        elided_bytes: 0,
    }
}

fn exit_status(code: i32) -> std::process::ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code << 8)
    }

    #[cfg(not(unix))]
    {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code as u32)
    }
}

fn fake_command_run_with_code(stdout_data: &str, exit_code: i32) -> CommandRun {
    CommandRun {
        status: exit_status(exit_code),
        timed_out: false,
        stdout: fake_captured_output(stdout_data),
        stderr: fake_captured_output(""),
        secret_values: vec![],
        elapsed_ms: 10,
    }
}

fn fake_command_run(stdout_data: &str) -> CommandRun {
    fake_command_run_with_code(stdout_data, 0)
}

/// Parse cargo fixture, then convert to VerifyResult.
fn cargo_fixture_verify(name: &str) -> VerifyResult {
    let raw = read_fixture(name);
    let stdout = fixture_to_cargo_stdout(&raw);
    let result = TestRunner::parse_cargo_output(&stdout);
    let run = fake_command_run(&stdout);
    TestRunner::to_verify_result(run, result)
}

/// Parse pytest fixture, then convert to VerifyResult.
fn pytest_fixture_verify(name: &str) -> VerifyResult {
    let stdout = read_fixture(name);
    let result = TestRunner::parse_pytest_output(&stdout);
    let run = fake_command_run(&stdout);
    TestRunner::to_verify_result(run, result)
}

// ── Cargo tests ─────────────────────────────────────────────

#[test]
fn cargo_3_pass_0_fail() {
    let result = cargo_fixture_verify("cargo_3pass_0fail.jsonl");
    assert!(
        matches!(result, VerifyResult::Pass),
        "expected Pass, got {result:?}"
    );
}

#[test]
fn cargo_2_pass_1_fail() {
    let result = cargo_fixture_verify("cargo_2pass_1fail.jsonl");
    match result {
        VerifyResult::Fail { reason } => {
            assert!(
                reason.summary.contains("1 test(s) failed"),
                "summary should mention 1 failure: {}",
                reason.summary
            );
            assert!(
                reason.summary.contains("test_bad_divide"),
                "summary should contain failure name: {}",
                reason.summary
            );
            assert!(
                reason.summary.contains("assertion"),
                "summary should contain failure excerpt: {}",
                reason.summary
            );
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[test]
fn cargo_empty_no_tests() {
    let result = cargo_fixture_verify("cargo_empty.jsonl");
    match result {
        VerifyResult::Fail { reason } => {
            assert!(
                reason.summary.contains("no tests ran"),
                "expected 'no tests ran', got: {}",
                reason.summary
            );
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[test]
fn cargo_malformed_json_lines() {
    // The malformed fixture has a non-JSON line mixed with valid JSON lines.
    // The parser should skip the bad line and still count the valid ones.
    let raw = read_fixture("cargo_malformed.jsonl");
    let stdout = fixture_to_cargo_stdout(&raw);
    let result = TestRunner::parse_cargo_output(&stdout);
    assert_eq!(result.passed, 2, "should count 2 passing tests");
    assert_eq!(result.failed, 1, "should count 1 failing test");
}

// ── Pytest tests ────────────────────────────────────────────

#[test]
fn pytest_5_pass_0_fail() {
    let result = pytest_fixture_verify("pytest_5pass_0fail.json");
    assert!(
        matches!(result, VerifyResult::Pass),
        "expected Pass, got {result:?}"
    );
}

#[test]
fn pytest_4_pass_2_fail() {
    let result = pytest_fixture_verify("pytest_4pass_2fail.json");
    match result {
        VerifyResult::Fail { reason } => {
            assert!(
                reason.summary.contains("2 test(s) failed"),
                "summary should mention 2 failures: {}",
                reason.summary
            );
            assert!(
                reason.summary.contains("test_bar.py::test_bad"),
                "summary should contain first failure name: {}",
                reason.summary
            );
            assert!(
                reason.summary.contains("AssertionError"),
                "summary should contain failure excerpt: {}",
                reason.summary
            );
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[test]
fn pytest_noisy_multiline_output_is_parsed() {
    let stdout = r#"INFO: running tests
{
  "created": 1234567890,
  "duration": 0.1,
  "exitcode": 0,
  "root": "/tmp",
  "summary": {"passed": 1, "failed": 0, "total": 1, "collected": 1},
  "tests": [{"nodeid": "test.py::test_ok", "outcome": "passed"}]
}
INFO: done
"#;
    let result = TestRunner::parse_pytest_output(stdout);
    assert_eq!(result.passed, 1);
    assert_eq!(result.failed, 0);
}

#[test]
fn pytest_non_json_exit() {
    // Simulate process exits non-zero with no JSON written
    let stdout = "pytest: error: no tests found in test_runner/\n";
    let result = TestRunner::parse_pytest_output(stdout);
    assert_eq!(result.passed, 0);
    assert_eq!(result.failed, 0);

    // Build a command run with non-zero exit
    let run = CommandRun {
        status: exit_status(1),
        timed_out: false,
        stdout: fake_captured_output(stdout),
        stderr: fake_captured_output("ERROR: no tests collected\n"),
        secret_values: vec![],
        elapsed_ms: 15,
    };
    let vr = TestRunner::to_verify_result(run, result);
    match vr {
        VerifyResult::Fail { reason } => {
            assert!(
                reason.summary.contains("no tests ran"),
                "expected 'no tests ran', got: {}",
                reason.summary
            );
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[test]
fn verify_result_non_zero_exit_with_passing_tests_fails() {
    let stdout = r#"{"type":"test","event":"ok","name":"test_a","test_type":"unit"}
{"type":"test","event":"ok","name":"test_b","test_type":"unit"}
"#;
    let result = TestRunner::parse_cargo_output(stdout);
    let run = fake_command_run_with_code(stdout, 1);
    let vr = TestRunner::to_verify_result(run, result);
    match vr {
        VerifyResult::Fail { reason } => {
            assert!(
                reason.summary.contains("test runner exited with status 1"),
                "expected non-zero status summary, got: {}",
                reason.summary
            );
            assert!(
                reason.sandbox_violation.is_some(),
                "expected sandbox violation for non-zero exit"
            );
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}
