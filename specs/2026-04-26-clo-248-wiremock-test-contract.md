# Spec: Wiremock Unit-Test Contract for TensorZero Backend

**Created**: 2026-04-26
**Estimated scope**: S (1 new file, ~3 sub-tasks)
**Linear**: [CLO-248](https://linear.app/cloud-ai/issue/CLO-248/add-wiremock-unit-test-contract-for-tensorzero-backend)
**Source of truth**: `docs/plans/2026-04-25-m1-tensorzero-backend.md` §T-006 "Test contract"

## 1. Problem Statement

The M1 TensorZero backend (`src/backend/tensorzero.rs`) ships with comprehensive
inline `#[cfg(test)] mod tests` coverage that exercises the same six wiremock
scenarios this task enumerates (added during CLO-247, merged via PR #7 commit
`048b6cd`). However, those tests live inside the module and exercise both
public and private code paths together. Downstream work (CLO-257 "Implement
Strategy::SingleModel", CLO-252 "opt-in TensorZero integration test") needs a
**stable, public-API contract** that:

1. Survives internal refactors of `tensorzero.rs` private helpers
2. Is discoverable as a top-level test crate (`tests/tensorzero_backend.rs`)
3. Documents the M1 wire contract in one place: HTTP shape sent to the
   gateway, error mapping back to `BackendError`, retry classification

The Linear scope explicitly names `tests/tensorzero_backend.rs` and the M1
plan (T-006) requires this external file. The inline tests will be kept
in place for private-helper coverage (e.g. `classify_5xx_body`,
`classify_404_body`, endpoint normalization) but the six public-surface
cases must be re-asserted from outside the module to pin the contract
that downstream tasks rely on.

**Affected file** (new): `tests/tensorzero_backend.rs`
**Public API consumed**:
- `loker::backend::{Backend, BackendError, QueryOutput}` (re-exported from `src/backend/mod.rs`)
- `loker::backend::{TensorZeroBackend, TensorZeroConfig}` (re-exported at `src/backend/mod.rs:17`)

**Existing inline coverage to mirror** (in `src/backend/tensorzero.rs` `mod tests`):
- `returns_text_on_200_success` (line 426)
- `maps_429_to_rate_limit_retryable` (line 446)
- `maps_500_to_retryable_error` (line 467)
- `maps_401_to_auth_not_retryable` (line 488)
- `maps_malformed_json_to_parse_error` (line 509)
- `maps_request_timeout_to_timeout_error` (line 534)

## 2. Acceptance Criteria

- [ ] **AC1**: `tests/tensorzero_backend.rs` exists and compiles cleanly under `cargo test --no-run -q`.
- [ ] **AC2**: Test `success_200_returns_text` passes — wiremock serves an OpenAI-shaped 200 body, `Backend::query("hi", Path::new("."), None)` returns `Ok(QueryOutput)` whose `stdout` matches the served content, `backend == "tensorzero"`, and `model == Some("test-model")`.
- [ ] **AC3**: Test `rate_limit_429_is_retryable` passes — wiremock serves HTTP 429, result is `Err(BackendError::RateLimit { .. })` and `err.is_retryable() == true`.
- [ ] **AC4**: Test `server_error_500_is_retryable` passes — wiremock serves HTTP 500, result is `Err(BackendError::Network { .. })` and `err.is_retryable() == true`. (The current T-007 mapping returns `Network` for generic 5xx; if T-008 retry-policy work changes this, the variant assertion may need to relax to `is_retryable()`-only.)
- [ ] **AC5**: Test `auth_failure_401_is_not_retryable` passes — wiremock serves HTTP 401, result is `Err(BackendError::Auth { .. })` and `err.is_retryable() == false`. A second case `auth_failure_403_is_not_retryable` covers HTTP 403 with the same expectations.
- [ ] **AC6**: Test `malformed_json_returns_parse_error` passes — wiremock serves HTTP 200 with body `not valid json {{{`, result is `Err(BackendError::Parse { .. })`.
- [ ] **AC7**: Test `request_timeout_returns_timeout_error` passes — wiremock delays the response past the configured per-request budget, result is `Err(BackendError::Timeout { .. })` and `err.is_retryable() == true`.
- [ ] **AC8**: No test imports anything from `tensorzero` crate, OS-installed `tensorzero` gateway binary, or any network resource other than the in-process wiremock server.
- [ ] **AC9**: Full test file passes `cargo test --test tensorzero_backend -q` with zero failures and zero `#[ignore]` markers.
- [ ] **AC10**: `make check` passes (fmt + clippy + test) on the branch.

**Verification method**:
```bash
# Per-case verification (AC2-AC7)
cargo test --test tensorzero_backend -q

# Compile-only sanity (AC1)
cargo test --test tensorzero_backend --no-run -q

# Pre-merge gate (AC10)
make check

# Independence check (AC8) — should produce zero matches
rg -n "tensorzero::|use tensorzero" tests/tensorzero_backend.rs
```

## 3. Constraints

**Must**:
- Place tests in `tests/tensorzero_backend.rs` (the path is the deliverable).
- Use only the public API surface exported from `loker::backend` — no `pub(crate)` access, no `#[cfg(test)] use ...` shortcuts into the module.
- Use `wiremock = "0.6"` (already in `[dev-dependencies]`, Cargo.toml:63).
- Use `#[tokio::test]` for async tests (tokio is already a workspace dep).
- Set `TensorZeroConfig::endpoint = server.uri()` (the backend appends `/openai/v1/chat/completions` itself; verified at `src/backend/tensorzero.rs` `normalize_endpoint`). Do NOT pre-pend `/openai/v1`.
- Call `Backend::query` with the actual signature `query(prompt: &str, cwd: &Path, model: Option<&str>)`. Pass `None` for the model override — `TensorZeroConfig::model` carries the wire model.
- Set per-request timeout small enough (`Duration::from_millis(150)` mirroring the inline test) that the timeout test runs in <1s.
- Assert against `BackendError::is_retryable()` for retry-classification tests, not the discriminant — the variant chosen for 5xx is a T-008 implementation detail.
- For 401/403/auth tests, assert the variant **is** `BackendError::Auth` because that pin matters for the retry policy.

**Must-not**:
- Touch `src/backend/tensorzero.rs` unless a public-surface gap is uncovered while writing the tests (e.g. `TensorZeroConfig` field not `pub`). If touched, restrict to the minimum visibility change and note it in the PR description.
- Remove or duplicate the inline `mod tests` in `src/backend/tensorzero.rs` — those tests cover private helpers (`classify_5xx_body`, `classify_404_body`, endpoint normalization, request-shape assertions) and stay where they are.
- Add any new runtime dependency. Only dev-deps allowed and `wiremock`/`tokio`/`serde_json` should suffice.
- Depend on the `tensorzero` crate, an installed `tensorzero` gateway binary, or any external network resource.
- Use `#[ignore]`, `#[should_panic]`, or environment-gated tests — every case must run by default under `cargo test -q`.
- Introduce flakiness: no `sleep`-based synchronization beyond what the timeout test inherently requires.

**Prefer**:
- A single small helper `fn config_for(server: &MockServer) -> TensorZeroConfig` to keep each test focused on its scenario.
- A single small helper `fn openai_success_body(text: &str) -> serde_json::Value` to build the 200 response body — mirror the shape used by the existing inline tests so the contract stays aligned.
- One test fn per scenario (six fns minimum), named `<scenario>_<expected_outcome>` for grep-ability.
- Comments only where a non-obvious wiremock matcher or timing constant is in play.

**Escalate when**:
- A required public symbol (`TensorZeroBackend::new`, `TensorZeroConfig` fields, `BackendError::Auth`/`Parse`/`Timeout`/`RateLimit` variants) is not actually `pub` — the fix may need a separate small commit and the spec scope changes.
- The 5xx mapping in `tensorzero.rs` produces a non-retryable error (would contradict T-007/T-008 — surface as a bug, do not work around in the test).
- `wiremock` cannot simulate the timeout case within the per-request budget (very unlikely; if so, fall back to a `Delay::never()` responder and document).

## 4. Decomposition

1. **Scaffold + helpers**: Create `tests/tensorzero_backend.rs` with module preamble, imports (`wiremock`, `loker::backend::*`, `serde_json::json`), `config_for(&MockServer) -> TensorZeroConfig`, and `openai_success_body(&str) -> serde_json::Value`. - files: `tests/tensorzero_backend.rs` (new)
2. **Six scenario tests**: Implement the six `#[tokio::test]` functions one per acceptance criterion (AC2-AC7), plus the 403 variant under AC5. Each spins up a `MockServer`, mounts a single `Mock`, builds a `TensorZeroBackend` via the helper, calls `Backend::query("ping", &cwd, "default")`, and asserts on the result. - files: `tests/tensorzero_backend.rs`
3. **Verification + cleanup**: Run `cargo test --test tensorzero_backend -q`, fix any compile/runtime issues, run `make check`, ensure no clippy warnings. - files: `tests/tensorzero_backend.rs` (only)

**Dependency order**: 1 → 2 → 3 (strict; helpers must exist before scenario tests, scenarios must compile before verification).

## 5. Evaluation

| # | Test | Expected Result | How to Run |
|---|------|-----------------|------------|
| 1 | `success_200_returns_text` | `Ok(QueryOutput { text: "hello", backend: "tensorzero", .. })` | `cargo test --test tensorzero_backend success_200_returns_text -q` |
| 2 | `rate_limit_429_is_retryable` | `Err(RateLimit)`; `err.is_retryable() == true` | `cargo test --test tensorzero_backend rate_limit_429 -q` |
| 3 | `server_error_500_is_retryable` | `Err(_)`; `err.is_retryable() == true` | `cargo test --test tensorzero_backend server_error_500 -q` |
| 4 | `auth_failure_401_is_not_retryable` | `Err(Auth)`; `is_retryable() == false` | `cargo test --test tensorzero_backend auth_failure_401 -q` |
| 5 | `auth_failure_403_is_not_retryable` | `Err(Auth)`; `is_retryable() == false` | `cargo test --test tensorzero_backend auth_failure_403 -q` |
| 6 | `malformed_json_returns_parse_error` | `Err(Parse)` | `cargo test --test tensorzero_backend malformed_json -q` |
| 7 | `request_timeout_returns_timeout_error` | `Err(Timeout)`; `is_retryable() == true` | `cargo test --test tensorzero_backend request_timeout -q` |
| 8 | Whole-file run | All 7 tests pass, 0 ignored, 0 failed, runtime <2s | `cargo test --test tensorzero_backend -q` |
| 9 | Pre-merge gate | fmt + clippy + test all green | `make check` |
| 10 | Independence | Zero matches | `rg -n "use tensorzero" tests/tensorzero_backend.rs` |

**Edge cases to verify**:
- Wiremock mounted with `expect(1)` for at least one test to confirm the backend actually issued the HTTP call (not failing pre-flight).
- Timeout test does not leak the wiremock server (drops cleanly) — `MockServer` handles this on drop.
- 500 test asserts retryability without binding to a specific variant (Network vs RateLimit) so T-008 retry-policy refactor doesn't break this contract.
- `config_for` mirrors the inline helper at `src/backend/tensorzero.rs:401`: `endpoint: server.uri()` (NOT pre-pended with `/openai/v1`), `model: "test-model"`, `api_key: Some("test-key")`, `timeout: Duration::from_secs(5)` for non-timeout tests. The timeout test overrides `cfg.timeout = Duration::from_millis(150)` after `config_for` returns.
- `QueryOutput` field for response text is `stdout`, not `text` — confirmed at `src/backend/mod.rs` (the field name is shared with subprocess backends).
