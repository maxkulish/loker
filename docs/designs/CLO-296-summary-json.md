# Design: CLO-296 — summary.json with per-backend tokens + cost_budget_usd warning

## Problem

Loker's PhaseRunner (CLO-292) now executes phases, writes per-phase artefacts, a manifest, a trace.jsonl, and status markers — but emits no aggregate run summary. Every stakeholder (operators, CI pipelines, downstream tooling) who wants to answer "how much did this run cost in total?" or "did it exceed the configured budget?" must inspect trace.jsonl line by line or parse multiple marker files. PRD FR-23 mandates a `runs/<id>/summary.json` that aggregates per-phase outcomes, per-backend token totals, wall-clock duration, and a cost-budget warning. This is the T-032 roadmap row and a hard dependency for the M6 end-to-end reference workflow, where cost feedback is expected after every run.

## Goals / Non-goals

### Goals

- `SummaryWriter::finalize()` function invoked once per run (success or terminal failure) that writes `runs/<id>/summary.json`.
- Aggregates token usage from `trace.jsonl` keyed by `gen_ai.system` (backend) + `gen_ai.request.model`.
- Computes cost via a static `prices.toml` table keyed by `<backend>:<model>`; missing entries produce `cost_unknown: true` per-backend rather than failing the summary.
- Per-phase entries with outcome (`completed` / `failed` / `skipped`), attempt count, wall-clock ms, and strategy name, sourced from phase status markers + manifest.
- `cost_budget_warning: bool` when `total_cost_usd > cost_budget_usd` (advisory only — never fails the run).
- Summary is registered in the manifest (`Kind::SummaryJson`) with SHA-256 of the bytes.
- Idempotent on resume: re-finalize overwrites the existing summary.json.
- Output validates against `docs/schemas/summary.schema.json`.

### Non-goals

- Live cost streaming during a run (post-v0).
- HTML/markdown report generation (post-v0).
- Cost as a hard gate that fails runs (post-v0).
- Live OTLP export (post-v0).
- Per-step breakdown within a phase (the trace already has per-backend granularity; summary aggregates at the backend+model level).
- Automatic price-table updates or provider API queries for cost data.

## Architecture

### Modules

The implementation adds three new files and one TOML data file:

```
src/summary/
├── mod.rs           # SummarySink trait, SummaryWriter, Summary struct, TraceReader
├── prices.rs        # PriceTable: loads prices.toml, looks up cost per backend+model
└── reader.rs        # TraceReader: parses trace.jsonl, aggregates tokens per backend+model
docs/prices.toml     # Static price table (committed to repo)
tests/summary.rs     # TDD test contract (5 test cases)
```

### Data flow

```
Run completes (success or terminal failure)
  │
  ▼
SummaryWriter::finalize(run_dir, manifest, trace_path, cost_budget_usd)
  │
  ├── TraceReader::aggregate(trace_path)
  │     │
  │     ├── Parse each JSONL line matching gen_ai.* span records
  │     ├── Group by (gen_ai.system, gen_ai.request.model)
  │     ├── Sum gen_ai.usage.input_tokens, gen_ai.usage.output_tokens
  │     └── Return Vec<BackendUsage>
  │
  ├── PhaseCollector::collect(run_dir, manifest)
  │     │
  │     ├── Walk markers/*.completed, markers/*.failed
  │     ├── For each phase: derive outcome (completed/failed/skipped)
  │     ├── Read attempts count from phase_result.json or marker metadata
  │     └── Return Vec<PhaseSummary>
  │
  ├── PriceTable::lookup(backend, model)
  │     │
  │     ├── Load prices.toml on first access
  │     ├── Match (backend, model) key → cost per token
  │     ├── Compute cost_usd for each BackendUsage
  │     └── Flag cost_unknown: true if key missing
  │
  ├── Build Summary struct
  │     │
  │     ├── Set totals.input_tokens, totals.output_tokens, totals.cost_usd
  │     ├── Set backends[] with per-backend+model rollup + cost
  │     ├── Set phases[] with per-phase outcome + attempts + duration_ms
  │     ├── Compute duration_ms from earliest started_at → latest finished_at
  │     ├── Compare totals.cost_usd vs cost_budget_usd → budget_warning
  │     └── Serialize to JSON
  │
  ├── atomic_write(run_dir / "summary.json", json_bytes)
  ├── Manifest::append(ManifestEntry { kind: SummaryJson, sha256 })
  └── Return Ok(())
```

### Concrete Rust types

```rust
// src/summary/prices.rs

/// Static price per million tokens for a (backend, model) pair.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelPrice {
    pub input_per_mtok: f64,  // USD per million input tokens
    pub output_per_mtok: f64, // USD per million output tokens
}

/// Price table loaded from prices.toml.
/// Keys are "<backend>:<model>" (e.g. "anthropic:claude-opus-4-7").
#[derive(Debug, Clone)]
pub struct PriceTable {
    prices: HashMap<String, ModelPrice>,
}

impl PriceTable {
    /// Load from the canonical prices.toml path.
    /// A syntax error in prices.toml is non-fatal: `load` logs a warning
    /// and returns an empty PriceTable. All backends will have
    /// `cost_usd: None` but the summary is still emitted.
    pub fn load(path: &Path) -> Result<Self, PriceError>;

    /// Look up price for a (backend, model) pair.
    /// Returns None if the key is not in the table.
    pub fn lookup(&self, backend: &str, model: &str) -> Option<&ModelPrice>;

    /// Compute cost in USD for given token counts.
    /// Returns None if price is unknown.
    pub fn compute_cost(&self, backend: &str, model: &str, input_tokens: u64, output_tokens: u64) -> Option<f64>;
}


// src/summary/reader.rs

/// Aggregated token usage for a single (backend, model) pair.
#[derive(Debug, Clone, Serialize)]
pub struct BackendUsage {
    pub name: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// Parses trace.jsonl and aggregates token usage.
pub struct TraceReader;

impl TraceReader {
    /// Read all lines from trace.jsonl, parse gen_ai.* spans,
    /// and aggregate token usage per (backend, model).
    ///
    /// Only processes lines that have gen_ai.usage.input_tokens and
    /// gen_ai.usage.output_tokens. Lines without usage (e.g. aggregator/verify)
    /// are skipped silently.
    pub fn aggregate(path: &Path) -> Result<Vec<BackendUsage>, TraceReaderError>;
}


// src/summary/mod.rs

use crate::manifest::Manifest;
use crate::run_state::run_dir::RunDir;
use std::path::Path;

/// Per-phase outcome for the summary.
#[derive(Debug, Clone, Serialize)]
pub struct PhaseSummary {
    pub phase: String,
    pub status: PhaseStatus,
    pub attempts: u32,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strategy: Option<String>,
}

/// Phase outcome status.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseStatus {
    Completed,
    Failed,
    Skipped,
}

/// Overall run status for the summary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Success,
    Failed,
    Partial,
    Aborted,
}

/// Aggregate token/cost totals.
#[derive(Debug, Clone, Serialize)]
pub struct Totals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

/// The summary.json envelope.
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    #[serde(rename = "loker.run_id")]
    pub run_id: String,
    pub schema_version: u32,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: u64,
    pub status: RunStatus,
    pub totals: Totals,
    pub backends: Vec<BackendUsage>,
    pub phases: Vec<PhaseSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_budget_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_warning: Option<bool>,
}

/// SummarySink trait for testability (follows TraceSink pattern).
pub trait SummarySink: Send + Sync {
    fn finalize(&self, summary: &Summary, run_dir: &RunDir) -> Result<(), SummaryError>;
}

/// File-backed SummarySink that writes summary.json.
pub struct SummaryWriter {
    fsync: bool,
}

impl SummaryWriter {
    pub fn new(fsync: bool) -> Self;

    /// Finalize: read trace, collect phases, compute costs, write summary.json,
    /// register in manifest.
    pub fn finalize(
        &self,
        run_dir: &RunDir,
        manifest: &mut Manifest,
        trace_path: &Path,
        cost_budget_usd: Option<f64>,
    ) -> Result<Summary, SummaryError>;
}

impl SummarySink for SummaryWriter {
    fn finalize(&self, summary: &Summary, run_dir: &RunDir) -> Result<(), SummaryError>;
}

/// In-memory test sink (similar to InMemorySink for TraceSink).
pub struct InMemorySummarySink {
    summaries: Mutex<Vec<Summary>>,
}

impl InMemorySummarySink {
    pub fn new() -> Self;
    pub fn summaries(&self) -> Vec<Summary>;
}

impl SummarySink for InMemorySummarySink {
    fn finalize(&self, summary: &Summary, run_dir: &RunDir) -> Result<(), SummaryError>;
}

/// Error type for summary operations.
#[derive(Debug, thiserror::Error)]
pub enum SummaryError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Trace parse error: {0}")]
    TraceParse(String),
    #[error("Price table error: {0}")]
    Price(#[from] PriceError),
    #[error("Manifest error: {0}")]
    Manifest(#[from] crate::manifest::ManifestError),
}
```

## Test plan

### Performance note

The initial `TraceReader::aggregate` reads all trace lines into memory. For runs
with millions of trace events, this may use significant memory. This is accepted
for v0; a streaming aggregation approach (reading one line at a time) can be
adopted post-v0 if performance becomes a concern. The `TraceReader` trait design
can be extended to accept an iterator rather than a file path without changing
`SummaryWriter::finalize`'s signature.

### Unit tests (`tests/summary.rs`)

| # | Test | Scenario | Verification |
|---|------|----------|-------------|
| 1 | `single_phase_success` | Single-phase run with one backend call | Summary has 1 phase entry (`completed`), token totals match mocked backend response, one `backends[]` entry |
| 2 | `multi_backend_parallel` | Parallel fan-out with 3 different backends | Separate token rollups per backend/model, `backends[]` has 3 entries, totals are sum of all |
| 3 | `failed_phase_emitted` | Run where one phase failed but summary still emitted | Phase entry shows `failed`, summary status reflects partial/failed, summary file still written |
| 4 | `price_table_miss_no_panic` | Backend+model not in prices.toml | `cost_usd: None` for that backend, `cost_unknown` flag set, no panic, summary written successfully |
| 5 | `budget_exceeded_warning` | total_cost_usd > cost_budget_usd | `budget_warning: true` in output, exit code unchanged |

### Schema validation

The existing `tests/schema_validation.rs` harness already validates all fixtures under `tests/fixtures/schemas/summary/` against `docs/schemas/summary.schema.json`. The summary writer test should also validate its output against the schema programmatically using the `jsonschema` crate (as `tests/trace_jsonl.rs` does).

### Fixture-based tests

- Create synthetic trace.jsonl files with known token counts.
- Create synthetic marker directories with known phase outcomes.
- Verify the summary output matches expected values.

## Migration / rollout

This is a fully additive feature. No existing code paths are modified; no backends, strategies, or aggregators change. The `SummaryWriter::finalize` function is a new public API that the run-level executor (separate task) will call after all phases complete. Since the run-level executor doesn't exist yet, tests call `finalize` directly with mock RunDir+Manifest.

The `prices.toml` file is new and ships committed to the repo. It starts with a minimal set of known price points; operators extend it as they add backends/models.

## Open questions

1. **Pricing table location.** Should `prices.toml` live at `docs/prices.toml` (shipped with the repo as a static reference), at the project root (alongside `lok.toml` as user-configurable), or both with an override? The issue says "static price table `prices.toml`" without specifying where. Recommendation: default to `docs/prices.toml` as a built-in reference, with `lok.toml` optionally overriding the path.

2. **`cost_budget_usd` sourcing.** The issue says "if `cost_budget_usd` was set on the workflow" — but `Workflow` struct doesn't have this field. Should it be added to `Workflow` (modifying a stable type), passed as a parameter to `finalize()`, or read from `lok.toml`? Recommendation: pass as an `Option<f64>` parameter to `finalize()` to avoid coupling with the workflow parser. A follow-up task can thread it from TOML.

3. **Tri-state status computation.** The summary's top-level `status` field (`success` / `failed` / `partial` / `aborted`) needs to be computed from per-phase outcomes. The spec says "invoked once on run completion (success OR terminal failure)". What triggers `partial` vs `failed` vs `aborted`? Proposal: if all phases completed → `success`; if some failed but run continued → `partial`; if run terminated early → `aborted`; if every phase failed → `failed`.

4. **Partial-pricing totals.** If some backends have known prices and others don't, should `totals.cost_usd` be `None` (all unknown = no total) or `Some(f64)` (known-only subtotal)? The schema marks `cost_usd` as optional. Proposal: `Some(subtotal of known entries)` when at least one backend has a price; `None` when none do. Document the semantics.

5. **Manifest deduplication on re-finalize.** On resume, `finalize` is called again and must overwrite `summary.json`. The existing `Manifest::append` pushes a new entry — do we need a "replace existing of same kind" operation, or should `summary.json` not re-register in the manifest? Proposal: re-finalize overwrites the file in-place and updates the manifest entry's sha256 (not append). A `Manifest::replace(kind, entry)` method or a `deduplicate_entries()` helper.

6. **Run timestamp source of truth.** Should `started_at` / `finished_at` come from the trace (first/last event timestamps), from the markers (first `*.started` / last `*.completed`/`*.failed`), or from `run_dir` directory creation time? Proposal: `started_at` = timestamp of first marker write (earliest `*.started`), `finished_at` = timestamp of last terminal marker (latest `*.completed` or `*.failed`). Fall back to file mtimes for both.
