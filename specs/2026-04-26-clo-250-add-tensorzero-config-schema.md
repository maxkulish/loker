# Spec: Add TensorZero config schema to src/config.rs (CLO-250)

**Created**: 2026-04-26
**Estimated scope**: S (3 files touched, 6 sub-tasks)

## 1. Problem Statement

The TensorZero backend (`src/backend/tensorzero.rs`) is the M1 milestone's
HTTP-gateway backend. Its runtime constructor at
`src/backend/tensorzero.rs:46` already takes a config-shaped struct
(currently named `TensorZeroConfig` at `src/backend/tensorzero.rs:32-38`)
with fields `endpoint: String, model: String, api_key: Option<String>,
timeout: Duration`, but **nothing in the user-facing config layer produces
that struct**. Today every call site that needs to instantiate
`TensorZeroBackend` must hand-build the struct and resolve the API key from
the environment itself.

`src/config.rs:9-24` defines the strict (`#[serde(deny_unknown_fields)]`)
`Config` root that gets deep-merged from three TOML layers (defaults ->
`~/.config/lok/lok.toml` -> `./lok.toml`, see
`src/config.rs:308-339`). It already carries `defaults`, `conductor`,
`cache`, `backends: HashMap<String, BackendConfig>`, `tasks`, `roles`,
`teams` - but no slot for TensorZero. The `BackendConfig` pattern at
`src/config.rs:103-119` (used by codex / gemini / claude / ollama) is
**subprocess-shaped** - it carries `command`, `args`, `skip_lines`. Forcing
TensorZero into `backends.tensorzero` would either ignore those fields or
abuse them (TensorZero is HTTP, not a CLI), and it would not give us a
natural home for a TensorZero-only `retry_policy` substruct or for the
URL/timeout validation TensorZero needs.

The acceptance criteria from CLO-250 force two contracts:

1. **Round-trip stability** of the new section through serde (the existing
   round-trip pattern is at `src/config.rs:514-524`).
2. **Validation at parse time** with clear error messages for: missing
   endpoint, malformed URL, zero/missing timeout. Today, missing fields
   would surface as a serde error from `toml::from_str`; URL validity and
   `timeout > 0` are not checked anywhere.
3. **The backend constructor takes the config struct, no scattered env
   lookups inside the backend module.** `src/backend/tensorzero.rs`
   currently has *zero* `env::var` calls (verified by grep), but the
   constructor's `api_key: Option<String>` field assumes the caller has
   already done the `env::var(api_key_env)` resolution. The contract this
   AC asks us to honour is that the backend module *stays* env-free, with
   the env lookup and the resolved `api_key` produced by a `to_backend_opts`
   converter on the new config struct.

There is one naming collision to address: the runtime config struct in
`src/backend/tensorzero.rs:32` is already called `TensorZeroConfig`. The
new TOML-facing config struct must also live under that conceptual name
("the TensorZero section of `lok.toml`"). Resolution: rename the runtime
struct to `TensorZeroBackendOpts` (impact: one re-export at
`src/backend/mod.rs:18` and one test import at
`tests/tensorzero_backend.rs:11`); the new public config-layer name
`TensorZeroConfig` then takes its place in `src/config.rs` and is
re-exported from there.

`reqwest = "0.12"` is already a direct dependency (`Cargo.toml`), so URL
validation can use `reqwest::Url::parse` without adding a `url` crate.
`toml = "0.8"` is the parser; `serde = "1"` is the framework; nothing new
is needed.

## 2. Acceptance Criteria

- [ ] **AC1**: A new public struct `TensorZeroConfig` exists in
  `src/config.rs` with fields `endpoint: String`, `default_model: String`,
  `api_key_env: Option<String>`, `timeout_secs: u64`, `retry_policy:
  RetryPolicy`. It derives `Debug, Deserialize, Serialize, Clone` and uses
  `#[serde(deny_unknown_fields)]`.

- [ ] **AC2**: A new public struct `RetryPolicy` exists in `src/config.rs`
  with fields `max_retries: u32`, `delay_ms: u64`. Same derives + serde
  attrs as AC1. Implements `Default` (max_retries: 0, delay_ms: 1000 -
  matching the existing `default_retries` / `default_retry_delay_ms`
  helpers at `src/config.rs:54-60`).

- [ ] **AC3**: `Config` (line 9-24 of `src/config.rs`) gains
  `pub tensorzero: Option<TensorZeroConfig>` with `#[serde(default)]`.
  Default is `None` (matches the existing pattern for optional sections,
  e.g. `defaults.team`).

- [ ] **AC4**: `TensorZeroConfig::validate(&self) -> anyhow::Result<()>`
  exists. It returns `Err` (with a clear context message) when:
  - `endpoint` is empty, OR
  - `reqwest::Url::parse(&self.endpoint)` returns `Err`, OR
  - the parsed URL's scheme is not `http` or `https`, OR
  - `timeout_secs == 0`.
  All other inputs return `Ok(())`. Error messages embed the offending
  value and the field name (e.g. `"tensorzero.endpoint: not a valid URL:
  'foo bar'"`).

- [ ] **AC5**: `load_config_from_paths` (`src/config.rs:308-339`) calls
  `cfg.tensorzero.as_ref().map(|tz| tz.validate()).transpose()?`
  immediately before returning the deserialized `Config`. So a
  `lok.toml` with a malformed `[tensorzero]` table fails `load_config`
  with the validation message rather than parsing successfully and
  exploding at backend-construction time.

- [ ] **AC6**: The runtime struct currently named `TensorZeroConfig` in
  `src/backend/tensorzero.rs:32-38` is renamed to `TensorZeroBackendOpts`.
  Re-export in `src/backend/mod.rs:18` and import in
  `tests/tensorzero_backend.rs:11` are updated. No call sites remain that
  reference the old runtime name.

- [ ] **AC7**: `TensorZeroConfig` (the new config struct in
  `src/config.rs`) has a method
  `to_backend_opts(&self) -> anyhow::Result<TensorZeroBackendOpts>` that:
  - resolves `api_key_env` via `std::env::var` (using the same
    `with_context` shape as `src/backend/claude.rs:67-73`) when
    `api_key_env` is `Some`; passes through `None` otherwise,
  - converts `timeout_secs: u64` to `Duration::from_secs(timeout_secs)`,
  - copies `endpoint` and `default_model` (the new field name) into
    `endpoint` and `model` of `TensorZeroBackendOpts`.
  This is the **only** code path that performs an env lookup on behalf of
  the TensorZero backend. `rg "env::var" src/backend/tensorzero.rs` must
  return zero matches after this change.

- [ ] **AC8**: A new test
  `test_tensorzero_config_serialization_roundtrip` in `src/config.rs`'s
  `mod tests` block proves: build a `Config` with
  `tensorzero: Some(TensorZeroConfig { ... })`, serialize via
  `toml::to_string_pretty`, parse back via `toml::from_str`, and assert
  every field on the round-tripped `tensorzero` section matches the
  original. Mirrors the existing
  `test_config_serialization_roundtrip` at line 514.

- [ ] **AC9**: Three new parse-error tests in `src/config.rs`'s `mod
  tests` block:
  - `test_tensorzero_missing_endpoint_fails` - omits `endpoint`,
    asserts `toml::from_str::<Config>(...)` returns `Err` (serde
    rejects the missing required field; no need to invoke
    `validate`).
  - `test_tensorzero_invalid_url_fails` - sets `endpoint = "not a
    url"`, parses successfully, then asserts
    `cfg.tensorzero.as_ref().unwrap().validate()` returns `Err` whose
    string representation contains `"endpoint"`.
  - `test_tensorzero_zero_timeout_fails` - sets `timeout_secs = 0`,
    asserts the same `validate()` shape returns `Err` whose string
    contains `"timeout"`.

- [ ] **AC10**: `docs/handoff.md` gains a sample `[tensorzero]` block
  (with a `[tensorzero.retry_policy]` sub-table) inserted in whatever
  section currently shows `lok.toml` examples. The sample is the minimal
  valid form: `endpoint`, `default_model`, optional `api_key_env`,
  `timeout_secs = 60`, retry_policy with the documented defaults. If no
  `lok.toml` example currently lives in `docs/handoff.md`, append a new
  short section titled `### TensorZero backend (lok.toml)` near the
  existing M1 / TensorZero references.

- [ ] **AC11**: `make check` (fmt + clippy + test) passes. No new
  warnings.

**Verification method**: `cargo test config::tests::test_tensorzero` for
the four new tests; `make check` for the gate; `rg "env::var"
src/backend/tensorzero.rs` to confirm AC7; `cargo build` to confirm the
rename compiles end-to-end.

## 3. Constraints

**Must**:
- Use `#[serde(deny_unknown_fields)]` on every new struct (matches the
  rest of `src/config.rs`).
- Validate URL with `reqwest::Url::parse` (already in deps; no `url`
  crate).
- Keep the env::var lookup in `src/config.rs::TensorZeroConfig::to_backend_opts`,
  not in `src/backend/tensorzero.rs`.
- Preserve the runtime backend's existing public surface in spirit -
  callers of `TensorZeroBackend::new(opts)` keep the same signature shape;
  only the type name changes.
- Use `anyhow::Result` for `validate()` and `to_backend_opts()` to match
  the rest of `src/config.rs`'s error idiom.

**Must-not**:
- Add a `url` crate dependency (`reqwest::Url` is already re-exported
  through reqwest 0.12).
- Reuse `BackendConfig` for TensorZero. It is subprocess-shaped (`command`,
  `args`, `skip_lines`) and forcing TensorZero through it would either
  abuse those fields or require schema-divergent optionals.
- Introduce env::var calls in `src/backend/tensorzero.rs`. The backend
  module receives a resolved `api_key: Option<String>` from
  `to_backend_opts` and never reads the environment itself.
- Modify any default-derivation logic for the existing backends (codex /
  gemini / claude / ollama) - this task only adds a new top-level slot.

**Prefer**:
- Mirror existing naming and helper-function style: `default_<field>()`
  free functions plus `Default` impls (see `src/config.rs:46-73`).
- Keep `RetryPolicy` generic-named (not `TensorZeroRetryPolicy`) so the
  type can be reused if `BackendConfig` ever adopts a substruct.
- Insert the `pub tensorzero` field after `pub cache` and before
  `pub backends` in `Config` - it sits at the same conceptual level as
  `cache` (a single-instance optional subsystem) and reads naturally above
  the per-backend map.

**Escalate when**:
- The existing CLO-243 spike or CLO-247 reconciliation (in
  `docs/spikes/2026-04-25-tensorzero-roundtrip.md`) requires a field this
  spec does not list (e.g. mTLS cert path, tracing toggle, `function_name`
  override). Stop and ask before extending the schema.
- Any consumer outside `src/backend/tensorzero.rs` and
  `tests/tensorzero_backend.rs` imports the runtime
  `TensorZeroConfig` name. Stop and report; the rename impact would no
  longer be local.
- `make check` flips a clippy lint on the new struct that conflicts with
  existing project style elsewhere in `src/config.rs`. Do not silence with
  `#[allow]`; ask.

## 4. Decomposition

1. **ST1: Add `RetryPolicy` and `TensorZeroConfig` structs to
   `src/config.rs`** - define both structs with derives, serde attrs,
   default-fn helpers, `Default` impl on `RetryPolicy`. No `Default` on
   `TensorZeroConfig` (endpoint and default_model are required, no
   sensible default). Files: `src/config.rs`.

2. **ST2: Wire `pub tensorzero: Option<TensorZeroConfig>` into `Config`**
   - add the field with `#[serde(default)]` between `cache` and
   `backends`. No change to `Default` impl (None is the right default).
   Files: `src/config.rs`.

3. **ST3: Add `validate()` on `TensorZeroConfig`** - check empty endpoint,
   `reqwest::Url::parse`, scheme in `{http, https}`, `timeout_secs > 0`.
   Wire the call into `load_config_from_paths` just before the final
   return. Files: `src/config.rs`.

4. **ST4: Rename runtime `TensorZeroConfig` to `TensorZeroBackendOpts`** -
   rename in `src/backend/tensorzero.rs` (struct decl + every internal
   reference), update re-export in `src/backend/mod.rs:18`, update import
   in `tests/tensorzero_backend.rs:11`. Run `cargo build` to confirm.
   Files: `src/backend/tensorzero.rs`, `src/backend/mod.rs`,
   `tests/tensorzero_backend.rs`.

5. **ST5: Add `to_backend_opts()` conversion on the new
   `TensorZeroConfig`** - resolves api_key_env via `env::var` with
   `Context` (matching `src/backend/claude.rs:67-73` shape), maps
   `timeout_secs -> Duration`, hands `default_model` to the runtime's
   `model` field. Files: `src/config.rs`.

6. **ST6: Tests** - 4 unit tests added to `src/config.rs::tests`:
   `test_tensorzero_config_serialization_roundtrip`,
   `test_tensorzero_missing_endpoint_fails`,
   `test_tensorzero_invalid_url_fails`,
   `test_tensorzero_zero_timeout_fails`. Plus 1 sanity test
   `test_tensorzero_to_backend_opts_resolves_env` that sets a temp env
   var, calls `to_backend_opts`, asserts the api_key is
   propagated, then unsets the var. Files: `src/config.rs`.

7. **ST7: docs/handoff.md sample** - add the minimal `[tensorzero]` block
   (per AC10). Files: `docs/handoff.md`.

**Dependency order**: ST1 -> ST2 -> ST3 (each builds on the prior); ST4
is independent of ST1-ST3 and can run in parallel; ST5 depends on ST1
(needs the new struct) and ST4 (needs the renamed runtime type); ST6
depends on ST1-ST5; ST7 depends on ST1-ST3 (sample must reflect final
schema). Linear order: 1 -> 2 -> 3 -> 4 -> 5 -> 6 -> 7.

## 5. Evaluation

| # | Test | Expected Result | How to Run |
|---|------|-----------------|------------|
| 1 | `test_tensorzero_config_serialization_roundtrip` | Round-tripped `Config` carries an identical `tensorzero` section (endpoint, default_model, api_key_env, timeout_secs, retry_policy.max_retries, retry_policy.delay_ms all match). | `cargo test config::tests::test_tensorzero_config_serialization_roundtrip` |
| 2 | `test_tensorzero_missing_endpoint_fails` | `toml::from_str::<Config>(...)` on a `[tensorzero]` block missing `endpoint` returns `Err`; the error message names `endpoint`. | `cargo test config::tests::test_tensorzero_missing_endpoint_fails` |
| 3 | `test_tensorzero_invalid_url_fails` | `validate()` on a `[tensorzero]` with `endpoint = "not a url"` returns `Err` whose `to_string()` contains `"endpoint"`. | `cargo test config::tests::test_tensorzero_invalid_url_fails` |
| 4 | `test_tensorzero_zero_timeout_fails` | `validate()` on a `[tensorzero]` with `timeout_secs = 0` returns `Err` whose `to_string()` contains `"timeout"`. | `cargo test config::tests::test_tensorzero_zero_timeout_fails` |
| 5 | `test_tensorzero_to_backend_opts_resolves_env` | With a temp env var set, `to_backend_opts()` returns `Ok(TensorZeroBackendOpts)` whose `api_key` is `Some(<value>)`. | `cargo test config::tests::test_tensorzero_to_backend_opts_resolves_env` |
| 6 | env::var cleanliness | `rg "env::var" src/backend/tensorzero.rs` returns no matches. | `rg "env::var" src/backend/tensorzero.rs` |
| 7 | Compile after rename | `cargo build` and `cargo test --no-run` succeed; no stale references to the old runtime `TensorZeroConfig`. | `cargo build && cargo test --no-run` |
| 8 | Pre-merge gate | `make check` passes (fmt + clippy + 500+ unit + integration tests). No new warnings. | `make check` |
| 9 | Round-trip parity with `Config::default()` | The default `Config` (no `tensorzero` section) round-trips through `toml::to_string_pretty` -> `toml::from_str` and yields `tensorzero: None`, preserving existing `test_config_serialization_roundtrip` behaviour. | `cargo test config::tests::test_config_serialization_roundtrip` |

**Edge cases to verify**:
- A `[tensorzero]` section with no `[tensorzero.retry_policy]` sub-table
  parses successfully and uses `RetryPolicy::default()` (max_retries: 0,
  delay_ms: 1000).
- `endpoint = "ftp://gateway.local"` fails `validate()` because the
  scheme is not http/https.
- `endpoint = "https://gateway.local:3000/openai"` parses and validates
  successfully (the existing `normalize_endpoint` in
  `src/backend/tensorzero.rs` will handle path normalisation downstream;
  validate only enforces URL well-formedness, not path shape).
- `api_key_env = Some("LOK_DOES_NOT_EXIST_<random>")`: `to_backend_opts`
  returns `Err` with the missing-env-var context message. The four
  primary unit tests do **not** rely on this case to keep them
  hermetic; the env-resolving test uses a deliberately set temp var.
- Three-layer merge: a project `lok.toml` with `[tensorzero]` overrides
  the user-level `[tensorzero]`. This is automatically covered by the
  existing `deep_merge` machinery in `src/config.rs:270-283`; no
  additional test required for this spec, but verify by inspection.
