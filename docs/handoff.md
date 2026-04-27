# loker - handoff document

Project context, conventions, and constraints. Read this once per session
when picking up work. CLAUDE.md is the entry point; this is the depth.

## WHY

loker is an LLM orchestration engine built around three primitives that
current Rust crates lack as production-ready: cross-family aggregation
(parallel calls to anthropic/google/openai/zhipu, merged to reduce
correlated failures), escalating retry (cheap -> medium -> strong with
verify gates), and first-class verify hooks (shell command, LLM judge,
test runner gating retries).

Hard fork of [`ducks/lok`](https://github.com/ducks/lok) (MIT). Origin
commit of the fork is at the start of `git log` - everything before the
M0 commit was authored by ducks, and that copyright stays in `LICENSE`.

## Status

M0 (fork prep) done. Active milestone: M1 (TensorZero backend). For the
full milestone list with test contracts, read the design doc; for active
work, read `docs/plans/2026-04-25-m1-tensorzero-backend.md`.

## Intent (decision boundaries)

- TDD-first for orchestration primitives. Read the design doc §8 M_N
  test contract, write the failing test, then implement. The whole point
  is verifiable behavior - skipping the test contract defeats it.
- New primitives (Strategy, Aggregator, VerifyHook) land as new modules.
  The existing `consensus.rs` / `apply_verify/` code stays working until
  the new modules subsume it. Don't mutate-in-place.
- When implementing a backend, mock the HTTP layer (`wiremock`) before
  writing the impl. Real-gateway tests are opt-in via env var, never CI
  default.
- Don't promote primitives that aren't built yet. The README is honest
  about pre-v0; aspirational examples land when the milestone lands.
- Confirm with the user before `make release` - it auto-versions, tags,
  pushes, and installs to `/usr/local/bin`.

## HOW

```bash
make check                                     # fmt + clippy + test (pre-merge gate)
cargo test -q                                  # 466 unit + 6 integration as of M0
cargo run --bin loker -- doctor                # smoke-test the CLI
LOKER_TZ_INTEGRATION=1 cargo test              # opt-in TensorZero gateway tests
```

`make release` auto-versions `YYYYMMDD.0.X`, tags, builds, installs, and
pushes - run manually after confirmation. Note the Makefile still copies
to `/usr/local/bin/lok`; the binary is `loker` (separate cleanup).

## Constraints

- Preserve ducks's MIT copyright in `LICENSE`. Both copyright lines stay.
- `lok.toml` and `.lok/workflows/` keep their names until the
  config-rename milestone (with deprecation window for both names).
- `reqwest` stays - the existing backends still use it. M1 uses `genai`
  exclusively; coexistence is intentional.
- Unit tests must not depend on TensorZero being installed - use
  `wiremock`. Integration tests are opt-in via `LOKER_TZ_INTEGRATION=1`.
- v0 verification is binary pass/fail. No logprobs or semantic-similarity
  scoring until a later milestone (design doc §10 non-goals).

### How to run the live integration test

`tests/tensorzero_integration.rs` (CLO-252) drives one round-trip through
`TensorZeroBackend::query` against a real local gateway. It skips silently
when `LOKER_TZ_INTEGRATION` is unset, so plain `cargo test` is unaffected.
To run it:

1. `cd tensorzero && docker compose up -d` to start the Tier-2 stack.
2. Wait for `curl -fsS "${TENSORZERO_GATEWAY_URL:-http://localhost:3000}/health"`
   to return 200.
3. `LOKER_TZ_INTEGRATION=1 cargo test --test tensorzero_integration`.
4. Optional overrides: `TENSORZERO_GATEWAY_URL` (default
   `http://localhost:3000`), `LOKER_TZ_INTEGRATION_FUNCTION` (default
   `loker_d1_openai`), `TENSORZERO_API_KEY` (any non-empty token - the
   `genai` client prepends `Bearer ` automatically; the gateway accepts any
   value, upstream provider enforces auth).

## Function-name family convention (TensorZero, M1)

TensorZero functions exposed to loker MUST be named
`loker_<purpose>_<family>`, where `<family>` is the substring after the
final `_` and identifies the model family (`anthropic`, `openai`,
`google`, `zhipu`, ...). The cross-family judge guard (FR-13) and
`family_of(backend_id)` (CLO-251 / T-015) derive the family from this
suffix; unknown suffixes MUST be rejected at config-load.

Canonical examples (see `tensorzero/config/tensorzero.toml`):

- `loker_d1_anthropic` -> family `anthropic`
- `loker_d1_openai`    -> family `openai`

The convention is set by the D1 round-trip spike. Source of truth:
`docs/spikes/2026-04-25-tensorzero-roundtrip.md`. Path / header / error
mapping for the gateway also live there; the live skeleton is
`src/backend/tensorzero.rs`.

### TensorZero backend (lok.toml)

Declared as a top-level `[tensorzero]` table. The config layer validates
required fields, URL shape, and timeout at parse time, then resolves
`api_key_env` via `std::env::var` when constructing the runtime opts -
the backend module itself stays env-free.

```toml
[tensorzero]
endpoint = "https://gateway.local:3000"
default_model = "loker_d1_openai"
api_key_env = "TENSORZERO_API_KEY"
timeout_secs = 60

[tensorzero.retry_policy]
max_retries = 0
delay_ms = 1000
```

## Reference

| Need | Read |
|---|---|
| Architecture, primitives, milestones, test contracts | `/Users/mk/Work/investigations/sakana-fugu/loker-design.md` |
| HITL hook + browser UI (M10-M11) | `/Users/mk/Work/investigations/sakana-fugu/loker-hitl-design.md` |
| Active M1 task contract | `docs/plans/2026-04-25-m1-tensorzero-backend.md` |
| Find lok-name leftovers in code | `rg -n "lokomotiv\|\"lok\"" --type rust` |
| Milestones already shipped | `git log --oneline \| rg "^[0-9a-f]+ M[0-9]"` |
