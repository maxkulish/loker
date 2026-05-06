# Pre-PR validation: clo-308

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
---

## Reviewer Status

| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex (gpt-5.5) | REVIEW_FAILED | Shell heredoc escaping bug in the orchestrator wrapper — `unexpected EOF while looking for matching '` before the model was ever invoked. No model output produced. |
| Gemini (gemini-3.1-pro-preview, fallback 2.5-pro) | REVIEW_FAILED | Same root cause — shell heredoc escaping in the wrapper. No model output produced. |
| Claude fallback | OK | Read design + plan + full diff + Makefile; verified each finding against `src/doctor.rs` and `src/config.rs:181`. |

Both reviewer scripts failed identically before any LLM call (the `Bash` runner couldn't parse the embedded `$(cat <<EOF ... EOF)` heredoc), so synthesis rests on the Claude fallback grounded against the source.

## Verdict
approve_with_changes

## Must Fix Before PR

- **F1 — HTTP catch-all mislabels reachable-but-unhealthy as `UNREACHABLE (network)`** (src/doctor.rs:164-170). Any 404/408/429/3xx/other-4xx is reported as a transport-layer failure, which directly defeats the doctor's purpose of disambiguating cause. Fix: add a separate `UNHEALTHY (HTTP {status})` arm; reserve `network` for `reqwest::Error`.
- **F2 — DNS / connection-refused tests don't pin the classification** (src/doctor.rs:432-439, 456-463). They only assert `contains("UNREACHABLE")`, so a regression in `classify_transport_error` (the central new behavior) would pass. Plan Phase 3 explicitly enumerates these as distinct cases. Fix: assert `contains("DNS")` and `contains("connection refused")` in the respective tests.
- **F3 — Design "Must" violated: probe duplicates `to_backend_opts()` instead of calling it** (src/doctor.rs:174-185 vs src/config.rs:181-195). The design line 181 says: *"Resolve auth key through `TensorZeroConfig::to_backend_opts()` to reuse existing env-resolution logic."* Today the duplicate is equivalent, but any future change to `to_backend_opts()` will silently diverge from the doctor probe — which is the exact drift the design forbids. Fix: replace `resolve_tensorzero_opts` with `tz.to_backend_opts()` and pull `endpoint`/`api_key` from the returned struct; the `#[allow(dead_code)]` on `to_backend_opts` then becomes deletable.

## Out of Scope / Deferred

- **F4 — `unwrap_or_else(|_| reqwest::Client::new())` silently drops the 5s timeout** (src/doctor.rs:119-122). Defensive code that hides a hang risk if the builder is ever extended. Trade for `.expect(...)` in a follow-up.
- **F5 — `cargo clippy --all-targets` flags 6× `useless_format` in tests** (src/doctor.rs:325, 351, 377, 403, 481, 513). `make check` uses `cargo clippy -- -D warnings` only (no `--all-targets`, confirmed via Makefile:67), so this is non-blocking today but will trip the moment CI scope tightens.
- **F6 — `is_critical: true` on the HEALTHY row** (src/doctor.rs:142-149). Semantically odd ("required for success" when nothing failed) but harmless under the current aggregator. Naming/refactor candidate.
- **F7 — Hardcoded section-name comparisons in `print_rows`** (src/doctor.rs:218, 235-238, 262). Adding a fourth backend means editing three places. KISS-acceptable until doctor grows further.

## False Positives / Tooling Artifacts

- **Codex and Gemini "REVIEW_FAILED"** are tooling artifacts (orchestrator shell escaping), not real review failures. Synthesis substitutes the Claude fallback grounded against source.

## Recommendation

PROCEED_WITH_FIXES. Land three small, bounded fixes inline before opening the PR:

1. Add a non-2xx/non-classified branch returning `UNHEALTHY (HTTP {status})` (F1).
2. Strengthen the two transport tests to assert the actual classifier label (`"DNS"`, `"connection refused"`) instead of only `"UNREACHABLE"` (F2).
3. Replace `resolve_tensorzero_opts` with a direct `tz.to_backend_opts()` call and read `endpoint`/`api_key` off the returned `TensorZeroBackendOpts`; drop the now-unneeded `#[allow(dead_code)]` (F3).

Each is local to `src/doctor.rs` (F3 also touches `src/config.rs`), under ~30 lines total, and verifiable with `make check`. F4–F7 are real but should not block this PR.
