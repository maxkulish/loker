# Gemini Architect Review: CLO-261 Design

## Verdict

approve_with_changes

## Summary

The design is appropriately scoped and matches discovery: close the `capabilities_for_name("tensorzero")` vs `create_backend("tensorzero")` divergence with a small dispatcher arm and adapter, not a broader config redesign. It preserves the public `Backend` trait and keeps TensorZero wire behaviour inside `src/backend/tensorzero.rs`, which is the right architectural boundary for this follow-up.

## Suggestions

### G1 — Add URL-scheme validation in the adapter (additive)

The adapter table says `command` is a required URL string but later says it only validates required fields and zero timeout. This creates a small inconsistency and could permit `command = "tensorzero"` to fail later in a less actionable way. Mirror the already-existing `TensorZeroConfig::validate` behaviour enough to require parseable `http`/`https`, while still leaving `/openai/v1/` normalization to `TensorZeroBackend`.

### G2 — Keep the adapter private and avoid a new supported-name public API (approve)

The design correctly avoids exposing a new dispatcher-name list. A public helper would create an additional source of truth. Unit tests can use a private helper or explicit assertions.

### G3 — Include env-var cleanup / isolation in unit tests (additive)

Adapter tests that set `api_key_env` should use a CLO-specific variable name and remove it afterward to avoid order-dependent test pollution.

### G4 — Document top-level `[tensorzero]` relationship explicitly (additive)

The migration section mentions top-level `[tensorzero]` remains valid for existing consumers. Add one sentence that CLO-261 does not auto-synthesize `[backends.tensorzero]` from `[tensorzero]`; that should be separate if desired. This prevents reviewers from expecting config-loader work.

### G5 — Consider changing `create_backend` to accept `Config` (flagged)

The cleaner long-term design is for TensorZero creation to read `Config.tensorzero` directly. However, that would touch many call sites and was already rejected in discovery as broader than this follow-up. Keep the narrow adapter for CLO-261.
