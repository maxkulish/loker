# M1 - TensorZero backend

**Status**: Next milestone after M0 (fork prep, done).
**Design doc**: `/Users/mk/Work/investigations/sakana-fugu/loker-design.md` §8 M1.

## Scope

Add a `Backend` impl that routes calls through a TensorZero gateway via the
`genai = "0.6.0-beta.17"` crate. Existing CLI-subprocess backends (claude,
codex, gemini, ollama, bedrock) keep working unchanged.

## Implementation steps

1. **New file**: `src/backend/tensorzero.rs` implementing the `Backend`
   trait. Read `src/backend/mod.rs` and an existing implementor (e.g.
   `claude.rs`, `ollama.rs`) first to learn the trait shape.
2. **HTTP layer**: use `genai::ServiceTargetResolver` to point all calls at
   the configured TensorZero endpoint with the appropriate auth headers.
3. **Config schema**: endpoint URL, default model, timeout, retry policy.
   Wire into `src/config.rs` alongside the other backend sections.
4. **Error mapping**: `genai` errors -> `BackendError` variants
   (see `src/backend/mod.rs` for the existing enum).
5. **Optional streaming**: off by default in v0.

## Test contract

Unit tests use `wiremock` (add to `[dev-dependencies]`: `wiremock = "0.6"`)
against a mocked HTTP server. Required coverage:

- 200 success
- 429 retry behavior
- 500 retry behavior
- malformed JSON response
- timeout
- auth failure

One integration test against a real local TensorZero gateway, gated by
`LOKER_TZ_INTEGRATION=1` (off by default in CI).

## Constraints (do not relitigate)

- Unit tests must not depend on TensorZero being installed - use `wiremock`.
- Don't drop `reqwest` yet; coexistence with `genai` is intentional until
  the rest of the backends migrate.
- v0 verification stays binary pass/fail (design doc §10 non-goals).

## Where to start

```bash
# Read the trait
# (use the Read tool on src/backend/mod.rs and src/backend/claude.rs)

# genai docs
cargo doc -p genai --open    # or https://docs.rs/genai/0.6.0-beta.17

# Add the dev-dep
# Edit Cargo.toml: under [dev-dependencies] add: wiremock = "0.6"

# Write the failing tests first in tests/tensorzero_backend.rs, then impl
```
