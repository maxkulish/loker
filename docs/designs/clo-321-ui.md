# Design: CLO-321 - [T-052] Daemon mode: loker ui --serve

## 1. Problem

Per the discovery report (`docs/discovery/clo-321.md`), operators running loker on a host have no unified surface for inspecting active, blocked, or completed runs: each run lives in an isolated directory under `<project_root>/runs/` and the only HTTP surface today is the T-051 per-gate fallback server (`src/hitl_server/one_shot.rs`), which binds for one HITL decision and exits. Filesystem navigation is the current substitute, which does not scale beyond a handful of runs and provides no live status. T-052 is needed now because Phase 12 (M11) sessions list (T-053), SSE tail-f (T-054), and threat-model tests (T-055) all assume a long-running daemon process exists; without this design landed, the rest of M11 has no host to attach to.

## 2. Goals / Non-goals

### Goals
- Add `loker ui --serve [--bind <ADDR>]` subcommand that starts a long-running axum daemon on `127.0.0.1:8080` by default.
- Discover all run directories under `<project_root>/runs/` on each request to `GET /` (no fs-notify in v0).
- Return a JSON array of run summaries (`id`, `path`, `workflow`, `run_id`, `created_at`, `phase_status`) from `GET /`.
- Reuse T-051 gate context rendering for `GET /gates/:phase` via shared free functions in `hitl_server::routes`, so the HITL surface (FR-27) is not duplicated.
- Graceful shutdown on SIGINT and SIGTERM within ~1 s via `axum::serve(...).with_graceful_shutdown(...)`.
- Skip and log any individual run directory whose state cannot be loaded; never crash the daemon on a corrupt run.
- Empty `runs/` returns `[]`, not an error.
- Pass `make check` (fmt + clippy + test) with new unit and integration coverage.

### Non-goals
- HTML rendering of the runs list (deferred to T-053).
- SSE tail-f of `trace.jsonl` (deferred to T-054).
- Threat-model test suite (deferred to T-055).
- Authentication, TLS, multi-host binding policies beyond the localhost default.
- `notify`/fs-notify push updates; v0 re-scans on each `GET /`.
- New top-level dependencies (axum, tokio, tower, serde, serde_json, chrono are already in `Cargo.toml`).
- POST endpoints (`/approve`, `/reject`) at the daemon root - the gate-decision endpoints live only under the nested `/gates` router exposed in v0 because T-053 will redesign that flow.
- Replacing or deprecating `hitl_server::one_shot` - the one-shot fallback continues to work unchanged.

## 3. Architecture

### 3.1 Module layout

```
src/
  ui/
    mod.rs          # pub use serve::serve, discovery::{discover_runs, RunSummary}
    serve.rs        # bind + axum::serve + shutdown_signal
    routes.rs       # ui_routes(project_root) -> Router; runs_list handler
    discovery.rs    # discover_runs, RunSummary, load_run_summary
  hitl_server/
    mod.rs          # unchanged exports
    routes.rs       # extended: free fns for gate context rendering
    one_shot.rs     # unchanged
  main.rs           # Commands::Ui { serve, bind } variant added
```

### 3.2 Data flow

```
+---------------------+         +-------------------+         +------------------------+
| loker ui --serve    | ------> | ui::serve::serve  | ------> | TcpListener bind/serve |
+---------------------+         +-------------------+         +------------------------+
                                          |                              |
                                          v                              v
                                  +---------------+              +-----------------+
                                  | ui::routes    |  composes    | hitl_server::   |
                                  | ui_routes()   | -----------> | routes (free fns|
                                  | "/" + /gates  |              | for gate views) |
                                  +---------------+              +-----------------+
                                          |
                              GET /  -----+--->  ui::discovery::discover_runs(project_root)
                                                          |
                                                          v
                                                  fs::read_dir("runs/")
                                                          |
                                                          v
                                                  load_run_summary(path)
                                                  - read manifest.json
                                                  - scan phase markers
                                                  - on Err: log warn, skip
```

### 3.3 Concrete types

- `ui::discovery::RunSummary` (Serialize): one row of the runs list.
- `ui::routes::AppState` (Clone): wraps `project_root: PathBuf` for `with_state`.
- `Commands::Ui { serve: bool, bind: String }`: clap subcommand variant.
- `ui::serve::serve(bind: &str, project_root: PathBuf) -> anyhow::Result<()>`: daemon entry point.
- Shutdown future composed from `tokio::signal::ctrl_c()` and `tokio::signal::unix::SignalKind::terminate()` (gated on `cfg(unix)`).

### 3.4 Reuse contract with T-051

The discovery report flags that T-051's `routes::router` and `AppState` are tied to a single `GateConfig` with a `oneshot::Sender`. The daemon must not fabricate a dummy sender, because POSTing to `/approve` would then panic on a missing receiver. The chosen path:

1. In `src/hitl_server/routes.rs`, extract the per-gate context rendering body (HTML + JSON view of one `GateConfig`) into a free function such as `render_gate_view(config: &GateConfig) -> Response`.
2. The one-shot router keeps its current shape and calls `render_gate_view` from its `GET /` handler.
3. The daemon's `ui::routes::ui_routes` is designed to compose a separate gate router at `/gates/:phase` in T-053. In v0, only `GET /` (runs list) is mounted — the gate view endpoint is deferred to T-053 alongside the sessions list. POST handlers are intentionally not wired in v0 (gate decision flow remains via the one-shot path until T-053).

This satisfies FR-27 (route handler reuse) without forcing dummy state into the daemon.

## 4. Public API surface

### 4.1 `src/main.rs` (CLI extension)

```rust
#[derive(Subcommand)]
enum Commands {
    // ... existing variants ...

    /// Start the loker UI daemon.
    Ui {
        /// Run as a long-lived daemon. Reserved for future `loker ui` subcommands.
        #[arg(long)]
        serve: bool,

        /// Bind address. Defaults to localhost:8080.
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: String,
    },
}
```

The `Ui` arm in `main` resolves `project_root` via the existing `find_project_root()` helper and calls `ui::serve::serve(&bind, project_root).await`. If `serve` is `false`, prints a usage error via `anyhow::bail!`.

### 4.2 `src/ui/mod.rs`

```rust
mod discovery;
mod routes;
mod serve;

pub use discovery::{discover_runs, RunSummary};
pub use serve::serve;
```

### 4.3 `src/ui/discovery.rs`

```rust
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Summary of one run for the runs-list endpoint.
#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub id: String,
    pub path: PathBuf,
    pub workflow: Option<String>,
    pub run_id: Option<String>,
    pub created_at: Option<String>,
    pub phase_status: BTreeMap<String, String>,
}

/// Scan `<project_root>/runs/` and return summaries for each run directory.
///
/// Errors on individual entries are logged to stderr and skipped; the function
/// itself never returns an error.
pub fn discover_runs(project_root: &Path) -> Vec<RunSummary>;

/// Load a single run summary. Internal; exposed only for tests.
pub(crate) fn load_run_summary(run_dir: &Path) -> anyhow::Result<RunSummary>;
```

### 4.4 `src/ui/routes.rs`

```rust
use std::path::PathBuf;

use axum::{routing::get, Router};

#[derive(Clone)]
pub struct AppState {
    pub project_root: PathBuf,
}

/// Build the daemon's top-level router.
///
/// - `GET /`              -> JSON runs list
/// - `GET /gates/:phase`  -> deferred to T-053 (per-run gate views)
pub fn ui_routes(project_root: PathBuf) -> Router;
```

### 4.5 `src/ui/serve.rs`

```rust
use std::path::PathBuf;

use anyhow::Result;

/// Bind to `bind`, serve `ui_routes(project_root)`, and run until SIGINT/SIGTERM.
pub async fn serve(bind: &str, project_root: PathBuf) -> Result<()>;

/// Spawn the daemon on port 0 and return the join handle + bound address.
/// Used by integration tests to avoid port collisions.
#[cfg(test)]
pub async fn spawn_test_daemon(project_root: PathBuf) -> (tokio::task::JoinHandle<()>, std::net::SocketAddr);
```

### 4.6 `src/hitl_server/routes.rs` (additions)

```rust
use axum::response::Response;

use super::GateConfig;

/// Render the per-gate view (JSON in v0; HTML once T-053 lands the template).
///
/// Shared between the one-shot fallback server and the `loker ui --serve` daemon.
pub fn render_gate_view(config: &GateConfig) -> Response;
```

The existing `router(state: AppState) -> Router` keeps its signature; its `GET /` handler is updated to delegate to `render_gate_view(&state.config)`.

## 5. Test plan

### 5.1 Unit tests - `src/ui/discovery.rs`

- `discover_runs_empty_runs_dir` - `runs/` exists but empty -> `[]`.
- `discover_runs_missing_runs_dir` - no `runs/` directory at all -> `[]`.
- `discover_runs_missing_manifest` - run dir without `manifest.json` is skipped; warning logged.
- `discover_runs_corrupt_manifest` - run dir with invalid JSON in `manifest.json` is skipped.
- `discover_runs_populated` - two valid run dirs return summaries with `workflow`, `run_id`, `phase_status` populated and ordered deterministically by directory name.
- `discover_runs_non_directories_in_runs_dir` - regular files (e.g. `.DS_Store`) under `runs/` are silently skipped.
- `discover_runs_partial_phase_markers` - run with `phase-X.started` but no `.completed` reports `started` for that phase.

Fixtures: build run dirs in a `tempfile::TempDir` using the same `manifest.json` shape produced by `RunDir::create`; do not rely on a live `RunState::load` round-trip beyond what `load_run_summary` actually needs.

### 5.2 Unit tests - `src/ui/routes.rs`

- `runs_list_returns_json_array` - call the handler directly with an `AppState` pointing at a temp project root containing one run; assert response status `200`, content-type `application/json`, and body is a single-element array with the expected `id`.
- `runs_list_empty_when_runs_missing` - same as above with no `runs/` dir; body is `[]`.

### 5.3 Unit tests - `src/hitl_server/routes.rs`

- `render_gate_view_one_shot_unchanged` - existing one-shot tests must still pass; add `render_gate_view_returns_expected_body` that calls the free function with a fixture `GateConfig` and asserts the rendered body matches the prior `GET /` snapshot.

### 5.4 Integration tests - `tests/ui_daemon.rs`

- `daemon_serves_runs_list` - bind to `127.0.0.1:0` via a helper that exposes the bound port; create a fixture run dir under a temp project root; `GET /` returns JSON with that run's id.
- `daemon_returns_empty_list_when_no_runs` - empty project root; `GET /` -> `[]`.
- `daemon_custom_bind_address` - start daemon with `--bind 127.0.0.1:0` (port 0 to avoid races); assert it responds on the actually-bound port.
- `daemon_shuts_down_gracefully_on_sigint` - spawn daemon on a JoinHandle, send `tokio::signal::ctrl_c` equivalent (or trigger the shutdown future directly), assert the task completes within 2 s.
- `daemon_skips_corrupt_run_directory` - place a valid run and a corrupt run in `runs/`; `GET /` returns one summary and the daemon stays up for a follow-up request.

The integration tests must bind to port `0` (not `8080`) to avoid collisions in CI; the default-port behaviour is verified at the CLI parser layer in unit tests, not by binding.

### 5.5 Manual verification

1. `cargo run -- ui --serve` in a project with one or more `runs/` entries -> visit `http://127.0.0.1:8080/`, confirm JSON list.
2. Add a new run while the daemon is running, refresh -> new run appears.
3. Delete or corrupt one run's `manifest.json`, refresh -> the corrupt run disappears, daemon stays alive, stderr shows a `WARN` line.
4. Press Ctrl-C -> daemon exits within ~1 s with `shutting down loker UI daemon` on stderr.
5. `cargo run -- ui --serve --bind 127.0.0.1:9090` -> daemon binds 9090.

### 5.6 Regression

- `cargo test -q` (existing 466 unit + 6 integration must stay green).
- `make check` (fmt + clippy + test).

## 6. Migration / rollout

- Additive only. New `src/ui/` module, new `Commands::Ui` variant. No changes to marker schema, manifest schema, run directory layout, `lok.toml`, or workflow files.
- One internal API change: `hitl_server::routes` gains the free function `render_gate_view` and its in-module `GET /` handler delegates to it. This is a private-module refactor; the `hitl_server::one_shot::start` public surface and `ServerHandle` shape are unchanged.
- No feature flag needed - the daemon path only runs when the user invokes `loker ui --serve`.
- No deprecations. The one-shot fallback server (T-051) is the default HITL gate path and continues to work as before.
- Rollout order: land the `render_gate_view` extraction first (with regression tests for one-shot), then the `ui` module + CLI variant on top. Both can ship in the same PR since the extraction is small and inert without the daemon.
- Nothing to migrate for users: existing `loker run`, `loker resume`, `loker ls-blocked` commands behave identically.

## 7. Open questions

- **CLI UX for no-flag invocation.** Resolved: `loker ui` without `--serve` prints `anyhow::bail!("use --serve to start the daemon; see loker ui --help")`. The `--serve` flag is explicitly required to leave room for future `loker ui <subcommand>` variants.

- **Port-conflict UX.** If `127.0.0.1:8080` is already bound by another process, `axum::serve` returns a bind error and the daemon exits. Should v0 fall back to `:0` and print the bound address (more forgiving), or keep the strict failure (more predictable for scripts)? The PRD says "default port 8080" but does not pin behaviour on conflict.
- **`project_root` resolution from CWD.** `find_project_root()` walks ancestors looking for `lok.toml`. If the user invokes `loker ui --serve` outside any project, should the daemon refuse to start, or start with an empty project root and serve `[]`? The discovery report assumes the former is implicit; the PRD does not say.
- **Sub-path layout for the gate view.** The PRD shows `GET /gates/:phase`, but a daemon serving multiple runs needs `:run_id` in the path too (e.g. `GET /runs/:run_id/gates/:phase`). The discovery report mentions per-run overview at `GET /runs/:id` as a future endpoint but does not finalize the gate-view path. Decide before T-053 builds the sessions list.
- **Re-scan cost.** Re-scanning `runs/` on every `GET /` is the v0 plan. For hosts with hundreds of runs this could be O(N) IO per request. Acceptable for v0, but the threshold at which we add an in-memory cache (refreshed by polling or fs-notify) is not defined.
- **Trade-off on the route-extraction approach (4.5 in the PRD).** The PRD's "Approach chosen" prefers extracting gate context rendering into free functions. An alternative was a no-op `AppState` with a dummy sender. The chosen path is captured in §3.4, but if `render_gate_view` ends up needing data that today only `AppState` carries (e.g. the decision sender), we may need to revisit and split the function further. Flag if encountered during implementation rather than guess now.
- **Logging format.** The PRD says "structured logging via `eprintln!` to stderr (no new dependency)." That is fine for v0 but is not actually structured. Decide whether T-052 ships plain `eprintln!` lines and a later milestone introduces a logging crate, or whether a minimal JSON-line format is worth implementing inline now.
