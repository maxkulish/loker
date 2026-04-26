# Spec: Map genai errors to BackendError variants (CLO-249)

**Created**: 2026-04-26
**Estimated scope**: S (2 files touched, 1 new module, 4 sub-tasks)

## 1. Problem Statement

The TensorZero backend is the first (and currently only) `Backend` impl that
talks to the `genai = "0.6.0-beta.17"` crate. Translation from `genai::Error`
(plus its embedded `genai::webc::Error` and HTTP status/body) into the project
`BackendError` enum currently lives entirely in
`src/backend/tensorzero.rs`:

- `map_genai_error(err: genai::Error, elapsed: Duration) -> BackendError` at
  `src/backend/tensorzero.rs:126` is the entry point and the *only*
  `genai::Error -> BackendError` site in the tree (single caller at
  `src/backend/tensorzero.rs:101`).
- It delegates to four helpers also private to that module: `map_webc_error`
  (line 152), `classify_5xx_body` (line 242), `classify_404_body` (line 295),
  `map_status` (line 304), plus the boundary-aware `contains_status_code` (line
  275).

Today this works for the single backend, but the upcoming
`Strategy::EscalatingRetry` walker (T-013, Linear CLO-258) is the *blocked-by*
consumer that needs to inspect retryability outside of `tensorzero.rs`. It will
read `BackendError::is_retryable()` (already defined in
`src/backend/mod.rs:70`) and assume that retryability is the canonical
property of a `BackendError`, regardless of which backend produced it.

Two things break that assumption today:

1. **The mapping is private to one module.** A future second genai-using
   backend (or a unit test in another module) cannot reuse the translation
   without an unauthorised peek at private items.
2. **Auth/RateLimit re-classification of 502 bodies (TensorZero-specific
   gateway behaviour) is intermixed with the generic genai mapping**, so
   `is_retryable()` is consistent today only by coincidence: the body
   inspection runs on every 502 from any future genai-using backend that
   shares the `From` impl.

The fix is centralisation behind a single `From<genai::Error> for
BackendError` impl, with retryability flags pinned by unit tests. The
existing `map_genai_error` body is the working basis - this is a refactor,
not a redesign. The 502 body-inspection coupling is acceptable for now
because TensorZero is the only genai consumer; the spec records the
coupling explicitly so a future split is cheap.

The `genai::Error::HttpError` variant carries a `status: reqwest::StatusCode`
and a `body: String`; `genai::webc::Error::ResponseFailedStatus` is the
parallel webc variant. Both must route through the same status/body
classifier.

The `BackendError::Timeout { elapsed_ms: u64 }` variant complicates a pure
`From` impl because `genai::Error` carries no elapsed-time information; only
the calling site knows. The existing function takes `(err, elapsed)` to
populate `elapsed_ms` for `webc::Error::Reqwest(e)` where `e.is_timeout()`.
We must preserve that signal without forcing every caller to plumb a
duration through `From::from`.

## 2. Acceptance Criteria

- [ ] **AC1**: A single `impl From<genai::Error> for BackendError` exists in
  `src/backend/genai_error.rs`. Callers convert via `e.into()` or `?`.
- [ ] **AC2**: The body of every former `tensorzero::map_genai_error` /
  `map_webc_error` / `map_status` / `classify_5xx_body` / `classify_404_body`
  / `contains_status_code` lives in `src/backend/genai_error.rs` (moved, not
  duplicated). `tensorzero.rs` calls them only through public entry points.
- [ ] **AC3**: `BackendError::Timeout::elapsed_ms` is preserved end-to-end.
  The `From` impl emits `elapsed_ms: 0` when no duration is known; callers
  that *do* know the duration (e.g. `TensorZeroBackend::query`) attach it via
  a `BackendError::with_elapsed(Duration)` builder method that mutates the
  `elapsed_ms` field on `Timeout` variants and is a no-op on every other
  variant.
- [ ] **AC4**: `tensorzero.rs::query()` line 101 reads
  `.map_err(BackendError::from_genai_with_elapsed(start.elapsed()))?` (or an
  equivalent that calls `From::from` then `with_elapsed` - the implementer
  picks the cleaner shape).
- [ ] **AC5**: Module-level doc comment on `src/backend/genai_error.rs`
  contains a Markdown table mapping each `genai::Error` and
  `genai::webc::Error` variant -> resulting `BackendError` -> retryable flag,
  and references `docs/spikes/2026-04-25-tensorzero-roundtrip.md` as the
  source of truth for the TensorZero-specific 502/404 body inspections.
- [ ] **AC6**: Unit tests in `genai_error.rs` pin each known variant to its
  mapped `BackendError` discriminant and to its `is_retryable()` value.
  Coverage matrix below in §5.
- [ ] **AC7**: Existing TensorZero tests in `src/backend/tensorzero.rs::tests`
  continue to pass without source modification beyond import path changes.
  The wiremock matrix (200, 401, 403, 404 unknown function, 429, 500 generic,
  502 upstream auth, 502 upstream rate-limit, 502 generic, malformed JSON,
  timeout) still asserts the same `BackendError` variants.
- [ ] **AC8**: `make check` (fmt + clippy + test) is green.
- [ ] **AC9**: No new `BackendError` enum variants are added.
- [ ] **AC10**: No second `From<genai::Error>`-style impl exists anywhere in
  `src/`.

**Verification method**:

| AC | How to prove |
|---|---|
| AC1 | `rg "impl From<genai::Error>" src/` returns exactly one hit, in `genai_error.rs`. |
| AC2 | `rg "fn map_(genai\|webc)_error\|fn classify_(404\|5xx)_body\|fn map_status\|fn contains_status_code" src/backend/tensorzero.rs` returns 0 hits. |
| AC3 | `cargo test --lib backend::genai_error::tests::with_elapsed_` |
| AC4 | `rg "map_genai_error\|from_genai" src/backend/tensorzero.rs:101` shows the new call shape. |
| AC5 | `head -80 src/backend/genai_error.rs` shows the mapping table and the spike reference. |
| AC6 | `cargo test --lib backend::genai_error::tests` runs the variant matrix. |
| AC7 | `cargo test --lib backend::tensorzero::tests` (existing wiremock tests) is green. |
| AC8 | `make check` exit code 0. |
| AC9 | `git diff src/backend/mod.rs` shows no enum-variant additions. |
| AC10 | `rg "From<genai::" src/` returns exactly one hit. |

## 3. Constraints

**Must**:

- Single `From<genai::Error> for BackendError` impl. No tuple-input variant,
  no scattered ad-hoc conversions.
- Preserve the existing `BackendError` discriminants returned for every
  fixture-covered HTTP shape. The wiremock tests are the contract.
- Keep `is_retryable()` consistent with current behaviour: `Timeout`,
  `RateLimit`, `Network` are retryable; `Auth`, `Parse`, `ExecutionFailed`,
  `Unavailable`, `Config` are not.
- Reference `docs/spikes/2026-04-25-tensorzero-roundtrip.md` from the new
  module's doc comment. The 502 body-inspection rules ARE
  TensorZero-specific - the doc comment must say so.

**Must-not**:

- Add new `BackendError` enum variants (scope explicitly forbids; per task
  description "Extend the enum only where a real variant has nowhere to
  land - do not pre-emptively add ones we can't observe").
- Take a `Duration` argument in the `From` impl (`From::from` is single-arg
  by signature; threading elapsed through tuple input was rejected at
  exploration time as awkward for the common no-elapsed call site).
- Change behaviour of the TensorZero 502 body-inspection or 404 body-inspection
  classifiers. The fixtures are fixed.
- Make `BackendError::with_elapsed` mutate non-`Timeout` variants.
- Make the TensorZero backend `pub use` private internal items - the new
  module's API must be stable enough that future genai backends (or unit
  tests) consume it without further refactor.

**Prefer**:

- Place new module at `src/backend/genai_error.rs` and declare `mod
  genai_error;` in `src/backend/mod.rs`. No need to re-export through
  `super::`; `From<genai::Error>` is automatically picked up by `?`.
- Make all helpers (`map_status`, `map_webc_error`, etc.) `pub(super)` or
  `pub(crate)` only as far as needed for tests in the same module - keep
  the public surface to `From` + `with_elapsed`.
- Put the variant-to-result table in the module-level doc comment so
  `cargo doc` renders it.
- For each `match` arm in the From impl, derive the `BackendError` value
  with the same string-formatting style already in `map_genai_error`
  (e.g. `"TensorZero <thing>: {err}"` becomes `"genai <thing>: {err}"`).
  The "TensorZero" prefix is misleading once the helper is generic; rename
  to `"genai"` in the *new* module. The wiremock tests assert on
  discriminants, not on message bodies, but verify with
  `cargo test --lib backend::tensorzero::tests` after each rename.

**Escalate when**:

- A test that previously asserted on `message` substring content fails after
  the rename - that means a non-discriminant assertion exists; pause and
  ask whether to keep the "TensorZero" prefix or update the test.
- A second backend appears in the codebase that uses `genai` while this
  task is in flight (i.e. the body-inspection assumption breaks before we
  ship). At that point the body-inspection helpers must move out of the
  generic `From` impl into a TensorZero-only post-processor. Pause and
  re-spec.

## 4. Decomposition

All sub-tasks are **sequential** because they all touch the same two files
in tightly coupled ways. The order minimises in-progress test breakage.

1. **ST1: Add `BackendError::with_elapsed`** - extend `src/backend/mod.rs`
   with a single builder method on `BackendError` (no enum changes). Unit
   test that `Timeout { elapsed_ms: 0 }.with_elapsed(Duration::from_millis(500))`
   yields `Timeout { elapsed_ms: 500 }` and that other variants pass through
   unchanged. Files: `src/backend/mod.rs`. Independent of all other
   sub-tasks; can be reviewed and merged on its own if it grows. ~30 min.

2. **ST2: Create `src/backend/genai_error.rs`, move helpers, add
   `From<genai::Error>` impl** - cut the bodies of `map_genai_error`,
   `map_webc_error`, `classify_5xx_body`, `classify_404_body`, `map_status`,
   `contains_status_code` from `tensorzero.rs` and paste them into the new
   file. Add `mod genai_error;` to `src/backend/mod.rs`. Wrap
   `map_genai_error(err, elapsed)` as the body of `From<genai::Error>` with
   `elapsed = Duration::ZERO` (i.e. the From impl produces
   `Timeout { elapsed_ms: 0 }` and callers patch via `with_elapsed`). The
   helpers stay private to `genai_error`. Files: `src/backend/genai_error.rs`
   (new), `src/backend/mod.rs` (one-line module declaration), `src/backend/tensorzero.rs`
   (delete moved code). ~45 min.

3. **ST3: Rewire `tensorzero.rs::query()` line 101** - replace
   `.map_err(|e| map_genai_error(e, start.elapsed()))?` with
   `.map_err(|e| BackendError::from(e).with_elapsed(start.elapsed()))?`.
   Delete now-unused imports. Verify wiremock tests in
   `src/backend/tensorzero.rs::tests` still pass unchanged (they assert
   on discriminants). Files: `src/backend/tensorzero.rs`. ~15 min.

4. **ST4: Add module-level doc table + variant-coverage unit tests** -
   write the genai-variant -> BackendError -> retryable Markdown table at
   the top of `src/backend/genai_error.rs`. Add unit tests pinning each
   variant; matrix in §5 below. Files: `src/backend/genai_error.rs`. ~45
   min.

**Dependency order**: ST1 -> ST2 -> ST3 -> ST4. Tests for ST1 land with ST1;
ST2 lands without behavioural change (helpers just move); ST3 swaps the
call site; ST4 adds the documentation and coverage tests.

## 5. Evaluation

| # | Test | Expected Result | How to Run |
|---|------|-----------------|------------|
| 1 | `with_elapsed` on `Timeout { elapsed_ms: 0 }` | Returns `Timeout { elapsed_ms: 500 }` when called with `Duration::from_millis(500)` | `cargo test --lib backend::tests::with_elapsed_overrides_timeout_elapsed` |
| 2 | `with_elapsed` on `Network { ... }` | Returns the same `Network { ... }` unchanged | `cargo test --lib backend::tests::with_elapsed_noop_on_non_timeout` |
| 3 | `From<genai::Error::WebModelCall { webc: Reqwest(timeout) }>` | `BackendError::Timeout { elapsed_ms: 0 }` (caller responsible for `with_elapsed`) and `is_retryable() == true` | `cargo test --lib backend::genai_error::tests::from_timeout_yields_retryable_timeout` |
| 4 | `From<genai::Error::WebModelCall { webc: Reqwest(connect) }>` | `BackendError::Network { ... }` and `is_retryable() == true` | `cargo test --lib backend::genai_error::tests::from_connect_yields_retryable_network` |
| 5 | `From<HttpError { status: 401, body }>` | `BackendError::Auth { ... }` and `is_retryable() == false` | `cargo test --lib backend::genai_error::tests::from_401_yields_non_retryable_auth` |
| 6 | `From<HttpError { status: 403, body }>` | `BackendError::Auth { ... }` and `is_retryable() == false` | `cargo test --lib backend::genai_error::tests::from_403_yields_non_retryable_auth` |
| 7 | `From<HttpError { status: 404, body: "Unknown function: ..." }>` | `BackendError::Config { ... }` and `is_retryable() == false` | `cargo test --lib backend::genai_error::tests::from_404_unknown_function_yields_config` |
| 8 | `From<HttpError { status: 404, body: other }>` | `BackendError::ExecutionFailed { ... }` and `is_retryable() == false` | `cargo test --lib backend::genai_error::tests::from_404_other_yields_execution_failed` |
| 9 | `From<HttpError { status: 429, body }>` | `BackendError::RateLimit { ... }` and `is_retryable() == true` | `cargo test --lib backend::genai_error::tests::from_429_yields_retryable_rate_limit` |
| 10 | `From<HttpError { status: 502, body: "{...authentication_error...}" }>` | `BackendError::Auth { ... }` (body-inspection, TensorZero-specific) and `is_retryable() == false` | `cargo test --lib backend::genai_error::tests::from_502_auth_body_yields_auth` |
| 11 | `From<HttpError { status: 502, body: "{...rate_limit...}" }>` | `BackendError::RateLimit { ... }` and `is_retryable() == true` | `cargo test --lib backend::genai_error::tests::from_502_rate_limit_body_yields_rate_limit` |
| 12 | `From<HttpError { status: 502, body: generic }>` | `BackendError::Network { ... }` and `is_retryable() == true` | `cargo test --lib backend::genai_error::tests::from_502_generic_body_yields_network` |
| 13 | `From<HttpError { status: 503 .. 599 }>` | `BackendError::Network { ... }` and `is_retryable() == true` | `cargo test --lib backend::genai_error::tests::from_5xx_other_yields_network` |
| 14 | `From<ChatResponseGeneration>` | `BackendError::Parse { ... }` and `is_retryable() == false` | `cargo test --lib backend::genai_error::tests::from_chat_response_generation_yields_parse` |
| 15 | `From<StreamParse>` | `BackendError::Parse { ... }` and `is_retryable() == false` | `cargo test --lib backend::genai_error::tests::from_stream_parse_yields_parse` |
| 16 | `From<Resolver>` | `BackendError::Config { ... }` and `is_retryable() == false` | `cargo test --lib backend::genai_error::tests::from_resolver_yields_config` |
| 17 | `From<RequiresApiKey \| NoAuthData \| NoAuthResolver>` | `BackendError::Auth { ... }` and `is_retryable() == false` | `cargo test --lib backend::genai_error::tests::from_no_auth_yields_auth` |
| 18 | `From<webc::Error::ResponseFailedInvalidJson>` | `BackendError::Parse { ... }` | `cargo test --lib backend::genai_error::tests::from_webc_invalid_json_yields_parse` |
| 19 | `From<webc::Error::ResponseFailedNotJson>` | `BackendError::Parse { ... }` | `cargo test --lib backend::genai_error::tests::from_webc_not_json_yields_parse` |
| 20 | `contains_status_code` boundary check (existing assertion preserved) | `"4011"` -> false; `" 401 "` -> true; `"H401X"` -> false; `{"status":429}` -> true | `cargo test --lib backend::genai_error::tests::contains_status_code_handles_punctuation_boundaries` |
| 21 | TensorZero wiremock 401 fixture | `BackendError::Auth` | Existing `cargo test --lib backend::tensorzero::tests::query_returns_auth_on_401` |
| 22 | TensorZero wiremock 502 anthropic auth-failure fixture | `BackendError::Auth` | Existing `cargo test --lib backend::tensorzero::tests::query_returns_auth_on_502_anthropic_auth_failure` |
| 23 | TensorZero wiremock 502 generic | `BackendError::Network` | Existing `cargo test --lib backend::tensorzero::tests::query_returns_network_on_502_generic` |
| 24 | `make check` clean | exit 0; 485+ unit tests pass, 6 integration, 1 schema | `make check` |

**Edge cases to verify**:

- `From<genai::Error>` for any *unmatched* variant (catch-all `other` arm)
  yields `BackendError::ExecutionFailed { exit_code: None, .. }` and is
  not retryable. Test: synthesise via `genai::Error::CustomError(...)` if
  available; otherwise document the catch-all in the doc comment.
- `Timeout { elapsed_ms: 0 }.with_elapsed(Duration::ZERO)` is idempotent and
  produces `Timeout { elapsed_ms: 0 }`.
- `with_elapsed` called twice picks up the *second* call's value (last
  wins, no accumulation).
- `BackendError` is `Clone`, so `with_elapsed` taking `self` by value is
  fine; verify by inspection.
- `genai 0.6.0-beta.17`'s `Error::HttpError` field naming - the From impl
  must compile against the exact `status: reqwest::StatusCode, body:
  String, ..` shape used in the existing `map_genai_error`. No upgrade in
  this task.

