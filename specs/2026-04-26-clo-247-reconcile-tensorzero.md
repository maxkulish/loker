# Spec: CLO-247 Reconcile tensorzero backend with D1 spike findings

**Created**: 2026-04-26
**Estimated scope**: M (4 files, ~5 sub-tasks)
**Linear**: [CLO-247](https://linear.app/cloud-ai/issue/CLO-247/reconcile-tensorzero-backend-with-d1-spike-findings)
**Source of truth**: `docs/spikes/2026-04-25-tensorzero-roundtrip.md` (D1 verdict, CLO-243)

## 1. Problem Statement

The `src/backend/tensorzero.rs` skeleton (369 lines) was written before the D1 round-trip spike (CLO-243) verified the gateway's actual behavior. The skeleton now drifts from the D1 verdict in three ways that will cause downstream tasks (CLO-248/249/250/251) to build on a faulty foundation:

1. **Wrong URL path prefix.** The runtime endpoint normalization (`src/backend/tensorzero.rs:200`) and every wiremock unit test (lines 226, 245, 266, 287, 308, 332) target `/v1/chat/completions`. The D1 spike confirmed the gateway exposes the OpenAI-compatible surface at `/openai/v1/chat/completions`. Hitting `/v1/` against a real gateway yields 404. See spike §"Path & headers".

2. **502-always-Network error mapping.** `map_status` at `src/backend/tensorzero.rs:174` collapses every 5xx into `BackendError::Network`. The D1 spike fixture `tests/fixtures/tensorzero/anthropic_auth_failure_response.json` shows the gateway wraps upstream auth failures as a **502** with the upstream 401 body embedded. Because `BackendError::is_retryable()` (in `src/backend/mod.rs:34`) returns `true` for `Network`, this misclassification triggers pointless retries on credential failures and obscures the real error class for callers. Acceptable variants per D1: auth-signature → `Auth`; rate_limit/429-signature → `RateLimit`; otherwise `Network`.

3. **Missing convention codification.** `family_of(backend_id)` (FR-13, scheduled for T-015 / CLO-251) must derive the model family from the function-name suffix (e.g. `loker_d1_anthropic` → `anthropic`). The D1 spike calls for this convention to be codified in `docs/handoff.md` and the `tensorzero/config/tensorzero.toml` template comments so config authors don't accidentally violate it. Neither file currently mentions the suffix rule. The module-level doc comment in `src/backend/tensorzero.rs:1-7` likewise does not reference the D1 spike as the canonical source.

This is a reconciliation, not a redesign. All architecture decisions are settled. Acceptance criteria are mechanically testable.

## 2. Acceptance Criteria

- [ ] **AC1**: `cargo build` produces zero errors and zero new warnings; `src/backend/tensorzero.rs` contains no `unimplemented!()`, `todo!()`, or `panic!()` placeholders on the non-test code paths.
- [ ] **AC2**: Runtime endpoint normalization in `TensorZeroBackend::new` (or equivalent helper) targets `/openai/v1/` and trailing-slash-normalizes correctly. Every wiremock test in `src/backend/tensorzero.rs::tests` uses `path("/openai/v1/chat/completions")`.
- [ ] **AC3**: Auth header sent on every outbound request matches D1: `Authorization: Bearer <token-from-config>` and `Content-Type: application/json`. Verified by a wiremock test that asserts both headers via `header(...)` matchers.
- [ ] **AC4**: Outbound request `model` field equals the configured TensorZero function name (e.g. `tensorzero::function_name::loker_d1_openai`). Verified by a wiremock test that decodes the request body and asserts `body["model"] == expected`.
- [ ] **AC5**: `map_status` (or its successor) maps a 502 carrying an upstream auth-error signature to `BackendError::Auth`, a 502 with rate-limit/429 signature to `BackendError::RateLimit`, and any other 5xx to `BackendError::Network`. Verified by three wiremock tests using fixtures `anthropic_auth_failure_response.json`, a synthesized rate-limit body, and a generic 5xx body.
- [ ] **AC6**: 404 with body matching `{"error":{"message":"Unknown function: ..."}}` (per `tests/fixtures/tensorzero/unknown_function_response.json`) maps to `BackendError::Config` (configuration error, non-retryable). Verified by a wiremock test.
- [ ] **AC7**: Module-level doc comment (`//!` block at top of `src/backend/tensorzero.rs`) explicitly references `docs/spikes/2026-04-25-tensorzero-roundtrip.md` as the source of truth for path, headers, and error mapping.
- [ ] **AC8**: `docs/handoff.md` contains a "Function-name family convention" section stating the rule (`loker_<purpose>_<family>`, family is the suffix after the last `_`) and that unknown suffixes must be rejected at config-load.
- [ ] **AC9**: `tensorzero/config/tensorzero.toml` carries a comment block above the `[functions]` section that restates the family-suffix convention with `loker_d1_anthropic` and `loker_d1_openai` as canonical examples.
- [ ] **AC10**: `make check` (fmt + clippy + test) exits 0.

**Verification method**:
- AC1, AC10: `make check` and inspection of the diff for placeholder macros.
- AC2-AC6: `cargo test --lib backend::tensorzero` (existing wiremock test harness, with new cases added).
- AC7-AC9: `rg "2026-04-25-tensorzero-roundtrip"` and `rg "family-suffix" docs/handoff.md tensorzero/config/tensorzero.toml`.

## 3. Constraints

**Must**:
- Preserve the `Backend` trait shape and `TensorZeroBackend` public surface unless the change is required for a listed AC. Downstream consumers (CLO-248..251) read this surface.
- Use the existing `genai = "0.6.0-beta.17"` crate (`AdapterKind::OpenAI`, `ServiceTargetResolver`). No transport rewrite.
- Derive error classification from the response **body**, not from headers alone (per D1, gateway does not propagate upstream `WWW-Authenticate` or `Retry-After`). Body inspection should match by stable substrings, not exact JSON shape, since the gateway concatenates upstream error text.
- Keep `unknown_function_response.json` and `anthropic_auth_failure_response.json` as the wiremock contract anchors. Do not edit fixture files.
- Update only files reachable from this spec: `src/backend/tensorzero.rs`, `docs/handoff.md`, `tensorzero/config/tensorzero.toml`, and (if needed) the workflow YAML.

**Must-not**:
- Introduce a new error variant on `BackendError`. T-007 / CLO-249 owns the variant set; this task only re-routes existing classifications.
- Implement `family_of(backend_id)` itself; that is T-015 / CLO-251. This task only **codifies the convention** in docs and config comments, and ensures the backend struct exposes whatever T-015 will need (typically the configured `function_name`).
- Change `BackendError::is_retryable()` semantics. The fix for the spurious-retry symptom is the upstream classification, not the retryability rule.
- Edit fixture JSONs under `tests/fixtures/tensorzero/`.
- Touch `examples/workflows/*.toml`; verified there are zero tensorzero references in those files.

**Prefer**:
- Body-substring matchers grouped behind a small private helper (e.g. `classify_5xx_body(&str) -> BackendError`) so the heuristic is unit-testable in isolation.
- Single-line `//!` doc lines that reference the spike doc with a relative path; avoid restating the verdict.
- Adding test fixtures via `include_str!` over hand-written JSON literals.

**Escalate when**:
- The 502 body inspection requires a more nuanced match than auth/rate_limit/other (e.g. the gateway turns out to forward yet another upstream class). Surface the new signature before adding a fourth branch.
- A wiremock test reveals the gateway request shape no longer matches the D1 fixture (e.g. `model` field renamed). Stop and re-validate the spike before patching tests.
- T-015's `family_of` requires anything beyond `function_name` from the backend struct; new public surface needs design review with CLO-251 in mind.

## 4. Decomposition

Five sub-tasks, each independently testable. Order matters only where indicated.

1. **Path-prefix fix** - flip `/v1/` to `/openai/v1/` in runtime endpoint construction and all wiremock tests. Files: `src/backend/tensorzero.rs` (lines ~200, 226, 245, 266, 287, 308, 332). Done when `cargo test --lib backend::tensorzero` is green with the existing assertions.
2. **Error-mapping reconciliation** - extract `classify_5xx_body` helper; update `map_status` at line ~174 to inspect 502 bodies for auth and rate-limit signatures; map 404 unknown-function to `BackendError::Config`. Files: `src/backend/tensorzero.rs`. Add three new wiremock tests: 502-auth, 502-ratelimit, 404-unknown-function. Done when those tests pass and existing 5xx-other test still maps to `Network`.
3. **Header & request-body wiremock contract** - add (or strengthen) tests asserting `Authorization: Bearer <…>`, `Content-Type: application/json`, and `body["model"] == "tensorzero::function_name::loker_d1_openai"`. Files: `src/backend/tensorzero.rs`. Done when the three matchers run and pass.
4. **Doc cross-reference** - update module `//!` block at top of `src/backend/tensorzero.rs` to point at `docs/spikes/2026-04-25-tensorzero-roundtrip.md` for path/headers/error-mapping. Files: `src/backend/tensorzero.rs:1-7`.
5. **Convention codification** - add a "Function-name family convention" section to `docs/handoff.md`; add a comment block above `[functions]` in `tensorzero/config/tensorzero.toml`. Files: `docs/handoff.md`, `tensorzero/config/tensorzero.toml`.

**Dependency order**: Sub-tasks 1, 4, and 5 are independent. Sub-task 2 depends on the test fixture paths being correct, so do it after 1. Sub-task 3 can be folded into 1 or 2 as the wiremock tests are touched. Recommended single-PR order: 1 → 2 → 3 → 4 → 5.

## 5. Evaluation

| # | Test | Expected Result | How to Run |
|---|------|-----------------|------------|
| 1 | Build clean | 0 errors, 0 new warnings | `cargo build` |
| 2 | Path prefix in runtime | endpoint string ends with `/openai/v1/` | `cargo test backend::tensorzero::tests::endpoint_normalizes_to_openai_v1` |
| 3 | Wiremock path | mock matches on `/openai/v1/chat/completions` | `cargo test backend::tensorzero::tests::happy_path_uses_openai_v1` |
| 4 | Auth header | mock sees `Authorization: Bearer test-token` | `cargo test backend::tensorzero::tests::sends_bearer_auth` |
| 5 | Content-Type | mock sees `Content-Type: application/json` | `cargo test backend::tensorzero::tests::sends_json_content_type` |
| 6 | Model field | request body `model == "tensorzero::function_name::loker_d1_openai"` | `cargo test backend::tensorzero::tests::sends_function_name_as_model` |
| 7 | 502 auth body | maps to `BackendError::Auth` (non-retryable) | `cargo test backend::tensorzero::tests::maps_502_auth_to_auth` |
| 8 | 502 rate-limit body | maps to `BackendError::RateLimit` (retryable) | `cargo test backend::tensorzero::tests::maps_502_rate_limit_to_rate_limit` |
| 9 | 502 generic body | maps to `BackendError::Network` (retryable) | `cargo test backend::tensorzero::tests::maps_502_generic_to_network` |
| 10 | 404 unknown function | maps to `BackendError::Config` (non-retryable) | `cargo test backend::tensorzero::tests::maps_404_unknown_function_to_config` |
| 11 | Doc cross-ref | grep finds spike path in module doc | `rg "2026-04-25-tensorzero-roundtrip" src/backend/tensorzero.rs` |
| 12 | Handoff convention | grep finds family-suffix section | `rg -i "family.*suffix\|function-name family" docs/handoff.md` |
| 13 | Config template comment | grep finds convention comment | `rg -B1 "loker_d1_" tensorzero/config/tensorzero.toml \| rg -i "family\|suffix"` |
| 14 | Pre-merge gate | exit 0 | `make check` |

**Edge cases to verify**:
- Endpoint with no trailing slash in config (e.g. `http://localhost:3000`) still normalizes to `…/openai/v1/`.
- Endpoint that already ends in `/openai/v1/` does not double-suffix.
- 502 with empty body falls through to `BackendError::Network` (no panic on `is_empty`).
- `classify_5xx_body` handles the auth signature appearing inside a multi-variant error blob (real D1 fixture is `"All variants failed with errors: haiku_v1: All model providers failed to infer with errors: anthropic: Error 401 Unauthorized…"`).
- Body-inspection is case-insensitive on the marker tokens (`401`, `unauthorized`, `rate_limit`, `429`).
- Wiremock test for 404 uses the exact `unknown_function_response.json` fixture, not a hand-written body.
