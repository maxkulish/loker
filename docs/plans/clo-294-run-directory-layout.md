# Plan: CLO-294 — Run Directory Layout (`runs/<workflow>-<timestamp>-<short-uuid>/`)

## Context
- **Design:** docs/designs/clo-294-run-dir-layout.md
- **Design review:** docs/reviews/clo-294-design-synthesis.md (verdict: approve_with_changes)
- **Linear:** https://linear.app/cloud-ai/issue/CLO-294
- **Branch:** `feat/clo-294-dir`

## Pre-flight: `cargo build` must compile before starting

Run `cargo build` once before starting sub-tasks to confirm the baseline
compiles cleanly. If it doesn't, fix pre-existing issues first (they are
not part of this task).

## Sub-tasks

### ST1 Add `slug` dependency to `Cargo.toml`
**Files:** `Cargo.toml`
**Acceptance:** `cargo check` compiles without errors
**Estimate:** S

Add `slug = "0.1.6"` under `[dependencies]`. Run `cargo check` to confirm
the dependency resolves and compiles. If `slug 0.1.6` fails under MSRV 1.80,
fall back to an inline `slugify` function in `src/run_state/run_dir.rs`
(design doc open question #1).

### ST2 Create `RunDir` struct and `RunDir::create` in `src/run_state/run_dir.rs`
**Files:** `src/run_state/run_dir.rs` (NEW)
**Acceptance:** `cargo check --lib` compiles; `RunDir::create(base_dir, name)` and `RunDir::create_in_cwd(name)` are callable
**Estimate:** M

Implement the full `RunDir` struct and its public API:

- `RunDir { path, run_id, workflow_slug }` with fields
- `RunDir::create(base_dir: &Path, workflow_name: &str) -> Result<Self, RunDirError>`
  - Derive slug via `slug::slugify` (or inline impl)
  - Format: `<base_dir>/runs/<slug>-<YYYYMMDD>-<HHMMSS>-<short_uuid>/`
  - Atomic creation protocol with retry-once collision handling
  - Cleanup guard (Drop-based) to remove leaf dir on partial failure
  - Write `manifest.json` via `atomic_write`
  - Create `attempts/` subdirectory
- `RunDir::create_in_cwd(workflow_name: &str) -> Result<Self, RunDirError>`
- Accessors: `path()`, `manifest_path()`, `trace_path()`, `attempt_dir()`, `run_id()`, `workflow_slug()`
- `RunDirError` enum with `Collision`, `Io`, `Manifest` variants
- `#[cfg(test)] mod tests` with:
  - A test that `create` with a temp `base_dir` produces a path matching `runs/<slug>-<YYYYMMDD>-<HHMMSS>-<short_uuid>/`
  - A test that two back-to-back creates produce distinct paths
  - A test verifying `manifest.json` exists with correct initial shape
  - A test verifying `attempts/` subdirectory exists
  - A test verifying `run_id()` matches `manifest.json`'s `loker.run_id`

### ST3 Register module in `src/run_state/mod.rs`
**Files:** `src/run_state/mod.rs`
**Acceptance:** `cargo check` compiles
**Estimate:** S

Add `pub(crate) mod run_dir;` and re-export `RunDir` and `RunDirError`.

### ST4 Create TDD contract tests in `tests/run_dir_layout.rs`
**Files:** `tests/run_dir_layout.rs` (NEW)
**Acceptance:** `cargo test --test run_dir_layout` passes (9 tests)
**Estimate:** M

Implement all 9 tests from the design doc test plan:

1. `created_dir_matches_expected_regex` — path matches
   `runs/[a-z0-9-]+-\d{8}-\d{6}-[0-9a-f]{8}/`
2. `two_creates_produce_distinct_paths` — same workflow name, different paths
3. `manifest_json_exists_with_correct_shape` — valid JSON with `loker.run_id`,
   `schema_version: 1`, `entries: []`
4. `attempts_subdirectory_exists` — `path().join("attempts")` is a dir
5. `accessors_return_paths_under_run_root` — each accessor path starts with `path()`
6. `collision_retry_succeeds` — simulate first-creation collision, verify retry
7. `workflow_slug_matches_expected_format` — `"My Workflow!"` → `my-workflow` in path
8. `run_id_is_consistent` — `run_id()` matches `manifest.json`'s `loker.run_id`
9. `attempt_dir_returns_correct_handle` — `attempt_dir("design", 0)` equals
   `AttemptDir::new(run_dir.path(), "design", 0)`

### ST5 Wire `RunDir` into CLI (`src/main.rs`)
**Files:** `src/main.rs`
**Acceptance:** `cargo test --test run_dir_layout` still passes; `cargo build --bin loker` compiles
**Estimate:** S

In `Commands::Run { name, dir, ... }` handler (shorthand for `workflow run`):
- Create a `RunDir` before invoking `run_workflow`
- Pass `RunDir::path()` as the run directory to the workflow runner

In `WorkflowCommands::Run { name, dir, ... }` handler:
- Same integration: create `RunDir` and pass its path

This is a minimal integration point — the `RunDir` is created but the full
PhaseRunner integration (passing run_id into PhaseContext) is wired in a
follow-up task.

### ST6 `make check` — final verification
**Files:** — (meta task)
**Acceptance:** `make check` passes (fmt + clippy + test)
**Estimate:** S

Run `cargo fmt --check`, `cargo clippy --all-targets`, `cargo test`.
Fix any issues. This is the pre-merge gate.

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Risks
| Risk | Mitigation |
|------|-----------|
| `slug 0.1.6` fails under MSRV 1.80 | Inline ~5-line `slugify` function as documented fallback |
| CLI integration may conflict with existing `loker run <workflow>` behaviour | Creates `RunDir` but doesn't change workflow runner semantics; old path preserved |
