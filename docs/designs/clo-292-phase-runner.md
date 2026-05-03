# Design: CLO-292 - PhaseRunner composing Strategy + Aggregator + VerifyHook

## 1. Problem

Per the discovery report, loker has the leaf primitives a phase needs - `Strategy` implementations (single, parallel, escalating), aggregator folds (concat, vote, llm_judge, any_fail), `VerifyHook` implementations (run_command, llm_verifier), `Manifest` append/rewrite, and `MarkerWriter` started/completed/failed - but no coordinator that composes them into a single durable phase outcome. Workflow authors and the M5 resume/run orchestration layer cannot today drive a phase from input artefacts to a verified canonical artefact with the run-state write protocol applied, so M5 trace/run/resume tasks (T-029, T-030, T-031) and the M6 reference workflow are blocked behind T-028. The desired behaviour is a `PhaseRunner::run(...)` API that dispatches by configured names, executes attempts, applies a verify hook, persists exactly one canonical artefact, registers exactly one manifest entry, and writes started/completed/failed markers in protocol order.

## 2. Goals / Non-goals

### Goals

- New module `src/phase_runner.rs` exposing `PhaseRunner::run(phase, inputs) -> Result<PhaseOutcome, PhaseError>`.
- Name-based dispatch over the issue's vocabulary:
  - strategies: `single`, `parallel`, `escalating_retry`;
  - aggregators: `first`, `concat`, `vote`, `any_fail`, `all_pass`;
  - verify hooks: `run_command`, `llm_verifier`.
- On success: write exactly one canonical artefact, append exactly one `ManifestEntry` whose `producer` matches the strategy, and write `<phase>.completed` via `MarkerWriter`.
- On every attempt: write `<phase>.started.<n>` before the strategy runs and archive failed-attempt debris under `attempts/<phase>/<n>/` before any subsequent attempt or terminal marker.
- Terminal failure path writes `<phase>.failed` with the configured `error_class` and propagates a typed `PhaseError` variant (`PhaseFailed`, `VerifyFailed`, `StrategyFailed`).
- All five PRD acceptance tests pass under `make check` with mocked backends and verifiers; no live network.

### Non-goals

- CLI integration, run-directory bootstrapping, run-id generation (T-030/T-031).
- Tracing, cost accounting, quota enforcement (M5+).
- Resumability decisions across phases - this slice writes the markers M5 will consume; it does not itself decide what to skip.
- New strategies, aggregators, or verify hooks. PhaseRunner only composes existing ones; the only new dispatch logic is the thin `first` / `all_pass` helpers explicitly called out in discovery.
- A workflow-TOML parser. PhaseRunner consumes a Rust config struct; mapping TOML to it is the next slice's job.
- Replacing the `consensus.rs` / `apply_verify/` legacy paths.

## 3. Architecture

### Composition

```
                              PhaseRunner::run
                                    |
                +-------------------+------------------+
                |                   |                  |
                v                   v                  v
          MarkerWriter        StrategyDispatch    AggregatorDispatch
          (started/             |                  |
           completed/           v                  v
           failed)            Strategy           Aggregator (first / concat / vote /
          + attempts/<n>/     ::execute          all_pass / any_fail)
                                |                  |
                                v                  v
                           StrategyOutput      canonical artefact bytes
                                                   |
                                                   v
                                            VerifyHookDispatch
                                            (run_command / llm_verifier / none)
                                                   |
                                                   v
                                              VerifyResult
                                                   |
                                                   v
                                          atomic_write artefact
                                                   |
                                                   v
                                          Manifest::append (one entry)
                                                   |
                                                   v
                                          MarkerWriter::write_completed
```

### Modules

- `src/phase_runner.rs` (new). Public surface.
- `src/phase_runner/dispatch.rs` (new, private). Three name-to-implementation tables: strategies, aggregators, verify hooks. Each table validates names at config build time, not at run time.
- `src/phase_runner/persist.rs` (new, private). Helpers that own the protocol order: `start_attempt`, `archive_failed_attempt`, `commit_success`, `record_terminal_failure`. These wrap `atomic_write`, `Manifest::append`, and `MarkerWriter` so the runner body stays linear.
- `src/aggregator/concat.rs` and `src/strategy/mod.rs` get additive `First` and `AllPass` variants so PhaseRunner does not introduce a parallel public aggregation trait.
- Re-exports from `src/lib.rs`: `PhaseRunner`, `PhaseConfig`, `PhaseInputs`, `PhaseOutcome`, `PhaseError`, `StrategyName`, `AggregatorName`, `VerifyHookName`. Internal modules use the existing root-level `#![allow(dead_code)]`.

### Data flow within one `run` call

1. `start_attempt(0)` writes `<phase>.started.0`.
2. The dispatcher resolves `cfg.strategy` to `Arc<dyn Strategy>` and calls `Strategy::execute(backends, prompt, ctx)`.
3. The strategy returns `StrategyOutput`. PhaseRunner reads the canonical bytes for the artefact - for `single` and `escalating_retry` this is the winning attempt's `output_path`; for `parallel` it is the aggregator output (see step 4).
4. The aggregator dispatcher folds branch outputs:
   - `first`: add an `aggregator::Aggregator::First` behavior and `strategy::Aggregator::First` schema label; take the first successful attempt's output path verbatim. Trivial for `single` / `escalating_retry`.
   - `concat`, `vote`: delegate to existing aggregator modules (`aggregate()` for concat, vote helper for vote).
   - `any_fail`: reuse existing `any_fail_evaluate` semantics and record the verdict; branch failures become a typed aggregator error.
   - `all_pass`: add an `aggregator::Aggregator::AllPass` behavior and `strategy::Aggregator::AllPass` schema label; collect every branch/verify verdict before failing so diagnostics include all failures.
5. The verify hook dispatcher resolves `cfg.verify` to `Option<Arc<dyn VerifyHook>>` and runs `verify(ctx)` against the canonical bytes. `Skipped` = configured `none`. `Pass` proceeds; `Fail` triggers retry semantics owned by the strategy (escalating) or terminal failure (single/parallel).
6. On success: `commit_success` atomically writes the artefact under `<run_dir>/<artefact_name>`, appends a `ManifestEntry { producer, kind, sha256, phase: Some(phase), attempt: Some(n) }`, then writes `<phase>.completed`.
7. On terminal failure: `record_terminal_failure` archives the live attempt directory to `attempts/<phase>/<n>/`, writes `<phase>.failed` with `error_class` derived from the typed error, and returns `PhaseError`.

### Concrete Rust types

- `PhaseRunner` is a unit-state struct - all per-run state lives in stack locals plus the `run_dir` it was constructed with. This matches the PRD line "no live in-memory state".
- `PhaseConfig` carries names plus the minimal constructor fields needed by existing strategies/hooks, so workflow parsing in T-030 can map TOML into this struct without owning execution.
- `PhaseInputs` carries the `&[Arc<dyn Backend>]` already-resolved and the `Prompt` to forward to `Strategy::execute`.
- `PhaseRunner` derives canonical bytes by reading the path already surfaced by `StrategyOutput` (`aggregate_output_path` for parallel aggregations, otherwise the first passing/winning `Attempt.output_path`). This keeps strategies as the source of candidate-output truth without adding byte buffers to their public output.
- `PhaseOutcome` reports the canonical artefact path, the manifest entry SHA, and the `StrategyOutput` for trace consumers in M5.

## 4. Public API surface

The public surface lives in `src/phase_runner.rs` and is re-exported from `src/lib.rs`. All non-public helpers are crate-private.

```rust
//! src/phase_runner.rs

use std::path::PathBuf;
use std::sync::Arc;

use crate::aggregator::{Aggregator, AggregatorError};
use crate::backend::Backend;
use crate::manifest::{ManifestEntry, ManifestError, Producer};
use crate::run_state::markers::MarkerError;
use crate::strategy::{
    PhaseContext, Prompt, StrategyError, StrategyKind, StrategyOutput,
};
use crate::strategy::verify::{VerifyHook, VerifyResult};

/// Strategy dispatch label. Mapped to a concrete `Arc<dyn Strategy>`
/// at config-build time; never re-resolved per attempt.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum StrategyName {
    Single,
    Parallel,
    EscalatingRetry,
}

/// Aggregator dispatch label.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum AggregatorName {
    First,
    Concat,
    Vote,
    AnyFail,
    AllPass,
}

/// Verify hook dispatch label. `None` is the explicit no-op.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum VerifyHookName {
    None,
    RunCommand,
    LlmVerifier,
}

/// Phase-level configuration. Built by callers (test code today,
/// workflow parser in T-030) and consumed by `PhaseRunner::run`.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PhaseConfig {
    pub phase: String,
    pub strategy: StrategyName,
    pub aggregator: AggregatorName,
    pub verify: VerifyHookName,
    /// File name (relative to `run_dir`) for the canonical artefact.
    pub artefact_name: String,
    /// Manifest `producer` - independent of `StrategyName` so the caller
    /// can keep schema mapping local.
    pub producer: Producer,
}

/// Per-call inputs. Backends are resolved by the caller so PhaseRunner
/// never holds backend handles or credentials beyond the duration of `run`.
pub struct PhaseInputs<'a> {
    pub backends: &'a [Arc<dyn Backend>],
    pub prompt: Prompt,
    pub ctx: PhaseContext,
    pub verify: Option<Arc<dyn VerifyHook>>,
    pub run_dir: PathBuf,
}

/// Result of a successful phase run.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PhaseOutcome {
    pub artefact_path: PathBuf,
    pub manifest_entry: ManifestEntry,
    pub strategy_output: StrategyOutput,
    pub strategy_kind: StrategyKind,
    pub verify: Option<VerifyResult>,
}

/// Typed terminal failure.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum PhaseError {
    #[error("strategy failed: {0}")]
    StrategyFailed(#[from] StrategyError),

    #[error("verify failed in phase `{phase}` at attempt {attempt}")]
    VerifyFailed {
        phase: String,
        attempt: u32,
        result: VerifyResult,
    },

    #[error("phase `{phase}` failed: {message}")]
    PhaseFailed { phase: String, message: String },

    #[error("aggregator failed: {0}")]
    Aggregator(#[from] AggregatorError),

    #[error("manifest write failed: {0}")]
    Manifest(#[from] ManifestError),

    #[error("marker write failed: {0}")]
    Marker(#[from] MarkerError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Phase-level coordinator. One instance is reusable across phases;
/// the `run_dir` argument scopes a single run.
#[derive(Debug, Default, Clone, Copy)]
pub struct PhaseRunner;

impl PhaseRunner {
    pub const fn new() -> Self {
        Self
    }

    /// Run one phase to a terminal state. Writes markers, the canonical
    /// artefact, and the manifest entry per `docs/run-state.md`.
    pub async fn run(
        &self,
        cfg: &PhaseConfig,
        inputs: PhaseInputs<'_>,
    ) -> Result<PhaseOutcome, PhaseError>;
}
```

Crate-private helpers (signatures only - bodies are an implementation concern):

```rust
// src/phase_runner/dispatch.rs

pub(crate) fn resolve_aggregator(name: AggregatorName) -> Result<Aggregator, PhaseError>;
pub(crate) fn resolve_strategy(cfg: &PhaseConfig) -> Result<Arc<dyn crate::strategy::Strategy>, PhaseError>;
pub(crate) fn canonical_bytes(
    run_dir: &Path,
    output: &StrategyOutput,
    aggregator: &Aggregator,
) -> Result<Vec<u8>, PhaseError>;

// src/phase_runner/persist.rs

pub(crate) fn start_attempt(
    markers: &MarkerWriter,
    phase: &str,
    attempt: u32,
) -> Result<(), MarkerError>;

pub(crate) fn archive_failed_attempt(
    run_dir: &Path,
    phase: &str,
    attempt: u32,
) -> Result<PathBuf, std::io::Error>;

pub(crate) fn commit_success(
    run_dir: &Path,
    cfg: &PhaseConfig,
    bytes: &[u8],
    attempt: u32,
) -> Result<(PathBuf, ManifestEntry), PhaseError>;

pub(crate) fn record_terminal_failure(
    markers: &MarkerWriter,
    run_dir: &Path,
    cfg: &PhaseConfig,
    attempts_made: u32,
    error_class: &str,
) -> Result<(), PhaseError>;
```

## 5. Test plan

All tests use `wiremock` for backend HTTP and stub `VerifyHook` impls for verify outcomes. No tests require `LOKER_TZ_INTEGRATION`.

### Unit tests (in `src/phase_runner.rs` `#[cfg(test)]` module)

- `name_dispatch_resolves_known_strategies` - asserts each `StrategyName` resolves to the expected concrete type via the dispatcher.
- `name_dispatch_resolves_known_aggregators` - same for aggregators including newly added `First` / `AllPass` variants.
- `name_dispatch_resolves_known_verify_hooks` - same for verify hooks plus the `None` variant.
- `archive_failed_attempt_moves_debris_to_attempts_dir` - exercises the persist helper with a temp dir.
- `commit_success_writes_artefact_and_appends_one_manifest_entry` - asserts atomic write order and SHA matches `sha256_hex(bytes)`.

### Integration tests (`tests/phase_runner_integration.rs`)

Each test maps to one PRD acceptance criterion. Names follow existing convention.

- `single_first_no_verify_emits_one_artefact_and_completed_marker` - PRD AC #1.
- `parallel_concat_any_fail_with_run_command_verifier_emits_review_completed` - PRD AC #2. Uses three wiremock-backed branches and a stub `RunCommand` that returns `VerifyResult::Pass`.
- `escalating_retry_all_pass_with_llm_verifier_recovers_after_two_failures` - PRD AC #3. Stub `LLMVerifier` returns Fail, Fail, Pass; assert attempt directories `attempts/<phase>/0/` and `.../1/` exist with debris and `.../2/` is the one written into the manifest.
- `terminal_verify_failure_writes_failed_marker_with_reason_and_propagates_phase_failed` - PRD AC #4. `error_class` field on the failed marker matches `"verify_failed"`.
- `make_check_remains_green_without_network` - meta-test: a parallel run with no env vars set; backed entirely by wiremock servers spawned in-process.

### Manual verification

- `make check` clean on the feature branch.
- `cargo test -q phase_runner` lists all five integration tests as passing.
- Manual inspection of an `escalating_retry` run directory shows: `markers/<phase>.started.0`, `markers/<phase>.started.1`, `markers/<phase>.started.2`, `markers/<phase>.completed`, `attempts/<phase>/0/`, `attempts/<phase>/1/`, one canonical artefact, one manifest entry.

## 6. Migration / rollout

- No data migration. Nothing currently writes the `<phase>.completed` / `<phase>.failed` markers from a single coordinator, so there is no legacy on-disk format to bridge.
- No feature flag. PhaseRunner is additive: existing callers of `Strategy::execute` directly (the `consensus.rs` / `apply_verify/` paths) remain untouched and keep working until M6 retires them.
- Workflow TOML parsing is out of scope (T-030). Until that lands, the only callers building `PhaseConfig` are the integration tests above and any in-tree fixture scaffolding the M6 reference workflow grows on top.
- Rollout order: land the dispatch + persist helpers first, then `PhaseRunner::run`, then the five integration tests. Each step keeps `make check` green.

## 7. Open questions

No implementation-blocking questions remain for CLO-292.

Resolved for this slice:

1. **Owner of branch debris under `attempts/<phase>/<n>/`.** PhaseRunner archives runner-owned canonical/tmp artefacts and a small failure summary only. It does not copy every internal parallel branch output unless that output is already surfaced as a runner-owned artefact path. Rich per-branch diagnostic archives are deferred to a future run-observability slice so PhaseRunner stays thin.
2. **Mapping `PhaseError` variants to `error_class` strings on the failed marker.** CLO-292 implementation will use the stable strings `strategy_failed`, `verify_failed`, `aggregator_failed`, `manifest_failed`, `marker_failed`, and `io_failed`. Schema-level enum enforcement is a follow-up for the first M5 consumer that needs a stricter contract.
