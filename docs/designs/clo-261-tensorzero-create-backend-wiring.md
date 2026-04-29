# Design: CLO-261 — TensorZero create_backend wiring

## Problem

Discovery (`docs/discovery/clo-261.md`) found a contained semantic divergence: `Workflow::validate_with_capabilities` can accept `backend: tensorzero` because `capabilities_for_name("tensorzero")` exists, but runtime instantiation through `create_backend("tensorzero", ...)` still falls through to `Unknown backend: tensorzero`. The canonical loker design names TensorZero as the new HTTP-gateway `Backend` implementor, so the dispatcher must construct that backend from loader-provided configuration without changing the backend trait or TensorZero wire protocol.

## Goals

- Add a `"tensorzero"` branch to `src/backend/mod.rs::create_backend`.
- Implement a narrow `BackendConfig -> TensorZeroBackendOpts` adapter in the backend module.
- Keep `create_backend` signature and all existing call sites unchanged.
- Keep endpoint normalization, OpenAI path construction, model canonicalization, and non-probing availability in `TensorZeroBackend`.
- Add tests that prove dispatcher support, adapter mapping, and capability/creation parity.

## Non-goals

- Redesign the long-term operator-facing top-level `[tensorzero]` TOML schema from CLO-250.
- Change `Backend`, `BackendCapabilities`, `TensorZeroBackend::query`, or `TensorZeroBackendOpts` public fields.
- Add production endpoint discovery or live TensorZero deployment checks.
- Make `is_available()` perform network or environment probing.

## Architecture

### Modules touched

- `src/backend/mod.rs`
  - Add private adapter function.
  - Add `tensorzero` match arm in `create_backend`.
  - Add unit tests for adapter and supported-name parity.
- `tests/tensorzero_create_backend.rs` (new)
  - External integration-style test using `wiremock` and the public `loker::backend::create_backend` path.

No changes are required in `src/backend/tensorzero.rs`, except if implementation reveals a visibility issue (not expected; `TensorZeroBackend` and `TensorZeroBackendOpts` are already re-exported).

### Data flow

```text
Workflow/config loader
  -> BackendConfig for name "tensorzero"
  -> create_backend("tensorzero", &backend_config, retry_policy)
  -> tensorzero_backend_opts_from_config(&backend_config)
  -> TensorZeroBackend::new(opts)
  -> Arc<dyn Backend>
  -> optional RetryExecutor wrapping
```

### Adapter mapping

`BackendConfig` is subprocess-shaped, but `create_backend` only receives this type today. The adapter therefore uses a documented, test-pinned mapping:

| `BackendConfig` field | `TensorZeroBackendOpts` field | Rule |
|---|---|---|
| `command` | `endpoint` | Required. Must be a non-empty parseable `http`/`https` URL string. Passed through exactly; `TensorZeroBackend::new` normalizes `/openai/v1/`. |
| `model` | `model` | Required. Must be non-empty. Represents the TensorZero function/model name passed to `TensorZeroBackend`. |
| `api_key_env` | `api_key` | Optional. If `Some(non_empty)`, resolve with `std::env::var`; if unset or empty, `api_key = None`. Missing env var is a construction error with context naming the variable. |
| `timeout` | `timeout` | Optional seconds. If absent, use `60s` to match `TensorZeroConfig` default. If `0`, return a config error. |
| retry fields | N/A | Already consumed by `get_retry_policy` and `RetryExecutor`, not duplicated in opts. |
| `args`, `skip_lines`, `enabled` | N/A | Ignored by adapter; existing loader/backends own these semantics. |

The adapter intentionally does **not** append `/openai/v1/` and does **not** duplicate full endpoint normalization. Runtime endpoint normalization and HTTP behaviour remain inside `TensorZeroBackend` and its existing tests. The adapter validates required fields, parseable `http`/`https` scheme, and zero timeout to avoid constructing nonsensical opts.

### Error model

`create_backend` returns `anyhow::Result<Arc<dyn Backend>>`, so adapter failures should use `anyhow::bail!` / `Context` with actionable messages:

- `tensorzero backend requires command endpoint`
- `tensorzero backend requires model`
- `tensorzero backend timeout must be greater than zero`
- `tensorzero backend endpoint: scheme must be http or https`
- `Missing environment variable: TENSORZERO_API_KEY`

`TensorZeroBackend::new` returns `Result<Self, BackendError>`; the dispatcher arm converts that via `?` into `anyhow` like other backend constructors.

### Availability semantics

`TensorZeroBackend::is_available()` remains `true`. Construction validates local config only; no network probe or required API key probe is introduced. If `api_key_env` is unset in `BackendConfig`, the backend is still constructible with `api_key = None`, matching the current optional-auth gateway behaviour. If `api_key_env` is configured but the environment variable is absent, construction fails early because the operator explicitly requested that credential.

### Supported-name parity

Add a private helper or unit-test-local list for dispatcher-supported names and compare it with capability-supported names that are intended for workflow validation. At minimum, pin `tensorzero` explicitly:

```rust
assert!(capabilities_for_name("tensorzero").is_some());
assert!(create_backend("tensorzero", &valid_tensorzero_config(), RetryPolicy { max_retries: 0, ... }).is_ok());
```

If a helper is added, keep it private to avoid expanding public API for a test-only concern.

## Public API surface

No public API changes are required. Existing signatures remain:

```rust
pub fn create_backend(
    name: &str,
    config: &BackendConfig,
    retry_policy: RetryPolicy,
) -> anyhow::Result<Arc<dyn Backend>>;

pub struct TensorZeroBackendOpts {
    pub endpoint: String,
    pub model: String,
    pub api_key: Option<String>,
    pub timeout: std::time::Duration,
}
```

New private helper:

```rust
fn tensorzero_backend_opts_from_config(
    config: &BackendConfig,
) -> anyhow::Result<tensorzero::TensorZeroBackendOpts>;
```

New dispatcher arm:

```rust
"tensorzero" => Arc::new(tensorzero::TensorZeroBackend::new(
    tensorzero_backend_opts_from_config(config)?,
)?),
```

## Test plan

### Unit tests (`src/backend/mod.rs`)

1. `tensorzero_adapter_maps_endpoint_model_auth_timeout`
   - Build a `BackendConfig` with `command = Some("http://127.0.0.1:3000")`, `model = Some("loker_d1_openai")`, `api_key_env = Some(CLO-specific temp var)`, `timeout = Some(7)`.
   - Assert opts endpoint/model/api_key/timeout match.
   - Remove the temp env var after the assertion to avoid test pollution.

2. `tensorzero_adapter_allows_missing_api_key_env_field`
   - `api_key_env = None` returns `api_key = None`.
   - Documents no `TENSORZERO_API_KEY` requirement unless configured.

3. `tensorzero_adapter_rejects_missing_endpoint_model_zero_timeout_and_bad_scheme`
   - Separate assertions or table cases for missing `command`, empty `command`, non-URL / non-http(s) `command`, missing/empty `model`, and `timeout = Some(0)`.

4. `tensorzero_create_backend_supported_when_capability_supported`
   - Valid config plus zero-retry policy returns `Ok` for `tensorzero`.
   - Assert `backend.name() == "tensorzero"` and `backend.is_available()`.
   - Assert no `Unknown backend` text appears for tensorzero failures; config failures should name TensorZero config fields instead.

### Integration test (`tests/tensorzero_create_backend.rs`)

- Start `wiremock::MockServer`.
- Build `BackendConfig` with `command = Some(server.uri())`, `model = Some("test-model")`, optional API key env unset or set.
- Call `create_backend("tensorzero", &cfg, zero_retry_policy())`.
- Mount a `/openai/v1/chat/completions` success response and call `backend.query("ping", Path::new("."), None)`.
- Assert `stdout`, `backend`, and `model` match existing `tests/tensorzero_backend.rs` style.

### Existing regression suite

- `cargo test backend::tests::tensorzero_adapter`
- `cargo test --test tensorzero_create_backend`
- `cargo test --test tensorzero_backend`
- `make check`

### Manual verification

Optional only: run existing live integration test with `LOKER_TZ_INTEGRATION=1` if a local gateway is already running. CLO-261 does not require live TensorZero.

## Migration / rollout

- Existing non-TensorZero backends are unaffected; `create_backend` retains its signature and retry wrapping.
- Existing configs that never mention `backends.tensorzero` see no behaviour change.
- Operators who want dispatcher construction can provide a `BackendConfig`/`[backends.tensorzero]` with `command` as the gateway URL and `model` as the TensorZero function/model. CLO-261 does not auto-synthesize `[backends.tensorzero]` from the documented top-level `[tensorzero]` schema; that remains valid for existing consumers, and any future synthesis from `[tensorzero]` into `backends.tensorzero` should be separate work.
- Rollout is safe behind tests because failure mode changes only for the previously unsupported `"tensorzero"` name.

## Open questions

None blocking. Follow-up candidate: if workflows should be configurable solely via top-level `[tensorzero]`, add config-load synthesis or a broader dispatcher signature in a separate issue rather than expanding CLO-261.
