# PRD: CLO-321 — Daemon mode `loker ui --serve`

| Field | Value |
|-------|-------|
| Author | pi (discovery phase) |
| Status | Draft |
| Created | 2026-05-07 |
| Task | CLO-321 |
| Depends on | CLO-320 (T-051 per-gate axum server) |
| FR References | FR-24, FR-27, FR-28 |

## 1. Goal

Long-running `loker ui --serve` daemon that binds to a localhost port, discovers all run directories on the host, and presents a unified web surface. Reuses 100% of the per-gate route handlers from T-051 at a nested path. Graceful shutdown (SIGINT/SIGTERM). Structured logging.

## 2. Scope

### In scope
- New CLI subcommand `loker ui --serve [--bind 127.0.0.1:PORT]`. Default port 8080.
- Daemon discovers all run directories under `<project_root>/runs/`.
- `GET /` returns a JSON list of all runs with summary state (active, blocked, complete).
- Per-run gate views at `GET /gates/:phase` reuse the T-051 route handlers via router composition.
- Graceful shutdown via tokio signal handling (SIGINT, SIGTERM).
- Logging via `eprintln!` / structured format to stderr (no new logging dependency).
- Daemon survives individual run failures / corrupt run directories — errors are logged and skipped.
- `make check` green (fmt + clippy + test).

### Out of scope (deferred to T-053 / T-054 / T-055)
- HTML rendering of runs list / sessions list (left pane) — T-053.
- SSE tail-f of `trace.jsonl` — T-054.
- Threat-model test suite — T-055.
- Authentication, TLS, multi-host — not part of M11.

## 3. Acceptance criteria

1. `loker ui --serve` starts a daemon on `127.0.0.1:8080` (default) that responds to HTTP requests.
2. `GET /` returns a JSON array of all run directories discovered under `runs/`, each with `id`, `workflow`, `status`, `created_at`, `phase_status`.
3. A run directory added to `runs/` while the daemon is running is discoverable on the next request (the daemon re-scans on each `GET /` — no fs-notify in v1).
4. The daemon reuses the T-051 gate route handlers at `GET /gates/:phase` (or similar sub-path).
5. Sending SIGINT or SIGTERM shuts down the daemon gracefully within 1 second.
6. A corrupt or partially-written run directory does not crash the daemon — it is skipped with a logged warning.
7. Empty `runs/` directory returns `[]` rather than an error.
8. `--bind 127.0.0.1:9090` overrides the default port.
9. `make check` passes (fmt + clippy + test).

## 4. Design direction

### 4.1 Module layout

```
src/
  ui/
    mod.rs         # Re-exports
    serve.rs       # Daemon bootstrap: bind, serve, graceful shutdown
    routes.rs      # Daemon-specific routes (GET / runs list) + composition of hitl_server routes
    discovery.rs   # Run directory scanner: list runs/ dirs, load manifest, return summaries
  hitl_server/
    routes.rs      # Unchanged. Composed at a sub-path.
    one_shot.rs    # Unchanged.
    mod.rs         # Unchanged.
```

### 4.2 CLI addition (`src/main.rs`)

```rust
#[derive(Subcommand)]
enum Commands {
    // ... existing commands ...
    /// Start the loker UI daemon
    Ui {
        /// Start the daemon (required flag for future `loker ui <subcommand>`)
        #[arg(long)]
        serve: bool,

        /// Bind address (default: 127.0.0.1:8080)
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: String,
    },
}
```

### 4.3 Run discovery (`src/ui/discovery.rs`)

```rust
/// Summary of a single run for the runs list endpoint.
#[derive(Serialize)]
pub struct RunSummary {
    pub id: String,
    pub path: String,
    pub workflow: Option<String>,
    pub run_id: Option<String>,
    pub created_at: Option<String>,
    pub phase_status: HashMap<String, String>,
}

/// Scan <project_root>/runs/ and return summaries.
/// IO errors are logged; corrupt runs are skipped.
pub fn discover_runs(project_root: &Path) -> Vec<RunSummary>;
```

Pattern follows `src/commands/ls_blocked.rs::scan_blocked_runs()`. For each directory entry in `runs/`:
1. Attempt to read `manifest.json`.
2. Attempt to scan markers.
3. Build summary.
4. On any error, log and skip.

### 4.4 Daemon bootstrap (`src/ui/serve.rs`)

```rust
pub async fn serve(bind: &str) -> Result<()> {
    let app = ui_routes();
    let listener = TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;

    eprintln!("loker UI daemon listening on http://{addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    eprintln!("shutting down loker UI daemon");
}
```

### 4.5 Route composition (`src/ui/routes.rs`)

```rust
pub fn ui_routes(project_root: PathBuf) -> Router {
    let hitl_router = hitl_server::routes::router(/* ... */);

    Router::new()
        .route("/", get(runs_list))
        .nest("/gates", hitl_router)  // compose T-051 routes at /gates
        .with_state(project_root)
}
```

The runs list handler calls `discovery::discover_runs(&state)` and returns JSON.

Note: The T-051 `routes::router` takes an `AppState` built around a single `GateConfig` and a `oneshot::Sender`. For daemon mode, this is unused (the one-shot handler never triggers). The daemon should either:
- Create a no-op `AppState` with a dummy sender (the gate routes will fail if called, which is acceptable — they're for the one-shot path), OR
- Extract the route handlers into reusable functions and build a separate daemon router without the gate-specific state.

The approach chosen: **Extract the gate context rendering handlers** into free functions in `hitl_server::routes` that take `GateConfig` by value, and let both the one-shot router and the daemon router call them. This keeps the sharing contract (FR-27) while avoiding the need for dummy state in the daemon.

### 4.6 Graceful run-failure handling

```rust
fn discover_runs(root: &Path) -> Vec<RunSummary> {
    let runs_dir = root.join("runs");
    if !runs_dir.exists() {
        return vec![];
    }

    let mut runs = vec![];
    for entry in fs::read_dir(&runs_dir).unwrap_or_else(|e| {
        eprintln!("WARN: failed to read runs dir: {e}");
        return vec![];
    }) {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => { eprintln!("WARN: skipping entry: {e}"); continue; }
        };
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let dir_name = entry.file_name().to_string_lossy().to_string();

        let summary = match load_run_summary(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("WARN: skipping run {dir_name}: {e}");
                continue;
            }
        };
        runs.push(summary);
    }
    runs
}
```

## 5. Test plan

### Unit tests (`src/ui/discovery.rs`)
- `discover_runs_empty_runs_dir` — empty `runs/` returns `[]`.
- `discover_runs_missing_manifest` — run dir without `manifest.json` is skipped with a log warning.
- `discover_runs_corrupt_manifest` — run dir with invalid manifest JSON is skipped.
- `discover_runs_populated` — directory with valid run dirs returns correct summaries with workflow name and phase status.
- `discover_runs_non_directories_in_runs_dir` — files (not dirs) in `runs/` are silently skipped.

### Integration tests (`tests/ui_daemon.rs`)
- `daemon_serves_runs_list` — start daemon on free port, create a fixture run dir, `GET /` returns JSON with that run's id.
- `daemon_returns_empty_list_when_no_runs` — start daemon on free port, assert `GET /` returns `[]`.
- `daemon_shuts_down_gracefully_on_sigint` — start daemon, send Ctrl-C, assert server task completes within 2 seconds.
- `daemon_custom_bind_address` — start daemon with `--bind 127.0.0.1:9090`, assert it responds on port 9090.

### Regression
- `cargo test` all existing T-051 / hitl_server tests still pass.
- `make check` (fmt + clippy + test) passes.

## 6. Migration / rollout

- Additive only: new `src/ui/` module, extended CLI `Commands` enum.
- No changes to marker schema, manifest schema, or run directory layout.
- No feature flag required; the daemon is only active when `loker ui --serve` is invoked.
- Backward compatible: existing `loker run`, `loker resume`, and the one-shot fallback server continue to work unchanged.

## 7. Security / threat model

- **Binding:** `127.0.0.1` enforced by default. The `--bind` flag allows customisation but defaults to localhost-only (FR-28).
- **CSRF:** Not a concern for v0 read-only endpoint (`GET /` returns JSON). The gate interaction endpoints (`POST /approve`, `POST /reject`) are only at the per-gate sub-path, which is not exposed by the daemon's router in v0.
- **Path traversal:** Run directory names are read from filesystem and included in JSON responses. The daemon reads `manifest.json` via canonicalised paths; no user-supplied paths are used for file reads.
- **DoS:** A run directory with an enormous manifest.json could slow down `GET /`. Acceptable for v0 — run counts are bounded by host resources.
