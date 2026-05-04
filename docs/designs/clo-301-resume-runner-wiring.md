# Design: ResumeRunner Execution Wiring (CLO-301 / T-031-follow-up)

## 1. Problem

`ResumeRunner::execute()` exists but is stubbed. `loker resume <run-dir>` bails with "resume execution is not yet implemented". Five blockers prevent end-to-end wiring. This design resolves them.

## 2. Goals / Non-goals

### Goals

- `loker resume <run-dir>` actually drives phase re-execution.
- `PhaseRunner::run()` accepts `initial_attempt: u32` to support resume continuation.
- `Workflow` gains a `to_phase_configs()` adapter so `ResumeRunner` can derive `Vec<PhaseConfig>` from the persisted workflow.
- `resume` CLI subcommand removes "not yet implemented" bail and calls `ResumeRunner::execute()`.
- `make check` green.

### Non-goals

- Replaying specific attempt directories — attempts are archived, not replayed.
- Cross-host coordination.
- Manual rewind (`--from <phase>`) — post-v0.

## 3. Architecture

### 3.1 Module layout

No new modules. All changes land in existing files:

```
src/
  phase_runner.rs          ← MODIFIED: initial_attempt parameter
  resume.rs                ← MODIFIED: use initial_attempt, wire PhaseRunner properly
  workflow/
    mod.rs                 ← MODIFIED: to_phase_configs() adapter
  main.rs                  ← MODIFIED: resume CLI subcommand
```

### 3.2 Data flow

```
CLI: resume <run-dir> [--ttl <seconds>]
  │
  ├─→ lock::acquire(run_dir/.lock)
  ├─→ sweep::stale_tmp(run_dir, ttl)
  ├─→ RunState::load(run_dir, ttl)
  ├─→ ResumePlanner::plan(run_state, phase_configs)  ← NEW: from to_phase_configs()
  ├─→ ResumeRunner::execute(plan)
  │      ├─ if Resume → archive_current_attempt()
  │      └─ PhaseRunner::run(cfg, inputs, initial_attempt)  ← NEW: initial_attempt
  └─→ lock::release()
```

### 3.3 Concrete types

No new types. Extension only:

```rust
// src/phase_runner.rs — PhaseRunner::run signature change
pub async fn run(
    &self,
    cfg: &PhaseConfig,
    inputs: PhaseInputs<'_>,
    initial_attempt: u32,  // NEW
) -> Result<PhaseOutcome, PhaseError> {
    persist::start_attempt(&markers, &cfg.phase, initial_attempt)?;
    // ... subsequent attempts: initial_attempt + 1, initial_attempt + 2, ...
}
```

```rust
// src/workflow/mod.rs — Workflow extension
impl Workflow {
    /// Convert workflow steps to PhaseConfig list for PhaseRunner.
    ///
    /// Maps:
    ///   step.backend / step.backends  → PhaseConfig.backend
    ///   step.prompt                    → PhaseConfig.prompt_template
    ///   step.name                     → PhaseConfig.phase
    ///   step.apply_edits              → VerifyHookName (if set → RunCommand)
    ///   step.verify                   → VerifyHookName::RunCommand
    ///   step.timeout / workflow.timeout → (not wired in PhaseConfig v0)
    ///
    /// Aggregator derived from step.consensus or step.get_consensus_strategy().
    /// Strategy derived from step.retries > 0 (EscalatingRetry with one rung) or 0 (Single).
    pub fn to_phase_configs(&self) -> Vec<PhaseConfig>;
}
```

## 4. Implementation details

### 4.1 Blockers resolution

| Blocker | Resolution |
|---|---|
| initial_attempt parameter | **DONE** — already added to `PhaseRunner::run()` signature; updated `persist::start_attempt()` calls and marker file names. |
| Workflow → PhaseConfig adapter | Add `Workflow::to_phase_configs()`. Maps `step.backend` → `PhaseConfig.backend`, `step.prompt` → `PhaseConfig.prompt_template`, etc. Shell steps skip backend. Consensus strategy → aggregator name. |
| Backend resolution at resume | Call `config.backends.get(name)` per `PhaseConfig.backend` string (same pattern as `WorkflowRunner`). No new resolution logic needed. |
| Prompt reconstruction | Reconstruct `Prompt` by loading artefact content from manifest entries for upstream phase outputs. `Prompt::with_artefact()` method added. |
| Verify hook + trace wiring | Pass `verify: Some(arc)` from CLI config. `TraceWriter` opened on `run_dir/trace.jsonl`. Both injected into `PhaseInputs`. |

### 4.2 Workflow → PhaseConfig derivation rules

| `Workflow::Step` field | → | `PhaseConfig` field |
|---|---|---|
| `step.name` | → | `phase` |
| `step.backend` or first of `step.backends` | → | `backend` (single) |
| `step.prompt` | → | `prompt_template` |
| `step.apply_edits` | → | `verify: VerifyHookName::RunCommand` |
| `step.verify` (shell command) | → | `verify: VerifyHookName::RunCommand` |
| `step.consensus` or `get_consensus_strategy()` | → | `aggregator: AggregatorName` |
| `step.retries > 0` | → | `strategy: EscalatingRetry` (one rung) |
| `step.retries == 0, step.backends.len() <= 1` | → | `strategy: Single` |
| `step.backends.len() > 1` | → | `strategy: Parallel, targets: backends` |
| `output_format` | → | `artefact_kind: Kind` (infer from output ext) |

Shell steps (`step.shell.is_some()`) are **excluded** from `to_phase_configs()` — they run via `WorkflowRunner` shell path, not `PhaseRunner`. **Resume-path limitation**: if a workflow fails on a shell step, `loker resume` will skip it. This is acceptable for v0. Shell-step resumption can be addressed in a follow-up by adding a `PhaseConfig::shell()` variant.

### 4.3 Workflow name persistence

Two options:

**Option A — Add to `manifest.json`** (chosen):
- Store `workflow_name: String` at root level of `manifest.json`.
- `RunDir` writes it on creation.
- `ResumeRunner` reads it from `manifest.json` to locate the workflow definition.
- **Note**: Before execution, `ResumeRunner` triggers the manifest orphan-entry sweep (defined in `docs/run-state.md` row 9) to handle partially-written manifest entries from interrupted runs.

**Option B — Infer from directory name**:
- `run_dir` path contains slug (e.g. `runs/design-doc-tdd-20260504-abc/`).
- `slug::unslugify()` or heuristic extraction (`runs/<slug>-<date>-<uuid>`).
- **Rejected** — slug→name is not bijective.

### 4.4 Prompt reconstruction

```rust
// src/strategy/mod.rs or src/strategy/prompt.rs
impl Prompt {
    /// Add upstream artefact content from manifest entry.
    pub fn with_artefact(mut self, name: &str, content: Vec<u8>) -> Self {
        self.artefacts.insert(name.to_string(), content);
        self
    }
}
```

`ResumeRunner` loads manifest entries for all `completed` phases before calling `PhaseRunner::run()`. For each `input` in `PhaseConfig` that references a prior phase, the artefact content is loaded and added to the `Prompt`.

### 4.5 Trace + verify wiring

From CLI entry point (before calling `ResumeRunner`):

```rust
let trace = Some(Arc::new(TraceWriter::new(run_dir.trace_path())?));
let verify = resolve_verify_hook_from_config(&config)?;  // from Step.verify

let inputs = PhaseInputs {
    backends: &backends,
    prompt,
    ctx,
    verify,
    run_dir: run_dir.clone(),
    trace,
};
```

## 5. Test plan

### 5.1 Unit tests

| Test | What it checks |
|---|---|
| `workflow_to_phase_configs_single` | `step.backend = "claude"` → `PhaseConfig.backend = Some("claude")`, `strategy = Single` |
| `workflow_to_phase_configs_parallel` | `step.backends = ["a","b"]` → `strategy = Parallel`, `targets = ["a","b"]` |
| `workflow_to_phase_configs_escalating` | `step.retries = 2` → `strategy = EscalatingRetry` |
| `workflow_to_phase_configs_verify` | `step.apply_edits = true` → `verify = VerifyHookName::RunCommand` |
| `workflow_to_phase_configs_shell_skipped` | `step.shell = Some(..)` → phase excluded from list |

### 5.2 Integration tests (`tests/resume.rs`)

| # | Scenario | Assertion |
|---|---|---|
| 1 | Kill mid-phase-2 | Resume re-runs phase 2 in `attempts/phase2/<n>/`, phase 3 runs |
| 2 | Already complete | Resume is no-op; manifest unchanged; exit 0 |
| 3 | initial_attempt > 0 | `PhaseRunner::run()` writes `markers/phase.started.N` with correct N |

## 6. Migration / rollout

No migration needed — `initial_attempt` parameter defaults to `0` for fresh runs (existing callers updated in this PR). Workflow name added to `manifest.json` schema is additive (`serde(default)` on both sides).

- `make check` must pass before merge.
- Update `loker resume --help` once subcommand is wired.

## 7. Open questions

| # | Question | Resolution |
|---|---|---|
| 1 | Should `to_phase_configs()` be on `Workflow` or a free function? | On `Workflow` — keeps the adapter in the workflow module. |
| 2 | How to handle `step.consensus` → `AggregatorName` mapping? | `Synthesis` → `First`, `Vote`/`WeightedVote` → `Vote`, `AllPass` → `AllPass`. |
| 3 | What `artefact_kind` for non-standard output formats? | Infer from `step.output_format`: "json" → `VerifyJson`, "lines" → `ResponseJson`, else default `DesignMd`. |

## 8. Dependencies

- CLO-295 (planner + scaffolding) — merged ✓
- CLO-292 PhaseRunner — merged ✓
- CLO-293 Trace — merged ✓
