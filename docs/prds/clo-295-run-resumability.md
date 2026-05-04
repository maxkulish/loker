# PRD: Run Resumability via Status Markers (CLO-295 / T-031)

## Goal

CLO-295 delivers the **resume planner + disk-state scaffolding** for run resumability: `loker resume <run-dir>` validates on-disk state (markers, manifest, heartbeat), sweeps stale tmp, acquires advisory lock, and produces a `ResumePlan` that decides Skip / Resume / RunFresh per phase. The actual execution wiring (`ResumeRunner` driving `PhaseRunner` end-to-end) is intentionally deferred to a follow-up issue because it requires resolving how `PhaseConfig` is derived from a persisted workflow.

## In Scope

1. **`loker resume <run-dir>`** hidden CLI subcommand (state validation + planning only; execution deferred).
2. **Resume planner**: scan markers; phases with `<phase>.completed` are skipped; the first phase without `completed` becomes the resume point.
3. **Stale-writer detection via heartbeat** (CLO-284): if heartbeat is fresh, refuse to resume with `RunInProgress` error; if stale, take over.
4. **Manifest verification** for completed phases (sha256 match) before trusting them; surface `ArtefactCorrupt` / `ArtefactMissing` from CLO-285.
5. **Advisory lock file** (`runs/<id>/.lock`) for concurrent-writer detection (row 14 of D3 kill matrix).
6. **Stale tmp sweep**: on resume, move any `*.tmp` files older than heartbeat TTL to `attempts/_orphan_tmp/<timestamp>/`.
7. **Archive operation**: atomic rename of current attempt to `attempts/<phase>/<n>/`.
8. **Heartbeat TTL persistence**: `heartbeat.json` stores `ttl_seconds` so resume reads the exact TTL used by the original run.
9. **TDD test contract** in `tests/resume.rs` covering the 5 planner scenarios below.

## Out of Scope (follow-up issue)

- `ResumeRunner::execute()` wiring that calls `PhaseRunner::run()` for each non-Skip phase.
- Deriving `Vec<PhaseConfig>` from a persisted workflow at resume time.
- `loker resume --from <phase>` (manual rewind).
- Multi-run-dir batch resume.
- Cross-host coordination beyond heartbeat + advisory lock.

## Acceptance Criteria

- `ResumePlanner::plan()` correctly classifies every phase as Skip / Resume / RunFresh for all 5 TDD scenarios.
- `RunState::load()` + heartbeat TTL recovery + advisory lock + stale tmp sweep work together without race conditions.
- Hidden CLI subcommand validates state and either exits 0 (all complete) or prints a clear "execution not yet implemented" message.
- No state file beyond markers + manifest + heartbeat.
- `make check` clean (clippy, test, fmt).

## TDD Test Contract

`tests/resume.rs` validates the **planner decisions** for 5 scenarios (execution wiring is follow-up):

1. **Kill mid-phase-2**: 3-phase workflow, kill mid-phase-2. Planner marks phase 2 as `Resume { next_attempt: 2 }`, phase 3 as `RunFresh`.
2. **Already complete**: 3-phase workflow, all phases completed. Planner marks all phases as `Skip`.
3. **Corrupt manifest entry**: sha mismatch surfaces `ArtefactCorrupt` via `RunState::load` before planner runs.
4. **Live writer**: heartbeat fresh rejects resume with `RunInProgress`.
5. **Stale writer**: heartbeat older than TTL is taken over, planner marks resume phase with correct next attempt.

## Data Model

Uses existing D3 protocol types:
- `RunState::load(run_dir, ttl)` → `entries`, `phase_status`, `heartbeat`, `dropped_orphans`
- `PhaseStatus::{None, Started, Completed, Failed}`
- `HeartbeatStatus::{Live, Stale}`
- `MarkerWriter` for writing markers
- `AttemptDir` for attempt directory management

## New Types

```rust
// src/resume.rs

#[derive(Debug, thiserror::Error)]
pub enum ResumeError {
    #[error("run is already in progress (heartbeat live at {last_tick})")]
    RunInProgress { last_tick: DateTime<Utc> },

    #[error("artefact corrupt: {path} (expected {expected}, found {found})")]
    ArtefactCorrupt { path: String, expected: String, found: String },

    #[error("artefact missing: {path}")]
    ArtefactMissing { path: String },

    #[error("manifest error: {0}")]
    Manifest(#[from] crate::manifest::ManifestError),

    #[error("load error: {0}")]
    Load(#[from] crate::run_state::LoadError),

    #[error("phase error: {0}")]
    Phase(#[from] crate::phase_runner::PhaseError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub struct ResumePlanner;

pub enum PhaseAction {
    Skip,                // completed marker present + sha verified
    Resume { attempt: u32 }, // started (stale) or failed — rerun from attempt N
    RunFresh,            // no markers — normal execution
}

pub struct ResumePlan {
    pub run_dir: PathBuf,
    pub actions: Vec<(PhaseConfig, PhaseAction)>,
    pub stale_tmp: Vec<PathBuf>,
}
```

## Execution Flow

1. Parse `<run-dir>` from CLI.
2. Acquire advisory lock on `run_dir/.lock` (non-blocking; fail if held).
3. `RunState::load(run_dir, ttl)` → validate manifest + markers.
4. **Stale tmp sweep**: collect `run_dir/**/*.tmp` with mtime > ttl, move to `attempts/_orphan_tmp/<timestamp>/`.
5. If heartbeat is `Live` → return `ResumeError::RunInProgress`.
6. Build `ResumePlan`: for each phase in workflow order:
   - `Completed` + sha match → `Skip`
   - `Failed` → `Resume { attempt = attempts_made + 1 }`
   - `Started` + stale heartbeat → archive current attempt, `Resume { attempt = next_attempt }`
   - `None` → `RunFresh`
7. If all phases are `Skip` → print "all phases complete", exit 0.
8. For each non-Skip phase in order:
   - If `Resume`, archive any existing `<phase>/` artefacts to `attempts/<phase>/<n>/`
   - Call `PhaseRunner::run()` with upstream artefacts loaded from manifest entries
   - On success, phase writes its own `completed` marker + manifest entry
   - On failure, phase writes its own `failed` marker
9. Release advisory lock on exit.

## Dependencies

- CLO-284 (markers + heartbeat) ✅ — implemented in `src/run_state/markers.rs`, `src/run_state/heartbeat.rs`
- CLO-286 (attempt directories) ✅ — implemented in `src/run_state/attempt_dir.rs`
- CLO-292 (PhaseRunner) ✅ — implemented in `src/phase_runner.rs`

## References

- `docs/run-state.md` — D3 atomic run-state write protocol
- PRD FR-21 (resumability contract)
- Design doc — kill matrix rows 4-9
