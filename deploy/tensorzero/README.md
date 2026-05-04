# TensorZero Tier-2 Deployment (local development)

This directory provides a one-command `docker compose up` recipe for a local
Tier-2 TensorZero stack — gateway + ClickHouse + UI — so you can run loker
against a real backend without manual setup.

## Quick start

```sh
# 1. Copy and edit the env template with your API keys
cp ../../tensorzero/.env.example .env
# Edit .env: set OPENAI_API_KEY (required), ANTHROPIC_API_KEY (optional)

# 2. Start the stack
docker compose up -d

# 3. Verify the gateway is healthy
curl http://localhost:3000/health
# Expected: {"gateway":"ok","clickhouse":"ok"}

# 4. Open the TensorZero UI
open http://localhost:4000
```

## Services

| Service | Image | Port | Purpose |
|---------|-------|------|---------|
| `gateway` | `tensorzero/gateway:2026.4.1` | 3000 | OpenAI-compatible HTTP inference gateway |
| `clickhouse` | `clickhouse:lts` | 8123 | Analytics store for gateway observability |
| `ui` | `tensorzero/ui:2026.4.1` | 4000 | Read-only inspector and playground |

## Configuration

The gateway loads its configuration from
[`../../tensorzero/config/tensorzero.toml`](../../tensorzero/config/tensorzero.toml),
which defines:

- Two model providers: Anthropic (`claude-haiku-4-5`) and OpenAI (`gpt-4o-mini`)
- Two chat functions: `loker_d1_anthropic` and `loker_d1_openai`
- Observability backed by the ClickHouse container

### Environment variables

| Variable | Required | Default | Description |
|----------|----------|---------|-------------|
| `OPENAI_API_KEY` | **Yes** | — | OpenAI API key for the `loker_d1_openai` function |
| `ANTHROPIC_API_KEY` | No | `placeholder-not-set` | Anthropic API key for the `loker_d1_anthropic` function |

## Running the integration test

With the stack running, execute the opt-in TensorZero round-trip test:

```sh
cd ../..  # project root
LOKER_TZ_INTEGRATION=1 cargo test --test tensorzero_integration
```

Optional overrides:
- `TENSORZERO_GATEWAY_URL` (default `http://localhost:3000`)
- `LOKER_TZ_INTEGRATION_FUNCTION` (default `loker_d1_openai`)
- `TENSORZERO_API_KEY` (any non-empty token)

## Clean up

```sh
docker compose down -v    # stops containers and removes clickhouse data volume
```

## Notes

- This is a **development-only** deployment. TLS, auth, and horizontal scaling
  are out of scope for Tier-2.
- The gateway binds to `0.0.0.0:3000` inside the container; the compose file
  maps it to the same port on the host. Change the left side of `ports:` to
  use a different host port.
- ClickHouse data persists in a named Docker volume (`clickhouse-data`).
  Use `docker compose down -v` to delete it.
