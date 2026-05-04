# Implementation Plan — CLO-301: Wire ResumeRunner Execution End-to-End

## Task Summary

Wire `ResumeRunner::execute()` end-to-end so `loker resume <run-dir>` actually resumes interrupted runs. Design doc: `docs/designs/clo-301-resume-runner-wiring.md`.

## Sub-tasks

### P1 · Add `Workflow::to_phase_configs()` adapter
**Design ref**: §4.2, §3.3  
**Files**: `src/workflow/mod.rs`  
**Acceptance**: `cargo test workflow_to_phase_configs_*` all green (5 unit tests)

1. Add `to_phase_configs(&self) -> Vec<PhaseConfig>` method to `impl Workflow`.
2. Map fields per derivation table: `step.name → phase`, `step.backend → backend`, `step.prompt → prompt_template`, etc.
3. Derive strategy from `step.retries > 0` (EscalatingRetry) or `step.backends.len() > 1` (Parallel).
4. Derive aggregator from `step.get_consensus_strategy()`: First/Synthesis → First, Vote/WeightedVote → Vote.
5. Derive verify from `step.apply_edits` or `step.verify` → RunCommand; else None.
6. Exclude shell steps (`step.shell.is_some()`).
7. Write 5 unit tests covering single, parallel, escalating, verify, shell-skipped paths.

### P2 · Wire `loker resume` CLI command
**Design ref**: §3.2, §4.5  
**Files**: `src/main.rs`  
**Acceptance**: `cargo test --test resume` passes; `make check` green

1. In `Commands::Resume` arm, replace "not yet implemented" bail with actual wiring:
   a. Load `manifest.json` → read `workflow_name` field.
   b. Locate workflow file (by slug or stored path).
   c. Call `workflow.to_phase_configs()` → `Vec<PhaseConfig>`.
   d. Resolve backends from config via `config.backends.get(name)`.
   e. Open `TraceWriter` on `run_dir/trace.jsonl`.
   f. Resolve verify hook from config.
   g. Call `ResumeRunner::execute(run_dir, phase_configs, backends, trace, verify)`.
2. Update `loker resume --help` text once wired.

### P3 · Add `tests/resume.rs` integration tests
**Design ref**: §5.2  
**Files**: `tests/resume.rs` (new)  
**Acceptance**: 3 integration tests green

| # | Scenario | What it asserts |
|---|---|---|
| 1 | Kill mid-phase-2 | Resume re-runs phase 2 in `attempts/phase2/<n>/`, phase 3 runs |
| 2 | Already complete | Resume is no-op; manifest unchanged; exit 0 |
| 3 | initial_attempt > 0 | `PhaseRunner::run()` writes `markers/phase.started.N` with correct N |

### P4 · Manifest workflow name persistence
**Design ref**: §4.3 (Option A)  
**Files**: `src/manifest.rs`, `src/run_state/run_dir.rs`  
**Acceptance**: `manifest.json` contains `workflow_name` after fresh run; ResumeRunner reads it

1. Add `workflow_name: Option<String>` field to `Manifest` struct with `#[serde(default)]`.
2. In `RunDir::create()`, accept and write `workflow_name` to manifest.
3. Verify `RunState::load()` handles missing field gracefully (existing runs without the field).

### P5 · Verify `make check` green end-to-end
**Files**: all modified  
**Acceptance**: `cargo fmt --check && cargo clippy && cargo test -q` all pass

1. Run full `make check` before PR.
2. Fix any new clippy warnings introduced by changes.

## Implementation Order

```
P4 (manifest) → P1 (adapter) → P2 (CLI wiring) → P3 (tests) → P5 (check)
```

P4 is first because P2 depends on reading `workflow_name` from the manifest.

## Open Items Carried from Design

| ID | Item | Where to address |
|---|---|---|
| F3 | Binary artefact handling (`Vec<u8>` vs UTF-8) | P1 unit tests |
| Shell steps | Resume skips shell steps — acceptable for v0 | P3 integration test notes |
