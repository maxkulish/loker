# Plan: CLO-322 — Sessions UI

## Context
- **Design**: `docs/designs/clo-322-sessions-ui.md`
- **Discovery**: `docs/discovery/clo-322.md`
- **PRD**: `docs/prds/clo-322-sessions-ui.md`
- **Linear**: https://linear.app/cloud-ai/issue/CLO-322/t-053-sessions-list-per-run-trace-pending-panel
- **Branch**: `feat/clo-322-sessions`
- **Approach**: Askama compile-time templates on axum 0.7

## Architecture overview

New files under `src/ui/`: `templates.rs`, `manifest.rs`, `gate_discovery.rs`, `trace_reader.rs`.
New `templates/` directory: `index.html`, `run_detail.html`, `pending.html`, `error.html`, `layout.html`.
Edited files: `Cargo.toml`, `src/ui.rs` (mod), `src/ui/routes.rs`, `src/ui/discovery.rs`.

## Sub-tasks

### ST1 Add askama dependency + templates scaffolding

**Files:**
- `Cargo.toml` — add `askama = { version = "0.12", features = ["with-axum"] }`
- `templates/layout.html` — shared HTML5 `<head>` + `<header>` + `<main>` + inline `<style>`
- `templates/error.html` — extends layout, shows `status_code` + `message`
- `src/ui/mod.rs` — add `pub mod templates;` and `pub mod gate_discovery;`

**Acceptance:** `cargo check` compiles.
**Estimate:** S

---

### ST2 Shared manifest parsing (`manifest.rs`)

**Files:**
- `src/ui/manifest.rs` — `read_manifest_entries(path) -> Vec<ManifestEntry>` and `build_phase_timeline(run_dir) -> Vec<PhaseStep>`
- `src/ui/discovery.rs` — export helpers so `load_run_summary` can call shared `read_manifest_entries`

**Tests:** `test_read_manifest_entries_valid` parses a fixture, `test_read_manifest_entries_corrupt` returns empty vec, `test_build_phase_timeline_states` maps markers, `test_build_phase_timeline_empty_markers` returns empty.

```bash
cargo test -p loker -- ui::manifest::tests
```

**Estimate:** S

---

### ST3 Trace reader (`trace_reader.rs`) + gate discovery (`gate_discovery.rs`)

**Files:**
- `src/ui/trace_reader.rs` — `tail_trace_file(path, n) -> Vec<TraceEventDisplay>`. Reads file, returns last N lines. Graceful on missing/unreadable.
- `src/ui/gate_discovery.rs` — `discover_pending_gates(project_root) -> Vec<PendingGate>`. Scans `runs/*/pending/*.json` across all runs. Sorted by severity (high → medium → low).

**Tests:**
```bash
cargo test -p loker -- ui::trace_reader::tests
cargo test -p loker -- ui::gate_discovery::tests
```

**Estimate:** S

---

### ST4 Askama template structs + index template

**Files:**
- `src/ui/templates.rs` — `IndexTemplate`, `RunDetailTemplate`, `PendingTemplate`, `ErrorTemplate` derive Askama. All public types (`ManifestEntry`, `PhaseStep`, `TraceEventDisplay`, `PendingGateDisplay`, `ErrorTemplate`).
- `templates/index.html` — extends `layout.html`. Table of runs: id, workflow, status, phase_status, timestamps. Empty state: "No runs yet."
- `templates/run_detail.html` — extends `layout.html`. Three sections: manifest entries table, phase timeline, last N trace events.
- `templates/pending.html` — extends `layout.html`. Form per gate with context summary + approve/reject `<button>`. Empty state: "All gates resolved."

**Acceptance:** `cargo check` compiles templates at build time. Templates render with fixture data. Snapshot tests will come in ST5 when routes are wired.
**Estimate:** M

---

### ST5 HTML routes + existing route integration

**Files:**
- `src/ui/routes.rs` — **Replace** `GET /` JSON handler with HTML `index_page`. Add `GET /health` (200 OK). Add `GET /runs/:id` → `run_detail` (sanitize `run_id`, reject `..` `/` empty). Add `GET /pending` → `pending_panel` (uses `discover_pending_gates`). Use `spawn_blocking` for sync I/O calls. Wire T-051 approval handler helpers.
- `src/ui/discovery.rs` — minor tweaks to export helpers already done in ST2.

**Tests (unit):**
```bash
cargo test -p loker -- ui::routes::tests
```

**Snapshot tests** — run with `cargo insta test` to generate snapshots for:
- `index_page_renders_runs_table` — 2 fixture runs
- `index_page_empty_state` — no runs
- `run_detail_page_renders_all_sections` — fixture run with manifest + markers + trace
- `run_detail_page_unknown_run` — invalid run_id → 404
- `pending_panel_renders_gates` — one active pending gate
- `pending_panel_empty_state` — no pending gates
- `health_check_returns_200`

**Estimate:** L

---

### ST6 HITL gate routes (approve/reject)

**Files:**
- `src/ui/routes.rs` — add `POST /gates/:run_id/:phase/approve` and `POST /gates/:run_id/:phase/reject`. Thin wrappers around `hitl_server::routes::approve`/`reject`, adapted to resolve `GateConfig` from `run_id` and daemon's `AppState`. Redirect 303 to `/pending` on success.

**Tests:**
```bash
cargo test -p loker -- ui::routes::tests::gate_
```

Specific tests: `approve_post_writes_response`, `reject_post_writes_response`, `approve_post_redirects_to_pending`, `hitl_approve_race_guard_conflict`.

**Estimate:** M

---

## Dependency graph

```
ST1 (cargo + layout)          ← foundation
  ├── ST2 (manifest parsing)  ← no deps on other new code
  ├── ST3 (trace + gates)     ← no deps on other new code
  └── ST4 (templates)         ← needs ST1 (askama dep, layout template)
        └── ST5 (HTML routes) ← needs ST2 + ST3 + ST4
              └── ST6 (gate POST) ← needs ST5
```

ST2 and ST3 are fully parallel. ST4 depends on ST1 only. The critical path is `ST1 → ST4 → ST5 → ST6`.

## Pre-merge gate

```bash
make check
```

This runs: `cargo fmt --check` + `cargo clippy -- -D warnings` + `cargo test --no-fail-fast` (with `insta` snapshot review).

## Risks

1. **Askama version compatibility.** Askama 0.12 requires a specific axum version. If `axum 0.7` in Cargo.toml is incompatible, may need askama 0.11 or a different feature flag. Mitigation: verify in ST1.
2. **Template path resolution.** Askama expects templates at `templates/` relative to the crate root. The `src/ui/templates.rs` derive macros resolve paths at compile time. Verify in ST4.
3. **T-051 route sharing.** The `approve()`/`reject()` handlers in `hitl_server/routes.rs` use `GateConfig` which expects specific run-dir paths. The daemon's `AppState` needs to construct `GateConfig` from `run_id`. Mitigation: create a `resolve_gate_config(run_dir, phase, project_root)` helper in `gate_discovery.rs`.
4. **Existing integration test breakage.** `serve::spawn_test_daemon` tests expect JSON at `/`. After ST5, `/` returns HTML. These tests MUST be updated to expect HTML or use `Accept: application/json` negotiation. Mitigation: update the test to match the new HTML response.
5. **`cargo insta` review.** Snapshot tests require `cargo insta review` for first-run acceptance. All test fixtures must be deterministic (no timestamps, no UUIDs, no random data).
