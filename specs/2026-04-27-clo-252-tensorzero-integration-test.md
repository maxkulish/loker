# Spec: CLO-252 Add opt-in tensorzero integration test gated by LOKER_TZ_INTEGRATION (FR-2)

**Created**: 2026-04-27
**Estimated scope**: S (1 new test file, ~1 doc line, ~3 sub-tasks)
**Linear**: [CLO-252](https://linear.app/cloud-ai/issue/CLO-252/add-opt-in-tensorzero-integration-test-gated-by-loker-tz-integration)
**PRD**: FR-2 (`docs/prd/2026-04-25-loker.md`)
**Roadmap**: T-009 (`docs/plans/001-implementation-roadmap.md`)
**Plan**: `docs/plans/2026-04-25-m1-tensorzero-backend.md` lines 37-38

## 1. Problem Statement

The M1 plan (`docs/plans/2026-04-25-m1-tensorzero-backend.md:37-38`) calls for
exactly two tiers of TensorZero coverage:

1. **Mocked unit tests** (`tests/tensorzero_backend.rs`, landed in CLO-248) -
   pin the wire contract against a `wiremock`-backed HTTP server. Run on every
   developer machine and on CI. Fast, hermetic, no daemon required.

2. **One opt-in integration test** against a *real* local TensorZero gateway -
   smoke-test that the same code path works end-to-end against the production
   stack, without forcing CI or a fresh checkout to depend on the gateway being
   installed.

The wiremock layer pins what loker *sends* and how it parses what it *receives*,
but it cannot detect drift between our mock body and what the real gateway
actually returns - or regressions in the bits the unit tests deliberately don't
cover (auth header reaching the upstream provider, the `genai`-OpenAI adapter
matching the gateway's `/openai/v1/chat/completions` path, the `model` field
echoed back in the variant-suffixed form `tensorzero::function_name::<fn>::variant_name::<v>`).
The D1 spike (`docs/spikes/2026-04-25-tensorzero-roundtrip.md`) proved the
round-trip works manually via `examples/tensorzero_spike.rs`; this spec's job is
to fold that proof into `cargo test` so it survives refactors.

The hard constraint from `docs/handoff.md:60-62` is: **unit tests must not
depend on TensorZero being installed**. The handoff also already advertises the
target invocation - `LOKER_TZ_INTEGRATION=1 cargo test` (handoff.md:47) - but no
test currently consumes that env var. This task closes that gap with one test.

## 2. Acceptance Criteria

- [ ] **AC1**: A new file `tests/tensorzero_integration.rs` contains a single
      `#[tokio::test]` named `tz_integration_round_trip_via_loker_d1_openai`.
      The body's first action is to read `std::env::var("LOKER_TZ_INTEGRATION")`
      and `return` early if the variable is unset or empty. The early-return is
      silent: no `eprintln!`, no `panic!`, no `println!` (matches the ticket's
      "skips silently when LOKER_TZ_INTEGRATION env var is unset").
- [ ] **AC2**: When `LOKER_TZ_INTEGRATION` is set (any non-empty value), the
      test:
      1. Reads the gateway URL from `TENSORZERO_GATEWAY_URL` (default
         `http://localhost:3000`), matching the spike's variable name
         (`examples/tensorzero_spike.rs:11`) so an operator who has the spike
         working has the test working.
      2. Reads the function name from `LOKER_TZ_INTEGRATION_FUNCTION` (default
         `loker_d1_openai`), so an operator on a Tier-2 deployment that named
         the function differently can override without editing the test.
      3. Reads the API key from the env var named in `TENSORZERO_API_KEY` (the
         actual Bearer token value, not the indirection used by `lok.toml`'s
         `api_key_env` field; the test doesn't need a TOML config). Empty/unset
         is acceptable - the spike (D1 §1) confirmed the gateway accepts any
         Bearer value because auth is enforced upstream.
      4. Builds a `TensorZeroBackend` via `TensorZeroBackend::new(opts)` with a
         5-second timeout (loose enough to tolerate cold-start latency on a
         freshly `docker compose up`'d stack, tight enough to fail fast if the
         gateway is wedged).
      5. Issues exactly one `backend.query("Reply with the single word: pong.",
         Path::new("."), None)` call. The prompt is identical to the spike's
         `openai_success` scenario so the response shape is the same one D1
         captured in the fixture.
- [ ] **AC3**: The test asserts a structurally valid response, *not* a string
      match on the model's exact reply. Concrete assertions, in order:
      1. `result.is_ok()` - any `BackendError` fails the test with the error's
         `Display` rendered into the panic message.
      2. `out.backend == "tensorzero"` - pins the backend name our code stamps
         on the output.
      3. `!out.stdout.is_empty()` - the gateway returned *some* assistant
         content. We do **not** assert `stdout == "pong"` because GPT-4o-mini
         occasionally adds punctuation, casing variants, or whitespace; the
         spike fixture (`tests/fixtures/tensorzero/openai_success_response.json`)
         is the source of truth for what "structurally valid" means here.
      4. `assert_eq!(out.model.as_deref().unwrap(), function)` where `function`
         is the env-resolved function name (default `loker_d1_openai`) - pins
         the contract that `TensorZeroBackend::query` sets `QueryOutput.model`
         from the *effective input* (the configured function name / per-call
         override), **not** from the gateway's echoed response `model` field
         (`tensorzero::function_name::<fn>::variant_name::<v>`). A future change
         that switches to propagating the response echo would trip this
         assertion deliberately.
      5. `out.usage.is_some()` and `out.usage.unwrap().prompt_tokens > 0` and
         `... .completion_tokens > 0` - pins that the genai layer extracted
         token counts from the response's `usage` block (D1 §4).
      6. `out.duration > Duration::ZERO` - sanity check that the call actually
         executed (a constructor-default `Duration::ZERO` would mean we built
         an output without making the call).
- [ ] **AC4**: `cargo test -q` (no `LOKER_TZ_INTEGRATION` in env) reports the
      test as **passed** silently - one line in the test summary, no extra
      output, no network call. Verified by running `cargo test -q
      --test tensorzero_integration` with the env var unset and observing
      `1 passed; 0 failed; 0 ignored`.
- [ ] **AC5**: `LOKER_TZ_INTEGRATION=1 cargo test --test tensorzero_integration`
      against a live local Tier-2 stack (the one declared in
      `tensorzero/docker-compose.yml`, configured by
      `tensorzero/config/tensorzero.toml`'s `loker_d1_openai` function) passes.
      Verified manually before merge; documented in the handoff snippet
      (AC7) so the next operator can reproduce.
- [ ] **AC6**: No CI workflow file under `.github/workflows/` sets
      `LOKER_TZ_INTEGRATION` or installs Docker / TensorZero. Verified by
      `rg "LOKER_TZ_INTEGRATION|tensorzero" .github/` returning zero matches
      outside of comments and existing dependency caching keys.
- [ ] **AC7**: `docs/handoff.md` already advertises the invocation
      (`LOKER_TZ_INTEGRATION=1 cargo test`, line 47). Extend the
      "Constraints" section's bullet on opt-in tests (lines 61-62) with a
      one-paragraph "How to run the live integration test" note that lists:
      (a) start the stack via `cd tensorzero && docker compose up -d`,
      (b) wait for `/health` to return 200 (the spike's first action),
      (c) run `LOKER_TZ_INTEGRATION=1 cargo test --test tensorzero_integration`,
      (d) optional `TENSORZERO_GATEWAY_URL` and `LOKER_TZ_INTEGRATION_FUNCTION`
      overrides. No README changes needed - handoff.md is the canonical
      "how to run" doc per `CLAUDE.md`'s "Read first" list.
- [ ] **AC8**: `make check` (fmt + clippy + test) exits 0 - in particular,
      the new test compiles cleanly under `-D warnings` and the early-return
      branch does not trigger `clippy::needless_return` or similar.
- [ ] **AC9**: The test imports only `loker::backend::{Backend,
      TensorZeroBackend, TensorZeroBackendOpts}` plus `std`-and-`tokio`
      essentials - the same surface CLO-248's wiremock tests use. No new
      `[dev-dependencies]` are added; `wiremock` is unused here and must not
      be imported.

**Verification method**:
- AC1, AC2, AC3, AC9: read the diff + `cargo build --tests`.
- AC4: `cargo test -q --test tensorzero_integration` (env var unset).
- AC5: manual, against a live local stack. Recorded as a one-line note in the
  PR description.
- AC6: `rg "LOKER_TZ_INTEGRATION|tensorzero" .github/`.
- AC7: read the handoff diff.
- AC8: `make check`.

## 3. Constraints

**Must**:
- Gate via env-var early-return inside the test body, not via `#[ignore]`.
  Reason: the AC explicitly says `LOKER_TZ_INTEGRATION=1 cargo test` (without
  `-- --ignored`) must run and pass the test. `#[ignore]`'d tests do not run
  under plain `cargo test` regardless of env state, so this attribute would
  break AC5.
- Skip silently. No `eprintln!` or `println!` on the skip path; the test must
  contribute a single "passed" line to the summary and nothing else. Reason:
  the ticket says "skips silently when env var unset" verbatim.
- Reuse the spike's defaults. `TENSORZERO_GATEWAY_URL` defaults to
  `http://localhost:3000`; the function name defaults to `loker_d1_openai`.
  Reason: D1 already validated this configuration end-to-end and the fixture
  evidence under `tests/fixtures/tensorzero/openai_success_response.json` is
  the de-facto specification of "what a structurally valid response looks
  like". Diverging would require new fixture evidence.
- Use `TensorZeroBackend::new` + `Backend::query`, not raw `reqwest`. Reason:
  the test's purpose is to pin the *loker* code path, not the gateway. A raw
  HTTP test would re-prove the spike but not catch regressions in the
  `genai` adapter wiring or our `BackendError` mapping.
- Keep the test self-contained in `tests/tensorzero_integration.rs`. Do not
  share fixtures with `tests/tensorzero_backend.rs` (the wiremock tests). The
  two files exercise different surfaces and should not import from each
  other. A small amount of duplication (the prompt string, the response-shape
  assertions) is preferable to a shared helper module that couples the two
  test crates.

**Must-not**:
- Add a CI job, GitHub Action, or scheduled workflow that runs the integration
  test. Reason: the AC says "no CI default config flips this on" verbatim.
- Add `wiremock`, `mockito`, or any other HTTP-mock dependency to this file.
  Reason: the integration test is, by definition, against a real gateway; a
  mock would defeat the purpose. The wiremock surface lives in CLO-248's
  file.
- Add a `[dev-dependencies]` entry to `Cargo.toml`. Reason: `tokio` and
  `loker` are already available; nothing else is needed.
- Pin the variant suffix on the response model. Reason: the gateway picks the
  variant (`mini_v1` today, `mini_v2` tomorrow) and the spike (D1 §2)
  explicitly documented this as opaque. Pinning would create a flaky test
  every time the operator's `tensorzero.toml` adds a variant.
- Pin the response stdout to an exact string (`"pong"`). Reason: GPT-4o-mini's
  output for this prompt has been observed as `"pong"`, `"Pong."`, and
  `"pong"` (lowercase, no period) across runs in the spike fixture. The AC
  asks for "structurally valid", not byte-equal.
- Block the test on `/health`. Reason: `Backend::query` already returns a
  `BackendError::Network` when the gateway is unreachable, with the
  underlying error in `Display`. A separate `/health` probe would add
  complexity without strengthening the failure message - the operator who
  set `LOKER_TZ_INTEGRATION=1` knows they are running against a live stack.
- Touch `src/backend/tensorzero.rs`, `src/config.rs`, or any wiremock test.
  Reason: those surfaces are already covered by CLO-247 / CLO-248 / CLO-250
  and are out of scope here.

**Prefer**:
- One `#[tokio::test]` (`tokio::test` flavor: default multi-thread is fine;
  the test makes one async call and is not contention-sensitive). Mirrors the
  six wiremock tests in `tests/tensorzero_backend.rs`.
- An inline `fn function_name() -> String { std::env::var("LOKER_TZ_INTEGRATION_FUNCTION").unwrap_or_else(|_| "loker_d1_openai".to_string()) }` over a const, so the override path is reachable without a recompile.
- `expect("LOKER_TZ_INTEGRATION round trip failed: {err}")` style panic
  messages over bare `unwrap()`. Reason: when the test runs and fails on a
  live gateway, the operator needs a clear signal of *which* assertion broke.
- Place the test in the M1 / TensorZero block of `docs/handoff.md` (between
  lines 60 and 65), not in a new section. Reason: keeps the constraint and
  its run instructions adjacent.

**Escalate when**:
- The default `loker_d1_openai` function turns out to be absent from the
  stack the operator happens to run (e.g. a leaner Tier-1 deployment). Stop
  and confirm whether to ship a fallback function (`loker_d1_anthropic`?) or
  document that the test is Tier-2-only.
- The `genai` crate version (currently 0.6.0-beta.17, see `Cargo.toml`)
  changes the auth header shape between draft and merge. Re-verify against a
  live stack before merging.
- A future change to `BackendError`'s `Display` would degrade the panic
  message quality. Surface in PR review; do not silently downgrade.

## 4. Decomposition

Three sub-tasks. ST1 must land before ST2 (ST2 depends on the file existing).
ST3 is independent and can land in the same commit.

1. **ST1: Create `tests/tensorzero_integration.rs` with the env-gated skip
   path.** Just the file with imports, the gate function, and the test body
   structured around `if env::var("LOKER_TZ_INTEGRATION").is_err() { return;
   }`. The body after the gate can be a `todo!()` or a placeholder
   `assert!(true)` - this sub-task only proves the file compiles and `cargo
   test -q` reports the test as passed without making a network call. Files:
   `tests/tensorzero_integration.rs`. Done when `cargo test -q --test
   tensorzero_integration` reports `1 passed`.

2. **ST2: Fill in the live round-trip body.** Replace the placeholder with
   the full call: read env vars, construct `TensorZeroBackendOpts`, build
   the backend, issue the query, assert the structurally-valid response per
   AC3. Files: `tests/tensorzero_integration.rs` (same file as ST1). Done
   when `LOKER_TZ_INTEGRATION=1 cargo test --test tensorzero_integration`
   passes against a live local stack (manual verification per AC5).

3. **ST3: Document the run procedure in `docs/handoff.md`.** Extend the
   constraints section (lines 60-62) with the four-step "How to run the live
   integration test" note (per AC7). Files: `docs/handoff.md`. Done when the
   diff reads cleanly to a fresh contributor and `make check` passes (the
   doc change does not break the test suite, but `make check` should still
   exit 0).

**Dependency order**: ST1 -> ST2 -> ST3. ST1 establishes the file shape and
proves the skip path is silent (the part that survives even if the gateway is
never available). ST2 fills in the actual round-trip body. ST3 is a doc
change that depends on ST2's final env-var names being settled.

## 5. Evaluation

| # | Test | Expected Result | How to Run |
|---|------|-----------------|------------|
| 1 | Build clean (no env var) | 0 errors, 0 warnings under `-D warnings` | `cargo build --tests` |
| 2 | Skip path silent | 1 passed; 0 failed; 0 ignored, no extra stdout | `cargo test -q --test tensorzero_integration` |
| 3 | Skip path also silent under verbose | same as above; no eprintln visible | `cargo test --test tensorzero_integration -- --nocapture` |
| 4 | Run path: live success | 1 passed; structural assertions all green | `LOKER_TZ_INTEGRATION=1 cargo test --test tensorzero_integration` (against live local stack) |
| 5 | Run path: gateway down | 1 failed; panic message includes `BackendError::Network` and the URL | `LOKER_TZ_INTEGRATION=1 cargo test --test tensorzero_integration` (with stack stopped) |
| 6 | Run path: function override | passes when `LOKER_TZ_INTEGRATION_FUNCTION=loker_d1_anthropic` and that function is configured (Tier-2) | `LOKER_TZ_INTEGRATION=1 LOKER_TZ_INTEGRATION_FUNCTION=loker_d1_anthropic cargo test --test tensorzero_integration` |
| 7 | Run path: gateway URL override | passes against a non-default port | `LOKER_TZ_INTEGRATION=1 TENSORZERO_GATEWAY_URL=http://localhost:3001 cargo test --test tensorzero_integration` |
| 8 | No CI auto-flip | zero matches outside comments | `rg "LOKER_TZ_INTEGRATION" .github/` |
| 9 | Existing wiremock tests still pass | 6 passed | `cargo test -q --test tensorzero_backend` |
| 10 | Existing integration tests still pass | unchanged | `cargo test -q --test integration` |
| 11 | Pre-merge gate | exit 0 | `make check` |
| 12 | Handoff doc readable | one paragraph, four numbered steps, env-var overrides listed | read `docs/handoff.md` diff |

**Edge cases to verify**:
- `LOKER_TZ_INTEGRATION=` (set but empty string) treats as unset and skips
  silently. Implementation: `env::var(...).ok().filter(|v| !v.is_empty()).is_none()`
  -> early return. Documented in the test's first comment.
- The gateway returns the OpenAI-shaped 200 envelope (not the native
  `/inference` shape) - the `genai` adapter we use forces the OpenAI route
  per D1 §1. We do not test the native shape; that path is unused.
- The gateway returns a 404 for an unknown function name. The test does *not*
  cover this case (the wiremock tests already do). If the operator
  misconfigures `LOKER_TZ_INTEGRATION_FUNCTION` to a name that does not exist
  on their stack, the test fails with `BackendError::Config`, which is the
  correct loud failure - documented as part of AC5's manual verification.
- The operator's gateway has `observability.enabled = true` (per
  `tensorzero/config/tensorzero.toml:10`). The test does not assert that the
  ClickHouse-backed observability sink received the call. Reason: that would
  require depending on ClickHouse being healthy too, and the wiremock layer
  already pins the request body the gateway logs.
- Concurrent test runs (e.g. `cargo test -- --test-threads=4` with multiple
  integration tests in the same crate). Single test, no shared state, no
  global env-var mutation - safe.
- Re-running the test in a tight loop. The gateway logs each call; this is
  fine. The test does not need cleanup.
- The `genai` crate's HTTP client connection pool. Each test invocation
  builds a fresh `Client` (via `TensorZeroBackend::new`) so there is no
  cross-test connection leak.
