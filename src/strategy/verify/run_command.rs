//! Shell-command verify hook with sandboxing.
//!
//! Spawns a subprocess, captures stdout/stderr with byte caps, and maps
//! exit status to [`VerifyResult::Pass`] or [`VerifyResult::Fail`].
//!
//! ## Sandboxing
//!
//! - **Default-deny environment**: child process starts with an empty
//!   environment; only keys in [`RunCommand::env_allowlist`] are inherited.
//! - **Wall-clock timeout**: `tokio::time::timeout` + SIGKILL to process
//!   group on overrun.
//! - **CPU timeout** (Unix only): `RLIMIT_CPU` via `pre_exec`.
//! - **Secret redaction**: allowlisted env values with known-secret-shaped
//!   names are redacted before flowing into [`FailureReason`].
//!
//! ## Platform notes
//!
//! Process-group kill and CPU limiting are Unix-only. On Windows the
//! hook still functions but with weaker sandbox guarantees.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;

use crate::strategy::verify::{VerifyContext, VerifyError, VerifyHook, VerifyResult};

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
    /// Environment variable names allowed in the child process.
    /// Default: empty (default-deny).
    pub env_allowlist: Vec<String>,
    /// Working directory for the child process.
    pub cwd: Option<PathBuf>,
    /// Wall-clock timeout before the process is SIGKILLed.
    pub wall_timeout: Duration,
    /// Optional CPU-time limit (Unix only via `RLIMIT_CPU`).
    pub cpu_timeout: Option<Duration>,
    /// Maximum bytes to capture from stdout.
    pub stdout_cap: usize,
    /// Maximum bytes to capture from stderr.
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
    /// Create a new `RunCommand` with the given command.
    pub fn new(cmd: impl Into<String>) -> Self {
        Self {
            cmd: cmd.into(),
            ..Self::default()
        }
    }

    /// Set arguments.
    pub fn with_args(mut self, args: impl Into<Vec<String>>) -> Self {
        self.args = args.into();
        self
    }

    /// Set the environment allowlist.
    pub fn with_env_allowlist(mut self, vars: &[&str]) -> Self {
        self.env_allowlist = vars.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Set the working directory.
    pub fn with_cwd(mut self, path: impl Into<PathBuf>) -> Self {
        self.cwd = Some(path.into());
        self
    }

    /// Set the wall-clock timeout.
    pub fn with_wall_timeout(mut self, timeout: Duration) -> Self {
        self.wall_timeout = timeout;
        self
    }

    /// Set the optional CPU timeout (Unix only).
    pub fn with_cpu_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.cpu_timeout = timeout;
        self
    }

    /// Set the stdout byte cap.
    pub fn with_stdout_cap(mut self, cap: usize) -> Self {
        self.stdout_cap = cap;
        self
    }

    /// Set the stderr byte cap.
    pub fn with_stderr_cap(mut self, cap: usize) -> Self {
        self.stderr_cap = cap;
        self
    }
}

#[async_trait]
impl VerifyHook for RunCommand {
    fn name(&self) -> &str {
        "RunCommand"
    }

    async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
        todo!("CLO-271: implement shell-out execution")
    }
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(rc.cpu_timeout.is_none());
        assert_eq!(rc.stdout_cap, DEFAULT_OUTPUT_CAP);
        assert_eq!(rc.stderr_cap, DEFAULT_OUTPUT_CAP);
    }

    #[tokio::test]
    #[should_panic(expected = "CLO-271: implement shell-out execution")]
    async fn verify_is_todo() {
        let rc = RunCommand::new("echo");
        let ctx = VerifyContext {
            stdout: String::new(),
            stderr: None,
            exit_code: None,
            backend_name: "test".to_string(),
            model: None,
            structured: None,
            duration: Duration::ZERO,
        };
        let _ = rc.verify(&ctx).await;
    }
}
