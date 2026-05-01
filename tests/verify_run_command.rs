#![cfg(unix)]

use std::env;
use std::process::Command;
use std::time::Duration;

use loker::backend::QueryOutput;
use loker::strategy::verify::{RunCommand, SandboxViolation};
use loker::strategy::VerifyContext;
use loker::strategy::{VerifyHook, VerifyResult};

fn ctx() -> VerifyContext {
    VerifyContext::from_query_output(&QueryOutput::from_text(
        String::new(),
        "run_command_integration".to_string(),
        Duration::ZERO,
    ))
}

fn has_process_containing(fragment: &str) -> bool {
    let output = Command::new("ps")
        .args(["-axo", "command="])
        .output()
        .expect("ps should run")
        .stdout;
    let ps = String::from_utf8_lossy(&output);
    ps.lines().any(|line| line.contains(fragment))
}

#[tokio::test]
async fn runcommand_echo_passes() {
    let rc = RunCommand::new("sh").with_args(vec!["-c".to_string(), "echo hello".to_string()]);

    let result = rc.verify(&ctx()).await.unwrap();
    assert!(matches!(result, VerifyResult::Pass));
}

#[tokio::test]
async fn runcommand_false_fails_with_exit_code() {
    let rc = RunCommand::new("sh")
        .with_args(vec!["-c".to_string(), "echo boom >&2; exit 1".to_string()]);

    let result = rc.verify(&ctx()).await.unwrap();
    match result {
        VerifyResult::Fail { reason } => {
            assert_eq!(reason.exit_code, Some(1));
            assert!(matches!(
                reason.sandbox_violation,
                Some(SandboxViolation::NonZeroExit { code: 1 })
            ));
        }
        other => panic!("expected fail, got {other:?}"),
    }
}

#[tokio::test]
async fn runcommand_sleeps_timeout() {
    let rc = RunCommand::new("sleep")
        .with_args(vec!["60".to_string()])
        .with_wall_timeout(Duration::from_millis(120));

    let result = rc.verify(&ctx()).await.unwrap();
    match result {
        VerifyResult::Fail { reason } => {
            assert!(matches!(
                reason.sandbox_violation,
                Some(SandboxViolation::Timeout)
            ));
        }
        other => panic!("expected timeout fail, got {other:?}"),
    }
}

#[tokio::test]
async fn runcommand_process_group_killed_on_timeout() {
    let marker = "sleep 777";

    let rc = RunCommand::new("sh")
        .with_args(vec!["-c".to_string(), "sleep 777".to_string()])
        .with_wall_timeout(Duration::from_millis(120));

    let result = rc.verify(&ctx()).await;
    assert!(matches!(result, Ok(VerifyResult::Fail { .. })));

    let has_orphan = has_process_containing(marker);
    assert!(
        !has_orphan,
        "orphaned timeout process still running: {marker}"
    );
}

#[tokio::test]
async fn runcommand_output_truncation_markers() {
    let rc = RunCommand::new("sh")
        .with_args(vec![
            "-c".to_string(),
            "printf 'abcdefgh'; printf 'ijklmnop' 1>&2; exit 1".to_string(),
        ])
        .with_stdout_cap(4)
        .with_stderr_cap(4);

    let result = rc.verify(&ctx()).await.unwrap();
    match result {
        VerifyResult::Fail { reason } => {
            assert!(reason.truncated);
            assert!(reason.stdout.contains("…[truncated"));
            assert!(reason.stderr.contains("…[truncated"));
        }
        other => panic!("expected fail, got {other:?}"),
    }
}

#[tokio::test]
async fn runcommand_env_allowlist() {
    let rc = RunCommand::new("sh")
        .with_args(vec!["-c".to_string(), "printenv; exit 1".to_string()])
        .with_env_allowlist(&["USER", "HOME"]);

    let result = rc.verify(&ctx()).await.unwrap();
    match result {
        VerifyResult::Fail { reason } => {
            assert!(reason.stdout.contains("USER="));
            assert!(reason.stdout.contains("HOME="));
            assert!(!reason.stdout.contains("PATH="));
        }
        other => panic!("expected fail, got {other:?}"),
    }
}

#[tokio::test]
async fn runcommand_secret_redaction_in_output() {
    let key = "CLO271_SECRET_TOKEN";
    let value = "plain-secret-value-that-should-be-redacted";
    let original = env::var_os(key);
    env::set_var(key, value);

    let rc = RunCommand::new("sh")
        .with_args(vec![
            "-c".to_string(),
            "echo ${CLO271_SECRET_TOKEN}; exit 1".to_string(),
        ])
        .with_env_allowlist(&[key]);

    let result = rc.verify(&ctx()).await.unwrap();
    match result {
        VerifyResult::Fail { reason } => {
            assert!(reason.stdout.contains("[REDACTED]"));
            assert!(!reason.stdout.contains(value));
        }
        other => panic!("expected fail, got {other:?}"),
    }

    match original {
        Some(v) => env::set_var(key, v),
        None => env::remove_var(key),
    }
}

#[tokio::test]
async fn runcommand_cpu_limit_forced_signal() {
    let rc = RunCommand::new("sh")
        .with_args(vec!["-c".to_string(), "while :; do :; done".to_string()])
        .with_cpu_timeout(Some(Duration::from_secs(1)))
        .with_wall_timeout(Duration::from_secs(4));

    let result = rc.verify(&ctx()).await.unwrap();
    match result {
        VerifyResult::Fail { reason } => {
            assert!(matches!(
                reason.sandbox_violation,
                Some(SandboxViolation::Signal { .. })
            ));
        }
        other => panic!("expected fail, got {other:?}"),
    }
}
