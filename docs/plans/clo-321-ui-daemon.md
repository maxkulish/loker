# Plan: CLO-321 — [T-052] Daemon mode: loker ui --serve

## Context

- **Design**: `docs/designs/clo-321-ui.md`
- **Discovery**: `docs/discovery/clo-321.md`
- **PRD**: `docs/prds/clo-321-daemon-mode.md`
- **Linear**: https://linear.app/cloud-ai/issue/CLO-321/t-052-daemon-mode-loker-ui-serve
- **Branch**: `feat/clo-321-ui`

## Architecture overview

```
src/
  ui/
    mod.rs          # Re-exports
    discovery.rs    # Run directory scanner (RunSummary, discover_runs, load_run_summary)
    routes.rs       # Daemon routes (GET / runs list), AppState
    serve.rs        # Daemon bootstrap (bind, axum::serve, graceful shutdown)
  hitl_server/
    routes.rs       # Extended: render_gate_view free function (shared with daemon)
    one_shot.rs     # Unchanged
    mod.rs          # Unchanged
  main.rs           # +Commands::Ui variant, +mod ui
tests/
  ui_daemon.rs      # Integration tests for daemon mode
```

## Sub-tasks

### ST1 Extract `render_gate_view` free function from `hitl_server::routes`

**Files:**
- `src/hitl_server/routes.rs` (modify)
- `src/hitl_server/mod.rs` (no change needed — already `pub mod routes`)

**Changes:**
- Extract the gate context HTML rendering logic (currently inside the `GET /` handler) into a public free function:
  ```rust
  pub fn render_gate_view(config: &GateConfig) -> Result<Html<String>, StatusCode>;
  ```
- Update the existing `GET /` handler to delegate to `render_gate_view(&state.config)`. No signature change to `router()`.
- The function takes `GateConfig` directly (not `AppState`), so the daemon can call it without fabricating a `oneshot::Sender`.

**Tests to add (in `src/hitl_server/routes.rs`):**
- `render_gate_view_returns_expected_body` — call `render_gate_view` with a fixture `GateConfig`, assert the HTML contains the phase name, workflow, severity, artefact path, and approve/reject buttons.
- `render_gate_view_fallback_for_empty_pending` — call with a `GateConfig` whose pending JSON doesn't exist and assert fallback HTML is rendered.
- `render_gate_view_sanitises_html` — confirm `html_escape` is called (injection test).

**Acceptance:**
```bash
cargo test -p loker -- hitl_server::routes::tests 2>&1 | tail -5
```
Existing gate-router tests continue to pass unchanged.

**Estimate:** S

---

### ST2 Write `ui::discovery` module

**Files (new):**
- `src/ui/mod.rs` — module declarations + re-exports
- `src/ui/discovery.rs` — `RunSummary`, `discover_runs()`, `load_run_summary()`

**Contracts:**
```rust
#[derive(Debug, Clone, Serialize)]
pub struct RunSummary {
    pub id: String,              // directory basename
    pub path: PathBuf,           // absolute path
    pub workflow: Option<String>, // from manifest.json
    pub run_id: Option<String>,  // from manifest.json
    pub created_at: Option<String>, // from manifest.json
    pub phase_status: BTreeMap<String, String>, // ".completed" / ".started" / ".failed"
}

pub fn discover_runs(project_root: &Path) -> Vec<RunSummary>;

pub(crate) fn load_run_summary(run_dir: &Path) -> anyhow::Result<RunSummary>;
```

**Key design decisions:**
- Scan `<project_root>/runs/` synchronously on each call (no caching in v0).
- Read `manifest.json` to extract `workflow`, `run_id`, `created_at`.
- Scan `markers/` directory for phase status (`.started`, `.completed`, `.failed`).
- IO errors on individual entries are `eprintln!`'d and skipped.
- Non-directory entries in `runs/` are silently skipped.

**Tests to add (in `src/ui/discovery.rs`):**
- `discover_runs_empty_runs_dir` — empty `runs/` returns `[]`.
- `discover_runs_missing_runs_dir` — no `runs/` directory returns `[]`.
- `discover_runs_missing_manifest` — run dir without `manifest.json` is skipped.
- `discover_runs_corrupt_manifest` — run dir with invalid JSON in `manifest.json` is skipped.
- `discover_runs_populated` — two valid run dirs return summaries with `workflow`, `run_id`, `phase_status` populated.
- `discover_runs_non_directories_in_runs_dir` — files (e.g. `.DS_Store`) are skipped.
- `discover_runs_partial_phase_markers` — run with `phase-X.started` but no `.completed` reports `started` for that phase.

**Acceptance:**
```bash
cargo test -p loker -- ui::discovery::tests 2>&1 | tail -5
```

**Estimate:** M

---

### ST3 Write `ui::routes` module

**Files (new):**
- `src/ui/routes.rs` — daemon route handlers

**Contracts:**
```rust
#[derive(Clone)]
pub struct AppState {
    pub project_root: PathBuf,
}

/// Build the daemon's top-level router.
///
/// - `GET /`  -> JSON runs list (via discovery::discover_runs)
pub fn ui_routes(project_root: PathBuf) -> Router;
```

**Key design decisions:**
- `GET /` returns `application/json` with a `Vec<RunSummary>`.
- `AppState` wraps only `project_root: PathBuf` in v0. Will grow in T-053.
- No T-051 gate route composition in v0 (gate routes deferred to T-053 alongside the sessions list). The `GET /gates/:phase` path is a documented extension point only.

**Tests to add (in `src/ui/routes.rs`):**
- `runs_list_returns_json_array` — call handler with a temp project root containing one run; assert status 200, content-type JSON, body is a single-element array.
- `runs_list_empty_when_runs_missing` — with no `runs/` dir; body is `[]`.

**Acceptance:**
```bash
cargo test -p loker -- ui::routes::tests 2>&1 | tail -5
```

**Estimate:** S

---

### ST4 Write `ui::serve` module

**Files (new):**
- `src/ui/serve.rs` — daemon bootstrap

**Contracts:**
```rust
pub async fn serve(bind: &str, project_root: PathBuf) -> Result<()>;

#[cfg(test)]
pub async fn spawn_test_daemon(project_root: PathBuf) -> (tokio::task::JoinHandle<()>, std::net::SocketAddr);
```

**Key design decisions:**
- Use `TcpListener::bind` then `axum::serve(listener, app).with_graceful_shutdown(...)`.
- Graceful shutdown listens for SIGINT (Ctrl-C) and SIGTERM (Unix only).
- Print "loker UI daemon listening on http://{addr}" to stderr on startup.
- `spawn_test_daemon` is a `#[cfg(test)]` helper that binds to port 0 and returns the join handle + bound address for use by integration tests.

**Acceptance:**
```bash
cargo test -p loker -- ui::serve::tests 2>&1 | tail -5
```

**Estimate:** S

---

### ST5 Wire up `Commands::Ui` in CLI

**Files (modify):**
- `src/main.rs` — add `mod ui;` and `Commands::Ui { serve, bind }` variant + match arm

**Changes:**
```rust
#[derive(Subcommand)]
enum Commands {
    // ... existing variants ...

    /// Start the loker UI daemon.
    Ui {
        /// Run as a long-lived daemon.
        #[arg(long)]
        serve: bool,

        /// Bind address. Defaults to localhost:8080.
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: String,
    },
}
```

Match arm:
```rust
Commands::Ui { serve, bind } => {
    if !serve {
        anyhow::bail!("use --serve to start the daemon; see loker ui --help");
    }
    let project_root = find_project_root()
        .ok_or_else(|| anyhow::anyhow!("no lok.toml found; run from a loker project directory"))?;
    ui::serve::serve(&bind, project_root).await?;
}
```

**Acceptance:**
```bash
cargo build 2>&1 | tail -3
cargo run -- ui --help 2>&1 | grep -q "serve"
```

**Estimate:** S

---

### ST6 Integration tests for daemon mode

**Files (new):**
- `tests/ui_daemon.rs` — integration tests

**Tests:**
- `daemon_serves_runs_list` — spawn daemon on port 0, create a fixture run dir, `GET /` returns JSON with that run's id.
- `daemon_returns_empty_list_when_no_runs` — empty project root; `GET /` returns `[]`.
- `daemon_custom_bind_address` — spawn daemon with explicit port; assert it responds on that port.
- `daemon_shuts_down_gracefully_on_sigint` — spawn daemon, trigger shutdown signal, assert task completes within 2s.
- `daemon_skips_corrupt_run_directory` — place a valid run and a corrupt run in `runs/`; `GET /` returns one summary and a second request also succeeds.

**Acceptance:**
```bash
cargo test --test ui_daemon 2>&1 | tail -5
```

**Estimate:** M

---

### ST7 Final `make check` pass

- Ensure `cargo fmt` produces no diffs.
- Ensure `cargo clippy` passes with no warnings.
- All unit + integration tests pass (existing + new).

```bash
make check 2>&1 | tail -10
```

If clippy flags minor issues (e.g., function naming, unnecessary derives), fix in this step.

**Estimate:** S

## Pre-merge gate

```bash
make check    # fmt + clippy + test
```

## Total estimate

| Sub-task | Estimate |
|----------|----------|
| ST1: Extract render_gate_view | S |
| ST2: ui::discovery | M |
| ST3: ui::routes | S |
| ST4: ui::serve | S |
| ST5: CLI wiring | S |
| ST6: Integration tests | M |
| ST7: make check | S |
| **Total** | **M (~1-2 sessions)** |

## Risks

- **ST1 extraction coupling**: If `render_gate_view` needs data beyond `GateConfig` (e.g., the `oneshot::Sender` path), the extraction may need to pass additional state. See Open Question §7 in the design doc. Flag if encountered — can add optional parameters.
- **Port 0 in CI**: Integration tests use `127.0.0.1:0` to avoid port collisions. The `spawn_test_daemon` helper returns the bound address. Ensure CI runners have no residual lock on the ephemeral port range.
- **Clippy strictness on new modules**: The `#![allow(dead_code)]` at the top of `main.rs` covers module declarations. New modules with `pub` items shouldn't trigger dead-code warnings, but `pub(crate)` items might. Watch for clippy during ST7.
- **Manifest schema in test fixtures**: Test fixtures write `manifest.json` by hand (not via `RunDir::create`). Must match the schema version used by the current `Manifest::from_json`. If the schema changes between T-052 and a later task, fixtures may need updating.
