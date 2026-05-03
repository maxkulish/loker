# Design: Run Resumability via Status Markers (CLO-295 / T-031)

## 1. Problem

The `loker run` command creates a fresh `RunDir` and unconditionally executes every workflow phase from scratch. If the process is killed mid-run (OOM, Ctrl-C, cloud preemption), the user must either restart from the beginning (wasting API tokens and wall-clock time) or manually piece together artefacts from a half-written run directory. The D3 protocol ([`docs/run-state.md`](docs/run-state.md)) already defines atomic markers, heartbeat, manifest, and attempt directories for crash recovery — but there is no orchestration layer that reads these artefacts to decide "skip vs rerun" per phase. This design closes that gap with a dedicated `ResumePlanner` + `ResumeRunner` that turns the on-disk state machine into a user-facing `loker resume <run-dir>` subcommand.

## 2. Goals / Non-goals

### Goals

- `loker resume <run-dir>` (hidden subcommand) validates on-disk state: lock, sweep, load, plan.
- The resume planner (`ResumePlanner::plan`) reads markers + manifest and decides Skip / Resume / RunFresh per phase.
- If the heartbeat is live (within TTL), refuse to resume with `RunInProgress`.
- If the heartbeat is stale, the planner marks the in-flight phase for resume with the correct next attempt counter.
- A fully-completed run is a no-op that prints "all phases complete" and exits `0`.
- Stale `*.tmp` files (mtime > heartbeat TTL) are swept to `attempts/_orphan_tmp/<timestamp>/` on resume start.
- Advisory lock (`run_dir/.lock`) prevents concurrent writers on the same run directory.
- All resume decisions are driven by the existing D3 protocol — no bespoke resume state file.
- Heartbeat TTL is persisted in `heartbeat.json` so resume reads the exact TTL.

### Non-goals

- `ResumeRunner::execute()` wiring that calls `PhaseRunner::run()` (follow-up issue — requires Workflow → PhaseConfig adapter).
- `loker resume --from <phase>` (manual rewind) — post-v0.
- Multi-run-dir batch resume.
- Cross-host coordination beyond heartbeat + advisory lock (row 14 of D3 kill matrix).
- Replaying or branching from historical attempt directories — attempts are archived, not replayed.

## 3. Architecture

### 3.1 Module layout

```
src/
  resume.rs           ← NEW — ResumePlanner, ResumeRunner, ResumePlan
  resume/
    lock.rs           ← NEW — advisory file lock helper (flock / LockFile)
    sweep.rs          ← NEW — stale tmp sweep logic
  main.rs             ← MODIFIED — add `resume` CLI subcommand
```

`src/resume.rs` is a single public module. Two private submodules (`lock`, `sweep`) keep the implementation granular without polluting the top-level namespace.

### 3.2 Data flow

```
CLI: resume <run-dir> [--ttl <seconds>]
  │
  ├─→ lock::acquire(run_dir/.lock)  ──► [fail] → ResumeError::LockInUse
  │
  ├─→ Determine TTL: read heartbeat.json["ttl_seconds"] if present,
  │    else CLI --ttl flag (default 300s). TTL must match the value
  │    used by the original run's HeartbeatWriter.
  │
  ├─→ sweep::stale_tmp(run_dir, ttl) ──► move *.tmp → attempts/_orphan_tmp/
  │    [fail on any IO error, including disk-full → hard error]
  │
  ├─→ RunState::load(run_dir, ttl)
  │      ├─ Manifest::load() + orphan sweep
  │      ├─ marker scan → phase_status map
  │      └─ heartbeat classification (Live / Stale)
  │
  ├─→ if heartbeat == Live → ResumeError::RunInProgress
  │
  ├─→ ResumePlanner::plan(run_state, phase_configs)
  │      ├─ per phase: Skip / Resume { attempt } / RunFresh
  │      └─ if all Skip → no-op
  │
  └─→ ResumeRunner::execute(plan, workflow, config)
         ├─ for each non-Skip phase:
         │    ├─ if Resume → archive_current_attempt():
         │    │            rename run_dir/<phase>/ → attempts/<phase>/<n>/
         │    │            (atomic; fail if target exists)
         │    │            then AttemptDir::create() for new attempt
         │    └─ PhaseRunner::run(phase_cfg, upstream_manifest_entries)
         │         ├─ strategy → aggregator → verify → persist
         │         ├─ writes .started, .completed/.failed markers
         │         ├─ appends to manifest atomically
         │         └─ trace.jsonl: append to existing file (not create new)
         └─ lock::release()
```

### 3.3 Concrete types

```rust
// src/resume.rs
use std::path::PathBuf;
use chrono::{DateTime, Utc};

#[derive(Debug, thiserror::Error)]
pub enum ResumeError {
    #[error("run is already in progress (heartbeat live at {last_tick})")]
    RunInProgress { last_tick: DateTime<Utc> },

    /// `ArtefactCorrupt` / `ArtefactMissing` surface through `LoadError`
    /// (from `RunState::load`) rather than as first-class `ResumeError`
    /// variants, keeping the error taxonomy flat.
    #[error("manifest error: {0}")]
    Manifest(#[from] crate::manifest::ManifestError),

    #[error("load error: {0}")]
    Load(#[from] crate::run_state::LoadError),

    #[error("phase error: {0}")]
    Phase(#[from] crate::phase_runner::PhaseError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Advisory lock is OS-cooperative only (flock / LockFileEx).
    /// Another loker process holds the lock; non-loker processes may ignore it.
    #[error("run directory is locked by another loker process")]
    LockInUse,
}

pub struct ResumePlanner;

impl ResumePlanner {
    /// Given the on-disk state and the ordered list of phases in a workflow,
    /// produce a plan that tells the executor what to do per phase.
    pub fn plan(
        run_state: &crate::run_state::RunState,
        phases: &[crate::phase_runner::PhaseConfig],
    ) -> Result<ResumePlan, ResumeError>;
}

pub enum PhaseAction {
    /// Phase is already completed and verified — do not invoke PhaseRunner.
    Skip,
    /// Phase was started (stale heartbeat) or failed. Archive current attempt
    /// and start a new one at the given counter.
    Resume { attempt: u32 },
    /// No markers exist for this phase — normal first-time execution.
    RunFresh,
}

pub struct ResumePlan {
    pub run_dir: PathBuf,
    /// Ordered actions aligned with the workflow phase list.
    pub actions: Vec<(PhaseConfig, PhaseAction)>,
    /// Tmp files that were swept before planning.
    pub swept_tmp: Vec<PathBuf>,
}

pub struct ResumeRunner {
    planner: ResumePlanner,
    // injected dependencies for PhaseRunner construction
    // (same fields as WorkflowRunner uses)
}

impl ResumeRunner {
    pub fn new(/* deps */) -> Self;

    /// Execute the resume plan. Returns Ok(()) when the run reaches completion
    /// (either because work was done or because all phases were already done).
    pub async fn execute(
        &self,
        plan: ResumePlan,
        workflow: &crate::workflow::Workflow,
    ) -> Result<(), ResumeError>;
}
```

### 3.4 Lock module (`src/resume/lock.rs`)

Uses `fs2::FileExt` (cross-platform `flock`) on Unix and Windows lockfile semantics. The lock file is `run_dir/.lock`. It is created empty if absent, then locked non-blocking. If the lock is already held, `ResumeError::LockInUse` is returned.

```rust
pub struct RunLock {
    file: std::fs::File,
}

impl RunLock {
    pub fn acquire(run_dir: &std::path::Path) -> Result<Self, ResumeError>;
    // Drop releases the lock automatically (close file descriptor).
}
```

### 3.5 Archive operation

When a phase is resumed, any existing canonical artefacts for that phase must be archived before a new attempt begins. The archive operation is a cross-filesystem-safe rename:

```rust
/// Move `run_dir/<phase>/` → `run_dir/attempts/<phase>/<attempt>/` atomically.
/// `attempt` is the *current* attempt number (from the existing `.started` marker
/// or from directory scanning). The target must not exist; if it does, this is
/// an invariant violation and returns `ResumeError::Io`.
pub fn archive_current_attempt(
    run_dir: &std::path::Path,
    phase: &str,
    attempt: u32,
) -> Result<(), std::io::Error>;
```

This is implemented as `std::fs::rename` (atomic on the same filesystem) followed by `fsync` on the parent `attempts/<phase>/` directory. If the run directory and attempts directory are on different mount points, the operation falls back to `fs_extra::dir::move_dir` (copy + delete), but this is not expected in normal loker deployments.

### 3.6 Sweep module (`src/resume/sweep.rs`)

```rust
/// Walk `run_dir` recursively, collect every `*.tmp` file whose mtime is
/// older than `ttl_seconds`, move them into `run_dir/attempts/_orphan_tmp/<ts>/`.
/// Returns the list of swept paths (for logging / test assertions).
pub fn sweep_stale_tmp(
    run_dir: &std::path::Path,
    ttl_seconds: u64,
) -> Result<Vec<PathBuf>, std::io::Error>;
```

## 4. Public API surface

The only new public entry point is the `resume` CLI subcommand. The `ResumePlanner` + `ResumeRunner` types are intentionally crate-internal (kept in `src/resume.rs`, not re-exported from `lib.rs`) because the orchestration layer is tied to the CLI binary, not the library surface.

```rust
// src/main.rs  —  CLI addition
#[derive(clap::Subcommand)]
enum Commands {
    // ... existing commands ...
    Resume {
        /// Path to the run directory to resume.
        run_dir: PathBuf,
        /// Heartbeat TTL in seconds. Defaults to the value stored in
        /// heartbeat.json, or 300 if absent. Must match the original run.
        #[arg(long)]
        ttl: Option<u64>,
    },
}

// In the run match arm:
Commands::Resume { run_dir, ttl } => {
    let lock = resume::lock::RunLock::acquire(&run_dir)?;
    let effective_ttl = ttl.unwrap_or_else(|| {
        // Read heartbeat.json["ttl_seconds"] if present
        resume::heartbeat_ttl(&run_dir).unwrap_or(300)
    });
    let swept = resume::sweep::sweep_stale_tmp(&run_dir, effective_ttl)?;
    let run_state = RunState::load(&run_dir, effective_ttl)?;
    // ... heartbeat check, plan, execute ...
}
```

`RunState::load()` and `PhaseRunner::run()` signatures remain unchanged; this design is purely additive.

## 5. Test plan

### 5.1 Unit tests (`src/resume.rs` — inline `#[cfg(test)]`)

| Test | What it checks |
|---|---|
| `planner_all_completed_returns_empty` | 3-phase workflow, all `.completed` markers → all `Skip`, no-op path. |
| `planner_first_started_becomes_resume` | Phase 1 completed, phase 2 started (no failed) → phase 2 `Resume { attempt: 2 }`, phase 3 `RunFresh`. |
| `planner_failed_increments_attempt` | Phase 1 completed, phase 2 failed → phase 2 `Resume { attempt: 2 }`. |
| `planner_none_is_run_fresh` | No markers → all `RunFresh`. |
| `planner_manifest_sha_mismatch_errors` | `.completed` marker references sha that does not match on-disk bytes → `ArtefactCorrupt`. |

### 5.2 Integration tests (`tests/resume.rs` — full TDD contract)

| # | Scenario | Setup | Assertion |
|---|---|---|---|
| 1 | Kill mid-phase-2 | 3-phase workflow, kill after phase 2 `.started` written. | Resume re-runs phase 2 in `attempts/phase2/2/`, phase 3 runs, manifest has exactly 3 entries (design, review, verify). |
| 2 | Already complete | All 3 phases have `.completed` + valid sha. | Resume is no-op; manifest unchanged; exit 0. |
| 3 | Corrupt manifest entry | Phase 1 `.completed` with sha, but on-disk bytes differ. | `LoadError::ArtefactCorrupt` (surfaced through `ResumeError::Load`) before any phase re-runs. |
| 4 | Live writer | Heartbeat tick within TTL. | `ResumeError::RunInProgress` immediately. |
| 5 | Stale writer | Heartbeat older than TTL, phase 2 `.started`. | Lock acquired, stale tmp swept, phase 2 resumed at attempt 2, phase 3 fresh. |

### 5.3 Manual test

1. `cargo run -- resume /tmp/runs/2026-05-03-abc123` on a directory with a live heartbeat → expect error.
2. Create a run dir with one completed phase, kill a mock runner mid-second phase, then resume → observe `attempts/phase2/2/` created and phase 3 follows.

## 6. Migration / rollout

No migration needed — this is a new subcommand. Existing `loker run` behaviour is unchanged.

- `make check` must pass before merge.
- The `resume` subcommand is hidden behind `#[command(hide = !cfg!(feature = "resume"))]` if we want an opt-in feature flag, but for v0 it can be public immediately since it does not mutate existing paths.
- Add `resume` to CLI help text and README usage examples once the TDD contract passes.

## 7. Open questions

| # | Question | Resolution |
|---|---|---|
| 1 | Should `ResumeRunner` live in `src/lib.rs` public surface? | **No** — it is a binary-only orchestration concern. Keep it crate-internal. |
| 2 | How does `ResumeRunner` obtain `PhaseConfig` instances from a `Workflow`? | **P1 — plan-phase sub-task:** Add a lightweight `Workflow::to_phase_configs()` adapter that derives `Vec<PhaseConfig>` from the workflow's named steps. If this cannot be done without refactoring `Workflow`, defer to a follow-up issue and accept that `resume` v0 only accepts programmatic phase lists (CLI passes `--phases` or reads from a workflow config file). |
| 3 | Advisory lock on Windows — `fs2::FileExt` handles both? | **Yes** — `fs2` abstracts `LockFileEx` on Windows and `flock` on Unix. Test in CI matrix. |
| 4 | Should stale tmp sweep be async? | **No** — it is synchronous fs walk + rename, negligible latency. Keep sync to avoid mixing async patterns. |
| 5 | How is the heartbeat TTL recovered on resume? | **P1 — add to schema:** `heartbeat.json` currently stores only `writer_pid`, `writer_host`, and `tick_at`. We will add an optional `ttl_seconds` field (default 300) so `resume` reads the exact TTL used by the original run. If absent, fall back to CLI `--ttl` (default 300). |
| 6 | What happens to `trace.jsonl` on resume? | **Append** to the existing `trace.jsonl` in the run directory. Each resumed phase appends its trace records. This means full-run idempotency excludes `trace.jsonl` timestamps (already documented in AC). |
| 7 | What if disk is full during stale tmp sweep? | **Hard error** — `sweep_stale_tmp` returns `Err(std::io::Error)` which propagates up and aborts the resume. Do not proceed with a dirty run directory. |

---

**Discovery context used:** `docs/discovery/clo-295.md`, `docs/prds/clo-295-run-resumability.md`

**Approach chosen:** Resume-as-separate ResumeRunner
