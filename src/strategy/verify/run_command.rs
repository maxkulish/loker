#![allow(unsafe_code)]

//! Shell-command verify hook with sandboxing.
//!
//! Spawns a subprocess, captures stdout/stderr with byte caps, and maps
//! exit status to [`VerifyResult`].

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::time::timeout;
use which::{which, which_in};

use crate::strategy::verify::{
    FailureReason, SandboxViolation, VerifyContext, VerifyError, VerifyHook, VerifyResult,
};
use crate::utils::redact_secrets;

/// Default wall-clock timeout for compile/test gates.
const DEFAULT_WALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Default byte cap for stdout and stderr capture.
const DEFAULT_OUTPUT_CAP: usize = 4096;

/// Shell-command verify hook.
#[derive(Debug, Clone)]
pub struct RunCommand {
    /// Command to execute (bare name resolved via `which`, absolute path used as-is).
    pub cmd: String,
    /// Arguments passed to the command.
    pub args: Vec<String>,
    /// Environment variable names allowed in child process.
    /// Default: empty (default-deny).
    pub env_allowlist: Vec<String>,
    /// Working directory for the command.
    pub cwd: Option<PathBuf>,
    /// Wall-clock timeout before process-group SIGKILL.
    pub wall_timeout: Duration,
    /// Optional CPU-time timeout (Unix only).
    pub cpu_timeout: Option<Duration>,
    /// Max bytes captured from stdout.
    pub stdout_cap: usize,
    /// Max bytes captured from stderr.
    pub stderr_cap: usize,
}

impl Default for RunCommand {
    fn default() -> Self {
        Self {
            cmd: String::new(),
            args: Vec::new(),
            env_allowlist: Vec::new(),
            cwd: None,
            wall_timeout: DEFAULT_WALL_TIMEOUT,
            cpu_timeout: None,
            stdout_cap: DEFAULT_OUTPUT_CAP,
            stderr_cap: DEFAULT_OUTPUT_CAP,
        }
    }
}

impl RunCommand {
    /// Construct with a command name/path.
    pub fn new(cmd: impl Into<String>) -> Self {
        Self {
            cmd: cmd.into(),
            ..Self::default()
        }
    }

    /// Replace command-line arguments.
    pub fn with_args(mut self, args: impl Into<Vec<String>>) -> Self {
        self.args = args.into();
        self
    }

    /// Replace env allow-list.
    pub fn with_env_allowlist(mut self, vars: &[&str]) -> Self {
        self.env_allowlist = vars.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Set working directory.
    pub fn with_cwd(mut self, path: impl Into<PathBuf>) -> Self {
        self.cwd = Some(path.into());
        self
    }

    /// Override wall-clock timeout.
    pub fn with_wall_timeout(mut self, timeout: Duration) -> Self {
        self.wall_timeout = timeout;
        self
    }

    /// Override CPU timeout.
    pub fn with_cpu_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.cpu_timeout = timeout;
        self
    }

    /// Override stdout capture cap.
    pub fn with_stdout_cap(mut self, cap: usize) -> Self {
        self.stdout_cap = cap;
        self
    }

    /// Override stderr capture cap.
    pub fn with_stderr_cap(mut self, cap: usize) -> Self {
        self.stderr_cap = cap;
        self
    }

    fn resolve_command(&self) -> Result<PathBuf, VerifyError> {
        if self.cmd.is_empty() {
            return Err(VerifyError::new("no command configured"));
        }

        let path = Path::new(&self.cmd);
        if path.is_absolute() {
            Ok(PathBuf::from(path))
        } else if let Some(cwd) = &self.cwd {
            // Resolve relative to the configured working directory, not the
            // orchestrator's CWD. This matters when cmd is a relative path
            // like `./scripts/verify.sh` — it should resolve against cwd.
            which_in(&self.cmd, std::env::var_os("PATH"), cwd.as_os_str())
                .map_err(|_| VerifyError::new(format!("command not found: {}", self.cmd)))
        } else {
            which(&self.cmd)
                .map_err(|_| VerifyError::new(format!("command not found: {}", self.cmd)))
        }
    }

    fn build_environment(&self) -> Vec<(String, String)> {
        self.env_allowlist
            .iter()
            .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
            .collect()
    }

    pub async fn run(&self) -> Result<CommandRun, VerifyError> {
        let command_path = self.resolve_command()?;

        let mut command = Command::new(&command_path);
        command
            .args(&self.args)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }

        let env_vars = self.build_environment();
        let secret_values: Vec<String> = env_vars
            .iter()
            .filter(|(name, _)| is_secret_like_env_key(name))
            .map(|(_, value)| value.clone())
            .collect();
        for (key, value) in env_vars {
            command.env(key, value);
        }

        // Put each child in its own process group, so SIGKILL on timeout
        // reaps descendants as well.
        #[cfg(unix)]
        command.process_group(0);
        command.kill_on_drop(true);

        #[cfg(unix)]
        if let Some(cpu_timeout) = self.cpu_timeout {
            let secs = cpu_timeout.as_secs().max(1);
            // SAFETY: this closure runs in the child before exec and only
            // mutates process-local resource limits.
            unsafe {
                command.pre_exec(move || {
                    let limit = libc::rlimit {
                        rlim_cur: secs as libc::rlim_t,
                        rlim_max: secs as libc::rlim_t,
                    };
                    let rc = libc::setrlimit(libc::RLIMIT_CPU, &limit);
                    if rc != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }

        let start = Instant::now();
        let mut child = command.spawn().map_err(|err| {
            VerifyError::new(format!("failed to spawn command '{}': {err}", self.cmd))
        })?;

        let stdout_stream = child
            .stdout
            .take()
            .ok_or_else(|| VerifyError::new("failed to capture child stdout pipe".to_string()))?;
        let stderr_stream = child
            .stderr
            .take()
            .ok_or_else(|| VerifyError::new("failed to capture child stderr pipe".to_string()))?;

        let stdout_handle = tokio::spawn(read_stream_bounded(stdout_stream, self.stdout_cap));
        let stderr_handle = tokio::spawn(read_stream_bounded(stderr_stream, self.stderr_cap));

        let (status, timed_out) = match timeout(self.wall_timeout, child.wait()).await {
            Ok(result) => {
                let status = result.map_err(|err| {
                    VerifyError::new(format!("failed to wait for command '{}': {err}", self.cmd))
                })?;
                (status, false)
            }
            Err(_) => {
                kill_process_group(child.id());
                let _ = child.kill().await;
                let status = child.wait().await.map_err(|err| {
                    VerifyError::new(format!(
                        "failed to reap timed-out command '{}': {err}",
                        self.cmd
                    ))
                })?;
                (status, true)
            }
        };

        let stdout = stdout_handle
            .await
            .map_err(|_| VerifyError::new("stdout reader task failed"))?;
        let stderr = stderr_handle
            .await
            .map_err(|_| VerifyError::new("stderr reader task failed"))?;

        Ok(CommandRun {
            status,
            timed_out,
            stdout,
            stderr,
            secret_values,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    }
}

#[derive(Debug)]
pub struct CommandRun {
    pub status: std::process::ExitStatus,
    pub timed_out: bool,
    pub stdout: CapturedOutput,
    pub stderr: CapturedOutput,
    pub secret_values: Vec<String>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone)]
pub struct CapturedOutput {
    pub data: Vec<u8>,
    pub truncated: bool,
    pub elided_bytes: usize,
}

impl CapturedOutput {
    pub fn to_reason_text(&self) -> String {
        let mut text = String::from_utf8_lossy(&self.data).into_owned();
        if self.truncated {
            text.push_str(&format!(
                " …[truncated, {} bytes elided]",
                self.elided_bytes
            ));
        }
        text
    }
}

/// Redact known secret patterns and specific allowlisted secret values from text.
pub(crate) fn redact_output(text: &str, secret_values: &[String]) -> String {
    let mut result = redact_secrets(text);
    for secret in secret_values {
        if !secret.is_empty() && result.contains(secret.as_str()) {
            result = result.replace(secret.as_str(), "[REDACTED]");
        }
    }
    result
}

async fn read_stream_bounded<R: AsyncRead + Unpin + Send + 'static>(
    mut reader: R,
    max_bytes: usize,
) -> CapturedOutput {
    let mut buf = [0u8; 8192];
    let mut data = Vec::new();
    let mut truncated = false;
    let mut elided_bytes = 0usize;

    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let remaining = max_bytes.saturating_sub(data.len());
                if remaining == 0 {
                    truncated = true;
                    elided_bytes += n;
                    elided_bytes += drain_stream(&mut reader).await;
                    break;
                }

                let take = n.min(remaining);
                data.extend_from_slice(&buf[..take]);

                if take < n {
                    truncated = true;
                    elided_bytes += n - take;
                    elided_bytes += drain_stream(&mut reader).await;
                    break;
                }
            }
            Err(_) => break,
        }
    }

    CapturedOutput {
        data,
        truncated,
        elided_bytes,
    }
}

async fn drain_stream<R: AsyncRead + Unpin + Send>(reader: &mut R) -> usize {
    let mut buf = [0u8; 8192];
    let mut total = 0usize;

    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                total += n;
            }
        }
    }

    total
}

fn is_secret_like_env_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    key.contains("SECRET")
        || key.contains("TOKEN")
        || key.contains("PASSWORD")
        || key.contains("API_KEY")
        || key.contains("AUTH")
}

#[cfg(unix)]
fn status_signal(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn status_signal(_status: &std::process::ExitStatus) -> Option<i32> {
    None
}

#[cfg(unix)]
fn kill_process_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        // SAFETY: this sends SIGKILL to a process group spawned by us and avoids
        // orphaned children. If the pid is stale or already dead, this is a
        // best-effort cleanup attempt.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_process_group(pid: Option<u32>) {
    let _ = pid;
}

#[async_trait]
impl VerifyHook for RunCommand {
    fn name(&self) -> &str {
        "RunCommand"
    }

    async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
        let run = self.run().await?;

        let stdout = redact_output(&run.stdout.to_reason_text(), &run.secret_values);
        let stderr = redact_output(&run.stderr.to_reason_text(), &run.secret_values);
        let truncated = run.stdout.truncated || run.stderr.truncated;
        let signal = status_signal(&run.status);

        if run.timed_out {
            return Ok(VerifyResult::Fail {
                reason: FailureReason::new(format!("command timed out: {}", self.cmd))
                    .with_stdout(stdout)
                    .with_stderr(stderr)
                    .with_truncated(truncated)
                    .with_sandbox_violation(SandboxViolation::Timeout),
            });
        }

        if let Some(code) = run.status.code() {
            if code == 0 {
                return Ok(VerifyResult::Pass);
            }

            return Ok(VerifyResult::Fail {
                reason: FailureReason::new(format!("command exited with status {code}"))
                    .with_exit_code(code)
                    .with_stdout(stdout)
                    .with_stderr(stderr)
                    .with_truncated(truncated)
                    .with_sandbox_violation(SandboxViolation::NonZeroExit { code }),
            });
        }

        if let Some(sig) = signal {
            return Ok(VerifyResult::Fail {
                reason: FailureReason::new(format!("command killed by signal {sig}"))
                    .with_stdout(stdout)
                    .with_stderr(stderr)
                    .with_truncated(truncated)
                    .with_sandbox_violation(SandboxViolation::Signal { signal: sig }),
            });
        }

        Ok(VerifyResult::Fail {
            reason: FailureReason::new(format!("command terminated unexpectedly: {}", self.cmd))
                .with_stdout(stdout)
                .with_stderr(stderr)
                .with_truncated(truncated),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> VerifyContext {
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

    #[test]
    fn run_command_builder_api() {
        let rc = RunCommand::new("cargo")
            .with_args(vec!["test".to_string(), "--quiet".to_string()])
            .with_env_allowlist(&["PATH", "HOME"])
            .with_cwd("/tmp")
            .with_wall_timeout(Duration::from_secs(10))
            .with_cpu_timeout(Some(Duration::from_secs(5)))
            .with_stdout_cap(1024)
            .with_stderr_cap(2048);

        assert_eq!(rc.cmd, "cargo");
        assert_eq!(rc.args, vec!["test", "--quiet"]);
        assert_eq!(rc.env_allowlist, vec!["PATH", "HOME"]);
        assert_eq!(rc.cwd, Some(PathBuf::from("/tmp")));
        assert_eq!(rc.wall_timeout, Duration::from_secs(10));
        assert_eq!(rc.cpu_timeout, Some(Duration::from_secs(5)));
        assert_eq!(rc.stdout_cap, 1024);
        assert_eq!(rc.stderr_cap, 2048);
    }

    #[test]
    fn run_command_default_values() {
        let rc = RunCommand::new("echo");
        assert_eq!(rc.cmd, "echo");
        assert!(rc.args.is_empty());
        assert!(rc.env_allowlist.is_empty());
        assert!(rc.cwd.is_none());
        assert_eq!(rc.wall_timeout, DEFAULT_WALL_TIMEOUT);
        assert_eq!(rc.cpu_timeout, None);
        assert_eq!(rc.stdout_cap, DEFAULT_OUTPUT_CAP);
        assert_eq!(rc.stderr_cap, DEFAULT_OUTPUT_CAP);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn verify_echo_passes() {
        let rc = RunCommand::new("sh").with_args(vec!["-c".to_string(), "echo hello".to_string()]);
        let result = rc.verify(&context()).await.unwrap();
        assert!(matches!(result, VerifyResult::Pass));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn verify_false_fails_with_code() {
        let rc = RunCommand::new("sh")
            .with_args(vec!["-c".to_string(), "echo err >&2; exit 1".to_string()]);
        let result = rc.verify(&context()).await.unwrap();
        match result {
            VerifyResult::Fail { reason } => {
                assert_eq!(reason.exit_code, Some(1));
                assert!(reason.stderr.contains("err"));
                assert!(matches!(
                    reason.sandbox_violation,
                    Some(SandboxViolation::NonZeroExit { code: 1 })
                ));
            }
            other => panic!("expected fail, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn verify_missing_command_fails() {
        let rc = RunCommand::new("__no_such_command__");
        let err = rc
            .verify(&context())
            .await
            .expect_err("expected verify error");
        assert!(err.message.contains("command not found"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn verify_sleeps_timeout() {
        let rc = RunCommand::new("sleep")
            .with_args(vec!["60".to_string()])
            .with_wall_timeout(Duration::from_millis(100));
        let result = rc.verify(&context()).await.unwrap();
        match result {
            VerifyResult::Fail { reason } => {
                assert!(matches!(
                    reason.sandbox_violation,
                    Some(SandboxViolation::Timeout)
                ));
            }
            other => panic!("expected fail, got {other:?}"),
        }
    }
}
