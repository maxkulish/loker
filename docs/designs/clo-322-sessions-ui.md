# Design Document: CLO-322 — Sessions UI

**Status**: Draft
**Task**: [T-053] Sessions list + per-run trace + pending panel
**Approach**: Askama compile-time templates on axum 0.7
**Created**: 2026-05-07

---

## 1. Problem

Lok users lack a visual interface to browse workflow runs and resolve
human-in-the-loop gates. The T-052 daemon (`loker ui --serve`) runs an
axum server on localhost with a `GET /` returning JSON, but there is no
HTML rendering. Users must `cat manifest.json` and manually tail trace
files to inspect runs.

This task builds three server-rendered HTML views on top of the existing
T-052 daemon infrastructure:
1. **Index (`/`)** — sessions table (id, workflow, status, timestamps,
   current phase)
2. **Run detail (`/runs/:id`)** — manifest entries, phase timeline, last N
   trace events
3. **Pending panel (`/pending`)** — all active HITL gates with approve/reject
   forms

Server-rendered HTML only. No JavaScript framework. Minimal CSS. Localhost-only.

## 2. Goals / Non-goals

### Goals

- Replace `GET /` JSON response with HTML sessions table
- Add `GET /runs/:id` per-run detail page
- Add `GET /pending` aggregated HITL gates panel
- Mount T-051 approve/reject POST handlers in the daemon router
- Snapshot tests (insta) on rendered HTML for all three views
- Minimal CSS, no external stylesheet dependencies

### Non-goals

- SSE live updates (T-054)
- Auth, theming, mobile layout
- Client-side JavaScript
- Changing the existing T-051 one-shot server behavior

## 3. Architecture

### 3.1 Module layout

```
src/ui/
  mod.rs          # re-exports: serve, discovery, routes, templates
  serve.rs        # unchanged — daemon bootstrap
  discovery.rs    # EXTENDED — RunSummary, discover_runs() (minor: export helpers)
  routes.rs       # EXTENDED — new HTML routes + T-051 gate mount
  templates.rs    # NEW — Askama template structs
  manifest.rs     # NEW — shared manifest.json parsing (used by both discovery + routes)
  gate_discovery.rs  # NEW — scan all runs for pending/*.json

templates/        # NEW directory
  index.html      # sessions/runs table
  run_detail.html # per-run manifest + timeline + trace
  pending.html    # pending gates aggregate panel
  error.html      # 404 / 500 error responses
  layout.html     # shared <head>/<header>/<footer>
```

### 3.2 Route changes

Current router (`routes.rs`):
```
GET /  → runs_list() → JSON
```

New router:
```
GET /            → index_page()     → HTML (Askama)
GET /health      → health_check()   → 200 OK plain text
GET /runs/:id    → run_detail()     → HTML (Askama) — sanitizes :id
GET /pending     → pending_panel()  → HTML (Askama)
POST /gates/:run_id/:phase/approve  → hitl_approve()
POST /gates/:run_id/:phase/reject   → hitl_reject()
```

The `/gates/...` routes are thin wrappers around the existing T-051
`approve()`/`reject()` handlers from `src/hitl_server/routes.rs`,
adapted to resolve gate configs from the daemon's `AppState`.

### 3.3 AppState

```rust
#[derive(Clone)]
pub struct AppState {
    /// Project root — passed to discovery and gate resolution.
    pub project_root: PathBuf,
    /// Max trace events to display per run detail page.
    pub max_trace_events: usize,  // default 50
}
```

### 3.4 Data flow

```
Browser GET /
  → index_page()
    → discovery::discover_runs(&project_root)
    → Vec<RunSummary>
    → IndexTemplate { runs } → askama::Template::render()
    → Html<String> → 200 response

Browser GET /runs/:id
  → run_detail(id)
    → resolve run_dir = project_root/runs/:id
    → read manifest.json → ManifestData
    → scan markers/ → phase timeline
    → tail trace.jsonl (last N lines) → Vec<TraceLine>
    → RunDetailTemplate { ... } → render → 200

Browser GET /pending
  → pending_panel()
    → gate_discovery::discover_pending_gates(&project_root)
    → Vec<PendingGate>
    → PendingTemplate { gates } → render → 200

Browser POST /gates/:run_id/:phase/approve
  → hitl_approve(run_id, phase)
    → resolve GateConfig from run_id
    → hitl_server::routes::approve(State, Form) logic
    → redirect to /pending (303 See Other)
```

### 3.5 Askama template structs

```rust
use askama::Template;

#[derive(Template)]
#[template(path = "index.html")]
pub struct IndexTemplate {
    pub runs: Vec<RunSummary>,
    pub active_milestone: String,
}

#[derive(Template)]
#[template(path = "run_detail.html")]
pub struct RunDetailTemplate {
    pub run_id: String,
    pub workflow: Option<String>,
    pub manifest_entries: Vec<ManifestEntry>,
    pub phase_timeline: Vec<PhaseStep>,
    pub trace_events: Vec<TraceEventDisplay>,
}

#[derive(Template)]
#[template(path = "pending.html")]
pub struct PendingTemplate {
    pub gates: Vec<PendingGateDisplay>,
}

/// Error page template data for 4xx/5xx responses.
#[derive(Template)]
#[template(path = "error.html")]
pub struct ErrorTemplate {
    pub status_code: u16,
    pub message: String,
}
```

## 4. Public API surface

### 4.1 New public types

```rust
/// A pending HITL gate discovered in a run directory.
#[derive(Debug, Clone, Serialize)]
pub struct PendingGate {
    pub run_id: String,
    pub run_dir: PathBuf,
    pub phase: String,
    pub workflow: String,
    pub severity: String,
    pub artefact_path: String,
    pub pending_file_path: PathBuf,
}

/// Display-friendly pending gate for Askama template rendering.
/// Subset of PendingGate — no filesystem paths exposed to templates.
#[derive(Debug, Clone, Serialize)]
pub struct PendingGateDisplay {
    pub run_id: String,
    pub phase: String,
    pub workflow: String,
    pub severity: String,
    pub artefact_path: String,
}

/// Display-friendly trace event for Askama template rendering.
#[derive(Debug, Clone, Serialize)]
pub struct TraceEventDisplay {
    pub timestamp: String,
    pub event_type: String,
    pub summary: String,
}

/// Entry from manifest.json.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestEntry {
    pub name: String,
    pub kind: String,
    pub schema_version: u32,
    pub sha256: Option<String>,
}

/// Phase timeline step.
#[derive(Debug, Clone, Serialize)]
pub struct PhaseStep {
    pub name: String,
    pub status: String,       // "completed", "started", "failed", "pending"
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}
```

### 4.2 New public functions

```rust
/// Scan all run directories for pending/<phase>.json files.
/// Returns gates sorted by severity (high → medium → low).
pub fn discover_pending_gates(project_root: &Path) -> Vec<PendingGate>;

/// Tail the last N lines of a trace file. Returns empty vec if file
/// missing or unreadable.
pub fn tail_trace_file(trace_path: &Path, n: usize) -> Vec<TraceEventDisplay>;

/// Read manifest entries from a manifest.json file.
pub fn read_manifest_entries(manifest_path: &Path) -> Vec<ManifestEntry>;

/// Build phase timeline from markers/ directory + manifest.json.
pub fn build_phase_timeline(run_dir: &Path) -> Vec<PhaseStep>;
```

### 4.3 Modified route function signatures

```rust
// routes.rs — replaces existing runs_list
pub fn ui_routes(project_root: PathBuf) -> Router;

// New handlers
async fn index_page(State(state): State<AppState>) -> impl IntoResponse;
async fn health_check() -> impl IntoResponse;
async fn run_detail(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> impl IntoResponse;
async fn pending_panel(State(state): State<AppState>) -> impl IntoResponse;
async fn hitl_approve(
    State(state): State<AppState>,
    Path((run_id, phase)): Path<(String, String)>,
    Form(form): Form<DecisionForm>,
) -> impl IntoResponse;
async fn hitl_reject(
    State(state): State<AppState>,
    Path((run_id, phase)): Path<(String, String)>,
    Form(form): Form<DecisionForm>,
) -> impl IntoResponse;
```

## 5. Test plan

### 5.1 Unit tests

| Test | What it verifies |
|------|-----------------|
| `discover_pending_gates_empty` | Empty or missing runs dir yields no gates |
| `discover_pending_gates_populated` | Two runs, one with pending/review.json → one gate returned |
| `discover_pending_gates_sorted` | High-severity gates appear before medium |
| `tail_trace_file_empty` | Missing file returns empty vec |
| `tail_trace_file_last_n` | 100-line file tailed to 10 returns exactly 10 |
| `tail_trace_file_truncated` | 5-line file tailed to 10 returns 5 |
| `read_manifest_entries_valid` | Valid manifest.json → correct entries |
| `read_manifest_entries_corrupt` | Invalid JSON → empty vec (no panic) |
| `build_phase_timeline_states` | Markers map to correct phase states |
| `health_check_returns_200` | GET /health → 200 OK |

### 5.2 Integration / snapshot tests (insta)

| Test | What it verifies |
|------|-----------------|
| `index_page_renders_runs_table` | Fixture with 2 runs → HTML table with correct metadata |
| `index_page_empty_state` | No runs → "No runs yet" message |
| `run_detail_page_renders_all_sections` | Fixture run → manifest, timeline, trace all present |
| `run_detail_page_unknown_run` | Invalid run_id → 404 with helpful message |
| `pending_panel_renders_gates` | One active gate → form with approve/reject visible |
| `pending_panel_empty_state` | No pending gates → "All clear" message |
| `approve_post_writes_response` | POST to /gates/.../approve → response.json written |
| `reject_post_writes_response` | POST to /gates/.../reject → response.json written |
| `approve_post_redirects_to_pending` | POST → 303 See Other to /pending |
| `hitl_approve_race_guard_conflict` | Second POST after first → 409 Conflict |

### 5.3 Manual verification

1. Start daemon: `cargo run -- ui --serve`
2. Open `http://localhost:XXXX/` — verify sessions table
3. Click a run → verify detail page with timeline + trace
4. Open `http://localhost:XXXX/pending` — verify empty state
5. Create a pending gate (via one-shot) → verify it appears
6. Click Approve → verify gate disappears, response written

### 5.4 Regression checks

- Existing `serve::spawn_test_daemon` integration test still passes
- T-051 one-shot HITL server routes unchanged

## 6. Migration / rollout

### Dependencies to add

```toml
# Cargo.toml additions
askama = { version = "0.12", features = ["with-axum"] }
```

`askama_axum` is auto-enabled via the `with-axum` feature in askama 0.12+.

### Build configuration

`askama` compiles templates at build time from the `templates/` directory.
No `build.rs` needed — the `#[template(path = "...")]` attribute macro
resolves paths relative to a configurable `template_dir`. Set in
`askama.toml` or via the `Template` derive attribute:

```rust
#[derive(Template)]
#[template(path = "index.html")]
// template_dir defaults to "templates/"
```

### Files changed / added

| File | Action | Description |
|------|--------|-------------|
| `Cargo.toml` | Edit | Add `askama` dependency |
| `src/ui/discovery.rs` | Edit | Export shared manifest helpers |
| `src/ui/routes.rs` | Edit | Replace JSON handler, add HTML + gate routes |
| `src/ui/templates.rs` | Create | Askama template structs |
| `src/ui/manifest.rs` | Create | Shared manifest.json parsing |
| `src/ui/gate_discovery.rs` | Create | Pending gate scanning |
| `templates/index.html` | Create | Sessions table template |
| `templates/run_detail.html` | Create | Per-run detail template |
| `templates/pending.html` | Create | Pending gates panel template |
| `templates/error.html` | Create | Error responses (404, 500) |
| `templates/layout.html` | Create | Shared HTML structure |
| `src/ui/trace_reader.rs` | Create | tail_trace_file (last N trace events) |

### Rollout

- Single-branch PR against `main`
- Passes `make check` (rustfmt, clippy, test with no network)
- No breaking changes to CLI or daemon startup

## 7. Open questions

1. **Should `/` content-negotiate?** The old `/` returned JSON. Should it
   honor `Accept: application/json` and return JSON, or is the transition
   to HTML-only acceptable? *Decision: HTML-only. The JSON data was only
   used by the test suite; the daemon UI is the intended consumer.*

2. **Static CSS: inline vs. `/static/style.css`?** Inline in each template
   avoids extra route complexity. A separate `/static/style.css` route is
   standard but adds an axum `get_service`. *Decision: inline `<style>` in
   the layout template. Minimal CSS (~100 lines), negligible bytes-on-wire.
   Can be extracted later if CSS grows.*

3. **Trace event formatting.** Trace events follow OpenTelemetry GenAI
   conventions. Should the detail view render raw JSON or formatted
   key-value pairs? *Decision: formatted key-value pairs. Events are
   JSONL; display timestamp, event type, and a human-readable summary.*

4. **T-051 route sharing.** The `render_gate_view()` function is in
   `hitl_server/routes.rs`. For the pending panel, we need a lighter
   inline representation. Should we reuse `render_gate_view()` or create
   new rendering? *Decision: new rendering in Askama templates. The
   per-gate HTML form is too verbose for the aggregated panel.*

5. **Max trace events configurable?** The daemon currently has no
   configuration file. *Decision: hardcoded default (50) in AppState.
   Can be made configurable post-v0.*
