# Spec: JSON Schemas for Run Artefacts (CLO-244)

**Created**: 2026-04-25
**Estimated scope**: M (~20 new files: 8 schemas + 16 fixtures + 1 validator test)
**Linear**: [CLO-244](https://linear.app/cloud-ai/issue/CLO-244/author-json-schemas-for-run-artefacts-d2)
**Branch**: `feat/clo-244-json`
**Roadmap**: T-002 / PRD §11 D2 [parallel-ok]

## 1. Problem Statement

`loker` will write a tree of artefacts to `runs/<workflow>-<timestamp>-<short-uuid>/`
during every run: an append-only `trace.jsonl` of OTel GenAI events, a
`manifest.json` registering each artefact with content hashes, per-phase
result files (one per `Strategy` variant), HITL `pending/<phase>.json` and
`responses/<phase>.json` requests/responses, and an optional `summary.json`
produced by a final aggregator pass.

Today none of these formats exist on disk and none have a contract. The
writers (T-024 manifest, T-029 trace, T-048/T-050 HITL artefacts) and the
later consumers (resume logic in M5, browser UI in M11, `loker show` in
M9) will land across multiple milestones authored at different times.
Without a fixed wire format, every consumer must either trust the writer's
in-memory struct (creating an undocumented coupling) or invent its own
parser (creating drift).

The requirement, lifted from PRD §11 D2 (lines 353-359), is to author
**Draft-2020-12 JSON Schemas** for each artefact, park sample fixtures
beside them, and gate `make check` on a CI validator so any future shape
drift fails the merge gate.

The schemas must follow OpenTelemetry GenAI semantic conventions where
applicable (`gen_ai.system`, `gen_ai.request.model`, `gen_ai.usage.*`,
`gen_ai.response.finish_reasons`) and namespace loker-specific fields
under `loker.*` (`loker.run_id`, `loker.phase`, `loker.strategy`,
`loker.attempt`). HITL pending/response shapes are dictated by the HITL
design doc §3.3 / §3.4.

The work touches one concern: schemas + fixtures + a Rust integration
test that walks both. No production code path changes. No dependencies on
unbuilt primitives - schemas describe the *intended* wire format, which
later writer tasks will conform to.

## 2. Acceptance Criteria

- [ ] **AC-1**: Eight Draft-2020-12 schemas committed under `docs/schemas/` with stable, lower-case file names:
  - `trace_event.schema.json` (one record from `trace.jsonl`)
  - `manifest.schema.json` (`manifest.json` envelope + entries)
  - `summary.schema.json` (`summary.json`, FR-23a)
  - `pending.schema.json` (`pending/<phase>.json`, HITL design §3.3)
  - `response.schema.json` (`responses/<phase>.json`, HITL design §3.4)
  - `phase_result_single.schema.json` (`SingleModel` strategy output)
  - `phase_result_parallel.schema.json` (`ParallelFanOut` strategy output)
  - `phase_result_escalating.schema.json` (`EscalatingRetry` strategy output)
- [ ] **AC-2**: Each schema declares `$schema: "https://json-schema.org/draft/2020-12/schema"`, a stable `$id` of the form `https://loker.dev/schemas/<name>.schema.json`, a `title`, a `description`, and uses `additionalProperties: false` on all objects whose fields are fully enumerated by the schema (trace events allow forward-compatible extra `loker.*` keys via `patternProperties`).
- [ ] **AC-3**: For each schema, at least one positive and one negative fixture lives under `tests/fixtures/schemas/<schema_basename>/{positive,negative}/*.json`.
  - Positive fixtures MUST validate.
  - Negative fixtures MUST fail validation, and each negative fixture must encode exactly one violation (missing required field, wrong type, additional property, enum violation, etc.) named in the filename (e.g. `missing_run_id.json`, `wrong_type_tokens.json`).
- [ ] **AC-4**: A Rust integration test at `tests/schema_validation.rs` walks `docs/schemas/*.schema.json` and the fixtures tree, asserting positive fixtures pass and negative fixtures fail. The test fails the build if any schema lacks at least one positive and one negative fixture, or if any fixture file is not paired with a schema.
- [ ] **AC-5**: The validator is wired into the merge gate. Because `make check` already invokes `cargo test`, adding the integration test is sufficient; no Makefile change is required. The test must run by default (no env-var gate).
- [ ] **AC-6**: `jsonschema` (Draft-2020-12 capable) is added as a `[dev-dependencies]` entry in `Cargo.toml` only - no production dependency churn.
- [ ] **AC-7**: `make check` is green on the branch with all schemas + fixtures + validator in place.

**Verification method**:
- AC-1, AC-2, AC-6: file listing + grep + manual schema read.
- AC-3, AC-4, AC-5: `make check` exits 0; deliberately mutating a fixture (e.g., dropping a required field from a positive fixture) makes `make check` exit non-zero.
- AC-7: CI run on the PR.

## 3. Constraints

**Must**:
- Use Draft-2020-12 (`$schema` URL must match exactly).
- Reflect OTel GenAI conventions for trace events: `gen_ai.system`, `gen_ai.request.model`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.response.finish_reasons`. Use these exact key names (with dots) inside the JSON object.
- Namespace every loker-specific key with `loker.` prefix (`loker.run_id`, `loker.phase`, `loker.strategy`, `loker.attempt`, `loker.event` for the discriminator).
- Mirror HITL design doc §3.3 / §3.4 verbatim for `pending` and `response` schemas: required fields, enum values, and ISO-8601 timestamp formats are non-negotiable.
- Match PRD FR-23b for manifest entries: each entry has `name`, `kind`, `schema_version`, `sha256` (hex content hash), and `producer` (one of `single`, `parallel`, `escalating`, `verify`, `hitl`).
- Make every timestamp field a `string` with `format: "date-time"` (RFC 3339 / ISO 8601).
- Keep negative fixtures minimal: one violation each, deviation captured in the filename.
- Add `jsonschema` only to `[dev-dependencies]`.

**Must-not**:
- Introduce a separate `make schema-check` target (FR-D2 wants it folded into `make check`).
- Couple the validator to network access or external schema registries (`$id` is just a stable identifier - no resolution required at validation time).
- Add a runtime dependency on `jsonschema` - schemas exist for the wire contract, not for runtime validation in M1.
- Require fixtures to come from fully-built writers - schemas land before writers; fixtures are hand-authored examples that match the contract.
- Define schemas for artefacts the PRD does not require yet (no `cost.json`, no `metrics.json`, etc.).

**Prefer**:
- One file per schema (no `$defs` inlined across schemas) so future docs can `$ref` cleanly. Inside a single schema, use `$defs` for shared sub-shapes (e.g., `tokens`, `error`).
- Lower-case `snake_case` for all field names except the OTel `gen_ai.*` and HITL pre-defined keys, which retain their canonical forms.
- Place each fixture as its own `*.json` file (no JSONL test bundles) so a failing test names the offending file.
- Use `oneOf` only when discriminating on a tagged union; prefer `enum` + closed objects for finite sets.

**Escalate when**:
- A schema requirement contradicts the HITL design doc or PRD - flag, do not silently diverge.
- A negative fixture would require violating a Rust ownership / type rule that the production writer can't actually emit (e.g. integer overflow, NaN). Skip those - we're testing the wire format, not exhaustive serialiser bugs.
- Ambiguity in OTel GenAI between two competing key names (e.g. `prompt_tokens` vs `input_tokens`). Default to the latest spec (`input_tokens` / `output_tokens`) and document the choice in the schema's `description`.

## 4. Decomposition

Tasks are grouped by dependency. The validator harness lands first; per-schema authoring is parallel after that.

1. **Validator harness + dev-dep + dir scaffold** - files: `Cargo.toml`, `tests/schema_validation.rs`, `docs/schemas/.gitkeep`, `tests/fixtures/schemas/.gitkeep`.
   - Add `jsonschema = "0.18"` (or latest Draft-2020-12 capable version) to `[dev-dependencies]`.
   - Write the integration test that: (a) globs `docs/schemas/*.schema.json`, (b) for each schema name, globs `tests/fixtures/schemas/<name>/positive/*.json` and `.../negative/*.json`, (c) asserts at least one of each, (d) compiles the schema and validates each fixture, asserting positive passes and negative fails.
   - The test should produce one failure per offending file with a clear path.
   - Ship the harness with one trivial dummy schema + paired fixture pair to prove the harness works in isolation; remove the dummy in task 2.

2. **`trace_event.schema.json` + fixtures** - files: `docs/schemas/trace_event.schema.json`, `tests/fixtures/schemas/trace_event/positive/{phase_started,llm_call,verify_pass}.json`, `tests/fixtures/schemas/trace_event/negative/{missing_run_id,wrong_event_enum,extra_unknown_field}.json`.
   - Required fields: `loker.run_id` (uuid), `loker.event` (enum: `phase_started`, `phase_finished`, `llm_call`, `verify_invoked`, `verify_result`, `hitl_pending`, `hitl_resolved`, `error`), `loker.phase` (string), `timestamp` (date-time).
   - For `llm_call` events: `gen_ai.system`, `gen_ai.request.model`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`, `gen_ai.response.finish_reasons` (array of string).
   - Use `oneOf` keyed on `loker.event` to enforce per-event-kind required fields.

3. **`manifest.schema.json` + fixtures** - files: `docs/schemas/manifest.schema.json`, `tests/fixtures/schemas/manifest/positive/{empty,single_phase,multi_phase_with_hitl}.json`, `tests/fixtures/schemas/manifest/negative/{bad_sha256_length,unknown_kind,missing_schema_version}.json`.
   - Envelope: `schema_version` (integer, ==1), `run_id` (uuid), `created_at` (date-time), `entries` (array).
   - Entry: `name` (string), `kind` (enum: `design.md`, `review.md`, `verify.json`, `phase_result.json`, `pending.json`, `response.json`, `summary.json`, `changes/` `trace.jsonl`), `schema_version` (integer), `sha256` (string, pattern `^[0-9a-f]{64}$`), `producer` (enum).

4. **`summary.schema.json` + fixtures** - files: `docs/schemas/summary.schema.json`, `tests/fixtures/schemas/summary/positive/{minimal,with_failures}.json`, `tests/fixtures/schemas/summary/negative/{negative_duration,unknown_status}.json`.
   - Required: `schema_version`, `run_id`, `workflow`, `started_at`, `finished_at`, `status` (enum: `success`, `partial`, `failed`, `aborted`), `phases` (array of `{name, status, attempts, tokens, duration_ms}`).

5. **`pending.schema.json` + fixtures** (HITL design §3.3) - files: `docs/schemas/pending.schema.json`, `tests/fixtures/schemas/pending/positive/{design_review,verify_failure}.json`, `tests/fixtures/schemas/pending/negative/{missing_severity,bad_timeout_format}.json`.
   - Required (per HITL doc §3.3): `schema_version` (==1), `run_id`, `workflow`, `phase`, `severity` (enum: `info`, `warn`, `error`), `opened_at` (date-time), `timeout_at` (date-time), `artefact` ({path, kind, preview_lines: integer}), `context` ({preceded_by: array, next_phase: string|null, prompt_summary: string}), `decision_options` (array of enum `{approve, reject, comment_only}`).

6. **`response.schema.json` + fixtures** (HITL design §3.4) - files: `docs/schemas/response.schema.json`, `tests/fixtures/schemas/response/positive/{approve,reject_with_comments}.json`, `tests/fixtures/schemas/response/negative/{missing_decided_at,unknown_decision}.json`.
   - Required: `schema_version` (==1), `phase`, `claimed_by` (string), `decided_at` (date-time), `decision` (enum: `approve`, `reject`), `global_comment` (string|null), `inline_comments_path` (string|null, relative path).

7. **`phase_result_single.schema.json` + fixtures** - files: `docs/schemas/phase_result_single.schema.json`, `tests/fixtures/schemas/phase_result_single/positive/{ok,verify_failed}.json`, `tests/fixtures/schemas/phase_result_single/negative/{missing_backend,bad_finish_reason}.json`.
   - Required: `schema_version`, `loker.strategy` (const `"single"`), `loker.phase`, `loker.run_id`, `attempts` (array, len 1), each attempt: `backend`, `model`, `finish_reasons`, `usage` (`input_tokens`, `output_tokens`), `output_path` (relative), `verify` ({status: enum `pass`/`fail`/`skipped`, hook: string|null}).

8. **`phase_result_parallel.schema.json` + fixtures** - files: `docs/schemas/phase_result_parallel.schema.json`, `tests/fixtures/schemas/phase_result_parallel/positive/{three_branches_concat,two_branches_judge}.json`, `tests/fixtures/schemas/phase_result_parallel/negative/{empty_branches,unknown_aggregator}.json`.
   - Required: `schema_version`, `loker.strategy` (const `"parallel"`), `loker.phase`, `loker.run_id`, `branches` (array, minItems: 1), each branch: same fields as a single attempt + `family` (enum: `anthropic`, `google`, `openai`, `zhipu`, `local`), `aggregator` (enum: `concat`, `llm_judge`, `any_fail`, `vote`), `aggregate_output_path` (string).

9. **`phase_result_escalating.schema.json` + fixtures** - files: `docs/schemas/phase_result_escalating.schema.json`, `tests/fixtures/schemas/phase_result_escalating/positive/{succeeded_on_medium,exhausted}.json`, `tests/fixtures/schemas/phase_result_escalating/negative/{missing_tier,non_monotonic_attempts}.json`.
   - Required: `schema_version`, `loker.strategy` (const `"escalating"`), `loker.phase`, `loker.run_id`, `attempts` (array, minItems: 1), each attempt: `tier` (enum: `cheap`, `medium`, `strong`), plus single-attempt fields, `final_status` (enum: `succeeded`, `exhausted`, `aborted`).

10. **Remove dummy harness fixture** + final `make check` - confirm the dummy schema/fixture from task 1 is gone, the harness still discovers all real schemas, run `make check` end-to-end.

**Dependency order**:
- Task 1 blocks tasks 2-9 (harness must exist before authored schemas can be exercised).
- Tasks 2-9 are independent of each other and parallel-safe.
- Task 10 runs last (after 2-9 are merged into the working tree).

## 5. Evaluation

| # | Test | Expected Result | How to Run |
|---|------|-----------------|------------|
| 1 | All eight schemas exist with correct names | `ls docs/schemas/*.schema.json` returns the eight named files | `ls docs/schemas/*.schema.json \| wc -l` -> 8 |
| 2 | Every schema declares Draft-2020-12 | `rg -L 'draft/2020-12' docs/schemas/*.schema.json` returns nothing | `rg -L 'draft/2020-12' docs/schemas/*.schema.json` |
| 3 | Every schema has at least one positive + one negative fixture | Validator test asserts pairing; build passes | `cargo test --test schema_validation` |
| 4 | Every positive fixture validates | Build passes | `cargo test --test schema_validation` |
| 5 | Every negative fixture fails validation | Build passes (the harness asserts negative fixtures must fail) | `cargo test --test schema_validation` |
| 6 | Drift detection: mutate a positive fixture | Removing a required field from `tests/fixtures/schemas/manifest/positive/empty.json` makes the test fail | edit + `cargo test --test schema_validation` |
| 7 | Drift detection: weaken a schema | Changing `additionalProperties: false` to `true` AND adding a stray field to a negative fixture that previously failed must now pass and break the test | edit + `cargo test` |
| 8 | `make check` integrates the validator | Green on clean branch, red after a deliberate fixture mutation | `make check` (twice - clean, then with mutation reverted) |
| 9 | No production runtime dependency added | `cargo tree -e=normal --depth=1 \| rg jsonschema` returns nothing; `cargo tree -e=dev --depth=1 \| rg jsonschema` returns one line | `cargo tree -e=normal --depth=1` and `cargo tree -e=dev --depth=1` |
| 10 | OTel GenAI keys present in trace schema | `rg 'gen_ai\.usage\.input_tokens' docs/schemas/trace_event.schema.json` finds the key | `rg gen_ai docs/schemas/trace_event.schema.json` |

**Edge cases to verify**:
- A schema with zero fixtures must fail the harness (otherwise drift is undetectable).
- A fixture under `tests/fixtures/schemas/<name>/` where `<name>` does not match a schema basename must fail the harness (catches typos).
- A positive fixture file with valid JSON but wrong shape (e.g. extra unknown top-level field on a closed object) must fail.
- A negative fixture file that *passes* validation must fail the harness (means the negative case wasn't actually negative).
- HITL `response.json` with `decision: "reject"` and `global_comment: null` must validate (per HITL doc §3.4 - rejection without comment is allowed).
- HITL `pending.json` with `decision_options: ["comment_only"]` must validate (single-option case).
- `phase_result_parallel` with one branch must validate (`minItems: 1`), but with zero branches must fail.
- Manifest entry with `sha256` of length 63 or 65 must fail (regex anchored).
- Trace event with an unknown `loker.event` enum value must fail.
- Trace event with extra `loker.foo` keys via `patternProperties` must validate (forward-compat).
