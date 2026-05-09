# Pre-PR validation: clo-324

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-08
**Pipeline**: lok implement-gate
---

Verification complete. All Claude fallback findings hold up: no `src/run_state/` changes (ST4 unimplemented), 5 design §5 tests genuinely missing from `tests/ui_threat_model.rs`, `run_trace_sse` has no Origin/Sec-Fetch-Site enforcement, `tower-http` declared in `Cargo.toml` but zero `tower_http::` imports anywhere, and `axum::body::Body` + `axum::http::Request` imports at lines 8-9 are unused.

## Reviewer Status

| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Wrapper script broke at shell parse time — heredoc nested inside `OUTPUT=$(... <<EOF ... EOF)` with an unescaped `'%s'` literal closed the outer single-quoted heredoc. No model invocation occurred. |
| Gemini | REVIEW_FAILED | Same wrapper-script defect (`printf '%s'` inside the outer single-quoted heredoc). Both primary and fallback model paths unreachable. |
| Claude (fallback) | OK | 7 findings produced; verified F1–F5 against the working tree (ST4 missing, 5 §5 tests missing, SSE has no origin check, `tower-http` unused in `src/`, 2 unused imports in test file). |

## Verdict
rework

## Must Fix Before PR
- **F1 — ST4 (advisory-lock heartbeat-expiry test hook) is unimplemented.** Plan/design make ST4 an explicit close-gate deliverable. `git diff main...HEAD` touches zero files under `src/run_state/`, and no `lock_heartbeat_expiry_releases_lock` test exists. Either land the `#[cfg(test)]` accessor + `T-LOCK-2` test, or formally drop ST4 from the design/plan/Linear scope before merging.
- **F2 — Design §5 coverage table overstates reality.** Five named tests are absent: `loopback_bind_rejects_external_interface` (T-BIND-1), `gate_url_has_sufficient_entropy` (T-COOKIE-1), `concurrent_approval_honors_advisory_lock` (T-LOCK-1), `sse_rejects_cross_origin_request`, `lock_heartbeat_expiry_releases_lock` (T-LOCK-2). `docs/threat-model.md` advertises them as covered. Land the tests or trim the design + threat-model table to match the actual suite — shipping the close gate with a lying coverage table is the worst outcome.
- **F3 — SSE handler has no cross-origin protection.** `src/ui/routes.rs:437` (`run_trace_sse`) inspects only `run_id` and `Last-Event-ID`; design §7 left "SSE Origin enforcement" as an open question. Either add a `Sec-Fetch-Site: same-origin` / Origin allow-list check (and a `T-SSE-CSRF` test), or document the deferral explicitly in `docs/threat-model.md` and the design — don't leave it implicit.
- **F4 — `tower-http = "0.6.10"` declared but unused.** No `tower_http::` import in `src/`. Drop the dependency or actually use `SetResponseHeaderLayer` in place of the hand-rolled `with_headers` wrapper.
- **F5 — Unused imports in `tests/ui_threat_model.rs:8-9`.** `axum::body::Body` and `axum::http::Request` are dead. Remove them; this is the kind of lint that breaks `make check` the moment `-D warnings` lands.

## Out of Scope / Deferred
- **F6 (loose `400 || 404` assertion in T-TRAVERSAL-1).** Quality nit, not a correctness regression. Tighten in a follow-up if the bounded-fix iteration is already crowded.
- **F7 (design §3 says JSON, code requires form-encoding).** Stale doc, not a code defect; bundle into the same docs sweep as F2 if convenient, otherwise defer.

## False Positives / Tooling Artifacts
- The Codex and Gemini "REVIEW_FAILED" outcomes are tooling artifacts of the wrapper script in `.lok/workflows/implement-gate.toml` (or wherever the heredoc lives), not signals about the branch. Independent fix needed for the orchestrator wrapper, but it does not affect this PR's verdict.

## Recommendation
STOP_FOR_USER. This is the M11 close gate; F1+F2 together mean the branch ships a lock that doesn't lock — the design and `docs/threat-model.md` claim coverage (ST4, T-BIND-1, T-COOKIE-1, T-LOCK-1, T-LOCK-2, SSE CSRF) that the suite does not actually exercise. The user needs to choose explicitly: (a) finish ST4 + the missing §5 tests + decide SSE origin policy in another implement loop, or (b) consciously shrink the close-gate scope by editing the design, plan, threat-model table, and Linear ticket to match what landed, then re-enter validation. Either path is defensible; silently merging is not. The hardening that *did* land (security headers, POST guard with paired tests, artefact resolver with traversal/symlink containment) is high quality and worth keeping — this is a scope-honesty problem, not a code-quality one.
