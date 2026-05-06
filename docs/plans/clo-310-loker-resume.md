# Plan: CLO-310 — `loker resume <run_id>` CLI subcommand

## Context

- **Design:** docs/designs/clo-310-loker-resume.md
- **Discovery:** docs/discovery/clo-310.md
- **Linear:** https://linear.app/cloud-ai/issue/CLO-310/t-041-loker-resume-run-id-cli-subcommand
- **Branch:** `feat/clo-310-resume`

## Sub-tasks

### ST1 Unhide resume subcommand & rename arg

**Files:** `src/main.rs`
**Acceptance:** `cargo build` compiles; `loker --help` shows `resume`; `loker resume --help` shows new help text.

**Changes:**
1. Remove `#[command(hide = true)]` from the `Commands::Resume` variant.
2. Rename field `run_dir: PathBuf` → `run_id: String`.
3. Replace stale help text `"Resume a partially-completed run (planner scaffolding; execution not yet wired)"` with the proper help text from the design doc — describes both `<run_id>` formats and distinguishes from `loker run --rerun`.
4. In the handler (line 925): destructure `run_id` instead of `run_dir`; pass through to `resolve_run_dir`.

**Estimate:** S

### ST2 Add `resolve_run_dir` and `find_project_root` helpers

**Files:** `src/main.rs`
**Acceptance:** `cargo build` compiles; `cargo test test_resolve_run_dir_*` passes (unit tests in `src/main.rs`).

**Changes:**
1. Add `fn find_project_root() -> Option<PathBuf>` — walks ancestors from CWD looking for `lok.toml`.
2. Add `fn resolve_run_dir(run_id: &str) -> anyhow::Result<PathBuf>` — three branches:
   - Absolute path → validate exists, return.
   - Relative path (component count > 1) → validate exists, return.
   - Bare name → resolve via `find_project_root() / runs / <run_id>`, validate exists.
3. Wire into the resume handler: `let run_dir = resolve_run_dir(&run_id)?;`
4. Add `#[cfg(test)]` unit tests for `resolve_run_dir` inline in `src/main.rs`: not-found (bare name), not-found (absolute path), found (bare name with tempdir).

**Estimate:** S

### ST3 Add guard clauses for fully-complete and no-resumable-state

**Files:** `src/main.rs`
**Acceptance:** `cargo build` compiles; guard clauses fire correctly in integration tests (ST4).

**Changes:**
1. After `RunState::load` and the heartbeat check (line 936 region), add:
   - **All-complete guard:** if all phases are `PhaseStatus::Completed` → `println!("All phases already complete. Nothing to resume."); return Ok(());` — with a comment noting the SHA-verification dependency on `RunState::load` (per F3).
   - **No-state guard:** if all phases are `PhaseStatus::None` → bail with `"No resumable state found in {}."`.
2. Ensure `PhaseStatus` is imported (already accessible via `crate::run_state::PhaseStatus`).

**Estimate:** S

### ST4 Integration tests for error paths and guard clauses

**Files:** `tests/resume.rs`
**Acceptance:** `cargo test --test resume -- test_resume_run_not_found test_resume_fully_complete_exit_zero test_resume_no_resumable_state` passes. `make check` green.

**Changes:**
1. `test_resume_run_not_found` — invokes `cargo run -- resume nonexistent-run` via `std::process::Command`; asserts exit code 1 and stderr contains "not found".
2. `test_resume_fully_complete_exit_zero` — creates a tempdir run dir with all phases `Completed` (reusing helpers from `tests/resume.rs`), invokes `cargo run -- resume <tmpdir>`; asserts exit 0 and stdout contains "All phases already complete.".
3. `test_resume_no_resumable_state` — creates a tempdir run dir with all phases `None`, invokes `cargo run -- resume <tmpdir>`; asserts exit code 1 and stderr contains "No resumable state found".

**Estimate:** M

### ST5 Round-trip pause/resume integration test (opt-in)

**Files:** `Cargo.toml`, `tests/resume.rs`
**Acceptance:** `LOKER_RESUME_INTEGRATION=1 cargo test --test resume test_resume_round_trip_kill_mid_phase` passes (~30 s). `make check` green (test is skipped by default).

**Changes:**
1. `Cargo.toml`: add `nix = "0.29"` to `[dev-dependencies]` for `SIGTERM` delivery.
2. Add `test_resume_round_trip_kill_mid_phase()` to `tests/resume.rs`:
   - Write a two-phase workflow TOML to a tempdir (phase 1: writes sentinel file + sleeps 30 s; phase 2: writes sentinel file).
   - Spawn `cargo run -- run <workflow>` as a child.
   - Poll for sentinel file (run is mid-phase-1 sleep).
   - Send `SIGTERM` via `nix::sys::signal::kill`.
   - Assert markers/phase-1/started exists, markers/phase-2/ does not.
   - Invoke `cargo run -- resume <run_dir>`.
   - Assert exit 0, stdout contains "Resume complete."
   - Assert both completed markers exist; phase-1 sentinel mtime unchanged.
3. Gate behind `LOKER_RESUME_INTEGRATION=1` matching the existing `LOKER_TZ_INTEGRATION=1` convention in `tests/tensorzero_integration.rs`.

**Estimate:** M

## Pre-merge gate

- `make check` (fmt + clippy + test)
- `make check` runs all non-gated tests; round-trip test is opt-in via env var

## Risks

- **`nix` dev-dependency** (ST5): Adds a crate for SIGTERM delivery. `libc` is already a unix-only dep; `nix` builds on it but adds compile time. Could alternatively use `libc::kill()` directly to avoid the dep. Tradeoff documented in design Q1 — defer decision to implementation.
- **Test `cargo run --` overhead** (ST4): Each integration test compiles + spawns the binary. At ~30 s compile time this makes tests slow. Mitigation: these tests are gated (ST4 tests compile once per `cargo test` run; ST5 is opt-in).
- **`resolve_run_dir` not accessible from integration tests**: The helper is private in `main.rs`. ST2 includes inline unit tests for the function directly; ST4 tests the binary end-to-end for guard clause scenarios.
