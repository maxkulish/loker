# TensorZero round-trip spike (D1) — verdict

- Linear: CLO-243
- Date: 2026-04-25
- Branch: `feat/clo-243-tensorzero`
- Stack: gateway `tensorzero/gateway:lts` + ClickHouse `clickhouse:lts` + UI `tensorzero/ui:lts`, all from `tensorzero/docker-compose.yml`
- Driver: `examples/tensorzero_spike.rs` (HTTP `reqwest` against `POST /openai/v1/chat/completions`)
- Raw evidence: `tests/fixtures/tensorzero/{anthropic_auth_failure,openai_success,unknown_function}_{request,response}.json` plus `summary.json`

## Verdict

**Go.** A loker process can drive TensorZero end-to-end through one HTTP endpoint with the OpenAI-shaped chat-completions schema. The gateway answers in two relevant shapes:

- **2xx** — OpenAI-compatible `chat.completion` envelope, plus a `tensorzero::function_name::<fn>::variant_name::<variant>` value in `model` and a TensorZero `episode_id` field.
- **non-2xx** — single-key `{"error": {"message": "..."}}` envelope. Status differentiates *which* shape: 404 for unknown function, 502 when an upstream provider rejected the call.

There are no surprises that block M1. There is one defect in the existing `src/backend/tensorzero.rs` error mapping (502 → retryable Network) that must be fixed in CLO-247, and one path mismatch in the same file's wiremock test config (`/v1/` vs the real `/openai/v1/`).

## 1. HTTP shape and request headers

Endpoint: `POST {gateway}/openai/v1/chat/completions`. Note the `/openai/v1/` prefix — a lok-style `/v1/chat/completions` returns 404. The gateway also exposes a native `POST /inference` endpoint, but the OpenAI-compat path is what the `genai` crate's `AdapterKind::OpenAI` will hit, so we lock in the OpenAI route.

Required request headers (verified live):

- `Authorization: Bearer <anything>` — gateway accepts any value by default; auth is enforced **upstream** (gateway forwards using `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` from its own env). Spike used `Bearer not-used`.
- `Content-Type: application/json`.

Body keys we use and the gateway accepts (see `*_request.json` fixtures):

```
{
  "model": "tensorzero::function_name::<function_name>",
  "messages": [{"role": "user", "content": "..."}],
  "max_tokens": 32,
  "temperature": 0.0
}
```

The `model` field doubles as routing: `tensorzero::function_name::<fn>` selects a function (which then picks variants); `tensorzero::model_name::<m>` would bypass the function layer. We will only use `function_name::` form — variants/observability live there.

## 2. Model-name mapping (loker → gateway)

- loker config field: `backend = "tensorzero/<function-name>"` (design-doc §5).
- loker strips the `tensorzero/` prefix and sends `tensorzero::function_name::<function-name>` as the `model` field.
- The gateway echoes `tensorzero::function_name::<function-name>::variant_name::<variant>` back in the response `model` on success. The variant is the gateway's choice, not loker's.

Spike example:

| Step | Value |
|---|---|
| loker workflow TOML | `backend = "tensorzero/loker_d1_openai"` |
| loker → gateway request `model` | `tensorzero::function_name::loker_d1_openai` |
| gateway → loker response `model` | `tensorzero::function_name::loker_d1_openai::variant_name::mini_v1` |

This implies loker has full control of the function-name namespace (we choose it when we author `tensorzero/config/tensorzero.toml`). It does **not** see the underlying provider in the response.

## 3. Family-id source of truth (resolves design-doc §11 Q4)

**Decision: `family_of(backend_id)` derives from the function name on the loker side. The gateway response carries no provider hint.**

Evidence: the success response model is `tensorzero::function_name::loker_d1_openai::variant_name::mini_v1`. There is no `provider`, `adapter`, or family field in the response body. The variant name `mini_v1` is opaque to loker and is *not* a stable provider signal — we picked it; tomorrow it could be `gemini_v2` behind the same function.

Implementation:

- `tensorzero/config/tensorzero.toml` is the single registry. Function names follow the convention `loker_<purpose>_<family>` (e.g. `loker_d1_openai`, `loker_d1_anthropic`) so a static prefix-strip lookup in loker covers the cross-family-judge guard (`require_judge_different_family`, design-doc §4.3).
- loker's `family_of()` implementation: parse the function name's family suffix from the configured `backend_id`. Reject unknown suffixes at config-load time so a typo cannot silently let a same-family judge run.
- We do **not** read TensorZero's gateway config at loker startup; the family registry stays in loker config. (We considered scraping `/inference/info` or similar, but the gateway has no such endpoint, and a registry that lives in two places drifts.)

Follow-up for CLO-247: codify the family-suffix convention in `docs/handoff.md` and surface it in the `tensorzero.toml` template comments.

## 4. Token-count fields

Verified `usage` keys on a 200 response:

```
completion_tokens: u32
prompt_tokens: u32
total_tokens: u32
prompt_tokens_details: { cached_tokens: u32 }
tensorzero_cost: number | null
```

Mapping to `crate::backend::TokenUsage` (currently `prompt: u32, completion: u32`):

- `prompt_tokens` → `TokenUsage.prompt` (existing).
- `completion_tokens` → `TokenUsage.completion` (existing).
- `total_tokens` is derivable; we drop it.
- `prompt_tokens_details.cached_tokens` and `tensorzero_cost` are useful for the future cost-summary feature (design-doc §6) but are out of scope for M1. Leave them on the wire, do not surface them yet.

No schema change needed for M1. Add a follow-up note for M5/M6 to bring `cached_tokens` and `tensorzero_cost` through `QueryOutput` once we wire `summary.json` aggregation.

## 5. Error envelopes and retryable error classes

Captured shapes:

| Scenario | HTTP | Body | Class loker should map to |
|---|---|---|---|
| Unknown function | 404 | `{"error": {"message": "Unknown function: <name>"}}` | `Config` (non-retryable; loker config typo) |
| Upstream auth error (Anthropic 401 wrapped) | **502** | `{"error": {"message": "All variants failed with errors: haiku_v1: All model providers failed to infer with errors: anthropic: Error 401 Unauthorized from anthropic client: ..."}}` | `Auth` (non-retryable) |
| Successful upstream call | 200 | OpenAI `chat.completion` | n/a |

Untested in this spike (gated on real keys + induced load):

- 429 from upstream — assumed wrapped in 502 + body string `"rate_limit"` based on TensorZero docs. Should map to `RateLimit` (retryable).
- Genuine gateway 5xx (gateway crash, ClickHouse down) — should map to `Network` (retryable).
- Connect/timeout from loker → gateway — already handled in `webc::Reqwest` arm of `map_genai_error`.

### Defect found in existing `src/backend/tensorzero.rs::map_status`

```rust
500..=599 => BackendError::Network { message: msg },   // <-- broken for upstream auth
```

The gateway's habit of returning **502 for upstream provider errors regardless of underlying class** means the current branch will mis-classify a permanent auth failure as a retryable network error and trigger pointless retries against a wrong key. Fix in CLO-247:

- For 5xx, inspect the body. If the message contains an upstream auth signature (`"401"`, `"Unauthorized"`, `"invalid x-api-key"`, `"authentication_error"`), return `BackendError::Auth`. Otherwise keep the `Network` mapping.
- Same body-inspection trick for `"rate_limit"` / `"429"` → `RateLimit`.
- Add wiremock cases for the two captured fixtures (`anthropic_auth_failure_response.json` and `unknown_function_response.json`) to lock the mapping in.

This defect was discoverable only via the live round-trip — wiremock unit tests today fabricate clean 401/429/500 statuses that the gateway never actually emits.

## 6. Open follow-ups (rolled into CLO-247 unless noted)

1. **Path prefix mismatch (CLO-247):** `src/backend/tensorzero.rs` test config builds the endpoint as `format!("{}/v1/", server.uri())`. The real gateway path is `/openai/v1/`. Today both work because the unit tests mock whatever path is asked, but a production smoke test against the gateway will 404. Update both the runtime config helper and `examples/workflows/*` defaults.
2. **502 body-inspection error mapping (CLO-247):** see §5.
3. **Streaming behaviour (defer to M2 / CLO-?):** spike used non-streaming. `chat_completion` variants accept `stream: true` and return SSE; need a second spike before strategy work that wants partial output.
4. **Real Anthropic capture (defer):** `anthropic_auth_failure_response.json` is the wrapped-401 path because the local env had no key. A success-path fixture for Anthropic (`anthropic_success_response.json`) should be captured the next time someone runs the stack with a valid key. Schema is expected to mirror OpenAI (TensorZero normalises it on the way out), but worth verifying.
5. **429 / timeout fixtures (defer):** induce by saturating an upstream provider in a load test; out of scope for D1.
6. **Health endpoint surface (informational):** `/health` reports `{gateway, clickhouse, postgres, valkey, valkey_cache}` even when postgres/valkey are not externally configured — the gateway runs them embedded. Worth noting in the deployment-recipe doc but not blocking.
7. **`max_tokens: 32` in `examples/tensorzero_spike.rs`** is intentionally small for the spike. Real callers will use TensorZero variant defaults (no `max_tokens` in body).

## 7. Reproduce

```bash
cd tensorzero
cp .env.example .env   # fill OPENAI_API_KEY (and ANTHROPIC_API_KEY if available)
docker compose up -d
curl -s http://localhost:3000/health   # expect all "ok"

cd ..
cargo run --quiet --example tensorzero_spike
# writes tests/fixtures/tensorzero/<scenario>_{request,response}.json + summary.json

cd tensorzero && docker compose down
```

Override env knobs: `TENSORZERO_GATEWAY_URL` (default `http://localhost:3000`), `LOKER_TZ_FIXTURE_DIR` (default `tests/fixtures/tensorzero`).
