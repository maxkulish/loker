//! Test runner verify hook.
//!
//! Runs a project's test suite via [`RunCommand`] internals, parses
//! structured output for pass/fail counts, and returns `Pass` only when
//! zero tests failed and at least one ran.
//!
//! # Supported runners
//!
//! | Kind | Command | Parser |
//! |------|---------|--------|
//! | `Cargo` | `cargo test --message-format=json --no-fail-fast` | JSON‑lines per [`cargo::test` message format](https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages) |
//! | `Pytest` | `pytest --json-report --json-report-file=-` | [pytest-json-report](https://pypi.org/project/pytest-json-report/) summary output |

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde::de::Deserialize;

use super::run_command::{redact_output, RunCommand};
use super::{FailureReason, VerifyContext, VerifyError, VerifyHook, VerifyResult};

/// Supported test runner kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestRunnerKind {
    Cargo,
    Pytest,
}

/// Sandboxing configuration for test runner execution.
///
/// Maps to [`RunCommand`] fields but is kept as a separate struct so the
/// `TestRunner` API doesn't expose `RunCommand` internals directly.
#[derive(Debug, Clone)]
pub struct SandboxOpts {
    /// Environment variable names allowed in child process.
    /// Default: empty (default‑deny).
    pub env_allowlist: Vec<String>,
    /// Wall‑clock timeout before process‑group SIGKILL.
    pub wall_timeout: Duration,
    /// Max bytes captured from stdout.
    pub stdout_cap: usize,
    /// Max bytes captured from stderr.
    pub stderr_cap: usize,
}

impl Default for SandboxOpts {
    fn default() -> Self {
        Self {
            env_allowlist: Vec::new(),
            wall_timeout: Duration::from_secs(120),
            stdout_cap: 8192,
            stderr_cap: 8192,
        }
    }
}

/// Test runner verify hook.
///
/// Executes a test suite via the configured [`TestRunnerKind`], parses the
/// structured output, and returns:
///
/// - `Pass` when `failed == 0 && passed > 0`.
/// - `Fail` when `failed > 0`, carrying `{failed, passed, first_failure_name, first_failure_excerpt}`.
/// - `Fail { reason: "no tests ran" }` when `passed == 0 && failed == 0`.
#[derive(Debug, Clone)]
pub struct TestRunner {
    /// Which test runner to use.
    pub runner: TestRunnerKind,
    /// Working directory for the command.
    pub cwd: PathBuf,
    /// Extra arguments passed through to the test command (after the
    /// runner‑specific base args).
    pub extra_args: Vec<String>,
    /// Sandboxing options (timeouts, caps, env allowlist).
    pub sandbox: SandboxOpts,
}

impl TestRunner {
    /// Construct a new test runner.
    pub fn new(runner: TestRunnerKind, cwd: impl Into<PathBuf>) -> Self {
        Self {
            runner,
            cwd: cwd.into(),
            extra_args: Vec::new(),
            sandbox: SandboxOpts::default(),
        }
    }

    /// Append extra arguments passed to the test command.
    pub fn with_extra_args(mut self, args: impl IntoIterator<Item: AsRef<str>>) -> Self {
        self.extra_args = args.into_iter().map(|s| s.as_ref().to_string()).collect();
        self
    }

    /// Override sandbox options.
    pub fn with_sandbox(mut self, opts: SandboxOpts) -> Self {
        self.sandbox = opts;
        self
    }

    // ── internal helpers ────────────────────────────────────

    fn build_run_command(&self) -> RunCommand {
        let mut rc = match self.runner {
            TestRunnerKind::Cargo => {
                let mut args = vec![
                    "test".to_string(),
                    "--message-format=json".to_string(),
                    "--no-fail-fast".to_string(),
                ];
                args.extend(self.extra_args.clone());
                RunCommand::new("cargo").with_args(args)
            }
            TestRunnerKind::Pytest => {
                let mut args = vec![
                    "--json-report".to_string(),
                    "--json-report-file=-".to_string(),
                ];
                args.extend(self.extra_args.clone());
                RunCommand::new("pytest").with_args(args)
            }
        };

        rc = rc
            .with_cwd(self.cwd.clone())
            .with_env_allowlist(
                &self
                    .sandbox
                    .env_allowlist
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>(),
            )
            .with_wall_timeout(self.sandbox.wall_timeout)
            .with_stdout_cap(self.sandbox.stdout_cap)
            .with_stderr_cap(self.sandbox.stderr_cap);

        rc
    }

    /// Parse cargo test JSON‑lines output into pass/fail counts.
    ///
    /// Each line is a JSON object per cargo's `--message-format=json`
    /// spec. We look for messages with `"type":"test"` and examine
    /// the `event` field (`"ok"`, `"failed"`, `"ignored"`, etc.).
    /// Malformed JSON lines are silently skipped.
    pub fn parse_cargo_output(stdout: &str) -> TestResult {
        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut first_failure_name: Option<String> = None;
        let mut first_failure_excerpt: Option<String> = None;

        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let value: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue, // skip malformed lines
            };

            // Only process test events
            if value.get("type").and_then(|v| v.as_str()) != Some("test") {
                continue;
            }

            let event = value.get("event").and_then(|v| v.as_str()).unwrap_or("");

            // Ignored tests have `"ignored": true` alongside `"event": "ok"`.
            // Skip them — they didn't actually run.
            let ignored = value
                .get("ignored")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            match (event, ignored) {
                ("ok", false) => {
                    passed += 1;
                }
                ("ok", true) => {
                    // ignored test — not counted
                }
                ("failed" | "timeout", _) => {
                    failed += 1;
                    if first_failure_name.is_none() {
                        first_failure_name = value
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        // Cargo puts the failure stdout in the `stdout` field
                        first_failure_excerpt =
                            value.get("stdout").and_then(|v| v.as_str()).map(|s| {
                                let s = s.trim();
                                // Truncate to first 200 chars for a concise excerpt
                                truncate_excerpt(s, 200)
                            });
                    }
                }
                _ => {
                    // "ignored", "measured" (benchmarks) — not counted
                }
            }
        }

        TestResult {
            passed,
            failed,
            first_failure_name,
            first_failure_excerpt,
        }
    }

    /// Parse pytest JSON report output.
    ///
    /// Expects a single JSON object with a `summary` field containing
    /// `passed` and `failed` integers (from `pytest-json-report`).
    pub fn parse_pytest_output(stdout: &str) -> TestResult {
        // Try parsing the entire stdout as JSON first (handles multi-line reports).
        // Fall back to line-by-line search if that fails.
        let maybe_value: Option<serde_json::Value> =
            serde_json::from_str(stdout).ok().or_else(|| {
                stdout.find('{').and_then(|start| {
                    let mut de = serde_json::Deserializer::from_str(&stdout[start..]);
                    serde_json::Value::deserialize(&mut de).ok()
                })
            });

        let value = match maybe_value {
            Some(v) => v,
            None => {
                return TestResult {
                    passed: 0,
                    failed: 0,
                    first_failure_name: None,
                    first_failure_excerpt: Some(
                        "could not parse pytest JSON report from stdout".to_string(),
                    ),
                };
            }
        };

        let summary = match value.get("summary") {
            Some(s) => s,
            None => {
                return TestResult {
                    passed: 0,
                    failed: 0,
                    first_failure_name: None,
                    first_failure_excerpt: Some(
                        "pytest report missing `summary` field".to_string(),
                    ),
                };
            }
        };

        let passed = summary.get("passed").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let failed = summary.get("failed").and_then(|v| v.as_u64()).unwrap_or(0) as u32
            + summary.get("error").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

        let first_failed_test = value
            .get("tests")
            .and_then(|v| v.as_array())
            .and_then(|tests| {
                tests.iter().find(|t| {
                    let outcome = t.get("outcome").and_then(|o| o.as_str());
                    outcome == Some("failed") || outcome == Some("error")
                })
            });

        let first_failure_name = first_failed_test
            .and_then(|t| t.get("nodeid").and_then(|n| n.as_str()))
            .map(|s| s.to_string());

        let first_failure_excerpt = first_failed_test
            .and_then(|t| {
                t.get("call")
                    .and_then(|c| c.get("longrepr"))
                    .or_else(|| t.get("longrepr"))
                    .and_then(|l| l.as_str())
            })
            .map(|s| truncate_excerpt(s, 200));

        TestResult {
            passed,
            failed,
            first_failure_name,
            first_failure_excerpt,
        }
    }

    fn parse_output(&self, stdout: &str) -> TestResult {
        match self.runner {
            TestRunnerKind::Cargo => Self::parse_cargo_output(stdout),
            TestRunnerKind::Pytest => Self::parse_pytest_output(stdout),
        }
    }

    pub fn to_verify_result(
        run_command_run: super::run_command::CommandRun,
        result: TestResult,
    ) -> VerifyResult {
        let runner_stdout = redact_output(
            &run_command_run.stdout.to_reason_text(),
            &run_command_run.secret_values,
        );
        let runner_stderr = redact_output(
            &run_command_run.stderr.to_reason_text(),
            &run_command_run.secret_values,
        );
        let truncated = run_command_run.stdout.truncated || run_command_run.stderr.truncated;
        let exit_code = run_command_run.status.code();
        let signal = run_command_run
            .status
            .code()
            .is_none()
            .then(|| {
                #[cfg(unix)]
                {
                    std::os::unix::process::ExitStatusExt::signal(&run_command_run.status)
                }
                #[cfg(not(unix))]
                {
                    None::<i32>
                }
            })
            .flatten();

        // Check for sandbox violations first
        if run_command_run.timed_out {
            return VerifyResult::Fail {
                reason: FailureReason::new("test runner timed out")
                    .with_stdout(runner_stdout)
                    .with_stderr(runner_stderr)
                    .with_truncated(truncated)
                    .with_sandbox_violation(crate::strategy::verify::SandboxViolation::Timeout),
            };
        }

        if let Some(sig) = signal {
            return VerifyResult::Fail {
                reason: FailureReason::new(format!("test runner killed by signal {sig}"))
                    .with_stdout(runner_stdout)
                    .with_stderr(runner_stderr)
                    .with_truncated(truncated)
                    .with_sandbox_violation(crate::strategy::verify::SandboxViolation::Signal {
                        signal: sig,
                    }),
            };
        }

        // Parse test output
        if let Some(code) = exit_code {
            if code != 0 && result.failed == 0 {
                if result.passed == 0 {
                    return VerifyResult::Fail {
                        reason: FailureReason::new("no tests ran")
                            .with_stdout(runner_stdout)
                            .with_stderr(runner_stderr)
                            .with_truncated(truncated)
                            .with_exit_code(code)
                            .with_sandbox_violation(
                                crate::strategy::verify::SandboxViolation::NonZeroExit { code },
                            ),
                    };
                }

                return VerifyResult::Fail {
                    reason: FailureReason::new(format!("test runner exited with status {code}"))
                        .with_stdout(runner_stdout)
                        .with_stderr(runner_stderr)
                        .with_truncated(truncated)
                        .with_exit_code(code)
                        .with_sandbox_violation(
                            crate::strategy::verify::SandboxViolation::NonZeroExit { code },
                        ),
                };
            }
        }

        if result.passed == 0 && result.failed == 0 {
            return VerifyResult::Fail {
                reason: FailureReason::new("no tests ran")
                    .with_stdout(runner_stdout)
                    .with_stderr(runner_stderr)
                    .with_truncated(truncated)
                    .with_exit_code(exit_code.unwrap_or(1)),
            };
        }

        if result.failed > 0 {
            let summary = match (&result.first_failure_name, &result.first_failure_excerpt) {
                (Some(name), Some(excerpt)) => {
                    format!(
                        "{} test(s) failed (first: {name}: {excerpt})",
                        result.failed
                    )
                }
                (Some(name), None) => {
                    format!("{} test(s) failed (first: {name})", result.failed)
                }
                (None, _) => {
                    format!("{} test(s) failed", result.failed)
                }
            };

            return VerifyResult::Fail {
                reason: FailureReason::new(summary)
                    .with_stdout(runner_stdout)
                    .with_stderr(runner_stderr)
                    .with_truncated(truncated)
                    .with_exit_code(exit_code.unwrap_or(1)),
            };
        }

        // All passed
        VerifyResult::Pass
    }
}

#[async_trait]
impl VerifyHook for TestRunner {
    fn name(&self) -> &str {
        match self.runner {
            TestRunnerKind::Cargo => "TestRunner(cargo)",
            TestRunnerKind::Pytest => "TestRunner(pytest)",
        }
    }

    async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
        let rc = self.build_run_command();
        let run = rc.run().await?;
        let parsed = self.parse_output(&run.stdout.to_reason_text());
        Ok(Self::to_verify_result(run, parsed))
    }
}

/// Structured result from parsing test runner output.
#[derive(Debug, Clone)]
pub struct TestResult {
    pub passed: u32,
    pub failed: u32,
    pub first_failure_name: Option<String>,
    pub first_failure_excerpt: Option<String>,
}

fn truncate_excerpt(text: &str, max_chars: usize) -> String {
    let normalized = text.trim();

    let mut chars = normalized.chars();
    let mut result: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

#[cfg(test)]
fn exit_status_from_code(code: i32) -> std::process::ExitStatus {
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

// ── unit tests (parser logic only, no subprocess) ───────────

#[cfg(test)]
mod tests {
    use super::*;

    fn cargo_test_result(fixture: &str) -> TestResult {
        TestRunner::parse_cargo_output(fixture)
    }

    fn pytest_test_result(fixture: &str) -> TestResult {
        TestRunner::parse_pytest_output(fixture)
    }

    // ── Cargo parser ────────────────────────────────────────

    #[test]
    fn cargo_3_pass_0_fail() {
        let output = r#"{"type":"test","event":"ok","name":"it_works","test_type":"unit"}
{"type":"test","event":"ok","name":"test_add","test_type":"unit"}
{"type":"test","event":"ok","name":"test_subtract","test_type":"unit"}
"#;
        let result = cargo_test_result(output);
        assert_eq!(result.passed, 3);
        assert_eq!(result.failed, 0);
        assert!(result.first_failure_name.is_none());
        assert!(result.first_failure_excerpt.is_none());
    }

    #[test]
    fn cargo_2_pass_1_fail() {
        let output = r#"{"type":"test","event":"ok","name":"it_works","test_type":"unit"}
{"type":"test","event":"failed","name":"test_bad_add","test_type":"unit","stdout":"assertion `left == right` failed\n  left: 3\n right: 5\n"}
{"type":"test","event":"ok","name":"test_good_add","test_type":"unit"}
"#;
        let result = cargo_test_result(output);
        assert_eq!(result.passed, 2);
        assert_eq!(result.failed, 1);
        assert_eq!(result.first_failure_name.as_deref(), Some("test_bad_add"));
        assert!(result
            .first_failure_excerpt
            .as_deref()
            .unwrap_or("")
            .contains("left == right"));
    }

    #[test]
    fn cargo_empty_no_tests() {
        let output = r#"{"type":"test","event":"ok","name":"test_dummy","test_type":"unit","ignored":true}
"#;
        // Only ignored tests — no actual tests ran
        let result = cargo_test_result(output);
        assert_eq!(result.passed, 0);
        assert_eq!(result.failed, 0);
    }

    #[test]
    fn cargo_malformed_json_line_skipped() {
        let output = r#"{"type":"test","event":"ok","name":"good_test","test_type":"unit"}
this is not json at all
{"type":"test","event":"ok","name":"another_test","test_type":"unit"}
{"type":"test","event":"failed","name":"bad_test","test_type":"unit","stdout":"failure!"}
"#;
        let result = cargo_test_result(output);
        assert_eq!(result.passed, 2);
        assert_eq!(result.failed, 1);
        assert_eq!(result.first_failure_name.as_deref(), Some("bad_test"));
    }

    #[test]
    fn cargo_skips_compiler_messages() {
        let output = r#"{"type":"compiler","message":"compiling foo"}
{"type":"test","event":"ok","name":"test_foo","test_type":"unit"}
{"type":"artifact","profile":"test"}
{"type":"test","event":"ok","name":"test_bar","test_type":"unit"}
"#;
        let result = cargo_test_result(output);
        assert_eq!(result.passed, 2);
        assert_eq!(result.failed, 0);
    }

    #[test]
    fn cargo_first_failure_preserves_stdout_excerpt() {
        let output = r#"{"type":"test","event":"ok","name":"passing","test_type":"unit"}
{"type":"test","event":"failed","name":"failing_test","test_type":"unit","stdout":"thread 'failing_test' panicked at src/main.rs:42:\nassertion `left == right` failed\n  left: 1\n right: 2\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n"}
{"type":"test","event":"failed","name":"also_failing","test_type":"unit","stdout":"another failure"}
"#;
        let result = cargo_test_result(output);
        assert_eq!(result.passed, 1);
        assert_eq!(result.failed, 2);
        assert_eq!(result.first_failure_name.as_deref(), Some("failing_test"));
        let excerpt = result.first_failure_excerpt.unwrap();
        assert!(excerpt.contains("panicked"));
        assert!(excerpt.contains("left == right"));
    }

    #[test]
    fn cargo_first_failure_truncates_utf8_excerpt_safely() {
        let unicode_excerpt = "😀".repeat(260);
        let output = format!(
            "{{\"type\":\"test\",\"event\":\"ok\",\"name\":\"passing\",\"test_type\":\"unit\"}}\n{{\"type\":\"test\",\"event\":\"failed\",\"name\":\"failing_test\",\"test_type\":\"unit\",\"stdout\":\"{unicode_excerpt}\"}}",
        );
        let result = cargo_test_result(&output);

        let excerpt = result
            .first_failure_excerpt
            .expect("expected excerpt to be captured");

        assert_eq!(excerpt.chars().count(), 201);
        assert!(excerpt.ends_with('…'));
        assert!(excerpt.len() > 200);
        assert!(excerpt.len() < 810);
    }

    // ── Pytest parser ───────────────────────────────────────

    #[test]
    fn pytest_5_pass_0_fail() {
        let output = r#"{"created": 1234567890, "duration": 0.15, "exitcode": 0, "root": "/tmp", "summary": {"passed": 5, "failed": 0, "total": 5, "collected": 5}, "tests": [{"nodeid": "test_foo.py::test_a", "outcome": "passed"}, {"nodeid": "test_foo.py::test_b", "outcome": "passed"}]}"#;
        let result = pytest_test_result(output);
        assert_eq!(result.passed, 5);
        assert_eq!(result.failed, 0);
        assert!(result.first_failure_name.is_none());
    }

    #[test]
    fn pytest_4_pass_2_fail() {
        let output = r#"{"created": 1234567890, "duration": 0.3, "exitcode": 1, "root": "/tmp", "summary": {"passed": 4, "failed": 2, "total": 6, "collected": 6}, "tests": [{"nodeid": "test_foo.py::test_a", "outcome": "passed"}, {"nodeid": "test_bar.py::test_bad", "outcome": "failed", "call": {"longrepr": "AssertionError: expected 5 got 3"}}, {"nodeid": "test_baz.py::test_bad2", "outcome": "failed", "call": {"longrepr": "TypeError: unsupported operand"}}]}"#;
        let result = pytest_test_result(output);
        assert_eq!(result.passed, 4);
        assert_eq!(result.failed, 2);
        assert_eq!(
            result.first_failure_name.as_deref(),
            Some("test_bar.py::test_bad")
        );
        assert!(result
            .first_failure_excerpt
            .as_deref()
            .unwrap_or("")
            .contains("AssertionError"));
    }

    #[test]
    fn pytest_empty_no_tests() {
        let output = r#"{"created": 1234567890, "duration": 0.01, "exitcode": 0, "root": "/tmp", "summary": {"passed": 0, "failed": 0, "total": 0, "collected": 0}, "tests": []}"#;
        let result = pytest_test_result(output);
        assert_eq!(result.passed, 0);
        assert_eq!(result.failed, 0);
    }

    #[test]
    fn pytest_missing_summary_field() {
        let output = r#"{"created": 1234567890, "duration": 0.01, "exitcode": 0}"#;
        let result = pytest_test_result(output);
        assert_eq!(result.passed, 0);
        assert_eq!(result.failed, 0);
        assert!(result
            .first_failure_excerpt
            .as_deref()
            .unwrap_or("")
            .contains("missing `summary` field"));
    }

    #[test]
    fn pytest_non_json_output() {
        let output = "pytest: error: no tests found";
        let result = pytest_test_result(output);
        assert_eq!(result.passed, 0);
        assert_eq!(result.failed, 0);
        assert!(result
            .first_failure_excerpt
            .as_deref()
            .unwrap_or("")
            .contains("could not parse"));
    }

    // ── VerifyResult conversion ─────────────────────────────

    fn fake_captured_output(data: &str) -> super::super::run_command::CapturedOutput {
        super::super::run_command::CapturedOutput {
            data: data.as_bytes().to_vec(),
            truncated: false,
            elided_bytes: 0,
        }
    }

    fn fake_command_run(stdout_data: &str) -> super::super::run_command::CommandRun {
        super::super::run_command::CommandRun {
            status: exit_status_from_code(0),
            timed_out: false,
            stdout: fake_captured_output(stdout_data),
            stderr: fake_captured_output(""),
            secret_values: vec![],
            elapsed_ms: 10,
        }
    }

    #[test]
    fn verify_result_from_passing_tests() {
        let stdout = r#"{"type":"test","event":"ok","name":"test_a","test_type":"unit"}
{"type":"test","event":"ok","name":"test_b","test_type":"unit"}
"#;
        let result = TestRunner::parse_cargo_output(stdout);
        let run = fake_command_run(stdout);
        let vr = TestRunner::to_verify_result(run, result);
        assert!(matches!(vr, VerifyResult::Pass));
    }

    #[test]
    fn verify_result_from_failing_tests() {
        let stdout = r#"{"type":"test","event":"ok","name":"test_a","test_type":"unit"}
{"type":"test","event":"failed","name":"test_bad","test_type":"unit","stdout":"assertion failed!"}
"#;
        let result = TestRunner::parse_cargo_output(stdout);
        let run = fake_command_run(stdout);
        let vr = TestRunner::to_verify_result(run, result);
        match vr {
            VerifyResult::Fail { reason } => {
                assert!(reason.summary.contains("1 test(s) failed"));
                assert!(reason.summary.contains("test_bad"));
                assert!(reason.summary.contains("assertion failed!"));
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn verify_result_no_tests_ran() {
        let stdout = r#"{"type":"test","event":"ignored","name":"ignored_test","test_type":"unit"}
"#;
        let result = TestRunner::parse_cargo_output(stdout);
        let run = fake_command_run(stdout);
        let vr = TestRunner::to_verify_result(run, result);
        match vr {
            VerifyResult::Fail { reason } => {
                assert!(reason.summary.contains("no tests ran"));
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn verify_result_timed_out() {
        let stdout = r#"{"type":"test","event":"ok","name":"test_a","test_type":"unit"}
"#;
        let result = TestRunner::parse_cargo_output(stdout);
        let mut run = fake_command_run(stdout);
        run.timed_out = true;
        let vr = TestRunner::to_verify_result(run, result);
        match vr {
            VerifyResult::Fail { reason } => {
                assert!(reason.summary.contains("timed out"));
                assert!(reason.sandbox_violation.is_some());
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn verify_result_killed_by_signal() {
        let stdout = r#"{"type":"test","event":"ok","name":"test_a","test_type":"unit"}
"#;
        let result = TestRunner::parse_cargo_output(stdout);
        let mut run = fake_command_run(stdout);
        use std::os::unix::process::ExitStatusExt;
        run.status = std::process::ExitStatus::from_raw(9); // SIGKILL
        let vr = TestRunner::to_verify_result(run, result);
        match vr {
            VerifyResult::Fail { reason } => {
                assert!(reason.summary.contains("killed by signal"));
                assert!(reason.sandbox_violation.is_some());
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }
}
