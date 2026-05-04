//! Integration tests for `loker run` CLI subcommand flags (CLO-309).
//!
//! Tests:
//! - `--spec <path>`: injects a spec file into `{{ spec }}`
//! - `--var key=value`: injects template variables into `{{ var.<name> }}`
//! - `--rerun phase=<name>`: forces re-execution of a completed phase
//! - Flag combinations
//! - Exit codes: 0 on success, non-zero on failure

#![cfg(test)]

use std::path::Path;
use std::process::Command;

/// Helper: run `loker run` with the given args and return (success, combined_output).
fn run_loker_run(args: &[&str]) -> (bool, String) {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let test_spec_path = manifest_dir.join("tests/test_specs/test_spec.md");

    let mut cmd_args = vec!["run", "--quiet", "--bin", "loker", "--", "run"];
    for arg in args {
        // Resolve relative paths for --spec
        if *arg == "TEST_SPEC_PATH" {
            cmd_args.push(test_spec_path.to_str().unwrap());
        } else {
            cmd_args.push(arg);
        }
    }

    let output = Command::new("cargo")
        .args(&cmd_args)
        .current_dir(manifest_dir)
        .output()
        .expect("Failed to execute loker run");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = format!("{}{}", stdout, stderr);

    (output.status.success(), combined)
}

// ---------------------------------------------------------------------------
// Test 1: --spec flag injects spec file content
// ---------------------------------------------------------------------------

#[test]
fn test_run_spec_flag() {
    let (success, output) = run_loker_run(&[
        "tests/workflows/test_run_spec.toml",
        "--spec",
        "TEST_SPEC_PATH",
    ]);

    assert!(success, "Workflow with --spec failed: {}", output);
    assert!(
        output.contains("spec: # Test Spec"),
        "Expected spec content in output, got: {}",
        output
    );
}

// ---------------------------------------------------------------------------
// Test 2: --var flag injects template variables
// ---------------------------------------------------------------------------

#[test]
fn test_run_var_flag() {
    let (success, output) = run_loker_run(&[
        "tests/workflows/test_run_vars.toml",
        "--var",
        "foo=hello",
        "--var",
        "bar=world",
    ]);

    assert!(success, "Workflow with --var failed: {}", output);
    assert!(
        output.contains("vars: hello / world"),
        "Expected var interpolation in output, got: {}",
        output
    );
}

// ---------------------------------------------------------------------------
// Test 3: --spec + --var combined
// ---------------------------------------------------------------------------

#[test]
fn test_run_spec_and_var_combined() {
    let (success, output) = run_loker_run(&[
        "tests/workflows/test_run_spec_var.toml",
        "--spec",
        "TEST_SPEC_PATH",
        "--var",
        "foo=hello",
        "--var",
        "bar=world",
    ]);

    assert!(success, "Workflow with --spec and --var failed: {}", output);
    assert!(
        output.contains("spec:"),
        "Expected spec content in output, got: {}",
        output
    );
}

// ---------------------------------------------------------------------------
// Test 4: --rerun flag forces re-execution
// ---------------------------------------------------------------------------

#[test]
fn test_run_rerun_flag() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workflow_path = manifest_dir.join("tests/workflows/test_run_rerun.toml");
    let workflow_path_str = workflow_path.to_str().unwrap();

    // First run: execute all phases
    let (success1, output1) = run_loker_run(&[workflow_path_str]);
    assert!(success1, "First run failed: {}", output1);
    assert!(
        output1.contains("PHASE_ONE_RAN"),
        "Missing phase_one: {}",
        output1
    );
    assert!(
        output1.contains("PHASE_TWO_RAN"),
        "Missing phase_two: {}",
        output1
    );
    assert!(
        output1.contains("PHASE_THREE_RAN"),
        "Missing phase_three: {}",
        output1
    );

    // Second run with --rerun phase=phase_two: should re-execute phase_two and phase_three
    let (success2, output2) = run_loker_run(&[workflow_path_str, "--rerun", "phase=phase_two"]);

    assert!(success2, "Rerun workflow failed: {}", output2);
    // Phase_two and phase_three should run again
    assert!(
        output2.contains("PHASE_TWO_RAN"),
        "Missing phase_two in rerun: {}",
        output2
    );
    assert!(
        output2.contains("PHASE_THREE_RAN"),
        "Missing phase_three in rerun: {}",
        output2
    );
}

// ---------------------------------------------------------------------------
// Test 5: Invalid --var format is rejected
// ---------------------------------------------------------------------------

#[test]
fn test_run_invalid_var_format_rejected() {
    // --var without = should fail
    let (success, output) = run_loker_run(&[
        "tests/workflows/test_run_spec.toml",
        "--var",
        "invalidformat",
    ]);

    assert!(!success, "Expected failure for invalid --var format");
    assert!(
        output.contains("expected key=value"),
        "Expected error message about key=value format, got: {}",
        output
    );
}

// ---------------------------------------------------------------------------
// Test 6: Invalid --rerun format is rejected
// ---------------------------------------------------------------------------

#[test]
fn test_run_invalid_rerun_format_rejected() {
    // --rerun without phase= prefix should fail
    let (success, output) = run_loker_run(&[
        "tests/workflows/test_run_rerun.toml",
        "--rerun",
        "badformat",
    ]);

    assert!(!success, "Expected failure for invalid --rerun format");
    assert!(
        output.contains("expected phase="),
        "Expected error message about phase= format, got: {}",
        output
    );
}

// ---------------------------------------------------------------------------
// Test 7: Empty --var key is rejected
// ---------------------------------------------------------------------------

#[test]
fn test_run_empty_var_key_rejected() {
    let (success, output) =
        run_loker_run(&["tests/workflows/test_run_spec.toml", "--var", "=value"]);

    assert!(!success, "Expected failure for empty --var key");
    assert!(
        output.contains("Empty key"),
        "Expected 'Empty key' error message, got: {}",
        output
    );
}

// ---------------------------------------------------------------------------
// Test 8: Empty --rerun phase name is rejected
// ---------------------------------------------------------------------------

#[test]
fn test_run_empty_rerun_phase_rejected() {
    let (success, output) =
        run_loker_run(&["tests/workflows/test_run_rerun.toml", "--rerun", "phase="]);

    assert!(!success, "Expected failure for empty --rerun phase name");
    assert!(
        output.contains("Empty phase name"),
        "Expected 'Empty phase name' error message, got: {}",
        output
    );
}
