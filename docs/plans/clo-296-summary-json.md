# Plan: CLO-296 — summary.json with per-backend tokens + cost_budget_usd warning

## Context
- **Design:** docs/designs/clo-296-summary-json.md
- **Discovery:** docs/discovery/clo-296.md
- **Linear:** https://linear.app/cloud-ai/issue/CLO-296/t-032-implement-summaryjson-with-per-backend-tokens-cost-budget-usd
- **Branch:** `feat/clo-296-summary`

## Sub-tasks

### ST1 PriceTable — prices.toml + pricing lookup
**Files:** `src/summary/prices.rs`, `docs/prices.toml`
**Acceptance:** `cargo test -p loker -- summary::prices` passes
**Estimate:** S

Create `src/summary/prices.rs` with `PriceTable` struct that loads `docs/prices.toml` (TOML keyed by `<backend>:<model>`) and provides `lookup()` / `compute_cost()` methods. Add initial price entries for common models (Claude Opus 4, GPT-5, Gemini 3.1 Pro, etc.). Parse errors are non-fatal — return empty table and log warning.

### ST2 TraceReader — trace.jsonl token aggregation
**Files:** `src/summary/reader.rs`
**Acceptance:** `cargo test -p loker -- summary::reader` passes
**Estimate:** M

Create `src/summary/reader.rs` with `TraceReader::aggregate(path)` that:
- Opens and reads `trace.jsonl` line by line
- Parses each JSON line, extracts `gen_ai.system`, `gen_ai.request.model`, `gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`
- Groups by `(backend, model)` and sums token counts
- Returns `Vec<BackendUsage>`
- Accepts `InMemorySink`-style test capture for deterministic tests

### ST3 SummaryWriter — summary.json emission + manifest registration
**Files:** `src/summary/mod.rs`
**Acceptance:** `cargo test -p loker -- summary::writer` passes
**Estimate:** M

Create `src/summary/mod.rs` with:
- `Summary` struct (serializes to summary.schema.json shape)
- `SummarySink` trait with `fn finalize(&self, &Summary, &RunDir) -> Result<()>`
- `SummaryWriter` impl: serializes Summary to JSON, writes via atomic write to `run_dir/summary.json`, registers entry in manifest (`Kind::SummaryJson` with sha256)
- `InMemorySummarySink` for test capture
- Phase collector that reads marker files (`*.completed`, `*.failed`) from `run_dir/markers/` to build per-phase entries

### ST4 SummaryWriter::finalize — integration wiring
**Files:** `src/summary/mod.rs`, `src/lib.rs`
**Acceptance:** `cargo test -p loker -- summary::finalize` passes
**Estimate:** M

Add `SummaryWriter::finalize(run_dir, manifest, trace_path, cost_budget_usd)` that:
- Calls `TraceReader::aggregate(trace_path)` for token rollups
- Calls `PriceTable::load(docs/prices.toml)` for cost computation
- Calls phase collector for per-phase outcomes
- Constructs `Summary` struct with all fields populated
- Computes `budget_warning` when `cost_budget_usd` is set and exceeded
- Writes via `SummaryWriter` + manifest registration
- Is idempotent on re-finalize (overwrites existing summary.json + updates manifest sha256 in-place)

### ST5 tests/summary.rs — TDD test contract
**Files:** `tests/summary.rs`
**Acceptance:** `cargo test --test summary` passes all 5 test cases
**Estimate:** M

Write the 5 contracted integration tests:
1. `single_phase_success` — 1 phase, 1 backend, token totals match
2. `multi_backend_parallel` — 3 backends, separate rollups per backend/model
3. `failed_phase_emitted` — phase shows `failed`, summary still emitted
4. `price_table_miss_no_panic` — unknown backend, cost_usd=None, no panic
5. `budget_exceeded_warning` — budget_warning=true, exit code unchanged

### ST6 Schema validation + `make check`
**Files:** `tests/schema_validation.rs` (already auto-harnessed)
**Acceptance:** `make check` green (fmt + clippy + test)
**Estimate:** S

- Ensure summary writer output validates against `docs/schemas/summary.schema.json` (the auto-harness in `tests/schema_validation.rs` covers fixture validation; writer test should also validate programmatically via `jsonschema`)
- Run full `make check` suite and fix any clippy warnings or compilation errors in new code
- Register `src/summary` module in `src/lib.rs` as `pub mod summary;`

## Pre-merge gate
- `make check` (fmt + clippy + test) — must pass clean
- All 5 `tests/summary.rs` tests green

## Risks
- **CLO-292 dependency**: SummaryWriter::finalize is called by the run-level executor which doesn't exist yet. Tests mock the caller; integration with the executor is deferred to a follow-up task.
- **Trace format drift**: If trace.jsonl fields change (gen_ai.* renames), TraceReader must keep in sync. Mitigation: the trace_span.schema.json is the source of truth; any schema update triggers a corresponding TraceReader update.
- **Price table staleness**: Provider pricing changes over time. The static prices.toml must be periodically reviewed. Mitigation: the design emits `cost_unknown` for missing entries rather than silently using stale prices.
- **Large traces**: Reading the entire trace.jsonl into memory may be slow for runs with millions of events. Mitigation: documented as accepted for v0; streaming aggregation deferred to post-v0.
