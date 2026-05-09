# Pre-PR validation: clo-324

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-08
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [high] ST4 (phase-lock heartbeat exposure) not implemented
**Where:** `src/run_state/phase_lock.rs` (unchanged on this branch); `tests/ui_threat_model.rs` (no `lock_heartbeat_expiry_releases_lock` test)
**What:** Plan §ST4 ("Expose advisory-lock heartbeat expiry for testing") is a stated acceptance criterion of CLO-324 and is required to drive `T-LOCK-2`. The branch ships none of it: no `force_expire`/`heartbeat_deadline`/`tick_for_test` accessor, no test, and no diff in `phase_lock.rs`. The threat model's heartbeat-expiry scenario is therefore unverified by automation, which is the entire point of the close gate.
**Suggested fix:** Implement ST4 as planned — add a `#[cfg(test)] pub` (or `pub(crate)` behind `cfg(test)`) accessor that lets a test advance/expire the heartbeat, then add the missing `T-LOCK-2` integration test. If the work is being deferred, update the plan/design and Linear ticket so the close-gate scope is honest.

### F2 [medium] Several design §5 tests missing from `tests/ui_threat_model.rs`
**Where:** `tests/ui_threat_model.rs`
**What:** Design §5 enumerates the suite that closes M11. Comparing to the file: `loopback_bind_rejects_external_interface` (T-BIND-1), `gate_url_has_sufficient_entropy` (T-COOKIE-1/entropy), `concurrent_approval_honors_advisory_lock` (T-LOCK-1), `sse_rejects_cross_origin_request`, and `lock_heartbeat_expiry_releases_lock` (T-LOCK-2) are all absent. The threat-model summary table in `docs/threat-model.md` advertises these IDs as covered, so docs and code disagree.
**Suggested fix:** Either land the missing tests, or trim the design §5 list and `docs/threat-model.md` table so the documented coverage matches what the suite actually exercises. Don't ship the close gate with a coverage table that overstates reality.

### F3 [medium] SSE trace endpoint does not enforce same-origin
**Where:** `src/ui/routes.rs` (`run_trace_sse` handler)
**What:** Design §7 explicitly flagged "SSE Origin enforcement" as an open question; the code resolves it by doing nothing. `EventSource` cannot set custom headers, but the browser still attaches `Origin` (and `Sec-Fetch-Site`) on cross-origin SSE, so a check is feasible and matches the threat model's stance on CSRF on streaming endpoints.
**Suggested fix:** Apply a lightweight same-origin check on the SSE handler — either reuse `check_post_origin`-style logic (Origin in allow-list, or `Sec-Fetch-Site: same-origin`) or document explicitly in `docs/threat-model.md` why SSE is out of scope. Add `T-SSE-CSRF` test alongside whichever choice you make.

### F4 [low] `tower-http` dependency added but unused
**Where:** `Cargo.toml`
**What:** `tower-http = "0.6.10"` is declared but no `use tower_http::…` exists in `src/` or `tests/`. The design preferred avoiding new deps; this one buys nothing.
**Suggested fix:** Drop the dependency, or actually use it (e.g., `SetResponseHeaderLayer` instead of the hand-rolled `with_headers` wrapper). Dead deps inflate the supply-chain surface that this very ticket is trying to harden.

### F5 [low] New clippy warnings in test file
**Where:** `tests/ui_threat_model.rs` (top of file)
**What:** `axum::body::Body` and `axum::http::Request` are imported but unused, producing fresh `unused_imports` warnings. `make check` runs `cargo clippy`; if the Makefile passes `-D warnings` (or starts to), this branch breaks the gate it is supposed to defend.
**Suggested fix:** Remove the unused imports.

### F6 [low] Loose status assertion in `T-TRAVERSAL-1`
**Where:** `tests/ui_threat_model.rs::t_traversal_1`
**What:** The test accepts `400 || 404`. Axum's URL normalization is deterministic for `..` segments via the percent-decoded path, so the response is predictable; an `||` assertion masks regressions where the wrong layer rejects the request (e.g. router 404 vs. handler 400 — only one demonstrates the artefact resolver actually rejected).
**Suggested fix:** Pin the expected status. If both are legitimate today, split into two named tests (raw `..` segment vs. percent-encoded `..`) and assert the exact code per case.

### F7 [info] POST guard requires form encoding, not JSON as design states
**Where:** `src/ui/security.rs::PostGuardConfig::for_loopback`, design §3
**What:** Implementation requires `application/x-www-form-urlencoded` to match the `Form<>` extractor. The design doc says `application/json`. The code is correct for the actual handlers; the doc is stale.
**Suggested fix:** Update §3 of `docs/designs/clo-324-threat-model-suite.md` to reflect the form encoding (and note that JSON would only return after switching extractors). Drift between design and code rots faster than either alone.

## Verdict

**rework**

The hardening that *did* land — security headers, POST Origin/Content-Type guard with paired tests, the artefact resolver with traversal + symlink containment and a thorough unit suite — is well-engineered and largely sufficient for the threats it covers. But this is the M11 *close gate*, and the close gate's job is to prove the threat model is enforced. ST4 is entirely missing, ~5 design §5 tests aren't written (and the published threat-model table claims them anyway), SSE cross-origin remains an open §7 question rather than a resolved one, and a dead `tower-http` dep + new clippy warnings in the test file leave the merge gate brittle. Land the missing ST4 work and §5 tests (or shrink the design and Linear scope to match), pick a defensible SSE stance, and clean up the dep/clippy noise before merging.
