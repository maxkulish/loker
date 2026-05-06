# Plan: CLO-313 [T-044] `loker ls --blocked` enumerating HITL-pending runs (Should)

## Context
- Design: `docs/designs/clo-313-ls-blocked.md`
- Discovery: `docs/discovery/clo-313.md`
- PRD supplement: `docs/prds/clo-313-ls-blocked.md`
- Linear: https://linear.app/cloud-ai/issue/CLO-313/t-044-loker-ls-blocked-enumerating-hitl-pending-runs-should

## Sub-tasks

### ST1 Implement blocked-run scanner model
**Files:** `src/commands/ls_blocked.rs`, `src/commands/mod.rs`

Create the `ls_blocked` command module with:
- `BlockedEntry` carrying run id, phase, severity, opened/timeout timestamps, pending path, response path, and project-relative response display path.
- `scan_blocked(root: &Path) -> anyhow::Result<Vec<BlockedEntry>>` scanning `root/runs/*/pending/*.json`.
- Filtering that excludes entries with matching `responses/<phase>.json`.
- Stable ordering by `opened_at`, then `(run_id, phase)`.
- Malformed pending JSON warnings to stderr without aborting the scan.

**Acceptance:** `cargo test commands::ls_blocked::tests::scan_blocked_lists_unmatched_pending commands::ls_blocked::tests::scan_blocked_skips_resolved_phase commands::ls_blocked::tests::scan_blocked_sorts_oldest_first`

**Estimate:** M

### ST2 Implement rendering and age formatting
**Files:** `src/commands/ls_blocked.rs`

Add deterministic presentation helpers:
- `render_table(entries, now, writer)` writes `no blocked runs\n` for empty input.
- Non-empty output includes run id, phase, severity, compact age, and `runs/<run_id>/responses/<phase>.json`.
- Age formatting uses compact units (`s`, `m`, `h`, `d`) with injected `now` for tests.

**Acceptance:** `cargo test commands::ls_blocked::tests::render_table_empty_writes_no_blocked_runs commands::ls_blocked::tests::render_table_renders_age_severity_paths commands::ls_blocked::tests::render_table_age_units`

**Estimate:** S

### ST3 Wire the CLI surface
**Files:** `src/main.rs`, `src/commands/mod.rs`

Add `loker ls --blocked` to clap:
- Introduce `Commands::Ls { blocked: bool }`.
- Error on bare `loker ls` with a clear message because no other listing modes exist in v1.
- Resolve project root via existing `find_project_root().or_else(current_dir)` pattern.
- Dispatch `loker::commands::ls_blocked::run(&root)?`.

**Acceptance:** `cargo test --test ls_blocked_cli`

**Estimate:** S

### ST4 Add integration snapshot fixture
**Files:** `tests/ls_blocked.rs`, `tests/snapshots/ls_blocked__snapshot_mixed_blocked_and_completed.snap`

Create a temp project fixture with mixed state:
- One fully resolved run.
- One blocked run.
- One multi-phase run with one resolved phase and one blocked phase.

Assert the rendered snapshot is oldest-first and includes only unresolved pending phases. Also cover the CLI empty case or bare `ls` error in the same integration target if practical.

**Acceptance:** `cargo test --test ls_blocked`

**Estimate:** M

### ST5 Run pre-merge gate and tidy docs
**Files:** `docs/plans/clo-313-ls-blocked.md`, any files touched by implementation

Run the full repo gate and fix any formatting, clippy, or test failures. If implementation discovers a necessary design deviation, update `docs/designs/clo-313-ls-blocked.md` before PR.

**Acceptance:** `make check`

**Estimate:** S

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Risks
- `PendingRequest` currently stores timestamps as strings; implementation must parse RFC3339 carefully and skip malformed entries rather than panicking.
- CLI integration test names may need to align with existing test conventions if clap/binary invocation helpers differ from the initial estimate.
- Snapshot output should use project-relative response paths to avoid tempdir-specific churn.
- Bare `loker ls` behavior must remain intentionally narrow so future listing modes are additive.
