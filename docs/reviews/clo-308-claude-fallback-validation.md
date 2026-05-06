# Pre-PR validation: clo-308

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

Pre-merge gate passes locally: `cargo fmt --check` clean, `cargo clippy -- -D warnings` clean (default scope), `cargo test` passes — including 10 new doctor tests. `cargo clippy --all-targets` does flag 6 useless-format lints in the new tests, but `make check` doesn't use `--all-targets`, so it doesn't block.

## Findings

### F1 [medium] HTTP non-2xx / non-401-403 / non-5xx mislabeled as `UNREACHABLE (network)`
**Where:** src/doctor.rs:164-170
**What:** Any status outside {200..=299, 401..=403, 500..=599} (e.g. 404 from a misrouted proxy, 408 timeout-from-server, 429 rate-limit, 301/302 to a non-2xx, 400) lands in the catch-all and is reported as a transport-layer `UNREACHABLE (network)` failure. That mis-attributes a server response as a network problem and points operators at the wrong root cause — exactly what a doctor probe is supposed to disambiguate.
**Suggested fix:** Add a separate branch for "reachable but unhealthy": `status => CheckRow { status: format!("UNHEALTHY (HTTP {status})"), ... }`. Reserve `network` for `reqwest::Error` cases.

### F2 [medium] DNS test does not actually verify DNS classification
**Where:** src/doctor.rs:443-464 (`tensorzero_dns_failure`) and src/doctor.rs:419-440 (`tensorzero_connection_refused`)
**What:** Both tests assert only that `row.status.contains("UNREACHABLE")`, not that the classifier emitted `"DNS"` vs `"connection refused"`. The whole point of `classify_transport_error` (src/doctor.rs:187-202) is to distinguish these — if the DNS branch silently regresses to `"connection refused"`, the test still passes. The plan's classification table loses its only enforcement.
**Suggested fix:** `assert!(row.status.contains("DNS"), "got {:?}", row.status)` in the DNS test, and similarly `contains("connection refused")` in the refused-port test.

### F3 [medium] Design "Must" violated: probe re-implements `to_backend_opts()` instead of calling it
**Where:** src/doctor.rs:174-185 (`resolve_tensorzero_opts`) vs. src/config.rs:181-195 (`TensorZeroConfig::to_backend_opts`)
**What:** The design explicitly mandates: "Resolve auth key through `TensorZeroConfig::to_backend_opts()` to reuse existing env-resolution logic." The implementation duplicates that logic in a new private helper. Today the two are equivalent; if env handling changes in `to_backend_opts()` (e.g. fallback envs, default key), doctor will silently diverge from the actual backend it's supposed to validate.
**Suggested fix:** Replace `resolve_tensorzero_opts` with a direct call to `tz.to_backend_opts()` and pull `endpoint`/`api_key` out of the returned `TensorZeroBackendOpts`. The `#[allow(dead_code)]` on `to_backend_opts` becomes deletable too.

### F4 [low] Builder fallback silently drops the 5s timeout
**Where:** src/doctor.rs:119-122
**What:** `reqwest::Client::builder().timeout(...).build().unwrap_or_else(|_| reqwest::Client::new())` falls back to a client with **no timeout** if the builder fails. In practice it cannot fail for these settings, so the fallback is dead code that hides a hang risk if anyone ever extends the builder (TLS roots, proxies). Defensive code that softens an invariant rather than asserting it.
**Suggested fix:** `.expect("reqwest client builder with default settings cannot fail")`.

### F5 [low] `cargo clippy --all-targets` fails on 6× `useless_format` in tests
**Where:** src/doctor.rs:325, 351, 377, 403, 481, 513
**What:** Every test endpoint uses `format!("{}", server.uri())`. `make check` doesn't pass `--all-targets`, so it's green today, but the lints will trip the moment CI tightens scope (a common upgrade), and they're trivially redundant.
**Suggested fix:** `endpoint: server.uri()` — `MockServer::uri()` already returns `String`.

### F6 [low] `is_critical = true` on the HEALTHY row is semantically odd
**Where:** src/doctor.rs:142-149
**What:** The healthy branch sets `is_critical: true, is_ok: true`. The aggregator at src/doctor.rs:29 only triggers on `is_critical && !is_ok`, so this is harmless — but `is_critical` reads as "this check is required for success", which is fine for the failure rows yet misleading on the green row. Minor naming smell that hints `is_critical` actually means "this row contributes to exit code".
**Suggested fix:** Either rename the field to `affects_exit_code`, or set `is_critical: false` on the healthy row (since by definition it didn't fail). Consider for a follow-up.

### F7 [low] `print_rows` filters sections by hardcoded name comparison
**Where:** src/doctor.rs:218, 235-238, 262
**What:** `if row.name == "codex" || row.name == "gemini" || row.name == "claude"` and the API-key triplet duplicate the lists from `check_binaries`/`check_api_keys`. Adding a fourth backend means editing three places and the bug won't surface until printing. KISS-acceptable for now, but worth a `section: Section` enum if doctor grows.
**Suggested fix:** Add `section: Section { Backend, ApiKey, Gateway }` to `CheckRow`; print by grouping on it.

## Verdict
**approve_with_changes**

The branch correctly extracts Doctor for testability, adds the `/health` probe, and preserves existing CLI output; `make check` passes and the 10 new tests cover the main scenarios. The blocker-class issues are F1 (404/3xx/4xx-other mislabeled as "network", which is exactly the kind of misleading diagnosis a doctor command must avoid) and F2 (the DNS/refused tests don't actually pin the classification, so the central new behavior is under-verified). F3 is a direct deviation from a "Must" constraint in the design and re-introduces drift between the probe and the live backend's env resolution. None are merge-blocking if treated as immediate follow-ups; fixing F1, F2, and F3 inline would be cheap and would land the change closer to the design's stated intent.
