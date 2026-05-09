# Pre-PR validation: clo-324

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-09
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [medium] Content-Type design/impl mismatch — design says JSON, code requires form-encoded
**Where:** `src/ui/security.rs:32-35` (and design `docs/designs/clo-324-threat-model-suite.md` §3)
**What:** The design says POST guard validates `Content-Type: application/json`. The implementation requires `application/x-www-form-urlencoded`. The implementation is correct because the gate handlers use `axum::Form` against an HTML form, but the design doc and threat-model description are now stale. A future reader reconciling the docs against the code will assume one is wrong; nothing tells them which.
**Suggested fix:** Update `docs/designs/clo-324-threat-model-suite.md` §3 (and the corresponding cell in `docs/threat-model.md`) to state `application/x-www-form-urlencoded` and explain it tracks the `Form` extractor. Optional: also accept `application/json` if a JSON gate API is planned, but only after that handler exists.

### F2 [medium] ST4 partially unimplemented — `phase_lock.rs` heartbeat expiry not exposed for tests
**Where:** `src/run_state/phase_lock.rs` (no diff vs `main`) and missing test `lock_heartbeat_expiry_releases_lock` in `tests/ui_threat_model.rs`
**What:** Plan sub-task ST4 calls for a `force_expire` / observable `heartbeat_deadline` accessor and a dedicated test asserting the lock releases when its heartbeat lapses. The diff stat shows `phase_lock.rs` was not touched. Functional coverage is partially salvaged by `t_lock_2_stale_lock_reclaimable` (writing a stale-TTL lock file directly), and the existing `stale_lock_by_ttl_is_reclaimable` / `stale_lock_with_dead_pid_is_reclaimable` unit tests cover similar ground, but the design's named heartbeat test is absent. The threat-model.md row for `T-LOCK-1…3` overstates coverage.
**Suggested fix:** Either (a) implement ST4 — add `pub fn heartbeat_deadline(&self) -> Instant` (or `force_expire(&self)`) and write `lock_heartbeat_expiry_releases_lock` against the real lock — or (b) explicitly drop ST4 from the plan and update `docs/threat-model.md` §4 + the design's open questions list to record that the existing stale-lock-by-TTL test is the chosen coverage.

### F3 [low] Symlink response is 403, design specifies 404
**Where:** `src/ui/routes.rs` `get_artefact` handler (the `ArtefactError::Symlink` arm)
**What:** Design §3 ("symlink-pointing-outside-root returns 404") asks for 404 to avoid confirming the symlink's existence to an attacker. The handler currently maps `Symlink` to `StatusCode::FORBIDDEN`. Functionally identical from a security standpoint, but it diverges from the documented contract and the verbiage suggests the design's information-leak concern was conscious.
**Suggested fix:** Either change the mapping to `StatusCode::NOT_FOUND` (matching `Traversal` semantics — both are "no such artefact" from the client's view) or amend the design doc to record that 403 was chosen deliberately. Pick one source of truth.

### F4 [low] Security headers applied inline rather than via a layer — risk of future omission
**Where:** `src/ui/security.rs:9-10` module comment; every handler in `src/ui/routes.rs` and `src/hitl_server/routes.rs` that calls `add_security_headers(&mut response)`
**What:** The design §4 specified `tower_http::set_header::SetResponseHeaderLayer` so headers attach unconditionally to every response. The implementation chose inline calls per handler ("avoiding complex axum 0.7 type gymnastics"). That's a defensible call, but a future contributor adding a new route won't get a compile error if they forget — they'll get a silently insecure endpoint. There is no test that *enumerates* routes and asserts each one carries the headers; tests cover specific endpoints by name.
**Suggested fix:** Add a "header-coverage" test that walks a list of every registered path and asserts CSP/XFO/CORP/Referrer/XCTO are present. Or revisit the layer approach now that the routes are stable — `axum::middleware::from_fn` is much simpler than a typed tower layer and avoids the manual call sites entirely.

### F5 [low] T-CSP-1 only verifies the header, not that the SPA renders under it
**Where:** `tests/ui_threat_model.rs` (the `t_csp_1_*` test)
**What:** The CSP test asserts the `Content-Security-Policy` response header is set to the expected string, but does not exercise the served HTML/JS to confirm the dashboard actually loads under `script-src 'self'; style-src 'self'`. If a template ever inlines a script or style tag, browsers will block it but the test will stay green.
**Suggested fix:** Either add an assertion that fetches `GET /` and greps the body for any `<script>...inline...</script>` or `style="..."` constructs (cheap, brittle), or add a Playwright/headless-browser smoke check to the M11 close-gate (heavier, covered properly). For this PR, a simple "no inline `<script>` or `style=` in any served template" string check is enough.

## Verdict

**approve_with_changes** — The security posture is solid: Origin/Content-Type guards reject every CSRF vector exercised by the suite, the artefact resolver chains run_id sanitisation, percent-decode, component-level traversal checks, symlink-walk, and a canonical-prefix check, and the loopback bind warning, SSE Origin defence-in-depth, and 5-header response set are all in place and tested. The integration suite covers every threat row in `docs/threat-model.md` §4 with the documented test IDs. Two issues warrant a follow-up before merge: the ST4 heartbeat-exposure work was skipped and partially papered over with a stale-lock-file test, and the design doc has drifted from the implementation in three places (Content-Type, symlink status code, inline-vs-layer headers). None of these block the security guarantees, but reconciling the docs and either implementing or formally dropping ST4 will keep the threat model honest. The remaining low-severity findings (header-coverage test, CSP-vs-template verification) are good follow-ups but not gating.
