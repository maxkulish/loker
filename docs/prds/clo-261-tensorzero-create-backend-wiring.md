# PRD: CLO-261 — TensorZero create_backend wiring

## Problem

Workflow authors configuring `backend: tensorzero` are affected today because validation and capability lookup already recognize `tensorzero`, but backend instantiation still falls through to `Unknown backend: tensorzero`. The desired behaviour is that the dispatcher can construct a `TensorZeroBackend` from loader-provided backend configuration, using the existing runtime `TensorZeroBackendOpts` surface and preserving current TensorZero HTTP/wire semantics. This is needed now because CLO-251 intentionally shipped capability declaration only, and PR review flagged the dispatcher divergence as the next blocking integration gap for M1.

## Goals

- Add `tensorzero` to `src/backend/mod.rs::create_backend` without changing the public `Backend` trait.
- Provide a small, tested `BackendConfig -> TensorZeroBackendOpts` adapter.
- Preserve `TensorZeroBackend::new` ownership of endpoint normalization and gateway behaviour.
- Ensure the static capability lookup and create dispatcher do not advertise different supported backend names.
- Add integration coverage proving `tensorzero` is no longer an unknown-backend fallthrough.

## Acceptance Criteria

- `create_backend("tensorzero", &cfg, retry_policy)` returns `Ok(Arc<dyn Backend>)` for a valid TensorZero-shaped `BackendConfig`.
- Adapter tests cover endpoint passthrough/normalization-at-runtime expectations, model passthrough, API-key env resolution, timeout mapping, and missing required fields.
- A `tests/` integration test uses a wiremock endpoint and exercises the public dispatcher path, not direct `TensorZeroBackend::new` only.
- `capabilities_for_name("tensorzero")` and the create dispatcher remain in sync via a pinning test.
- `is_available()` remains a non-probing `true` for constructed TensorZero backends; missing/invalid configuration fails during construction rather than at availability probing.

## Non-goals

- Changing the `Backend` trait or `TensorZeroBackend::query` wire protocol.
- Changing `BackendCapabilities` honesty values.
- Deploying or discovering production TensorZero endpoints.
- Reworking the top-level `[tensorzero]` config schema introduced by CLO-250 beyond the adapter/wiring needed for this dispatcher gap.
