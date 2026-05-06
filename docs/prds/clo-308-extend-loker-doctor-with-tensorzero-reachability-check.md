# PRD: CLO-308 — Extend `loker doctor` with TensorZero reachability check

## Context

Roadmap: T-039 (Phase 8). Parent PRD: FR-35.
The TensorZero config schema (CLO-250) and `create_backend("tensorzero")` wiring (CLO-261) are both complete. `loker doctor` currently validates local CLI binaries and API keys, but has no reachability probe for the TensorZero HTTP gateway.

## Goal

Add a TensorZero gateway reachability check to `loker doctor`. The check sends an HTTP `GET` to the configured gateway's `/health` endpoint and reports `HEALTHY` or `UNREACHABLE` with the underlying error class. Exit code remains non-zero only when at least one check fails.

## Acceptance criteria

1. `loker doctor` prints a `tensorzero` row with `HEALTHY` (green) or `UNREACHABLE` (red, with error class).
2. If `[tensorzero]` is not configured in `lok.toml`, `loker doctor` prints `tensorzero — not configured` (dimmed, non-failing).
3. The probe honours `lok.toml` `[tensorzero]` (`endpoint`, `api_key_env`) and env-overrides for `TENSORZERO_GATEWAY_URL` / `TENSORZERO_API_KEY`.
4. Error classes are: `DNS` (resolution failure), `ConnectionRefused`, `Timeout`, `Auth` (401/403), `ServerError` (5xx), `Network` (other transport).
5. Exit code is non-zero only when at least one check fails (preserves existing behaviour).
6. Unit tests in `src/doctor.rs` or `tests/doctor.rs` cover: 200 OK, connection refused, DNS fail, timeout, 401, 5xx — all via wiremock.

## Non-goals

- Capability discovery (model list, function inference) — future ticket.
- Latency benchmarking — only reachability.
- Changing the existing binary/API-key check behaviour.
- Adding a `Backend::health()` trait method.

## Design decisions

- Extract `Doctor` logic from `main.rs` into `src/doctor.rs` for testability.
- Use `reqwest::Client` directly for the health probe (lightweight, no `genai` overhead).
- Resolve `api_key_env` via `TensorZeroConfig::to_backend_opts()` to reuse existing env-resolution logic.
- Error classification by inspecting `reqwest::Error` kind + HTTP status code.

## References

- `docs/discovery/clo-308.md`
- `src/config.rs` — `TensorZeroConfig`
- `src/backend/tensorzero.rs` — endpoint normalisation pattern
- TensorZero docs: `/health` endpoint returns `{ "gateway": "ok", ... }`
