# Design: CLO-324 - Threat-model test suite (M11 close gate)

## 1. Problem

Per the discovery report (`docs/discovery/clo-324.md`), users running `loker ui --serve` to review HITL gates in a browser are exposed to a UI daemon whose security mitigations are enumerated in `docs/security/2026-04-25-ui-threat-model.md` but only partially enforced and largely untested: POST handlers accept any `Origin`, no `GET /runs/:id/artefact/:path` route exists, no security response headers are emitted, and the advisory phase-lock heartbeat expiry has no UI-layer test. M11 close is gated on locking this posture in (roadmap Phase 12) so security regressions in routes, SSE, and lock handling cannot slip into releases unnoticed.

## 2. Goals / Non-goals

### Goals

- Top-level `docs/threat-model.md` summarising trust boundaries, in-scope assets, and out-of-scope deployments (multi-user hosts), cross-linked to `docs/security/2026-04-25-ui-threat-model.md`.
- Dedicated `tests/ui_threat_model.rs` integration test file with 1:1 mapping to the §5 test list of the threat model doc, covering: loopback-only bind, per-gate URL entropy, replay rejection after gate resolution, concurrent approval honoring advisory lock, SSE cross-origin rejection, and `/runs/:id` path traversal rejection.
- Production hardening additions sufficient to make the new tests pass:
  - `GET /runs/:id/artefact/:path` route on the daemon router with traversal + symlink containment checks.
  - `Origin` and `Content-Type` validation on POST handlers shared by `src/ui/routes.rs` and `src/hitl_server/routes.rs`.
  - Security response headers (CSP, X-Frame-Options, CORP, Referrer-Policy) applied to all responses.
  - Advisory-lock heartbeat expiry observable through the UI layer for testing.
- Suite executes by default under `make check` (no opt-in env var).

### Non-goals

- External penetration test (PRD non-goal).
- Multi-tenant / shared-host hardening (out of scope per existing threat model).
- Rewriting security checks as a free-standing middleware crate or refactoring the seven existing routes into a middleware-first architecture (Approach C, rejected).
- Adding logprobs- or semantic-similarity-based verification (design doc §10 non-goal).
- Changing the on-disk run layout, lock format, or SSE wire format.

## 3. Architecture

The change is additive: one new route, one shared response-header layer, two shared validators on POST handlers, and a heartbeat-expiry hook in the lock module. No existing module is restructured.

```
                                +--------------------------+
                                |   tests/ui_threat_model  |
                                |   (axum::Server fixture) |
                                +-----------+--------------+
                                            | HTTP / SSE
                                            v
       +------------------------------------+------------------------------------+
       |                              src/ui/routes.rs                            |
       |   Router::new()                                                          |
       |     .route("/runs",                       get(list_runs))                |
       |     .route("/runs/:id",                   get(get_run))                  |
       |     .route("/runs/:id/artefact/*path",    get(get_artefact))   <-- NEW   |
       |     .route("/runs/:id/gates/:phase",      post(post_decision))           |
       |     .route("/runs/:id/events",            get(sse_events))               |
       |     .layer(security_headers_layer())                            <-- NEW   |
       |     .layer(post_guard_layer())            (Origin/CT validation) <-- NEW |
       +------------------------------------+------------------------------------+
                                            |
              +-----------------------------+-----------------------------+
              v                             v                             v
   +----------+----------+      +-----------+----------+      +-----------+----------+
   | run_state::layout   |      | run_state::phase_lock|      | ui::sse              |
   | resolve_artefact    |      | LockGuard + heartbeat|      | same-origin streams  |
   | (traversal + symlink|      | observable expiry    |      | no ACAO header       |
   |  containment)       |      |                      |      |                      |
   +---------------------+      +----------------------+      +----------------------+
```

### Module map

- `src/ui/routes.rs` (existing): registers the new artefact route, applies the two new layers. Handler signatures preserved for the existing seven routes.
- `src/ui/security.rs` (new, private): `security_headers_layer()` and `post_guard_layer()` constructors returning `tower::Layer` values; constants for the header set and the allowed `Origin` policy (loopback-only). Reused by `src/hitl_server/routes.rs`.
- `src/ui/artefact.rs` (new, private): `resolve_artefact(run_root: &Path, run_id: &str, rel: &str) -> Result<PathBuf, ArtefactError>`. Joins `run_root`, canonicalises, and refuses any result whose canonical form does not start with the canonical `run_root`. Symlink traversal is rejected by canonicalisation followed by prefix check.
- `src/run_state/phase_lock.rs` (existing): stale-lock reclaim is tested via the existing TTL + PID-liveness unit tests (`stale_lock_by_ttl_is_reclaimable`, `stale_lock_with_dead_pid_is_reclaimable`). The integration test `t_lock_2_stale_lock_reclaimable` in `tests/ui_threat_model.rs` covers the same behaviour at the UI layer.
- `src/hitl_server/routes.rs` (existing): import the same `security_headers_layer` and `post_guard_layer` so the one-shot fallback server cannot drift from the daemon's posture.
- `tests/ui_threat_model.rs` (new): one `#[tokio::test]` per §5 row from the threat model doc, plus a small `ThreatModelFixture` that boots the daemon on `127.0.0.1:0`, prepares a run directory, and tears down on drop.

### Data flow

1. Browser issues a request against `127.0.0.1:<port>`.
2. `post_guard_layer` rejects POSTs whose `Origin` is not `http://127.0.0.1:<port>` or `http://localhost:<port>`, or whose `Content-Type` is not `application/x-www-form-urlencoded`. SSE and `GET` traffic is unaffected.
3. The handler runs. For the artefact route, `resolve_artefact` canonicalises and prefix-checks the path; symlinks return `403 Forbidden`.
4. `security_headers_layer` decorates every response with `Content-Security-Policy: default-src 'self'`, `X-Frame-Options: DENY`, `Cross-Origin-Resource-Policy: same-origin`, `Referrer-Policy: no-referrer`, and `X-Content-Type-Options: nosniff`.
5. SSE `/events` continues to omit `Access-Control-Allow-Origin`; the test confirms cross-origin browsers cannot consume the stream.

### Concrete Rust types

```rust
pub(crate) struct ArtefactPath(PathBuf);

#[derive(Debug, thiserror::Error)]
pub(crate) enum ArtefactError {
    #[error("artefact path escapes run root")]
    Traversal,
    #[error("artefact not found")]
    NotFound,
    #[error("artefact io error: {0}")]
    Io(#[from] std::io::Error),
}

pub(crate) struct PostGuardConfig {
    pub allowed_origins: Vec<HeaderValue>,
    pub required_content_type: HeaderValue,
}

pub(crate) struct SecurityHeaders {
    pub csp: HeaderValue,
    pub x_frame_options: HeaderValue,
    pub corp: HeaderValue,
    pub referrer_policy: HeaderValue,
    pub x_content_type_options: HeaderValue,
}
```

## 4. Public API surface

The library's existing public surface in `src/lib.rs` is unchanged. The hardening lives in `pub(crate)` modules under `src/ui/`. The signatures below are what the daemon and the hitl_server reuse.

```rust
// src/ui/security.rs

use axum::http::HeaderValue;
use tower::Layer;

pub(crate) fn security_headers_layer()
    -> tower_http::set_header::SetResponseHeaderLayer<HeaderValue>;

pub(crate) fn post_guard_layer(config: PostGuardConfig)
    -> impl Layer<axum::routing::Route> + Clone + Send + Sync + 'static;

pub(crate) struct PostGuardConfig {
    pub allowed_origins: Vec<HeaderValue>,
    pub required_content_type: HeaderValue,
}

impl PostGuardConfig {
    pub(crate) fn for_loopback(port: u16) -> Self;
}
```

```rust
// src/ui/artefact.rs

use std::path::{Path, PathBuf};

pub(crate) fn resolve_artefact(
    run_root: &Path,
    run_id: &str,
    rel: &str,
) -> Result<PathBuf, ArtefactError>;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ArtefactError {
    #[error("artefact path escapes run root")]
    Traversal,
    #[error("artefact not found")]
    NotFound,
    #[error("artefact io error: {0}")]
    Io(#[from] std::io::Error),
}
```

```rust
// src/ui/routes.rs (additions only, existing signatures preserved)

pub(crate) fn router(state: UiState) -> axum::Router;

async fn get_artefact(
    State(state): State<UiState>,
    Path((run_id, rel)): Path<(String, String)>,
) -> Result<Response, ApiError>;
```

```rust
// src/run_state/phase_lock.rs (additions — none required; existing TTL + PID-liveness tests cover stale-lock reclaim)
```

## 5. Test plan

All new tests live in `tests/ui_threat_model.rs` and run under `cargo test` (and therefore `make check`). Function names match §5 of the threat model doc 1:1 so failures point straight at the row.

### Integration tests (`tests/ui_threat_model.rs`)

- `loopback_bind_rejects_external_interface` - boot daemon on `0.0.0.0:0`, assert it logs the WARN and that any non-loopback peer is refused at handler layer.
- `gate_url_has_sufficient_entropy` - sample N gate URLs from the daemon, assert path-token component carries >= 128 bits of entropy (Shannon estimate over bytes).
- `replay_after_gate_resolved_is_rejected` - resolve a gate via POST, replay the same payload, expect `409 Conflict`.
- `concurrent_approval_honors_advisory_lock` - spawn two tokio tasks POSTing the same gate; assert exactly one returns `200`, the other returns `409`, and the on-disk `responses/<phase>.json` was written once.
- `sse_rejects_cross_origin_request` - open `/runs/:id/events` with `Origin: http://evil.test`, assert `403 Forbidden` and no event frames.
- `path_traversal_on_runs_id_is_rejected` - GET `/runs/..%2Fetc%2Fpasswd`, expect `400 Bad Request`; symlink-pointing-outside-root returns `403 Forbidden`.
- `post_without_origin_header_is_rejected` - POST with no `Origin`, expect `403`.
- `post_with_wrong_content_type_is_rejected` - POST `text/plain`, expect `415 Unsupported Media Type`.
- `responses_carry_security_headers` - GET `/runs`, assert all five required headers present with expected values.
- `lock_heartbeat_expiry_releases_lock` - not needed: the existing unit tests `stale_lock_by_ttl_is_reclaimable` and `stale_lock_with_dead_pid_is_reclaimable` in `src/run_state/phase_lock.rs` verify the underlying mechanism, and the integration test `t_lock_2_stale_lock_reclaimable` covers it at the UI layer.

### Unit tests

- `src/ui/artefact.rs::tests::canonical_path_inside_root_is_allowed`.
- `src/ui/artefact.rs::tests::dotdot_segment_is_rejected`.
- `src/ui/artefact.rs::tests::symlink_escaping_root_is_rejected` (uses `tempfile` + `std::os::unix::fs::symlink`; gated `#[cfg(unix)]`).
- `src/ui/artefact.rs::tests::nonexistent_path_returns_not_found`.
- `src/ui/security.rs::tests::post_guard_allows_loopback_origin`.
- `src/ui/security.rs::tests::post_guard_rejects_foreign_origin`.
- `src/ui/security.rs::tests::security_headers_layer_sets_all_five_headers`.
- `src/run_state/phase_lock.rs::tests::stale_lock_by_ttl_is_reclaimable`.
- `src/run_state/phase_lock.rs::tests::stale_lock_with_dead_pid_is_reclaimable`.

`wiremock` is not needed here - these tests exercise the daemon directly. No new dependency is required if `tower-http` is already in the workspace (verify in implementation, otherwise raise as an open question rather than adding silently).

### Manual verification

- `cargo run --bin loker -- ui --serve --bind 127.0.0.1:8080`, open `http://127.0.0.1:8080`, confirm Network panel shows the five security headers on every response.
- From a second-origin local page (`python3 -m http.server 9999`) attempt `fetch('http://127.0.0.1:8080/...', { method: 'POST', body: '{}' })` and confirm browser-blocked or 403.
- `curl -i http://127.0.0.1:8080/runs/$RUN/artefact/../../../etc/passwd` returns `400`.

## 6. Migration / rollout

Nothing to migrate on disk or in `lok.toml`: the run directory layout, lock file format, and SSE wire format are unchanged. The new `GET /runs/:id/artefact/*path` route is purely additive; existing browsers and CLI clients ignore it.

No feature flag. The hardening tightens server behaviour (Origin/Content-Type checks, security headers); the targets are browser clients on loopback, which all send compliant `Origin` headers and `application/x-www-form-urlencoded` bodies. The one-shot HITL fallback server (`src/hitl_server/routes.rs`) picks up the same layers in the same change so the two surfaces cannot drift.

Rollout order within the PR:

1. Land `docs/threat-model.md` (read-only, no code).
2. Add `src/ui/security.rs`, `src/ui/artefact.rs`, and the `phase_lock` accessors with their unit tests (TDD).
3. Wire the layers and the artefact route into `src/ui/routes.rs` and `src/hitl_server/routes.rs`.
4. Add `tests/ui_threat_model.rs` and confirm `make check` is green.

If post-merge a real browser breaks on the stricter POST guard, the rollback is reverting the `post_guard_layer` registration line; the layer itself stays in tree but unused. No data migration is involved either way.

## 7. Open questions

- **Gate URL entropy threshold**: PRD says "unguessable", threat model doc does not pin a bit count. The design §4 explicitly rejected URL tokens in v0; entropy comes from the run_id path component. The integration test `t_entropy_1_gate_url_uses_run_id` documents this choice.
- **CSP strictness**: `default-src 'self'` blocks any inline script. The current daemon templates need to be audited for inline `<script>`/`<style>`; if any exist, the choice is between refactoring them out or relaxing CSP with a nonce. The doc lists this as a v0 question.
- **Threat-model doc location**: PRD asks for `docs/threat-model.md`; an existing detailed doc lives at `docs/security/2026-04-25-ui-threat-model.md`. The new file is a short summary that links to the dated doc.
