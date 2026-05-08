# Plan: CLO-324 Threat-model test suite (M11 close gate)

## Context

- **Design:** `docs/designs/clo-324-threat-model-suite.md`
- **Discovery:** `docs/discovery/clo-324.md`
- **Linear:** https://linear.app/cloud-ai/issue/CLO-324/t-055-threat-model-test-suite-m11-close-gate
- **Threat model:** `docs/security/2026-04-25-ui-threat-model.md`
- **PRD:** `docs/prds/clo-324-threat-model-suite.md`

---

## Sub-tasks

### ST1 Write top-level threat-model summary doc
**Files:** `docs/threat-model.md`
**Acceptance:** File exists, cross-links to `docs/security/2026-04-25-ui-threat-model.md`, covers trust boundaries / in-scope / out-of-scope per PRD.
**Estimate:** S

### ST2 Add shared security layers (`security.rs`) and unit tests
**Files:** `src/ui/security.rs` (new), `src/ui/mod.rs`, `src/hitl_server/routes.rs`
**Acceptance:** `cargo test ui::security::tests` passes; `POST` handlers reject wrong `Origin` and `Content-Type`; all responses carry the five required headers.
**Estimate:** M

### ST3 Add artefact route with traversal + symlink containment (`artefact.rs`)
**Files:** `src/ui/artefact.rs` (new), `src/ui/routes.rs`, `src/ui/mod.rs`
**Acceptance:** `cargo test ui::artefact::tests` passes; `GET /runs/:id/artefact/...` returns `404` for symlinks escaping run root and `400` for traversal attempts.
**Estimate:** M

### ST4 Expose advisory-lock heartbeat expiry for testing
**Files:** `src/run_state/phase_lock.rs`
**Acceptance:** `cargo test run_state::phase_lock::tests::heartbeat_expiry` passes; `force_expire` or short-TTL mechanism observable from test fixtures.
**Estimate:** S

### ST5 Write dedicated threat-model integration test suite
**Files:** `tests/ui_threat_model.rs`
**Acceptance:** `cargo test ui_threat_model` passes; covers loopback bind, gate URL entropy, replay rejection, concurrent lock, SSE cross-origin, path traversal, and security headers.
**Estimate:** M

### ST6 Wire artefact route and security layers into daemon + one-shot routers
**Files:** `src/ui/routes.rs`, `src/hitl_server/routes.rs`
**Acceptance:** `make check` green (fmt + clippy + all tests). Existing `tests/ui_daemon.rs` and `tests/hitl_server.rs` still pass.
**Estimate:** S

---

## Pre-merge gate

- `make check` (fmt + clippy + test)
- Manual: open `http://127.0.0.1:8080` after `cargo run --bin loker -- ui --serve`, verify security headers in browser Network panel.

---

## Risks

- **Browser SSE `Origin` header**: browsers do not send `Origin` on `EventSource`. The cross-origin SSE test may need to use `Sec-Fetch-Site` or a server-side origin inference; fallback is documenting the limitation.
- **`tower-http` availability**: if not in workspace, we hand-roll the header layer (extra ~30 lines, no new dep).
- **CSP vs inline templates**: if any existing template uses inline `<script>` or `<style>`, CSP must be relaxed with a hash/nonce or templates refactored out first.
