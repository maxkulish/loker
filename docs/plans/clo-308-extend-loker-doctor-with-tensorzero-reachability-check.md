# Plan: CLO-308 — Extend `loker doctor` with TensorZero reachability check

**Linear**: https://linear.app/cloud-ai/issue/CLO-308
**Design Doc**: ../design-docs/clo-308-extend-loker-doctor-with-tensorzero-reachability-check.md
**Discovery**: ../discovery/clo-308.md
**Created**: 2026-05-05

---

## Overview

This is a small, self-contained implementation with two phases:
1. **Extract** existing Doctor logic from `main.rs` into `src/doctor.rs` (refactor for testability).
2. **Add** TensorZero reachability probe + coloured output + exit-code logic.

Total estimated effort: **S** (single session).

---

## ST1 — Extract Doctor Module

**Goal**: Make Doctor logic testable without running the CLI binary.

### Steps

1. Create `src/doctor.rs`:
   - Define `CheckRow` struct (name, status, detail, is_ok, is_critical).
   - Copy existing binary-check loop (codex/npx/claude) and API-key check loop from `main.rs`.
   - Map existing `println!` output into `CheckRow` accumulation.
   - Implement `pub async fn run(config: &Config) -> (Vec<CheckRow>, bool)`.

2. Update `src/main.rs`:
   - Add `mod doctor;`.
   - Replace inline Doctor arm with:
     ```rust
     Commands::Doctor => {
         let (rows, failed) = doctor::run(&config).await;
         print_doctor_rows(&rows);
         std::process::exit(if failed { 1 } else { 0 });
     }
     ```
   - `print_doctor_rows` is a small pretty-printer using `colored` crate (already in deps).

3. Verify existing behaviour:
   - `cargo run -- doctor` should produce identical output to before.
   - `cargo test` still passes.

**Output**: `src/doctor.rs` exists and compiles; `main.rs` Doctor arm is refactored.

---

## ST2 — Implement TensorZero Reachability Probe

**Goal**: Add the `/health` probe inside `doctor::run`.

### Steps

1. In `src/doctor.rs`, implement `probe_tensorzero(config: &Config) -> CheckRow`:
   - If `config.tensorzero.is_none()` → dimmed row `tensorzero — not configured` (not critical).
   - Else resolve `TensorZeroConfig::to_backend_opts()`.
   - If resolution fails → `UNREACHABLE (config)` — print the missing env var name.
   - Build probe URL: `{endpoint}/health` (do NOT apply the OpenAI `/openai/v1/` normalisation — `/health` is a root endpoint).
   - Send `reqwest::get` with:
     - timeout = 5 seconds
     - `Authorization: Bearer <api_key>` if `api_key.is_some()`.
   - Classify response/error into `CheckRow`.

2. Error classification logic:

   | Condition | Row status | Critical |
   |-----------|-----------|----------|
   | HTTP 200–299 | `HEALTHY` | yes, passed |
   | HTTP 401–403 | `UNREACHABLE (auth)` | yes, failed |
   | HTTP 500–599 | `UNREACHABLE (server error)` | yes, failed |
   | `reqwest::Error::is_timeout()` | `UNREACHABLE (timeout)` | yes, failed |
   | `reqwest::Error::is_connect()` + DNS error | `UNREACHABLE (DNS)` | yes, failed |
   | `reqwest::Error::is_connect()` + other | `UNREACHABLE (connection refused)` | yes, failed |
   | Other `reqwest::Error` | `UNREACHABLE (network)` | yes, failed |

3. Wire into `doctor::run`:
   - Add the row to the vec.
   - `failed` bool accumulates any critical failure.

**Output**: `loker doctor` shows TensorZero row; classification branches implemented.

---

## ST3 — Unit Tests (wiremock)

**Goal**: All branches tested without a real gateway.

### Tests (add under `src/doctor.rs #[cfg(test)]` or `tests/doctor.rs`)

1. `tensorzero_healthy_200` — wiremock returns `{ "gateway": "ok" }`, 200.
2. `tensorzero_not_configured_passes` — config without `[tensorzero]` → not a failure.
3. `tensorzero_connection_refused` — probe to closed port.
4. `tensorzero_dns_failure` — probe to non-existent hostname (e.g. `http://invalid.invalid`).
5. `tensorzero_timeout` — wiremock with 20s delay + client timeout 1s.
6. `tensorzero_auth_401` — wiremock returns 401.
7. `tensorzero_server_503` — wiremock returns 503.
8. `exit_code_nonzero_when_fails` — assert `run()` returns `true` for `failed`.

**Output**: `cargo test doctor` passes 100%.

---

## ST4 — Integration / Manual Verification

1. **Unit-only**: `cargo test doctor`.
2. **Full suite**: `cargo test` (ensure no regressions).
3. **Manual (optional, local)**:
   ```bash
   # With TensorZero running
   LOKER_TZ_INTEGRATION=1 cargo run -- doctor   # should show HEALTHY
   # Stop stack
   docker compose -f deploy/tensorzero/docker-compose.yml down
   cargo run -- doctor                             # should show UNREACHABLE
   ```

**Output**: All checks green, manual verification documented.

---

## Rollback Plan

If any step fails catastrophically, the design and plan docs are preserved. Revert to the design doc checkpoint and re-assess.

## Dependencies

- CLO-250 (TensorZero config schema) — **done**. `TensorZeroConfig` + `to_backend_opts()` are available.
- CLO-261 (`create_backend("tensorzero")` wiring) — **done**. Ensures endpoint normalisation pattern exists for reference.

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| `main.rs` Doctor arm refactor changes output unexpectedly | Low | Low | Snapshot test or manual diff before/after |
| DNS test is flaky (depends on network resolution) | Low | Low | Use `http://invalid.invalid` which is a reserved name and guaranteed NXDOMAIN |
| Design doc says `/health` is a root endpoint but the actual behaviour is different | Low | High | Verified from TensorZero docs and the design doc cites the source. If implementation reveals different behaviour, escalate. |

---

## Acceptance Criteria Re-Check

- [ ] `cargo test doctor` passes with 8/8 tests.
- [ ] `cargo run -- doctor` with `[tensorzero]` configured prints `✓ tensorzero - HEALTHY` or `✗ tensorzero - UNREACHABLE (class)`.
- [ ] `cargo run -- doctor` without `[tensorzero]` prints dimmed `not configured` and exit code zero.
- [ ] Existing binary/API-key checks unchanged.
- [ ] Exit code non-zero only when at least one check fails.
