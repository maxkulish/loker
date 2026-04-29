# Plan: CLO-261 Wire create_backend("tensorzero") arm + BackendConfig→TensorZeroConfig adapter

## Context

- Design: `docs/designs/clo-261-tensorzero-create-backend-wiring.md`
- Discovery: `docs/discovery/clo-261.md`
- PRD: `docs/prds/clo-261-tensorzero-create-backend-wiring.md`
- Linear: https://linear.app/cloud-ai/issue/clo-261/wire-create-backendtensorzero-arm-backendconfigtensorzeroconfig

## Sub-tasks

### ST1 Add TensorZero BackendConfig adapter

**Files:** `src/backend/mod.rs`

Implement private helper:

```rust
fn tensorzero_backend_opts_from_config(
    config: &BackendConfig,
) -> anyhow::Result<tensorzero::TensorZeroBackendOpts>
```

Mapping:
- `command` → required `endpoint`, parseable `http`/`https`, passed through without `/openai/v1/` normalization.
- `model` → required TensorZero function/model.
- `api_key_env` → optional `std::env::var` resolution; absent/empty means `None`.
- `timeout` → seconds; default `60`; reject `0`.

**Acceptance:**

```bash
cargo test backend::tests::tensorzero_adapter
```

**Estimate:** S

### ST2 Wire `tensorzero` into `create_backend`

**Files:** `src/backend/mod.rs`

Add a `"tensorzero"` match arm before the unknown-backend fallthrough:

```rust
"tensorzero" => Arc::new(tensorzero::TensorZeroBackend::new(
    tensorzero_backend_opts_from_config(config)?,
)?),
```

Preserve existing retry wrapping after backend construction.

**Acceptance:**

```bash
cargo test backend::tests::tensorzero_create_backend_supported_when_capability_supported
```

**Estimate:** S

### ST3 Pin supported-name parity

**Files:** `src/backend/mod.rs`

Extend backend module tests so `tensorzero` is explicitly covered by both:
- `capabilities_for_name("tensorzero")`
- `create_backend("tensorzero", valid_config, zero_retry_policy)`

Avoid adding a new public supported-name API; keep parity assertions test-local/private.

**Acceptance:**

```bash
cargo test backend::tests::capabilities_for_name_matches_static_expectations && \
cargo test backend::tests::tensorzero_create_backend_supported_when_capability_supported
```

**Estimate:** S

### ST4 Add external dispatcher integration test

**Files:** `tests/tensorzero_create_backend.rs`

Create a wiremock-backed test that exercises the public dispatcher path:
1. Start `MockServer`.
2. Build `BackendConfig` with `command = Some(server.uri())`, `model = Some("test-model")`, and no required API key env.
3. Call `create_backend("tensorzero", &cfg, zero_retry_policy())`.
4. Mount `/openai/v1/chat/completions` 200 response.
5. Call `backend.query("ping", Path::new("."), None)`.
6. Assert `stdout`, `backend`, `model`, and `is_available()`.

**Acceptance:**

```bash
cargo test --test tensorzero_create_backend
```

**Estimate:** S

### ST5 Run regression gate and update docs if needed

**Files:** `src/backend/mod.rs`, `tests/tensorzero_create_backend.rs`, optionally `docs/handoff.md` only if implementation uncovers operator-facing drift.

Run focused TensorZero tests plus full pre-merge gate. If the implemented mapping differs from the design, update the design/plan before PR.

**Acceptance:**

```bash
cargo test --test tensorzero_backend && \
cargo test --test tensorzero_create_backend && \
make check
```

**Estimate:** S

## Pre-merge gate

- `make check` (fmt + clippy + tests)

## Risks

- `BackendConfig.command` is an overloaded endpoint field for TensorZero. This is intentional for CLO-261 because `create_backend` receives only `BackendConfig`; top-level `[tensorzero]` synthesis is out of scope.
- Env-var tests can pollute process state if not cleaned up. Use a CLO-specific variable and remove it after assertions.
- Adding a public supported-name helper would create a second source of truth. Keep parity checks private/test-local.
- URL validation should not duplicate TensorZero path normalization; only validate parseability and `http`/`https` scheme.
