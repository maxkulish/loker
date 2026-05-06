# Design: CLO-313 - `loker ls --blocked`

## Problem

Per the discovery report, developers and operators running Loker workflows with HumanVerifier gates have no read-only inventory of work paused on a human decision. Today a paused run is only visible if the operator already knows to inspect `runs/<run_id>/pending/`, because the existing CLI exposes only `run`, `resume`, `explain`, and `trace`. This matters now because FR-37 / T-044 is the downstream CLI surface for the HumanVerifier pending schema (`PendingRequest` in `src/strategy/verify/human_verifier.rs`) and the CLO-319 per-phase advisory lock work, and without it operators cannot triage HITL queues without manual filesystem walks.

## Goals / Non-goals

Goals:

- Add `loker ls --blocked` subcommand that scans `<project_root>/runs/*/pending/<phase>.json` and excludes entries whose matching `responses/<phase>.json` exists.
- Print one row per blocked phase showing run id, phase, severity, age (relative to wall clock), and a path the operator can act on.
- Default sort: oldest `opened_at` first; stable across invocations.
- Empty case: print `no blocked runs` to stdout, exit 0.
- Snapshot test covering mixed blocked / completed / multi-phase runs.
- Pure scan / render functions reachable from integration tests without invoking workflow execution, mirroring the `trace` command pattern.

Non-goals:

- Mutating, claiming, or submitting `pending/` or `responses/` files (acceptance criteria explicitly read-only).
- Rendering a UI (browser HITL UI is M10–M11, separate design).
- Replacing or wrapping the CLO-319 `PhaseLock` implementation; lock metadata is not a source of truth for this command.
- A streaming / watch mode (`--watch`); this is a one-shot listing.
- JSON output mode and custom sort flags; these can land later if requested.
- `loker ls` without `--blocked` (no other listing modes are in scope for T-044).

## Architecture

A new private module `src/commands/ls_blocked.rs` owns scan and render logic. The clap dispatcher in `src/main.rs` adds an `Ls { blocked: bool }` variant and forwards to `loker::commands::ls_blocked::run`. Project root resolution reuses the existing `find_project_root()` walk (looking for `lok.toml`); on miss it falls back to CWD, matching the convention used by `Trace` and `Resume`.

Data flow:

```
+---------------+       +-----------------------+       +--------------------+
| clap dispatch | --->  | ls_blocked::run(opts) | --->  | scan_blocked(root) |
+---------------+       +-----------------------+       +--------------------+
                                   |                              |
                                   v                              v
                       +-----------------------+       +--------------------+
                       | render_table(entries, |  <--  | Vec<BlockedEntry>  |
                       |   now, &mut writer)   |       | (sorted oldest    |
                       +-----------------------+       |  opened_at first) |
                                                       +--------------------+
```

`scan_blocked` walks `<root>/runs/` one directory deep, then for each run dir reads `pending/*.json` and skips any whose sibling `responses/<phase>.json` exists. Each surviving file is parsed as `PendingRequest` (already public via `src/strategy/verify/human_verifier.rs`), producing one `BlockedEntry`. Parse failures are reported on stderr with the run id and phase, and the entry is skipped — they do not abort the scan. The collected vector is sorted by `opened_at` ascending; ties broken by `(run_id, phase)`.

`render_table` formats entries into a fixed-column plain-text table (no color in v1) and writes to the provided `Write`. Age is computed from `now - opened_at` parsed via `chrono::DateTime::parse_from_rfc3339`, rendered as a compact human string (`12s`, `4m`, `3h`, `2d`). The `now` instant is injected so snapshot tests are deterministic.

The "decision URL/path" required by the PRD is rendered as the project-relative response path `runs/<run_id>/responses/<phase>.json` — the file the operator must create to unblock the gate. This avoids inventing a URL field that does not exist on `PendingRequest` today and keeps snapshot output deterministic.

## Public API surface

In `src/commands/mod.rs`:

```rust
pub mod ls_blocked;
pub mod trace;
```

In `src/commands/ls_blocked.rs`:

```rust
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::strategy::verify::HumanSeverity;

/// One unresolved HITL gate, materialised from a `pending/<phase>.json`
/// file with no matching `responses/<phase>.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedEntry {
    pub run_id: String,
    pub phase: String,
    pub severity: HumanSeverity,
    pub opened_at: DateTime<Utc>,
    pub timeout_at: Option<DateTime<Utc>>,
    pub pending_path: PathBuf,
    pub response_path: PathBuf,
    pub response_display_path: PathBuf,
}

/// Walk `<root>/runs/*/pending/*.json`, skip entries whose
/// `responses/<phase>.json` sibling exists, and return the survivors
/// sorted by `opened_at` ascending (ties: `(run_id, phase)`).
///
/// Malformed pending files are logged to stderr and skipped; the scan
/// does not abort.
pub fn scan_blocked(root: &Path) -> Result<Vec<BlockedEntry>>;

/// Render `entries` as a plain-text table to `writer`. `now` is injected
/// so age columns are deterministic in tests. Empty input writes
/// `no blocked runs\n` and returns Ok(()).
pub fn render_table<W: Write>(
    entries: &[BlockedEntry],
    now: DateTime<Utc>,
    writer: &mut W,
) -> Result<()>;

/// CLI entry point. Scans `root/runs` and renders the blocked list to
/// stdout. Exit code is 0 on success regardless of empty / non-empty.
pub fn run(root: &Path) -> Result<()>;
```

In `src/main.rs` (added to the existing `Commands` enum, after `Trace`):

```rust
/// List runs with state matching a filter. Today only `--blocked`
/// is supported; it enumerates HITL-pending phases with no response.
Ls {
    /// Show runs paused on an unmatched HITL pending request.
    #[arg(long)]
    blocked: bool,
},
```

Dispatch arm (added in `match cli.command`):

```rust
Commands::Ls { blocked } => {
    if !blocked {
        anyhow::bail!("`loker ls` requires --blocked (no other modes in v1)");
    }
    let root = find_project_root()
        .or_else(|| std::env::current_dir().ok())
        .ok_or_else(|| anyhow::anyhow!("could not determine project root"))?;
    loker::commands::ls_blocked::run(&root)?;
}
```

No new `pub use` in `src/lib.rs`; the existing `pub mod commands` line already exposes the module path used by `main.rs` and integration tests.

## Test plan

Unit tests in `src/commands/ls_blocked.rs` (use `tempfile::tempdir`, write fixture `pending/` and `responses/` JSON by hand to avoid coupling to `HumanVerifier`):

- `scan_blocked_empty_runs_dir_returns_empty` — `runs/` exists with no children.
- `scan_blocked_runs_dir_missing_returns_empty` — no `runs/` at all.
- `scan_blocked_skips_resolved_phase` — pending + matching response → not listed.
- `scan_blocked_lists_unmatched_pending` — pending without response → one entry.
- `scan_blocked_handles_multi_phase_run` — two pending files in one run, only one resolved → one entry.
- `scan_blocked_sorts_oldest_first` — three runs with distinct `opened_at`, assert ordering.
- `scan_blocked_logs_and_skips_malformed_pending` — invalid JSON pending file → eprintln warning, no panic, other entries returned.
- `render_table_empty_writes_no_blocked_runs` — empty input writes the literal `no blocked runs\n`.
- `render_table_renders_age_severity_paths` — assert columns contain run id, phase, severity string, and project-relative response path.
- `render_table_age_units` — table-driven check that 12s / 4m / 3h / 2d formatting is selected by `now - opened_at`.

Integration / snapshot test in `tests/ls_blocked.rs`:

- `snapshot_mixed_blocked_and_completed` — build a tempdir with three runs (one fully resolved, one blocked on `design`, one blocked on `review` with a separate resolved `implement` phase), call `scan_blocked` + `render_table` with a fixed `now`, assert against `tests/snapshots/ls_blocked__snapshot_mixed_blocked_and_completed.snap` using the same `insta` setup as `trace_pretty`.

Manual verification:

1. `cargo run --bin loker -- ls --blocked` in a project with no `runs/` → prints `no blocked runs`, exits 0.
2. Run any workflow that lands on a `HumanVerifier` gate (e.g. an `m6` design fixture wired with severity = High), then `loker ls --blocked` from any subdirectory of the project → row appears with the run id, phase, severity, age, and the `runs/<run_id>/responses/<phase>.json` path.
3. Drop a hand-crafted `responses/<phase>.json` matching that phase, rerun `ls --blocked` → row disappears.

## Migration / rollout

Nothing to migrate. This is an additive subcommand with no on-disk format change and no read of historical state. The `PendingRequest` schema (`SCHEMA_VERSION = 1`) is read as-is; entries with a different `schema_version` are treated as malformed (logged + skipped), consistent with how `HumanVerifier` itself rejects them. No feature flag is needed — the command compiles unconditionally and is documented in `--help` once merged.

Rollout order is single-step:

1. Land `src/commands/ls_blocked.rs`, `mod.rs` export, `Commands::Ls` clap variant, dispatch arm, unit tests, and snapshot test in one PR behind the standard `make check` gate.

## Open questions

None for implementation. Discovery uncertainties are resolved for this task as follows:

- **Decision URL vs. response path:** render `runs/<run_id>/responses/<phase>.json`, the project-relative response path. A future browser UI can add URL support with a schema addition or display option.
- **Lock metadata inclusion:** defer; blocked truth comes from unmatched pending/response files. Lock metadata can be added later behind an extra column or flag without changing the scanner.
- **Age format and column widths:** use compact `12s` / `4m` / `3h` / `2d` age strings and hand-formatted fixed columns, stabilized by snapshot tests.
- **Behaviour when `--blocked` is omitted:** return an error (`loker ls requires --blocked`) so future listing filters remain additive.
