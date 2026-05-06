#![cfg(test)]

use std::process::Command;

#[test]
fn cli_ls_blocked_empty_project_prints_no_blocked_runs() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("lok.toml"), "").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_loker"))
        .args(["ls", "--blocked"])
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "no blocked runs\n"
    );
}

#[test]
fn cli_ls_without_blocked_errors() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("lok.toml"), "").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_loker"))
        .arg("ls")
        .current_dir(tmp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("requires --blocked"),
        "stderr was: {stderr}"
    );
}
