# Pre-PR validation: clo-310

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

I've gathered enough context. `make check` is green (793/809 tests passing). Below are the findings.

## Findings

### F1 [HIGH] Round-trip test does not exercise pause/resume — substantial scope reduction from the approved design
**Where:** tests/resume.rs:343-446
**What:** The design's headline acceptance test (`test_resume_round_trip_kill_mid_phase`) was specified to spawn `loker run`, send SIGTERM mid phase 1, then verify `loker resume` finishes phase 2 without re-running phase 1 (assert "Resume complete." in stdout, both completed markers exist, phase-1 sentinel mtime unchanged). The shipped test does none of that: it pre-creates a fully-completed run dir and just re-asserts the all-complete guard (`output.status.success()` and "All phases already complete"), duplicating `test_resume_fully_complete_exit_zero`. The sentinel mtime assertion is meaningless because no phase ever ran. Plan ST5's stated acceptance ("`LOKER_RESUME_INTEGRATION=1 ... passes (~30 s)` ... Send SIGTERM ... Assert stdout contains 'Resume complete.'") is unmet. The PRD's "ship a pause/resume round-trip integration test" is unmet. This is the only test that exercises end-to-end execution; without it, the resume runner path is uncovered by CLI-level tests.
**Suggested fix:** Either (a) implement the test as designed (spawn binary, kill mid-phase, resume, assert "Resume complete." and that phase-2 markers exist while phase-1 was skipped) using `nix::sys::signal::kill` with SIGTERM, or (b) explicitly downgrade the design/plan via a follow-up ticket with sign-off and rename this test (it should not claim "round_trip" or "kill_mid_phase").

### F2 [MEDIUM] `nix` dev-dependency added but unused
**Where:** Cargo.toml:69, Cargo.lock entry for nix 0.29
**What:** Plan ST5 added `nix = { version = "0.29", features = ["signal", "process"] }` for SIGTERM delivery. No source file imports `nix::` (verified via grep — only doc files mention it). It compiles into the test artifact for nothing, costing build time on a downstream-heavy crate.
**Suggested fix:** Remove `nix` from Cargo.toml dev-dependencies (and re-lock) until F1 is resolved; or, when F1 is implemented, actually use it.

### F3 [MEDIUM] `test_find_project_root_no_lok_toml` mutates process-global CWD; races other tests
**Where:** src/main.rs:2528-2536
**What:** The test calls `std::env::set_current_dir(...)` twice. Cargo runs unit tests in parallel threads; CWD is per-process, so concurrent tests that read CWD (or resolve relative paths) can observe the temporary CWD or fail if they outlive the `set_current_dir(original)` restore. This pattern is a well-known source of flakes on macOS/Linux when CI parallelism is high. There is no `serial_test` guard or equivalent.
**Suggested fix:** Refactor `find_project_root` to accept an optional starting path (e.g. `find_project_root_from(start: &Path)`), drive it from the test directly, and keep the public no-arg wrapper that uses CWD. Alternatively, gate with `serial_test::serial`.

### F4 [LOW] `loker_binary()` hardcodes `target/debug/loker`; non-portable and not built-on-demand
**Where:** tests/resume.rs:248-253
**What:** Cargo provides `env!("CARGO_BIN_EXE_loker")` which (a) guarantees the binary is built before integration tests run, (b) selects debug/release correctly, (c) appends `.exe` on Windows. The hardcoded path silently breaks under `cargo test --release`, and on a fresh checkout where the binary may not be present yet the three new integration tests will fail with "no such file" rather than rebuilding. Not a regression of `make check` today, but fragile.
**Suggested fix:** Replace `loker_binary()` body with `std::path::PathBuf::from(env!("CARGO_BIN_EXE_loker"))`.

### F5 [LOW] Misleading test/comment text — "round_trip", "Sentiment", em-dashes
**Where:** tests/resume.rs:332-341, 440, 444
**What:** The block comment claims the run "validates the full data path through resolve_run_dir → RunLock → RunState::load → guard clauses" — that's accurate, but then the test name says `round_trip_kill_mid_phase`, the comment line 440 says "Sentiment unchanged" (typo of "Sentinel"), and several em-dashes appear in comments (project CLAUDE.md mandates regular dashes).
**Suggested fix:** If F1 is downgraded rather than implemented, rename the test to `test_resume_via_binary_all_complete_guard` and rewrite the surrounding comments. Fix the "Sentiment" typo. Replace em-dashes with regular dashes.

### F6 [LOW] Workflow status YAML still shows `implement: pending`
**Where:** docs/status/clo-310-workflow.yaml:43
**What:** The yaml says `implement: status: pending` while implementation is complete and committed. Probably will be updated by tooling on PR, but worth flagging — reviewers reading the status file get a misleading signal.
**Suggested fix:** Update `phases.implement.status` to `complete` (or let the workflow tool do it before PR).

## Verdict
rework

The CLI polish (unhide, rename `run_dir` → `run_id`, `resolve_run_dir`, guard clauses, error messages, help text) is correct, well-tested at the unit level, and matches the design exactly — `make check` is green. The blocker is F1: the design's headline integration test was the entire justification for shipping under M7/M8 Slice B (the PRD's "ship a pause/resume round-trip integration test"), and it was silently swapped for a duplicate of an existing guard-clause test. F2 (unused `nix` dep) is a direct downstream consequence. Either reinstate the round-trip test as designed or get explicit sign-off to descope it (and remove `nix`, rename the test). F3-F6 are minor and can be folded into the same rework pass.
