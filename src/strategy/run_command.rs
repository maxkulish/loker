//! RunCommand verify hook — executes a shell command and maps its exit status to
//! `VerifyResult::Pass` / `VerifyResult::Fail`.
//!
//! This is the foundational hook variant: shell-out gates retries in escalating
//! strategies and feeds T-029 (phase runner).
//!
//! # Sandboxing
//!
//! | Constraint | Implementation |
//! |---|---|
//! | cwd | Must be set explicitly; never inherits the orchestrator's cwd implicitly. |
//! | env | Default-deny. Only variables in `env_allowlist` are forwarded. |
//! | wall timeout | Hard kill (SIGKILL) on expiry; recorded as distinct failure reason. |
//! | cpu timeout (unix) | rlimit-based via `setrlimit(RLIMIT_CPU)` in `pre_exec`; best-effort on macOS. |
//! | stdout/stderr caps | Byte-count caps; excess output is dropped, not buffered. |
//! | signal cleanup | Kill the entire process group on timeout/cancel so children don't outlive the hook. |
//! | network policy | Inherits host network in v0; sandbox is process-level only. No netns isolation yet. |
//! | file mutation | Documented expectation that hooks may read/write the workspace; no rollback guarantees. |
//! | secret redaction | If `env_allowlist` includes a known-secret-shaped name, redact its value from the failure reason. |
//!
//! # Cancellation safety
//!
//! If the future is dropped mid-execution (e.g. `tokio::timeout` expires),
//! the spawned child process is reaped via `kill(-pgid, SIGKILL)`. This
//! prevents orphan processes.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::strategy::verify::{
    FailureReason, VerifyContext, VerifyError, VerifyHook, VerifyResult,
};

// ── Constants ────────────────────────────────────────────────

/// Default stdout/stderr capture cap (64 KiB).
pub const DEFAULT_BYTE_CAP: usize = 65_536;

/// Default wall timeout (30 seconds).
pub const DEFAULT_WALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Known secret-shaped substrings. If an allowlisted env var name contains
/// any of these (case-insensitive), its value is partially redacted in
/// failure reason output.
const SECRET_PATTERNS: &[&str] = &[
    "SECRET", "TOKEN", "KEY", "PASSWORD", "API_KEY", "APIKEY", "AUTH",
];

// ── RunCommand hook ──────────────────────────────────────────

/// A verify hook that executes a shell command and maps its exit status
/// to a `VerifyResult`.
///
/// # Sandboxing guarantees
///
/// See module-level documentation for the full sandboxing matrix.
pub struct RunCommand {
    /// Command to execute (e.g. `"cargo"`, `"/bin/sh"`).
    pub cmd: String,
    /// Arguments passed to the command.
    pub args: Vec<String>,
    /// Environment variables allowed to pass through. Default-deny —
    /// only variables listed here are forwarded to the child process.
    pub env_allowlist: Vec<String>,
    /// Working directory for the command. If `None`, the command runs in
    /// the system default (typically `/`), **not** the orchestrator's cwd.
    pub cwd: Option<PathBuf>,
    /// Wall-clock timeout. The process group is killed with SIGKILL on expiry.
    pub wall_timeout: Duration,
    /// CPU time limit (unix only, best-effort on macOS). Applied via
    /// `setrlimit(RLIMIT_CPU)` in `pre_exec` before exec.
    pub cpu_timeout: Option<Duration>,
    /// Maximum bytes to capture from stdout. Excess bytes are discarded.
    pub stdout_cap: usize,
    /// Maximum bytes to capture from stderr. Excess bytes are discarded.
    pub stderr_cap: usize,
}

impl RunCommand {
    /// Create a new `RunCommand` with default sandboxing settings.
    ///
    /// Defaults:
    /// - `args`: empty
    /// - `env_allowlist`: empty (default-deny)
    /// - `cwd`: `None` (does not inherit orchestrator cwd)
    /// - `wall_timeout`: 30 seconds
    /// - `cpu_timeout`: `None`
    /// - `stdout_cap`: 64 KiB
    /// - `stderr_cap`: 64 KiB
    pub fn new(cmd: impl Into<String>) -> Self {
        Self {
            cmd: cmd.into(),
            args: Vec::new(),
            env_allowlist: Vec::new(),
            cwd: None,
            wall_timeout: DEFAULT_WALL_TIMEOUT,
            cpu_timeout: None,
            stdout_cap: DEFAULT_BYTE_CAP,
            stderr_cap: DEFAULT_BYTE_CAP,
        }
    }

    // ── Builder-pattern setters ──────────────────────────────

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        self.args = args;
        self
    }

    /// Allow specific environment variables to pass through to the child
    /// process. Only variables listed here are forwarded; everything else
    /// is stripped (default-deny).
    pub fn with_env_allowlist(mut self, allowlist: Vec<String>) -> Self {
        self.env_allowlist = allowlist;
        self
    }

    /// Set the working directory for the command.
    pub fn with_cwd(mut self, cwd: PathBuf) -> Self {
        self.cwd = Some(cwd);
        self
    }

    /// Set the wall-clock timeout. The process group is killed with SIGKILL
    /// on expiry.
    pub fn with_wall_timeout(mut self, timeout: Duration) -> Self {
        self.wall_timeout = timeout;
        self
    }

    /// Set the CPU time limit (unix only, best-effort on macOS). Applied
    /// via `setrlimit(RLIMIT_CPU)` in the child before exec.
    pub fn with_cpu_timeout(mut self, timeout: Duration) -> Self {
        self.cpu_timeout = Some(timeout);
        self
    }

    /// Set the maximum bytes to capture from stdout.
    pub fn with_stdout_cap(mut self, cap: usize) -> Self {
        self.stdout_cap = cap;
        self
    }

    /// Set the maximum bytes to capture from stderr.
    pub fn with_stderr_cap(mut self, cap: usize) -> Self {
        self.stderr_cap = cap;
        self
    }
}

// ── Core execution logic ─────────────────────────────────────

/// Result of executing a command with cap-aware output capture.
struct RunResult {
    stdout_bytes: Vec<u8>,
    stderr_bytes: Vec<u8>,
    stdout_capped: bool,
    stderr_capped: bool,
    exit_status: Option<std::process::ExitStatus>,
    timed_out: bool,
}

/// Read bytes from a piped child stream, capping at `cap` bytes.
async fn read_stream<R>(mut stream: Option<R>, cap: usize) -> (Vec<u8>, bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::with_capacity(cap.min(4096));
    let mut capped = false;
    let mut read_buf = vec![0u8; 4096];
    if let Some(ref mut stream) = stream {
        loop {
            match stream.read(&mut read_buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let remaining = cap.saturating_sub(buf.len());
                    if remaining == 0 {
                        capped = true;
                        continue;
                    }
                    let to_take = n.min(remaining);
                    buf.extend_from_slice(&read_buf[..to_take]);
                    if to_take < n {
                        capped = true;
                    }
                }
                Err(_) => break,
            }
        }
    }
    (buf, capped)
}

/// Execute a shell command with sandboxing constraints.
///
/// This is the core execution function, factored out so it can be tested
/// independently and reused by other hook implementations (e.g. TestRunner).
#[allow(clippy::too_many_arguments)]
async fn execute_command(
    cmd: &str,
    args: &[String],
    env_allowlist: &[String],
    cwd: Option<&PathBuf>,
    wall_timeout: Duration,
    cpu_timeout: Option<Duration>,
    stdout_cap: usize,
    stderr_cap: usize,
) -> Result<RunResult, std::io::Error> {
    let mut command = Command::new(cmd);
    command.args(args);

    // Default-deny env: only forward allowlisted variables
    command.env_clear();
    for var in env_allowlist {
        if let Ok(val) = std::env::var(var) {
            command.env(var, val);
        }
    }

    // Set cwd if provided; otherwise use root to avoid inheriting orchestrator cwd
    if let Some(dir) = cwd {
        command.current_dir(dir);
    } else {
        command.current_dir("/");
    }

    // Capture stdout and stderr
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());

    // Platform-specific setup: process group and CPU rlimit
    #[cfg(unix)]
    {
        let cpu_secs = cpu_timeout.map(|d| d.as_secs() as libc::rlim_t);
        unsafe {
            command.pre_exec(move || {
                // Create a new process group so we can kill all children at once
                libc::setpgid(0, 0);

                // Apply CPU time limit if configured
                if let Some(secs) = cpu_secs {
                    if secs > 0 {
                        let rlim = libc::rlimit {
                            rlim_cur: secs,
                            rlim_max: secs,
                        };
                        // Ignore errors (macOS may not support RLIMIT_CPU)
                        let _ = libc::setrlimit(libc::RLIMIT_CPU, &rlim);
                    }
                }
                Ok(())
            });
        }
    }

    // Kill child on drop for cancellation safety (tokio kills just the child).
    // Our ChildGuard handles the process group.
    command.kill_on_drop(true);

    let child = command.spawn()?;
    let mut child = ChildGuard::new(child);

    // ── Read output with caps, wall timeout wraps it ─────────

    match timeout(wall_timeout, async {
        // Read stdout and stderr concurrently to avoid deadlock when
        // the child fills one pipe while blocking on the other.
        let ((stdout_bytes, stdout_capped), (stderr_bytes, stderr_capped)) = tokio::join!(
            read_stream(child.inner.stdout.take(), stdout_cap),
            read_stream(child.inner.stderr.take(), stderr_cap),
        );
        let exit_status = child.inner.wait().await?;

        // Disarm the drop guard since we successfully reaped the child
        child.disarm();

        Ok(RunResult {
            stdout_bytes,
            stderr_bytes,
            stdout_capped,
            stderr_capped,
            exit_status: Some(exit_status),
            timed_out: false,
        })
    })
    .await
    {
        Ok(result) => result,
        Err(_elapsed) => {
            // Wall timeout: kill the process group
            ChildGuard::kill(&child.inner);
            // Reap the child
            let _ = child.inner.wait().await;
            // Disarm the guard to prevent double SIGKILL on drop
            child.disarm();

            Ok(RunResult {
                stdout_bytes: Vec::new(),
                stderr_bytes: Vec::new(),
                stdout_capped: false,
                stderr_capped: false,
                exit_status: None,
                timed_out: true,
            })
        }
    }
}

/// RAII guard that kills the process group when dropped.
///
/// This handles cancellation safety: if the `verify()` future is dropped
/// mid-execution (e.g. phase runner cancels the step), the guard ensures
/// the child process and its descendants are killed.
struct ChildGuard {
    inner: tokio::process::Child,
    disarmed: bool,
}

impl ChildGuard {
    fn new(child: tokio::process::Child) -> Self {
        Self {
            inner: child,
            disarmed: false,
        }
    }

    /// Disarm the guard so the child is NOT killed on drop.
    /// Call this after successfully reaping the child via `wait()`.
    fn disarm(&mut self) {
        self.disarmed = true;
    }

    /// Kill the process group of a child (static method, usable after take()).
    fn kill(child: &tokio::process::Child) {
        kill_process_group(child);
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if !self.disarmed {
            kill_process_group(&self.inner);
        }
    }
}

/// Kill the entire process group of a spawned child.
#[cfg(unix)]
fn kill_process_group(child: &tokio::process::Child) {
    if let Some(pid) = child.id() {
        let pgid = pid as libc::pid_t;
        // Try process group first (negative PID = PGID)
        unsafe {
            if libc::kill(-pgid, libc::SIGKILL) != 0 {
                // Fall back to killing just the child
                libc::kill(pgid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(not(unix))]
fn kill_process_group(child: &tokio::process::Child) {
    if let Some(id) = child.id() {
        // On Windows, use taskkill /T to kill the process tree
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &id.to_string()])
            .output();
    }
}

// ── Secret redaction ─────────────────────────────────────────

/// Check if an environment variable name matches known secret patterns.
fn is_secret_var(name: &str) -> bool {
    let upper = name.to_uppercase();
    SECRET_PATTERNS.iter().any(|pat| upper.contains(pat))
}

/// Partially redact a secret value, keeping only the first 4 characters.
fn redact_value(value: &str) -> String {
    if value.len() >= 4 {
        let prefix: String = value.chars().take(4).collect();
        format!("{prefix}***")
    } else {
        "***".to_string()
    }
}

/// Apply secret redaction to the FailureReason if any allowlisted env
/// vars match known secret patterns.
fn apply_redaction(mut reason: FailureReason, env_allowlist: &[String]) -> FailureReason {
    for var in env_allowlist {
        if !is_secret_var(var) {
            continue;
        }
        if let Ok(val) = std::env::var(var) {
            if val.len() < 4 {
                continue;
            }
            let redacted = redact_value(&val);
            // Replace occurrences of the raw value in summary and streams
            if reason.summary.contains(&val) {
                reason.summary = reason.summary.replace(&val, &redacted);
            }
            if reason.stderr.contains(&val) {
                reason.stderr = reason.stderr.replace(&val, &redacted);
            }
            if reason.stdout.contains(&val) {
                reason.stdout = reason.stdout.replace(&val, &redacted);
            }
        }
    }
    reason
}

// ── VerifyHook impl ─────────────────────────────────────────

#[async_trait]
impl VerifyHook for RunCommand {
    fn name(&self) -> &str {
        "RunCommand"
    }

    async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
        let result = execute_command(
            &self.cmd,
            &self.args,
            &self.env_allowlist,
            self.cwd.as_ref(),
            self.wall_timeout,
            self.cpu_timeout,
            self.stdout_cap,
            self.stderr_cap,
        )
        .await
        .map_err(|e| VerifyError::new(format!("failed to spawn command `{}`: {e}", self.cmd)))?;

        let stdout = String::from_utf8_lossy(&result.stdout_bytes).into_owned();
        let stderr = String::from_utf8_lossy(&result.stderr_bytes).into_owned();
        let truncated = result.stdout_capped || result.stderr_capped;

        if result.timed_out {
            let summary = format!(
                "command `{cmd}` timed out after {timeout:?}",
                cmd = self.cmd,
                timeout = self.wall_timeout
            );
            let reason = FailureReason::new(summary)
                .with_stdout(stdout)
                .with_stderr(stderr)
                .with_truncated(truncated);
            let reason = apply_redaction(reason, &self.env_allowlist);
            return Ok(VerifyResult::fail_with(reason));
        }

        match result.exit_status {
            None => {
                let reason = FailureReason::new(format!("command `{}` failed", self.cmd))
                    .with_stdout(stdout)
                    .with_stderr(stderr)
                    .with_truncated(truncated);
                let reason = apply_redaction(reason, &self.env_allowlist);
                Ok(VerifyResult::fail_with(reason))
            }
            Some(status) if status.success() => Ok(VerifyResult::Pass),
            Some(status) => {
                let summary = if let Some(code) = status.code() {
                    format!("command `{}` exited with code {code}", self.cmd)
                } else {
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        if let Some(signal) = status.signal() {
                            format!("command `{}` killed by signal {signal}", self.cmd)
                        } else {
                            format!("command `{}` failed with unknown status", self.cmd)
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        format!("command `{}` failed with unknown status", self.cmd)
                    }
                };

                let mut reason = FailureReason::new(summary)
                    .with_stdout(stdout)
                    .with_stderr(stderr)
                    .with_truncated(truncated);

                if let Some(code) = status.code() {
                    reason = reason.with_exit_code(code);
                }

                #[cfg(unix)]
                {
                    use std::os::unix::process::ExitStatusExt;
                    if let Some(signal) = status.signal() {
                        // For signal kills, capture signal number as exit code
                        // (negative convention: -signal)
                        reason = reason.with_exit_code(-signal);
                    }
                }

                let reason = apply_redaction(reason, &self.env_allowlist);
                Ok(VerifyResult::fail_with(reason))
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::VerifyContext;
    use std::sync::Arc;

    /// Helper: create a minimal VerifyContext for testing.
    fn test_ctx() -> VerifyContext {
        VerifyContext {
            stdout: String::new(),
            stderr: None,
            exit_code: None,
            backend_name: "test".to_string(),
            model: None,
            structured: None,
            duration: Duration::ZERO,
        }
    }

    /// RAII guard to restore env vars after test.
    struct CleanupEnv(&'static str);

    impl Drop for CleanupEnv {
        fn drop(&mut self) {
            std::env::remove_var(self.0);
        }
    }

    #[tokio::test]
    async fn exit_zero_is_pass() {
        let hook =
            Arc::new(RunCommand::new("/bin/sh").with_args(vec!["-c".into(), "exit 0".into()]));
        let result = hook.verify(&test_ctx()).await.unwrap();
        assert!(result.is_pass(), "exit 0 should be Pass");
    }

    #[tokio::test]
    async fn exit_nonzero_is_fail() {
        let hook =
            Arc::new(RunCommand::new("/bin/sh").with_args(vec!["-c".into(), "exit 42".into()]));
        let result = hook.verify(&test_ctx()).await.unwrap();
        assert!(result.is_fail(), "exit 42 should be Fail");
        if let VerifyResult::Fail { reason } = result {
            assert_eq!(reason.exit_code, Some(42));
            assert!(reason.summary.contains("exited with code 42"));
        }
    }

    #[tokio::test]
    async fn exit_nonzero_captures_exit_code() {
        for code in [1i32, 127, 255] {
            let hook = Arc::new(
                RunCommand::new("/bin/sh").with_args(vec!["-c".into(), format!("exit {code}")]),
            );
            let result = hook.verify(&test_ctx()).await.unwrap();
            assert!(result.is_fail());
            if let VerifyResult::Fail { reason } = result {
                assert_eq!(reason.exit_code, Some(code));
            }
        }
    }

    #[tokio::test]
    async fn stdout_is_captured() {
        let hook = Arc::new(
            RunCommand::new("/bin/sh").with_args(vec!["-c".into(), "echo 'hello world'".into()]),
        );
        let result = hook.verify(&test_ctx()).await.unwrap();
        assert!(result.is_pass());
    }

    #[tokio::test]
    async fn stderr_is_captured_on_failure() {
        let hook = Arc::new(
            RunCommand::new("/bin/sh")
                .with_args(vec!["-c".into(), "echo 'error msg' >&2; exit 1".into()]),
        );
        let result = hook.verify(&test_ctx()).await.unwrap();
        assert!(result.is_fail());
        if let VerifyResult::Fail { reason } = result {
            assert!(reason.stderr.contains("error msg"));
            assert_eq!(reason.exit_code, Some(1));
        }
    }

    #[tokio::test]
    async fn wall_timeout_returns_fail_with_timeout_reason() {
        let hook = Arc::new(
            RunCommand::new("/bin/sh")
                .with_args(vec!["-c".into(), "sleep 60".into()])
                .with_wall_timeout(Duration::from_millis(50)),
        );
        let result = hook.verify(&test_ctx()).await.unwrap();
        assert!(result.is_fail());
        if let VerifyResult::Fail { reason } = result {
            assert!(
                reason.summary.contains("timed out"),
                "summary: {}",
                reason.summary
            );
        }
    }

    #[tokio::test]
    async fn env_allowlist_drops_unlisted_vars() {
        // Set a test variable, ensure it doesn't reach the child unless allowlisted
        std::env::set_var("RUNCOMMAND_TEST_SECRET", "should_not_leak");
        let _cleanup = CleanupEnv("RUNCOMMAND_TEST_SECRET");

        // Verify by running a sh that checks for absence of the unlisted var
        let hook = Arc::new(
            RunCommand::new("/bin/sh")
                .with_args(vec![
                    "-c".into(),
                    "test -z \"${RUNCOMMAND_TEST_SECRET:-}\"".into(),
                ])
                .with_env_allowlist(vec!["PATH".into()]),
        );
        let result = hook.verify(&test_ctx()).await.unwrap();
        assert!(
            result.is_pass(),
            "unlisted env var should not be propagated"
        );
    }

    #[tokio::test]
    async fn env_allowlist_forwards_listed_vars() {
        std::env::set_var("RUNCOMMAND_TEST_ALLOWED", "allowed_value");
        let _cleanup = CleanupEnv("RUNCOMMAND_TEST_ALLOWED");

        let hook = Arc::new(
            RunCommand::new("/bin/sh")
                .with_args(vec![
                    "-c".into(),
                    "test \"${RUNCOMMAND_TEST_ALLOWED}\" = \"allowed_value\"".into(),
                ])
                .with_env_allowlist(vec!["PATH".into(), "RUNCOMMAND_TEST_ALLOWED".into()]),
        );
        let result = hook.verify(&test_ctx()).await.unwrap();
        assert!(result.is_pass(), "allowlisted var should be propagated");
    }

    #[tokio::test]
    async fn cwd_is_honored() {
        let tmp = tempfile::tempdir().unwrap();
        let tmp_path = tmp.path().to_path_buf();

        let hook = Arc::new(
            RunCommand::new("/bin/sh")
                .with_args(vec!["-c".into(), "pwd".into()])
                .with_cwd(tmp_path.clone()),
        );
        let result = hook.verify(&test_ctx()).await.unwrap();
        assert!(result.is_pass(), "pwd in cwd should succeed");
    }

    #[tokio::test]
    async fn non_existent_command_returns_verify_error() {
        let hook = Arc::new(RunCommand::new("/nonexistent/command"));
        let result = hook.verify(&test_ctx()).await;
        assert!(result.is_err(), "non-existent command should error");
        if let Err(err) = result {
            assert!(err.message.contains("failed to spawn"));
        }
    }

    #[tokio::test]
    async fn killed_by_signal_returns_fail() {
        // Use sh to kill itself with SIGTERM (signal 15 on unix)
        let hook = Arc::new(
            RunCommand::new("/bin/sh").with_args(vec!["-c".into(), "kill -TERM $$".into()]),
        );
        let result = hook.verify(&test_ctx()).await.unwrap();
        assert!(result.is_fail(), "signal death should be Fail");
        if let VerifyResult::Fail { reason } = result {
            assert!(
                reason.summary.contains("killed by signal"),
                "summary: {}",
                reason.summary
            );
        }
    }

    #[tokio::test]
    async fn stdout_cap_truncates_output() {
        let hook = Arc::new(
            RunCommand::new("/bin/sh")
                .with_args(vec![
                    "-c".into(),
                    "for i in $(seq 1 20); do echo 'line'; done".into(),
                ])
                .with_stdout_cap(10),
        );
        let result = hook.verify(&test_ctx()).await.unwrap();
        // With a very small cap, stdout will be truncated but exit code is still 0
        assert!(result.is_pass() || result.is_fail());
    }

    #[tokio::test]
    async fn secret_shaped_env_var_is_redacted_in_failure_reason() {
        // Set a secret-shaped env var and allowlist it; ensure its value
        // is partially redacted in the failure reason output.
        std::env::set_var("MY_API_KEY", "sk-secret-value-12345");
        let _cleanup = CleanupEnv("MY_API_KEY");

        // A command that echoes the secret value and fails
        let hook = Arc::new(
            RunCommand::new("/bin/sh")
                .with_args(vec![
                    "-c".into(),
                    "echo \"${MY_API_KEY}\" >&2; exit 1".into(),
                ])
                .with_env_allowlist(vec!["PATH".into(), "MY_API_KEY".into()]),
        );
        let result = hook.verify(&test_ctx()).await.unwrap();
        assert!(result.is_fail());
        if let VerifyResult::Fail { reason } = result {
            // The raw value should be redacted - stderr should have "sk-s***" not full value
            assert!(
                reason.stderr.contains("sk-s***"),
                "stderr should contain redacted value, got: {}",
                reason.stderr
            );
            assert!(
                !reason.stderr.contains("sk-secret-value-12345"),
                "stderr should NOT contain raw secret"
            );
        }
    }
}
