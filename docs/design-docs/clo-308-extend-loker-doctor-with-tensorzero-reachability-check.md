# CLO-308: Extend `loker doctor` with TensorZero reachability check

**Linear Task**: https://linear.app/cloud-ai/issue/CLO-308
**Status**: Design
**Author**: Team
**Created**: 2026-05-05

---

## Summary

Add a TensorZero gateway reachability probe to `loker doctor`. The probe sends `GET {endpoint}/health` with the configured auth header and reports `HEALTHY` (green) or `UNREACHABLE` (red, with error class). If `[tensorzero]` is not configured in `lok.toml`, the check prints "not configured" and does not fail.

## Background

Today `loker doctor` validates only local LLM CLI binaries (`codex`, `npx`, `claude`) and API key env vars. With the TensorZero backend (CLO-250 config, CLO-261 wiring) becoming the primary M1 HTTP gateway, operators need a quick way to verify gateway reachability before launching a full orchestration run — without manually curling the `/health` endpoint.

The TensorZero gateway exposes `GET /health` which returns JSON `{"gateway":"ok"}` (optionally `{"gateway":"ok","postgres":"ok"}` when observability is enabled). Any 2xx response indicates the gateway is reachable and accepting HTTP.

### Prior Research

Discovery report (`docs/discovery/clo-308.md`) found:
- `TensorZeroConfig` already provides `endpoint` + `api_key_env` resolution via `to_backend_opts()`.
- `reqwest` is an existing dependency, sufficient for a lightweight probe.
- `Doctor` logic is currently inline in `main.rs` and untestable.
- Approach chosen: extract `src/doctor.rs` module for testability.

Baseline score: 9/10.

---

## Architecture

### Component Overview

A new `doctor` module (`src/doctor.rs`) owns all diagnostic checks. The CLI's `Doctor` command delegates to `doctor::run(config)`, which returns a `DoctorResult` indicating:
- Check outputs (Vec of pretty-printed rows)
- Whether any check failed (bool for exit code)

The TensorZero check lives inside `doctor::run` as a dedicated async helper.

### Affected Components

| Component | Change Type | Description |
|-----------|-------------|-------------|
| `src/main.rs` | Modified | Doctor arm delegates to `doctor::run` instead of inline logic. |
| `src/doctor.rs` | New | Extracted Doctor module with binary checks, API-key checks, and new TensorZero reachability probe. |

### Dependencies

- **Internal**: `src/config` (`TensorZeroConfig`, `load_config`); `src/backend` (for endpoint normalisation pattern, but not directly for the probe).
- **External**: `reqwest` (HTTP probe); `colored` (terminal output) — both already in `Cargo.toml`. `wiremock` for tests (dev-dependency, already present).

---

## Detailed Design

### Implementation Approach

1. **Extract existing Doctor logic** from `main.rs` into `src/doctor.rs`:
   - `pub async fn run(config: &Config) -> (Vec<CheckRow>, bool)`
   - `CheckRow` contains `name`, `status`, `detail`, `is_ok`

2. **Add TensorZero probe**:
   - If `config.tensorzero.is_none()` → row: `tensorzero — not configured` (dimmed)
   - Else resolve endpoint + api_key via `TensorZeroConfig::to_backend_opts()`
   - Build URL: `{endpoint}/health` (normalised the same way as `TensorZeroBackend`: append `/openai/v1/` does NOT apply to `/health` — health is at root)
   - Send `reqwest::get` with short timeout (5s) and `Authorization: Bearer <key>` header iff key present
   - Classify result:
     - `200..=299` → `HEALTHY`
     - `401..=403` → `UNREACHABLE (auth)`
     - `500..=599` → `UNREACHABLE (server error)`
     - `reqwest::Error::is_connect()` → `UNREACHABLE (connection refused | DNS)`
     - `reqwest::Error::is_timeout()` → `UNREACHABLE (timeout)`
     - Other → `UNREACHABLE (network)`

3. **Exit code**:
   - `run()` returns `failed = any(checks, |c| !c.is_ok && c.is_critical)`
   - Treat TensorZero check as **non-critical** when `tensorzero.is_none()` (not configured → not a failure)
   - Treat TensorZero check as **critical** when `tensorzero.is_some() && probe fails`

### Code Structure

```rust
// src/doctor.rs
use crate::config::Config;

#[derive(Debug, Clone)]
pub struct CheckRow {
    pub name: &'static str,
    pub status: String,
    pub detail: Option<String>,
    pub is_ok: bool,
    pub is_critical: bool,
}

pub async fn run(config: &Config) -> (Vec<CheckRow>, bool) {
    let mut rows = Vec::new();
    // existing binary checks extracted from main.rs
    // existing API-key checks
    // new TensorZero reachability
    rows.push(probe_tensorzero(config).await);
    let failed = rows.iter().any(|r| r.is_critical && !r.is_ok);
    (rows, failed)
}

async fn probe_tensorzero(config: &Config) -> CheckRow {
    let tz = match config.tensorzero {
        None => return CheckRow { ... not configured, not critical },
        Some(ref cfg) => cfg,
    };
    let opts = match tz.to_backend_opts() {
        Ok(o) => o,
        Err(_) => return CheckRow { ... UNREACHABLE (config), critical },
    };
    let url = format!("{}/health", opts.endpoint.trim_end_matches('/'));
    // ... reqwest call and classification
}
```

### Output Format

```
Checking backends...
  ✓ codex - ready
  ✗ gemini - not found
    Install Node.js (npx comes with npm)
  ✓ claude - ready

Checking API keys...
  ✓ ANTHROPIC_API_KEY - set (claude backend)
  ○ GOOGLE_API_KEY - not set (gemini backend)

Checking TensorZero gateway...
  ✓ tensorzero - HEALTHY
  # or
  ✗ tensorzero - UNREACHABLE (connection refused)
```

---

## Implementation Plan

### Phase 1: Extract Doctor module

- [ ] Create `src/doctor.rs` with `run(config: &Config)` returning `(Vec<CheckRow>, bool)`.
- [ ] Move existing binary/API-key checks from `main.rs` into `doctor::run`.
- [ ] Update `main.rs` Doctor arm to call `doctor::run(&config).await` and print rows.
- [ ] Verify `cargo run -- doctor` still produces identical output for existing checks.

### Phase 2: Add TensorZero reachability probe

- [ ] Implement `probe_tensorzero(config: &Config)` in `src/doctor.rs`.
- [ ] Use `reqwest::Client::new().get(url).timeout(...)` with `Authorization` header.
- [ ] Implement error classification:
  - HTTP status-based (401, 403, 5xx)
  - `reqwest::Error` kind-based (DNS, connection refused, timeout, generic network)
- [ ] Wire coloured output:
  - `HEALTHY` → green
  - `UNREACHABLE (class)` → red
  - `not configured` → dimmed

### Phase 3: Unit tests (wiremock)

- [ ] `tensorzero_healthy` — wiremock returns 200 with `{ "gateway": "ok" }`
- [ ] `tensorzero_not_configured` — config without `tensorzero` → not a failure
- [ ] `tensorzero_connection_refused` — wiremock shutdown, expect `connection refused`
- [ ] `tensorzero_dns_failure` — probe to `http://invalid.invalid` → `DNS`
- [ ] `tensorzero_timeout` — wiremock with `set_delay(20s)`, client timeout 1s → `timeout`
- [ ] `tensorzero_auth_failure` — wiremock returns 401 → `auth`
- [ ] `tensorzero_server_error` — wiremock returns 503 → `server error`
- [ ] `exit_code_nonzero_when_critical_check_fails` — when configured but unreachable
- [ ] `exit_code_zero_when_not_configured` — unconfigured → passes

---

## Constraints

**Must**:
- Use existing `reqwest` dependency for the probe (no new HTTP client).
- Resolve auth key through `TensorZeroConfig::to_backend_opts()` to reuse existing env-resolution logic.
- Preserve existing CLI output format for binary/API-key checks.
- Exit code non-zero only when at least one critical check fails.

**Must-not**:
- Import or depend on `genai` / `TensorZeroBackend` for the probe (overkill for a `GET /health`).
- Modify the `Backend` trait or any backend implementations.
- Block the async runtime with synchronous HTTP calls.

**Escalate when**:
- Changing the `Backend` trait or adding a `health()` method.
- Introducing a new dependency for HTTP or terminal output.
- The probe needs to validate actual inference capability (model list, function inference) — this is out of scope.

---

## Acceptance Criteria

- [ ] `cargo test doctor` passes with 0 failures — all 8 wiremock test cases pass.
- [ ] `cargo run -- doctor` with `[tensorzero]` configured and gateway running prints `✓ tensorzero - HEALTHY`.
- [ ] `cargo run -- doctor` with `[tensorzero]` configured but gateway down prints `✗ tensorzero - UNREACHABLE (class)` with correct class, and exit code is non-zero.
- [ ] `cargo run -- doctor` without `[tensorzero]` prints `tensorzero — not configured` (dimmed) and exit code is zero (preserves existing).
- [ ] Existing binary/API-key checks remain unchanged in output and behaviour.

**Verification method**: `cargo test doctor && cargo run -- doctor`

---

## Evaluation

| # | Test | Expected Result | Command / Steps |
|---|------|-----------------|-----------------|
| 1 | Healthy gateway | `is_ok = true`, status = `HEALTHY` | `probe_tensorzero` with wiremock returning 200 |
| 2 | Not configured | `is_critical = false`, `is_ok = true` | `probe_tensorzero` with `Config::default()` (no tensorzero) |
| 3 | Connection refused | Status contains `connection refused` | Wiremock shutdown between setup and request |
| 4 | DNS failure | Status contains `DNS` | Probe to non-existent hostname |
| 5 | Timeout | Status contains `timeout` | Wiremock with 20s delay, probe with 1s timeout |
| 6 | Auth failure | Status contains `auth` | Wiremock returns 401 |
| 7 | Server error | Status contains `server error` | Wiremock returns 503 |
| 8 | Exit codes | Non-zero for failed critical checks, zero otherwise | Assert on `failed` bool from `doctor::run` |

**Edge cases to cover**:
- Empty `api_key_env` → probe without `Authorization` header.
- Invalid endpoint URL → handled at config parse time (not doctor's concern).
- Network unreachable (no Wi-Fi) → caught as generic `network` error.
- Redirect from `/health` to something else → follow redirect; 2xx means healthy.

---

## Testing Strategy

- **Unit tests** (in `src/doctor.rs` `#[cfg(test)] mod tests`):
  - Wiremock-backed tests for all 7 probe scenarios.
  - Assert on `CheckRow` fields, not terminal output strings.
- **Integration test** (`tests/doctor.rs`):
  - Spawn `cargo run -- doctor` subprocess and assert on exit code + stdout.
  - Gated by `LOKER_TZ_INTEGRATION=1` when testing against real gateway.
- **Manual test**:
  1. Start local TensorZero gateway: `cd deploy/tensorzero && docker compose up -d`
  2. Run `cargo run -- doctor` → verify green HEALTHY row.
  3. Stop gateway → verify red UNREACHABLE row and non-zero exit.

---

## Open Questions

- None.

---

## References

- [Linear Task CLO-308](https://linear.app/cloud-ai/issue/CLO-308)
- [Discovery Report](../../docs/discovery/clo-308.md)
- [PRD](../../docs/prds/clo-308-extend-loker-doctor-with-tensorzero-reachability-check.md)
- `src/config.rs` — `TensorZeroConfig`
- `src/backend/tensorzero.rs` — endpoint normalisation pattern
- TensorZero docs — `GET /health` endpoint
