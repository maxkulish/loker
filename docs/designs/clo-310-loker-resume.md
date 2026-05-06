# Design: CLO-310 - loker resume \<run_id\> CLI subcommand
## 1. Problem
Developers running multi-phase loker workflows have always been able to resume interrupted runs programmatically (CLO-295 shipped `ResumePlanner`; CLO-301 wired `ResumeRunner` end-to-end), but the CLI entry point is hidden from `--help`, accepts only an absolute path, prints no diagnostic on common failure modes, and carries stale help text ("planner scaffolding; execution not yet wired") that contradicts the working implementation underneath. As a result, users who lose a run to OOM, preemption, or Ctrl-C have no discoverable way to pick it back up and must either restart from scratch or read source code. CLO-310 closes that gap by promoting the hidden subcommand to first-class CLI status, adding `<run_id>` name resolution, authoring useful error messages for every failure path, and validating the full pause-resume cycle with a round-trip integration test.
---
## 2. Goals / Non-goals
**Goals**
- Unhide `Commands::Resume` so it appears in `loker --help` and `loker resume --help`.
- Accept `<run_id>` as either a bare directory name under `runs/` or an absolute/relative path, resolved against `$PWD/runs/<run_id>` when not a path.
- Produce actionable error messages for every documented failure mode: run not found, fully complete (exit 0), no resumable state, live heartbeat, corrupted manifest / missing artefact.
- Distinguish `loker resume` from `loker run --rerun phase=X` clearly in `--help` text.
- Ship a pause/resume round-trip integration test that kills a run mid-phase via `SIGTERM`, calls `loker resume`, and asserts completion with no duplicate phase work.
- Pass `make check` (fmt + clippy + test).
**Non-goals**
- Resuming HITL-blocked phases - deferred to Phase 11/12.
- Automatic crash-detection and auto-resume - explicitly out of scope for v0.
- Changing `ResumePlanner` / `ResumeRunner` / `RunLock` / `sweep_stale_tmp` internals; this design is purely additive CLI polish.
- Renaming `run_dir` to `run_id` in internal structs - the internal representation stays `PathBuf`.
---
## 3. Architecture
### Module map
```
src/main.rs
  Commands::Resume { run_id: String, ttl: Option<u64> }   <- rename arg
  fn resolve_run_dir(run_id: &str) -> Result<PathBuf>     <- NEW helper
  guard clauses for fully-complete and no-resumable-state  <- NEW
src/resume.rs           (unchanged)
src/resume/lock.rs      (unchanged)
src/resume/sweep.rs     (unchanged)
src/run_state/load.rs   (unchanged)
tests/resume.rs         (additive)
  test_resume_round_trip_kill_mid_phase()   <- NEW
  test_resume_run_not_found()               <- NEW
  test_resume_fully_complete_exit_zero()    <- NEW
  test_resume_no_resumable_state()          <- NEW
```
### Data flow
```
loker resume <run_id> [--ttl N]
        |
        v
resolve_run_dir(run_id)
  +-- absolute / relative path? -> PathBuf::from(run_id) (validated exists)
  +-- bare name? -> <project_root>/runs/<run_id>   (validated exists)
        |  RunNotFound  ->  exit 1  "Run '<id>' not found..."
        v
RunLock::acquire(run_dir)
        |  LockInUse   ->  exit 1  "Run is still in progress at <path>..."
        v
sweep_stale_tmp(run_dir, effective_ttl)
        v
RunState::load(run_dir, effective_ttl)
        |  Corrupt / Missing  ->  exit 1  "Corrupted run at <path>..."
        |  HeartbeatStatus::Live ->  exit 1  "Run is still in progress..."
        v
all phases Completed?  ->  exit 0  "All phases already complete. Nothing to resume."
        v
all phases None?       ->  exit 1  "No resumable state found in <path>."
        v
load Manifest -> workflow_name -> load Workflow -> to_phase_configs()
        v
ResumePlanner::plan() -> ResumeRunner::execute(plan)
        v
println!("Resume complete.")   exit 0
```
---
## 4. Public API surface
No changes to `src/lib.rs`. All changes are in `src/main.rs`.
```rust
// Commands enum - updated variant
/// Resume a partially-completed run from the last completed phase marker.
///
/// <run_id> is either the directory name under runs/
/// (e.g. design-20260505-123456-abc1def2) or an absolute path to the run
/// directory.
///
/// Use `loker run --rerun phase=<name>` to force-rerun a specific phase in a
/// fresh invocation; use `loker resume` to continue an interrupted run without
/// re-executing already-completed phases.
Resume {
    /// Run directory name (under runs/) or absolute path.
    run_id: String,
    /// Heartbeat TTL in seconds. Defaults to the value recorded in the run's
    /// heartbeat.json, or 300 s if absent.
    #[arg(long)]
    ttl: Option<u64>,
},
// New private free function
fn resolve_run_dir(run_id: &str) -> anyhow::Result<std::path::PathBuf> {
    let p = std::path::Path::new(run_id);
    if p.is_absolute() || p.components().count() > 1 {
        if p.exists() {
            Ok(p.to_path_buf())
        } else {
            anyhow::bail!("Run '{}' not found.", run_id)
        }
    } else {
        // Walk ancestors looking for a project root (lok.toml), matching
        // the convention used by `loker run`. This ensures bare run_id
        // names work regardless of the user's CWD.
        let project_root = find_project_root()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let candidate = project_root.join("runs").join(run_id);
        if candidate.exists() {
            Ok(candidate)
        } else {
            anyhow::bail!(
                "Run '{}' not found in runs/ directory or as an absolute path.",
                run_id
            )
        }
    }
}
/// Walk ancestors from CWD looking for `lok.toml` to find the project root.
fn find_project_root() -> Option<std::path::PathBuf> {
    let mut cwd = std::env::current_dir().ok()?;
    loop {
        if cwd.join("lok.toml").exists() {
            return Some(cwd);
        }
        if !cwd.pop() {
            return None;
        }
    }
}
```
Guard clauses added to the handler **after** `RunState::load`, **before** the manifest load:
```rust
// All phases done - no-op, exit 0.
// NOTE: RunState::load() (called above) already verifies manifest entry SHA256
// integrity for every completed marker. If an artefact is corrupt, the error
// surfaces before this guard runs. Do NOT move this check before the load.
if !run_state.phase_status.is_empty()
    && run_state.phase_status.values().all(|s| *s == PhaseStatus::Completed)
{
    println!("All phases already complete. Nothing to resume.");
    return Ok(());
}
// No phase has any state - nothing to resume.
if run_state.phase_status.values().all(|s| *s == PhaseStatus::None) {
    anyhow::bail!("No resumable state found in {}.", run_dir.display());
}
```
---
## 5. Test plan
### Existing tests (unchanged)
The five planner scenarios in `tests/resume.rs` remain unchanged.
### New unit tests
| Function | What it covers |
|---|---|
| `test_resume_run_not_found` | `resolve_run_dir("no-such-run")` returns `Err` containing "not found" |
| `test_resume_run_absolute_not_found` | `resolve_run_dir("/tmp/loker-nonexistent-xyz")` returns `Err` |
| `test_resume_fully_complete_exit_zero` | Tempdir run with all phases `Completed`; handler returns `Ok(())`; stdout contains "All phases already complete." |
| `test_resume_no_resumable_state` | Tempdir run with all phases `None`; handler returns `Err` containing "No resumable state" |
### Round-trip integration test
```rust
#[test]
fn test_resume_round_trip_kill_mid_phase() {
    // 1. Write a two-phase workflow TOML to a tempdir.
    //    Phase 1: shell step that writes sentinel_phase1.txt and sleeps 30 s.
    //    Phase 2: shell step that writes sentinel_phase2.txt.
    // 2. Spawn `loker run <workflow>` as a child process.
    // 3. Poll until sentinel_phase1.txt appears (run is mid-phase-1 sleep).
    // 4. Send SIGTERM to the child; join with timeout.
    // 5. Assert runs/<id>/markers/phase-1/started exists.
    // 6. Assert runs/<id>/markers/phase-2/ does not exist.
    // 7. Invoke `loker resume <run_dir>` via std::process::Command.
    // 8. Assert exit code 0.
    // 9. Assert stdout contains "Resume complete."
    // 10. Assert runs/<id>/markers/phase-1/completed exists.
    // 11. Assert runs/<id>/markers/phase-2/completed exists.
    // 12. Assert sentinel_phase1.txt mtime is unchanged (phase 1 not re-run).
}
```
Signal delivery via `nix::sys::signal::kill(Pid::from_raw(child.id() as i32), Signal::SIGTERM)`. See Open question Q1.
This test is gated behind `LOKER_RESUME_INTEGRATION=1` to match the existing
`LOKER_TZ_INTEGRATION=1` convention for opt-in integration tests, since it
spawns a child process and adds ~30 s to the test suite.
### Manual verification steps
1. `loker --help` - `resume` appears in subcommand list.
2. `loker resume --help` - help text mentions both `<run_id>` formats and distinguishes resume from `run --rerun`.
3. `loker resume nonexistent-run` - exits 1, "not found".
4. Run a workflow to completion, then `loker resume <run_dir>` - exits 0, "All phases already complete."
5. `make check` - green.
---
## 6. Migration / rollout
No migration required. Removing `#[command(hide = true)]` and renaming `run_dir: PathBuf` to `run_id: String` are additive CLI-surface changes only. No serialized state, no internal structs, no stored configuration is affected. `resolve_run_dir` is a strict superset of the existing path-only behavior. No feature flags needed.
---
## 7. Open questions
**Q1 - `nix` crate for SIGTERM in the round-trip test.**
`std::process::Child::kill()` sends `SIGKILL`, which may skip the marker-flush path the round-trip test is meant to validate. `nix::sys::signal::kill` delivers `SIGTERM` correctly but must be added to `[dev-dependencies]` if absent. The tradeoff: correctness vs. a new dependency. Whether the phase runner flushes markers on `SIGKILL` is the deciding fact.
**Q2 - `runs/` anchor for bare `run_id` names.**
`resolve_run_dir` resolves bare names to `<project_root>/runs/<run_id>` by walking ancestors looking for `lok.toml`. This matches `loker run`'s convention. **Closed** — implemented as `find_project_root()` in the `resolve_run_dir` helper.
**Q3 - Exit code for fully-complete runs.**
The PRD specifies exit 0 for "already complete." CI callers may want a distinct code (e.g., exit 2) to mean "nothing to do" vs. "work was done successfully." The correct code for the already-complete case is unresolved in discovery and should be confirmed before shipping.
