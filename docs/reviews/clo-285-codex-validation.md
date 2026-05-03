Reading prompt from stdin...
OpenAI Codex v0.128.0 (research preview)
--------
workdir: /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
model: gpt-5.4
provider: openai
approval: never
sandbox: workspace-write [workdir, /tmp, $TMPDIR, /Users/mk/.codex/memories]
reasoning effort: high
reasoning summaries: none
session id: 019dec51-c551-7aa0-b2db-f77761a6ff12
--------
user
# Persona: Codex pre-PR validator (loker)

You are a meticulous Rust reviewer running the final pre-PR pass on a
loker change. You are NOT a generalist code reviewer - you are the gate
that decides whether the branch is safe to push.

This persona is called from `phases/implement.md` step 5 (the codex +
gemini validation gate). Your output is parsed by the orchestrator: the
verdict line drives whether the workflow can transition to `pr`.

## Stack context

- Pure Rust workspace. Pre-merge gate: `make check`.
- Backends communicate through TensorZero. Tests for backend code use
  wiremock; gateway integration tests are gated behind
  `LOKER_TZ_INTEGRATION=1`.
- Branch convention: `feat/clo-XX-<slug>`.
- The change must satisfy the spec / plan referenced in the workflow
  YAML (`docs/status/clo-XX-workflow.yaml`).

## Pre-PR checklist

Walk through these in order. Stop at the first failure and return
`rework` unless you can identify a one-line fix.

1. **Build is clean**
   - `cargo fmt --check` passes
   - `cargo clippy --all-targets --all-features -- -D warnings` passes
   - `cargo clippy --tests` passes
   - `cargo test` passes
   - `make check` passes end-to-end
2. **Spec / plan satisfied**
   - Every AC in the spec has a matching test or verification path
   - Every sub-task in the plan corresponds to a commit (or to one of
     the staged changes)
3. **No unintended public surface**
   - New `pub` items are intentional and documented
   - No internal types leak through trait bounds
4. **Error handling**
   - All `?` paths reach a meaningful error type, not a string
   - No `.unwrap()` on user-reachable code paths
5. **Tests**
   - Happy path covered
   - Error pass-through covered (where the design specifies)
   - Edge cases enumerated in the spec are covered
   - No new `#[ignore]` tests without a tracking issue
6. **Schema / docs**
   - JSON schemas under `docs/schemas/` updated if the output shape
     changed
   - Public API doc-comments present on new traits / structs

## Output format

```markdown
# Codex pre-PR validation - CLO-XX

## Context
- Branch: <branch>
- Plan / Spec: <path>
- Design: <path>

## Checklist
- [x] cargo fmt --check
- [x] cargo clippy -D warnings
- [x] cargo test (<n> passed)
- [x] make check green
- [x] All ACs covered
- [x] No unintended public surface
- [x] Error handling
- [x] Tests
- [x] Schema / docs

## Findings
### F1 [severity] <one-line>
**Where:** <file>:<line>
**What:** <2-3 sentences>
**Suggested fix:** <concrete>

## Verdict
approve | approve_with_changes | rework

<one-paragraph rationale referencing the failing checklist items, if any>
```

Severity: `blocker`, `major`, `minor`, `nit`.

The verdict line MUST appear verbatim and must be one of the three
canonical strings - the orchestrator parses it.

## Hard rules

- The verdict is binding. If you write `approve`, you are signing off
  on the change being PR-ready.
- Never recommend bypassing pre-commit hooks (`--no-verify`) or signing
  (`--no-gpg-sign`).
- Never recommend force-pushing an existing PR branch without warning.
- Never approve while any item in the checklist is `[ ]`.

You are a senior code reviewer. Review all changes on this branch against this task's design document and implementation plan.

Inputs:
- Branch: feat/clo-285-manifest-load
- Design: docs/designs/clo-285-manifest-load.md
- Plan: docs/plans/clo-285-manifest-load.md
- Diff: git diff main...HEAD

Check for correctness, completeness, regressions, code quality, security, schema/API compatibility, and scope creep.

Output markdown with findings grouped by severity. End with:
## Verdict
approve | approve_with_changes | rework

codex
I’m validating this branch against the design, plan, and the full pre-PR gate. I’ll inspect the documented scope first, then review `git diff main...HEAD`, and run the Rust checks in the same order as the checklist.
  → Read design, plan, and workflow context for CLO-285
  • Inspect branch diff against main for scope and correctness
  • Run required validation commands through make check
  • Compare implementation and tests to acceptance criteria
  • Produce pre-PR verdict with findings
exec
/bin/zsh -lc "sed -n '1,240p' docs/designs/clo-285-manifest-load.md" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
# Design: CLO-285 — Manifest-driven artefact load with orphan-entry sweep

## 1. Problem

`CLO-283` added manifest persistence, but the read path still performs only schema checks and a marker-based orphan drop. T-031 resumability needs a stricter loader that (a) verifies each manifest entry against current on-disk artefacts, (b) reports dropped orphans separately, and (c) exposes phase progress plus writer heartbeat state. Without this, resume logic cannot reliably choose whether to skip, rerun, or block on an active writer.

## 2. Goals / Non-goals

### Goals
- Introduce a dedicated loader surface (`src/run_state/load.rs`) that returns a typed `RunState` for downstream resume paths.
- Preserve existing `src/manifest.rs` append/write semantics and reuse its existing types/helpers (`Manifest`, `ManifestEntry`, `Kind`, `dir_digest`, `sha256_hex`).
- Add typed load errors (`LoadError`) that distinguish schema mismatch, missing artefacts, corrupt artefacts, and heartbeat state (`StaleWriter` / `LiveWriter`).
- Keep orphan handling deterministic: only keep manifest entries whose sha256 appears in `markers/*.completed`.
- Keep docs updated with a resume-path hint in rustdoc.

### Non-goals
- Implementing full phase resume orchestration (`T-031`).
- Mutating `manifest.json` to delete orphan rows from disk.
- Reworking marker writing (`CLO-284`).

## 3. Architecture

### Modules

- `src/manifest.rs` (existing): owns manifest data model and persistence primitives.
- `src/run_state/load.rs` (new): owns load-time verification, heartbeat and marker interpretation, and `RunState` output.
- `src/run_state/mod.rs` (new): re-export loader types for integration tests and downstream phase modules.
- `tests/run_state_load.rs` (new): TDD contract from issue body.

### Data flow

```
runs/<id>/manifest.json  --> parse manifest + schema_version check --> parse heartbeat marker files --> parse markers --> per-entry verify --> orphan sweep
                                   |                                                             |
                                   +--------------------> RunState(entries, dropped_orphans, phase_status, heartbeat)
```

### `Load` algorithm

1. Read and parse `manifest.json` into `Manifest`.
2. Enforce manifest-level schema version and per-entry `entry.schema_version == 1`.
3. Read phase markers from `runs/<id>/markers/*.completed`, collect referenced sha256 set.
4. Split manifest entries into:
   - `entries` (sha256 present in completed set, or no completed markers exist)
   - `dropped_orphans` (sha256 not present).
5. For every surviving entry, resolve its artefact path relative to run directory and verify SHA-256.
6. Detect heartbeat (`runs/<id>/heartbeat.json`) freshness using `heartbeat_ttl_seconds`.
   - missing heartbeat file -> no warning and continue as `NoHeartbeat`.
   - stale -> `HeartbeatStatus::StaleWriter`
   - live -> `HeartbeatStatus::LiveWriter`
7. For each marker file set (`*.started`, `*.completed`, `*.failed`) compute a per-phase status map.

### Phase status precedence

If multiple markers exist, precedence is:
`Completed` > `Failed` > `Started` > `None`.

## 4. Public API surface

```rust
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("manifest schema mismatch: expected {expected}, found {found}")]
    ArtefactSchemaMismatch { expected: u32, found: u32, path: String },

    #[error("artefact missing: {path}")]
    ArtefactMissing { path: String },

    #[error("artefact corrupt: {path} (expected {expected}, found {found})")]
    ArtefactCorrupt { path: String, expected: String, found: String },

    #[error("live writer at pid={writer_pid}, host={writer_host}")]
    LiveWriter { writer_pid: i64, writer_host: String },

    #[error("stale writer: last_tick={last_tick}, ttl={ttl_seconds}s")]
    StaleWriter { last_tick: chrono::DateTime<chrono::Utc>, ttl_seconds: u64 },

    #[error("IO: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhaseStatus { Started, Completed, Failed, None }

#[derive(Debug)]
pub struct RunState {
    pub run_id: String,
    pub entries: Vec<ManifestEntry>,
    pub dropped_orphans: Vec<ManifestEntry>,
    pub phase_status: std::collections::HashMap<String, PhaseStatus>,
    pub heartbeat: Option<HeartbeatStatus>,
}

#[derive(Debug, Clone, Copy)]
pub enum HeartbeatStatus { Live(Heartbeat), Stale, Missing }

pub struct Heartbeat {
    pub writer_pid: i64,
    pub writer_host: String,
    pub tick_at: chrono::DateTime<chrono::Utc>,
}

impl RunState {
    pub fn load(
        run_dir: &std::path::Path,
        heartbeat_ttl_seconds: u64,
    ) -> Result<Self, LoadError>;
}
```

### Logging requirement

Each dropped orphan should log at `WARN` level with `phase`, `kind`, and `sha256` (use `eprintln!` with TODO comment until a logger exists).

## 5. Test plan

- Unit/integration tests in `tests/run_state_load.rs`:
  1. happy path with completed markers -> all entries retained
  2. manifest schema mismatch -> `ArtefactSchemaMismatch`
  3. changed file bytes -> `ArtefactCorrupt`
  4. deleted file -> `ArtefactMissing`
  5. orphan sweep -> dropped entries listed
  6. stale heartbeat -> `StaleWriter`
  7. fresh heartbeat -> `LiveWriter`
  8. empty manifest -> no entries, no dropped
  9. phase-status derivation from started/completed/failed markers
  10. missing `markers/` directory tolerated (all entries survive)
  11. `changes/` entry verifies via deterministic `dir_digest`

- Keep existing manifest tests unchanged; extend if needed once `run_state` API is consumed.

## 6. Migration / rollout

1. Add module `src/run_state/mod.rs` and `src/run_state/load.rs`.
2. Keep `src/manifest.rs` APIs stable.
3. Add `tests/run_state_load.rs` and validate all paths.
4. Use `#[doc = "... resume path ..."]` comment on `RunState::load`.

## 7. Open questions

- **Heartbeat missing**: should missing heartbeat be treated as `LiveWriter`-safe or neutral? This design treats it as neutral (`None`)
  and leaves resume orchestration to decide, to avoid false positives.
- **Marker conflict**: if a phase has both `started` and `completed`, use `Completed` and if both `failed` and `completed` exist, use `Completed`.
- **Directory ownership**: keep orphan logging as `eprintln!` until centralized logging exists (`CLO-029`/trace logger landings).

exec
/bin/zsh -lc 'git branch --show-current' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
feat/clo-285-manifest-load

exec
/bin/zsh -lc "sed -n '1,260p' docs/plans/clo-285-manifest-load.md" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
exec
/bin/zsh -lc "sed -n '1,260p' docs/status/clo-285-workflow.yaml" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
# Plan: CLO-285 Manifest-driven artefact load with orphan-entry sweep

## Context
- Design: docs/designs/clo-285-manifest-load.md
- Discovery: docs/discovery/clo-285-manifest-load.md
- Linear: https://linear.app/cloud-ai/issue/CLO-285/implement-manifest-driven-artefact-load-with-orphan-entry-sweep

## Sub-tasks

### ST1 Add run-state module scaffold and public API
**Files:** `src/run_state/mod.rs`, `src/run_state/load.rs`
- Create `PhaseStatus`, `HeartbeatStatus`, `RunState`, and `LoadError` types.
- Add `RunState::load(run_dir, heartbeat_ttl_seconds)` API and re-export via `src/lib.rs` if needed.

**Files:** `src/family.rs` (if new cross-module status enum references require updates)

**Acceptance:** `cargo test --test run_state_load -- --nocapture` compiles due type-level assertions in test scaffolding (after tests are added).

**Estimate:** M

### ST2 Implement typed load orchestration logic
**Files:** `src/run_state/load.rs`
- Parse `manifest.json`, enforce manifest/schema version checks.
- Collect completed-marker SHA set from `markers/*.completed` and derive phase status map from `*.started|*.completed|*.failed`.
- Split entries into `entries` and `dropped_orphans`.
- Verify each surviving entry against file bytes or `dir_digest`.
- Evaluate heartbeat freshness and produce `HeartbeatStatus`.
- Log each dropped orphan entry with phase/kind/sha256.

**Acceptance:** `cargo test --test run_state_load --run-only orphan_sweep_drops_orphans` passes.

**Estimate:** L

### ST3 Add `tests/run_state_load.rs` contract tests
**Files:** `tests/run_state_load.rs`
- Add tests for happy path, schema mismatch, corrupt entry, missing entry, orphan sweep, stale/live heartbeat, empty manifest, phase status derivation.
- Include changes-dir digest verification test.

**Acceptance:** `cargo test --test run_state_load -- --nocapture` passes.

### ST4 Wire module surface and docs
**Files:** `src/lib.rs`
- Export `run_state` module publicly for integration tests.
- Add rustdoc note on `RunState::load` with resume-path behavior.

**Acceptance:** `cargo test` compiles all integration tests referencing `run_state` and `cargo test --test run_state_load`.

### ST5 Keep manifest tests intact and check compatibility
**Files:** `tests/manifest.rs`, existing module surface
- Ensure `src/manifest.rs` APIs remain backward-compatible and continue to pass.

**Acceptance:** `cargo test --test manifest` passes.

### ST6 Full check gate
**Files:** none (repo-wide)

**Acceptance:** `make check` (fmt + clippy + test) passes.

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Risks
- `tests/run_state_load.rs` needs a stable marker schema contract from `CLO-284`; the plan assumes fields in `docs/run-state.md`.
- Heartbeat status semantics are conservative; resume orchestration must decide how to treat missing heartbeat files.

 succeeded in 0ms:
task_id: clo-285
task_title: Implement manifest-driven artefact load with orphan-entry sweep
task_type: development
classification_reason: Title starts with Implement; manifest load recovery work is mechanical
pending_human_action: null
task_profile:
  has_backend: false
  has_frontend: false
  has_data_model: false
  has_external_deps: false
  skip_probe: false
linear:
  team: Cloud-ai
  project: Loker
  status_at_start: Backlog
  blocks: []
  blocked_by: []
  priority: High
  branch_suggested: kmamemo/clo-285-implement-manifest-driven-artefact-load-with-orphan-entry
  branch_actual: feat/clo-285-manifest-load
workflow:
  current_phase: implement
  status: active
  created_at: '2026-05-02T14:19:48.376Z'
  updated_at: '2026-05-03T05:30:38.927Z'
phases:
  discovery:
    status: complete
    approved: true
    problem_framed: true
    prd_exists: true
    prd_file: docs/prds/clo-285-manifest-load.md
    discovery_report: docs/discovery/clo-285-manifest-load.md
    discovery_debt: []
    baseline_score: 4
    approaches_identified: 2
    approach_chosen: run_state-focused loader module
    prd_created: true
  design:
    status: complete
    design_doc: docs/designs/clo-285-manifest-load.md
    draft_ready: true
    discovery_context_used: true
    review_completed: true
    review_gemini: docs/reviews/clo-285-design-gemini.md
    review_synthesis: docs/reviews/clo-285-design-synthesis.md
    review_verdict: approve_with_changes
    review_skip_reason: |
      design-review.toml parse error in .lok/workflows/design-review.toml; plannotator was used instead
    applied_suggestions: []
    flagged_suggestions: []
    human_review_completed: true
    plannotator_annotations: ''
    finalized: true
  plan:
    status: complete
    approved: true
    plan_file: docs/plans/clo-285-manifest-load.md
  implement:
    status: validating
    commits:
      - 4152cbe
      - 153ed7f
      - 47387fa
  pr:
    status: pending
  complete:
    status: pending
history:
  - timestamp: '2026-05-02T14:19:48.376Z'
    action: workflow_started
    phase: init
    details: Workflow initialized for clo-285 as development
  - timestamp: '2026-05-02T14:23:09.922Z'
    action: pre_flight_checks_passed
    phase: init
    details: >-
      CLO-283 (manifest writer) is merged. CLO-284 (markers) is In Progress — will design marker-compatible structures.
      CLO-285 builds richer load: RunState, per-entry sha256 verify, heartbeat check, LoadError enum.
  - timestamp: '2026-05-02T14:24:06.050Z'
    action: init_classified
    phase: init
    details: 'Classified as development: title starts with Implement and scope is mechanical manifest-loading work.'
  - timestamp: '2026-05-02T14:24:06.051Z'
    action: project_sync_skipped
    phase: init
    details: No PROJECT.md/ROADMAP.md/DEPENDENCIES.md exist in this repo.
  - timestamp: '2026-05-02T14:24:06.051Z'
    action: phase_transition
    phase: init
    details: Transitioned from init to discovery
  - timestamp: '2026-05-02T14:24:24.616Z'
    action: branch_created
    phase: discovery
    details: Branch feat/clo-285-manifest-load checked out from current task branch context
  - timestamp: '2026-05-02T14:24:29.537Z'
    action: discovery_approved
    phase: discovery
    details: 'Approach: run_state module wrapping existing manifest types. Baseline 4/10. 2 approaches considered.'
  - timestamp: '2026-05-02T14:24:34.763Z'
    action: phase_transition
    phase: discovery
    details: Transitioned from discovery to design
  - timestamp: '2026-05-02T14:26:51.820Z'
    action: design_draft_ready
    phase: design
    details: Draft generated at docs/designs/clo-285-manifest-load.md
  - timestamp: '2026-05-02T14:28:37.002Z'
    action: design_review_complete
    phase: design
    details: Design review workflow failed due .lok/workflows/design-review.toml parse error; gemini/synthesis reviews skipped.
  - timestamp: '2026-05-02T14:28:37.003Z'
    action: design_human_review_complete
    phase: design
    details: Plannotator review approved.
  - timestamp: '2026-05-02T14:28:37.004Z'
    action: design_finalized
    phase: design
    details: Design finalized and approved for implementation planning.
  - timestamp: '2026-05-02T14:28:46.214Z'
    action: phase_transition
    phase: design
    details: Transitioned from design to plan
  - timestamp: '2026-05-02T14:28:50.420Z'
    action: plan_created
    phase: plan
    details: Plan with 6 sub-tasks at docs/plans/clo-285-manifest-load.md
  - timestamp: '2026-05-02T14:28:50.421Z'
    action: plan_approved
    phase: plan
    details: Plan approved. 6 sub-tasks queued.
  - timestamp: '2026-05-02T14:28:54.198Z'
    action: phase_transition
    phase: plan
    details: Transitioned from plan to implement
  - timestamp: '2026-05-02T14:29:56.914Z'
    action: subtask_complete
    phase: implement
    details: 'ST1-5 landed: run_state module, manifest loader tests, docs/status. Commit 4152cbe'
  - timestamp: '2026-05-02T14:30:59.454Z'
    action: subtask_complete
    phase: implement
    details: Adjusted loader/tests for clippy and failing test edge cases; commit 153ed7f
  - timestamp: '2026-05-03T05:30:32.447Z'
    action: subtask_complete
    phase: implement
    details: >-
      ST6 follow-up: fixed heartbeat API surface (RunState.heartbeat field), changes/ missing artefact returns typed
      ArtefactMissing. Commit 47387fa.
  - timestamp: '2026-05-03T05:30:38.927Z'
    action: implementation_complete
    phase: implement
    details: 'All sub-tasks landed. make check green. 3 commits: 4152cbe, 153ed7f, 47387fa.'

codex
I have the intended scope and the recorded implementation steps. Next I’m checking the actual diff and commit structure to see whether the branch matches the plan before I trust the green gate claim in the workflow file.
exec
/bin/zsh -lc 'git log --oneline --decorate --no-merges main..HEAD' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
47387fa (HEAD -> feat/clo-285-manifest-load) feat(CLO-285): expose heartbeat in RunState and fix changes/ missing artefact error
153ed7f fix(CLO-285): resolve clippy issues and stabilize tests
4152cbe feat(CLO-285): implement manifest run-state load surface

exec
/bin/zsh -lc 'git diff --name-only main...HEAD' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
docs/designs/clo-285-manifest-load.md
docs/discovery/clo-285-manifest-load.md
docs/plans/clo-285-manifest-load.md
docs/prds/clo-285-manifest-load.md
docs/status/clo-285-workflow.yaml
src/lib.rs
src/run_state/load.rs
src/run_state/mod.rs
tests/run_state_load.rs

exec
/bin/zsh -lc 'git diff --stat main...HEAD' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
 docs/designs/clo-285-manifest-load.md   | 148 ++++++++++++
 docs/discovery/clo-285-manifest-load.md |  63 +++++
 docs/plans/clo-285-manifest-load.md     |  64 +++++
 docs/prds/clo-285-manifest-load.md      |  46 ++++
 docs/status/clo-285-workflow.yaml       | 135 +++++++++++
 src/lib.rs                              |   1 +
 src/run_state/load.rs                   | 397 ++++++++++++++++++++++++++++++++
 src/run_state/mod.rs                    |   3 +
 tests/run_state_load.rs                 | 354 ++++++++++++++++++++++++++++
 9 files changed, 1211 insertions(+)

exec
/bin/zsh -lc 'git diff --unified=80 main...HEAD -- src/lib.rs src/run_state/mod.rs src/run_state/load.rs tests/run_state_load.rs tests/manifest.rs' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
diff --git a/src/lib.rs b/src/lib.rs
index 6b21301..63f1ee0 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,24 +1,25 @@
 // Most modules are private to this library and are surfaced only because
 // they sit in the dependency closure of the public ones (`strategy` →
 // `template` → `workflow` → ...). The binary (src/main.rs) re-declares
 // them via its own private `mod` tree, so this file's job is just to
 // satisfy the lib-side compile and expose the integration-test surface.
 #![allow(dead_code)]
 
 pub mod aggregator;
 pub mod backend;
 pub mod family;
 pub mod manifest;
+pub mod run_state;
 pub mod strategy;
 pub mod template;
 
 mod apply_verify;
 mod cache;
 mod config;
 mod consensus;
 mod context;
 mod git_agent;
 mod role;
 mod utils;
 mod workflow;
 mod workflows;
diff --git a/src/run_state/load.rs b/src/run_state/load.rs
new file mode 100644
index 0000000..2944ba5
--- /dev/null
+++ b/src/run_state/load.rs
@@ -0,0 +1,397 @@
+use std::collections::{HashMap, HashSet};
+use std::path::{Path, PathBuf};
+
+use chrono::{DateTime, Duration, Utc};
+use serde::Deserialize;
+
+use crate::manifest::{dir_digest, Kind, Manifest, ManifestEntry};
+
+/// Per-phase resume status inferred from marker presence.
+#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
+pub enum PhaseStatus {
+    Started,
+    Completed,
+    Failed,
+    None,
+}
+
+/// Indicates whether the run is currently being written to or stale.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub enum HeartbeatStatus {
+    /// Writer heartbeat is present and within TTL.
+    Live(Heartbeat),
+    /// Writer heartbeat is present but older than TTL.
+    Stale {
+        /// Last heartbeat tick timestamp.
+        last_tick: DateTime<Utc>,
+        /// TTL window used by the caller, in seconds.
+        ttl_seconds: u64,
+    },
+    /// No heartbeat file exists.
+    Missing,
+}
+
+/// Snapshot of `heartbeat.json` loaded from disk.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub struct Heartbeat {
+    /// OS process id that wrote the heartbeat.
+    pub writer_pid: i64,
+    /// Host name of the writer.
+    pub writer_host: String,
+    /// Timestamp of the last heartbeat tick.
+    pub tick_at: DateTime<Utc>,
+}
+
+/// Typed errors emitted while loading run state from a run directory.
+#[derive(Debug, thiserror::Error)]
+pub enum LoadError {
+    #[error("manifest schema mismatch: expected {expected}, found {found} at {path}")]
+    ArtefactSchemaMismatch {
+        expected: u32,
+        found: u32,
+        path: String,
+    },
+
+    #[error("artefact missing: {path}")]
+    ArtefactMissing { path: String },
+
+    #[error("artefact corrupt: {path} (expected {expected}, found {found})")]
+    ArtefactCorrupt {
+        path: String,
+        expected: String,
+        found: String,
+    },
+
+    #[error("stale writer heartbeat: last tick {last_tick} older than ttl {ttl_seconds}s")]
+    StaleWriter {
+        last_tick: DateTime<Utc>,
+        ttl_seconds: u64,
+    },
+
+    #[error("live writer heartbeat: pid={writer_pid}, host={writer_host}")]
+    LiveWriter {
+        writer_pid: i64,
+        writer_host: String,
+    },
+
+    #[error("io error: {0}")]
+    Io(#[from] std::io::Error),
+
+    #[error("json error: {0}")]
+    Json(#[from] serde_json::Error),
+}
+
+#[derive(Debug)]
+pub struct RunState {
+    pub run_id: String,
+    pub entries: Vec<ManifestEntry>,
+    pub dropped_orphans: Vec<ManifestEntry>,
+    pub phase_status: HashMap<String, PhaseStatus>,
+    pub heartbeat: Option<HeartbeatStatus>,
+}
+
+#[derive(Debug, Deserialize)]
+struct CompletedMarker {
+    manifest_entry_sha256: String,
+    #[serde(default)]
+    phase: Option<String>,
+}
+
+#[derive(Debug, Deserialize)]
+struct HeartbeatMarker {
+    writer_pid: i64,
+    writer_host: String,
+    tick_at: DateTime<Utc>,
+}
+
+#[derive(Debug)]
+struct MarkerScanState {
+    phase_status: HashMap<String, PhaseStatus>,
+    completed_hashes: HashSet<String>,
+    has_completed_markers: bool,
+}
+
+impl RunState {
+    /// Load and validate run state from `<run_dir>/manifest.json`.
+    ///
+    /// Resume contract:
+    /// - Load manifest and verify schema.
+    /// - Drop orphaned entries only when marker metadata exists.
+    /// - Verify kept entries' on-disk digests.
+    /// - Detect live/stale heartbeat state.
+    pub fn load(run_dir: &Path, heartbeat_ttl_seconds: u64) -> Result<Self, LoadError> {
+        let manifest = Self::load_manifest(run_dir)?;
+        let marker_scan = Self::load_markers(run_dir)?;
+
+        let (entries, dropped_orphans) = if marker_scan.has_completed_markers {
+            Self::orphan_sweep(manifest.entries, &marker_scan.completed_hashes)
+        } else {
+            (manifest.entries, Vec::new())
+        };
+
+        let phase_status = marker_scan.phase_status;
+
+        let heartbeat = Self::read_heartbeat(run_dir)?.map(|hb| {
+            let age = Utc::now().signed_duration_since(hb.tick_at);
+            if age > Duration::seconds(heartbeat_ttl_seconds as i64) {
+                HeartbeatStatus::Stale {
+                    last_tick: hb.tick_at,
+                    ttl_seconds: heartbeat_ttl_seconds,
+                }
+            } else {
+                HeartbeatStatus::Live(hb)
+            }
+        });
+
+        Self::verify_entries(run_dir, &entries)?;
+
+        Ok(Self {
+            run_id: manifest.run_id,
+            entries,
+            dropped_orphans,
+            phase_status,
+            heartbeat,
+        })
+    }
+
+    fn load_manifest(run_dir: &Path) -> Result<Manifest, LoadError> {
+        let manifest_path = manifest_path(run_dir);
+        let text = std::fs::read_to_string(&manifest_path)?;
+        let manifest: Manifest = Manifest::from_json(&text)?;
+
+        if manifest.schema_version != 1 {
+            return Err(LoadError::ArtefactSchemaMismatch {
+                expected: 1,
+                found: manifest.schema_version,
+                path: manifest_path.display().to_string(),
+            });
+        }
+
+        for entry in &manifest.entries {
+            if entry.schema_version != 1 {
+                return Err(LoadError::ArtefactSchemaMismatch {
+                    expected: 1,
+                    found: entry.schema_version,
+                    path: entry.name.clone(),
+                });
+            }
+        }
+
+        Ok(manifest)
+    }
+
+    fn load_markers(run_dir: &Path) -> Result<MarkerScanState, LoadError> {
+        let mut status = HashMap::new();
+        let mut completed = HashSet::new();
+        let mut has_completed_markers = false;
+        let markers_dir = run_dir.join("markers");
+
+        if !markers_dir.exists() {
+            return Ok(MarkerScanState {
+                phase_status: status,
+                completed_hashes: completed,
+                has_completed_markers: false,
+            });
+        }
+
+        for dir_entry in std::fs::read_dir(&markers_dir)? {
+            let dir_entry = dir_entry?;
+            let path = dir_entry.path();
+            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
+                continue;
+            };
+
+            let Some(phase) = marker_phase(file_name) else {
+                continue;
+            };
+            if file_name.ends_with(".completed") {
+                has_completed_markers = true;
+            }
+
+            if file_name.ends_with(".completed") {
+                if let Ok(text) = std::fs::read_to_string(&path) {
+                    if let Ok(marker) = serde_json::from_str::<CompletedMarker>(&text) {
+                        completed.insert(marker.manifest_entry_sha256);
+                    }
+                }
+                let _ = update_phase_status(&mut status, phase.clone(), PhaseStatus::Completed);
+                continue;
+            }
+
+            if file_name.ends_with(".failed") {
+                let _ = update_phase_status(&mut status, phase, PhaseStatus::Failed);
+                continue;
+            }
+
+            if file_name.ends_with(".started") {
+                let _ = update_phase_status(&mut status, phase, PhaseStatus::Started);
+            }
+        }
+
+        Ok(MarkerScanState {
+            phase_status: status,
+            completed_hashes: completed,
+            has_completed_markers,
+        })
+    }
+
+    fn verify_entries(run_dir: &Path, entries: &[ManifestEntry]) -> Result<(), LoadError> {
+        for entry in entries {
+            let path = run_dir.join(&entry.name);
+            match entry.kind {
+                Kind::ChangesDir => {
+                    if !path.exists() {
+                        return Err(LoadError::ArtefactMissing {
+                            path: path.display().to_string(),
+                        });
+                    }
+                    let computed = dir_digest(&path).map_err(LoadError::Io)?;
+                    if computed != entry.sha256 {
+                        return Err(LoadError::ArtefactCorrupt {
+                            path: entry.name.clone(),
+                            expected: entry.sha256.clone(),
+                            found: computed,
+                        });
+                    }
+                }
+                Kind::DesignMd
+                | Kind::ReviewMd
+                | Kind::VerifyJson
+                | Kind::PhaseResultJson
+                | Kind::PendingJson
+                | Kind::ResponseJson
+                | Kind::SummaryJson
+                | Kind::TraceJsonl => {
+                    if !path.exists() {
+                        return Err(LoadError::ArtefactMissing {
+                            path: path.display().to_string(),
+                        });
+                    }
+
+                    let bytes = std::fs::read(&path)?;
+                    let computed = crate::manifest::sha256_hex(&bytes);
+                    if computed != entry.sha256 {
+                        return Err(LoadError::ArtefactCorrupt {
+                            path: path.display().to_string(),
+                            expected: entry.sha256.clone(),
+                            found: computed,
+                        });
+                    }
+                }
+            }
+        }
+        Ok(())
+    }
+
+    fn orphan_sweep(
+        entries: Vec<ManifestEntry>,
+        completed_hashes: &HashSet<String>,
+    ) -> (Vec<ManifestEntry>, Vec<ManifestEntry>) {
+        let mut kept = Vec::new();
+        let mut dropped = Vec::new();
+
+        for entry in entries {
+            if completed_hashes.contains(&entry.sha256) {
+                kept.push(entry);
+            } else {
+                let phase = entry
+                    .phase
+                    .clone()
+                    .or_else(|| entry.name.split('/').next().map(ToString::to_string));
+                let phase = phase.unwrap_or_else(|| "unknown".to_string());
+                let kind = kind_name(&entry.kind);
+                eprintln!(
+                    "orphan manifest entry dropped: phase={phase}, kind={kind}, sha256={}",
+                    entry.sha256
+                );
+                dropped.push(entry);
+            }
+        }
+
+        (kept, dropped)
+    }
+
+    fn read_heartbeat(run_dir: &Path) -> Result<Option<Heartbeat>, LoadError> {
+        let heartbeat_path = run_dir.join("heartbeat.json");
+        if !heartbeat_path.exists() {
+            return Ok(None);
+        }
+        let text = std::fs::read_to_string(&heartbeat_path)?;
+        let heartbeat: HeartbeatMarker = serde_json::from_str(&text)?;
+        Ok(Some(Heartbeat {
+            writer_pid: heartbeat.writer_pid,
+            writer_host: heartbeat.writer_host,
+            tick_at: heartbeat.tick_at,
+        }))
+    }
+
+    #[allow(dead_code)]
+    pub fn status_from_heartbeat(heartbeat: &Heartbeat, ttl_seconds: u64) -> HeartbeatStatus {
+        let age = Utc::now().signed_duration_since(heartbeat.tick_at);
+        if age > Duration::seconds(ttl_seconds as i64) {
+            HeartbeatStatus::Stale {
+                last_tick: heartbeat.tick_at,
+                ttl_seconds,
+            }
+        } else {
+            HeartbeatStatus::Live(heartbeat.clone())
+        }
+    }
+}
+
+fn manifest_path(run_dir: &Path) -> PathBuf {
+    run_dir.join("manifest.json")
+}
+
+fn marker_phase(file_name: &str) -> Option<String> {
+    file_name
+        .strip_suffix(".completed")
+        .or_else(|| file_name.strip_suffix(".failed"))
+        .or_else(|| file_name.strip_suffix(".started"))
+        .map(ToString::to_string)
+}
+
+fn update_phase_status(
+    map: &mut HashMap<String, PhaseStatus>,
+    phase: String,
+    status: PhaseStatus,
+) -> bool {
+    let next_rank = status_rank(status);
+    match map.get(&phase).copied() {
+        Some(current) => {
+            if status_rank(current) < next_rank {
+                map.insert(phase, status);
+                true
+            } else {
+                false
+            }
+        }
+        None => {
+            map.insert(phase, status);
+            true
+        }
+    }
+}
+
+fn status_rank(status: PhaseStatus) -> u8 {
+    match status {
+        PhaseStatus::None => 0,
+        PhaseStatus::Started => 1,
+        PhaseStatus::Failed => 2,
+        PhaseStatus::Completed => 3,
+    }
+}
+
+fn kind_name(kind: &Kind) -> &'static str {
+    match kind {
+        Kind::DesignMd => "design.md",
+        Kind::ReviewMd => "review.md",
+        Kind::VerifyJson => "verify.json",
+        Kind::PhaseResultJson => "phase_result.json",
+        Kind::PendingJson => "pending.json",
+        Kind::ResponseJson => "response.json",
+        Kind::SummaryJson => "summary.json",
+        Kind::ChangesDir => "changes/",
+        Kind::TraceJsonl => "trace.jsonl",
+    }
+}
diff --git a/src/run_state/mod.rs b/src/run_state/mod.rs
new file mode 100644
index 0000000..a291c23
--- /dev/null
+++ b/src/run_state/mod.rs
@@ -0,0 +1,3 @@
+pub mod load;
+
+pub use load::{HeartbeatStatus, LoadError, PhaseStatus, RunState};
diff --git a/tests/run_state_load.rs b/tests/run_state_load.rs
new file mode 100644
index 0000000..0f0f9c2
--- /dev/null
+++ b/tests/run_state_load.rs
@@ -0,0 +1,354 @@
+use std::fs;
+use std::path::Path;
+
+use chrono::{Duration, Utc};
+use loker::manifest::{Kind, Manifest, Producer};
+use loker::run_state::{LoadError, PhaseStatus, RunState};
+
+fn marker_completed(run_dir: &Path, phase: &str, sha: &str) {
+    let markers_dir = run_dir.join("markers");
+    fs::create_dir_all(&markers_dir).unwrap();
+    let payload = serde_json::json!({
+        "phase": phase,
+        "attempt": 1,
+        "completed_at": Utc::now().to_rfc3339(),
+        "manifest_entry_sha256": sha,
+        "artefact_paths": [format!("{phase}/out")],
+    });
+    fs::write(
+        markers_dir.join(format!("{phase}.completed")),
+        payload.to_string(),
+    )
+    .unwrap();
+}
+
+fn marker_started(run_dir: &Path, phase: &str) {
+    let markers_dir = run_dir.join("markers");
+    fs::create_dir_all(&markers_dir).unwrap();
+    let payload = serde_json::json!({
+        "phase": phase,
+        "attempt": 1,
+        "started_at": Utc::now().to_rfc3339(),
+        "writer_pid": 123,
+        "writer_host": "localhost",
+        "heartbeat_ttl_seconds": 300,
+    });
+    fs::write(
+        markers_dir.join(format!("{phase}.started")),
+        payload.to_string(),
+    )
+    .unwrap();
+}
+
+fn build_entry_payload(name: &str, payload: &[u8]) -> (loker::manifest::ManifestEntry, Vec<u8>) {
+    (
+        loker::manifest::ManifestEntry::from_payload(
+            name.to_string(),
+            Kind::DesignMd,
+            1,
+            Producer::Single,
+            Some("design".to_string()),
+            Some(1),
+            payload,
+        ),
+        payload.to_vec(),
+    )
+}
+
+fn write_manifest_with_run_state(
+    tmp: &std::path::Path,
+    entries: Vec<(loker::manifest::ManifestEntry, Vec<u8>)>,
+    run_id: &str,
+) -> Manifest {
+    let mut manifest = Manifest::new(run_id);
+    for (entry, bytes) in entries {
+        let relpath = Path::new(&entry.name);
+        let abs = tmp.join(relpath);
+        if let Some(parent) = abs.parent() {
+            fs::create_dir_all(parent).unwrap();
+        }
+        fs::write(abs, &bytes).unwrap();
+        manifest.entries.push(entry);
+    }
+    let manifest_path = tmp.join("manifest.json");
+    fs::write(manifest_path, manifest.to_json().unwrap()).unwrap();
+    manifest
+}
+
+#[test]
+fn happy_path_load_returns_surviving_entries() {
+    let tmp = tempfile::tempdir().unwrap();
+    let run_id = "run-001";
+
+    let (entry1, payload1) = build_entry_payload("design/design.md", b"hello design");
+    let (entry2, payload2) = build_entry_payload("review/review.md", b"hello review");
+
+    let manifest = write_manifest_with_run_state(
+        tmp.path(),
+        vec![(entry1.clone(), payload1), (entry2.clone(), payload2)],
+        run_id,
+    );
+
+    marker_completed(tmp.path(), "design", &entry1.sha256);
+    marker_completed(tmp.path(), "review", &entry2.sha256);
+
+    let run_state = RunState::load(tmp.path(), 300).unwrap();
+    assert_eq!(run_state.run_id, run_id);
+    assert_eq!(run_state.entries.len(), 2);
+    assert_eq!(run_state.dropped_orphans.len(), 0);
+    assert!(run_state.entries.iter().any(|e| e.name == entry1.name));
+    assert!(run_state.entries.iter().any(|e| e.name == entry2.name));
+    assert_eq!(manifest.run_id, run_id);
+}
+
+#[test]
+fn schema_mismatch_returns_artefact_schema_mismatch() {
+    let tmp = tempfile::tempdir().unwrap();
+    let text = r#"{"loker.run_id":"run-002","schema_version":2,"entries":[]}"#;
+    fs::write(tmp.path().join("manifest.json"), text).unwrap();
+
+    let err = RunState::load(tmp.path(), 300).unwrap_err();
+    match err {
+        LoadError::ArtefactSchemaMismatch {
+            expected, found, ..
+        } => {
+            assert_eq!(expected, 1);
+            assert_eq!(found, 2);
+        }
+        other => panic!("unexpected error: {other}"),
+    }
+}
+
+#[test]
+fn corrupt_entry_returns_artefact_corrupt() {
+    let tmp = tempfile::tempdir().unwrap();
+    let (entry, bytes) = build_entry_payload("design/design.md", b"good bytes");
+    write_manifest_with_run_state(tmp.path(), vec![(entry.clone(), bytes)], "run-003");
+    marker_completed(tmp.path(), "design", &entry.sha256);
+
+    fs::write(tmp.path().join("design/design.md"), b"bad bytes").unwrap();
+
+    let err = RunState::load(tmp.path(), 300).unwrap_err();
+    match err {
+        LoadError::ArtefactCorrupt {
+            path,
+            expected,
+            found,
+        } => {
+            assert!(path.ends_with("design/design.md"));
+            assert_eq!(expected, entry.sha256);
+            assert_ne!(found, expected);
+        }
+        other => panic!("unexpected error: {other}"),
+    }
+}
+
+#[test]
+fn missing_entry_returns_artefact_missing() {
+    let tmp = tempfile::tempdir().unwrap();
+    let (entry, bytes) = build_entry_payload("design/design.md", b"exists first");
+    write_manifest_with_run_state(tmp.path(), vec![(entry.clone(), bytes)], "run-004");
+    marker_completed(tmp.path(), "design", &entry.sha256);
+
+    fs::remove_file(tmp.path().join("design/design.md")).unwrap();
+
+    let err = RunState::load(tmp.path(), 300).unwrap_err();
+    match err {
+        LoadError::ArtefactMissing { path } => assert!(path.ends_with("design/design.md")),
+        other => panic!("unexpected error: {other}"),
+    }
+}
+
+#[test]
+fn orphan_sweep_drops_non_completed_entries() {
+    let tmp = tempfile::tempdir().unwrap();
+    let run_id = "run-005";
+    let (entry1, payload1) = build_entry_payload("design/design.md", b"keep this");
+    let (entry2, payload2) = build_entry_payload("review/review.md", b"drop this");
+    write_manifest_with_run_state(
+        tmp.path(),
+        vec![(entry1.clone(), payload1), (entry2.clone(), payload2)],
+        run_id,
+    );
+
+    marker_completed(tmp.path(), "design", &entry1.sha256);
+
+    let run_state = RunState::load(tmp.path(), 300).unwrap();
+    assert_eq!(run_state.entries.len(), 1);
+    assert_eq!(run_state.entries[0].name, entry1.name);
+    assert_eq!(run_state.dropped_orphans.len(), 1);
+    assert_eq!(run_state.dropped_orphans[0].name, entry2.name);
+}
+
+#[test]
+fn stale_heartbeat_is_reported() {
+    let tmp = tempfile::tempdir().unwrap();
+    let (entry, payload) = build_entry_payload("design/design.md", b"heartbeat test");
+    write_manifest_with_run_state(tmp.path(), vec![(entry.clone(), payload)], "run-006");
+
+    let stale = Utc::now() - Duration::seconds(120);
+    let heartbeat = serde_json::json!({
+        "writer_pid": 42,
+        "writer_host": "test-host",
+        "tick_at": stale.to_rfc3339(),
+    });
+    fs::write(tmp.path().join("heartbeat.json"), heartbeat.to_string()).unwrap();
+
+    let run_state = RunState::load(tmp.path(), 60).unwrap();
+    match run_state.heartbeat {
+        Some(loker::run_state::HeartbeatStatus::Stale {
+            ttl_seconds,
+            last_tick,
+        }) => {
+            assert_eq!(ttl_seconds, 60);
+            assert!(last_tick <= stale);
+        }
+        other => panic!("unexpected heartbeat: {other:?}"),
+    }
+}
+
+#[test]
+fn live_heartbeat_is_reported() {
+    let tmp = tempfile::tempdir().unwrap();
+    let (entry, payload) = build_entry_payload("design/design.md", b"live heartbeat");
+    write_manifest_with_run_state(tmp.path(), vec![(entry.clone(), payload)], "run-007");
+
+    let heartbeat = serde_json::json!({
+        "writer_pid": 77,
+        "writer_host": "host-x",
+        "tick_at": Utc::now().to_rfc3339(),
+    });
+    fs::write(tmp.path().join("heartbeat.json"), heartbeat.to_string()).unwrap();
+
+    let run_state = RunState::load(tmp.path(), 300).unwrap();
+    match run_state.heartbeat {
+        Some(loker::run_state::HeartbeatStatus::Live(hb)) => {
+            assert_eq!(hb.writer_pid, 77);
+            assert_eq!(hb.writer_host, "host-x");
+        }
+        other => panic!("unexpected heartbeat: {other:?}"),
+    }
+}
+
+#[test]
+fn empty_manifest_loads_empty_runstate() {
+    let tmp = tempfile::tempdir().unwrap();
+    let manifest = Manifest::new("run-008");
+    fs::write(
+        tmp.path().join("manifest.json"),
+        manifest.to_json().unwrap(),
+    )
+    .unwrap();
+
+    let run_state = RunState::load(tmp.path(), 300).unwrap();
+    assert_eq!(run_state.entries.len(), 0);
+    assert_eq!(run_state.dropped_orphans.len(), 0);
+    assert_eq!(run_state.phase_status.len(), 0);
+}
+
+#[test]
+fn missing_markers_directory_keeps_all_entries() {
+    let tmp = tempfile::tempdir().unwrap();
+    let (entry1, payload1) = build_entry_payload("design/design.md", b"entry one");
+    let (entry2, payload2) = build_entry_payload("review/review.md", b"entry two");
+    write_manifest_with_run_state(
+        tmp.path(),
+        vec![(entry1.clone(), payload1), (entry2.clone(), payload2)],
+        "run-009",
+    );
+
+    let run_state = RunState::load(tmp.path(), 300).unwrap();
+    assert_eq!(run_state.entries.len(), 2);
+    assert_eq!(run_state.dropped_orphans.len(), 0);
+}
+
+#[test]
+fn markers_without_completed_hashes_keeps_all_entries() {
+    let tmp = tempfile::tempdir().unwrap();
+    let (entry1, payload1) = build_entry_payload("design/design.md", b"entry one");
+    let (entry2, payload2) = build_entry_payload("review/review.md", b"entry two");
+    write_manifest_with_run_state(
+        tmp.path(),
+        vec![(entry1.clone(), payload1), (entry2.clone(), payload2)],
+        "run-009b",
+    );
+
+    marker_started(tmp.path(), "design");
+    fs::write(
+        tmp.path().join("markers/design.failed"),
+        serde_json::json!({
+            "phase": "design",
+            "attempts_made": 1,
+            "failed_at": Utc::now().to_rfc3339(),
+        })
+        .to_string(),
+    )
+    .unwrap();
+
+    let run_state = RunState::load(tmp.path(), 300).unwrap();
+    assert_eq!(run_state.entries.len(), 2);
+    assert_eq!(run_state.dropped_orphans.len(), 0);
+}
+
+#[test]
+fn phase_status_is_derived_from_markers() {
+    let tmp = tempfile::tempdir().unwrap();
+    let (design_entry, design_payload) = build_entry_payload("design/design.md", b"done now");
+    write_manifest_with_run_state(
+        tmp.path(),
+        vec![(design_entry.clone(), design_payload)],
+        "run-010",
+    );
+
+    marker_completed(tmp.path(), "design", &design_entry.sha256);
+    marker_started(tmp.path(), "review");
+
+    let run_state = RunState::load(tmp.path(), 300).unwrap();
+    assert_eq!(
+        run_state.phase_status.get("design"),
+        Some(&PhaseStatus::Completed)
+    );
+    assert_eq!(
+        run_state.phase_status.get("review"),
+        Some(&PhaseStatus::Started)
+    );
+}
+
+#[test]
+fn changes_dir_entry_is_verified_with_digest() {
+    let tmp = tempfile::tempdir().unwrap();
+
+    let dir_entry_digest = {
+        let digest_root = tmp.path().join("changes");
+        fs::create_dir_all(digest_root.join("sub")).unwrap();
+        fs::write(digest_root.join("a.txt"), b"alpha").unwrap();
+        fs::write(digest_root.join("sub/b.txt"), b"beta").unwrap();
+        loker::manifest::dir_digest(&digest_root).unwrap()
+    };
+
+    let manifest_entry = loker::manifest::ManifestEntry {
+        name: "changes/".to_string(),
+        kind: Kind::ChangesDir,
+        schema_version: 1,
+        sha256: dir_entry_digest,
+        producer: Producer::Single,
+        phase: Some("design".to_string()),
+        attempt: Some(1),
+        created_at: None,
+    };
+
+    let manifest = Manifest {
+        run_id: "run-010".to_string(),
+        schema_version: 1,
+        entries: vec![manifest_entry.clone()],
+    };
+    fs::write(
+        tmp.path().join("manifest.json"),
+        manifest.to_json().unwrap(),
+    )
+    .unwrap();
+    marker_completed(tmp.path(), "design", &manifest_entry.sha256);
+
+    let run_state = RunState::load(tmp.path(), 300).unwrap();
+    assert_eq!(run_state.entries[0].kind, Kind::ChangesDir);
+}

codex
I’ve got the implementation surface. Now I’m running the actual gate commands and, in parallel, checking the existing manifest contract so I can judge whether the loader behavior is compatible with the repo’s established types rather than only with the new tests.
exec
/bin/zsh -lc "nl -ba src/run_state/load.rs | sed -n '1,260p'" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
exec
/bin/zsh -lc "sed -n '1,260p' src/manifest.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, Write as _};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::family::PhaseError;

/// Completed marker — subset of the marker file schema, just enough for orphan sweep.
#[derive(Debug, Clone, Deserialize)]
struct CompletedMarker {
    manifest_entry_sha256: String,
}

#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{0}")]
    Phase(#[from] PhaseError),
}

/// Artefact kind. Serialises to the bare strings defined in manifest.schema.json.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Kind {
    #[serde(rename = "design.md")]
    DesignMd,
    #[serde(rename = "review.md")]
    ReviewMd,
    #[serde(rename = "verify.json")]
    VerifyJson,
    #[serde(rename = "phase_result.json")]
    PhaseResultJson,
    #[serde(rename = "pending.json")]
    PendingJson,
    #[serde(rename = "response.json")]
    ResponseJson,
    #[serde(rename = "summary.json")]
    SummaryJson,
    #[serde(rename = "changes/")]
    ChangesDir,
    #[serde(rename = "trace.jsonl")]
    TraceJsonl,
}

/// Producer backend that created the artefact.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Producer {
    #[serde(rename = "single")]
    Single,
    #[serde(rename = "parallel")]
    Parallel,
    #[serde(rename = "escalating")]
    Escalating,
    #[serde(rename = "verify")]
    Verify,
    #[serde(rename = "hitl")]
    Hitl,
}

/// A single entry in the manifest — one artefact produced by one phase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManifestEntry {
    pub name: String,
    pub kind: Kind,
    pub schema_version: u32,
    pub sha256: String,
    pub producer: Producer,
    pub phase: Option<String>,
    pub attempt: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Manifest envelope — the artefact registry for a single run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    #[serde(rename = "loker.run_id")]
    pub run_id: String,
    pub schema_version: u32,
    pub entries: Vec<ManifestEntry>,
}

/// sha256 hex string of arbitrary bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(hex, "{:02x}", b);
    }
    hex
}

/// Deterministic directory digest for `kind: changes/`.
/// Walks the directory recursively, collects (relative_path, sha256_hex(content))
/// for every regular file (flattened including subdirs), sorts by path,
/// produces "<path>\t<sha256>\n" per line, then sha256_hex of the whole
/// concatenation.
pub fn dir_digest(root: &Path) -> Result<String, std::io::Error> {
    fn walk<'a>(
        root: &'a Path,
        prefix: &'a Path,
        out: &mut Vec<(String, String)>,
    ) -> Result<(), std::io::Error> {
        for entry in std::fs::read_dir(root)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_file() {
                let rel = path.strip_prefix(prefix).unwrap_or(&path);
                let content = std::fs::read(&path)?;
                let hash = sha256_hex(&content);
                out.push((rel.to_string_lossy().into_owned(), hash));
            } else if file_type.is_dir() {
                walk(&path, prefix, out)?;
            }
        }
        Ok(())
    }

    let mut entries = Vec::new();
    walk(root, root, &mut entries)?;
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (rel, hash) in entries {
        hasher.update(rel.as_bytes());
        hasher.update(b"\t");
        hasher.update(hash.as_bytes());
        hasher.update(b"\n");
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        let _ = write!(hex, "{:02x}", b);
    }
    Ok(hex)
}

/// Atomic write helper: tmp → fsync → rename → parent-fsync.
fn atomic_write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(contents)?;
    tmp.as_file().sync_all()?;
    let _final_path = tmp.persist(path)?;

    // Parent-directory fsync on Unix ensures the directory entry update
    // is durable; on Windows this is a no-op (directories can't be opened
    // as regular files).
    #[cfg(unix)]
    {
        let parent_file = File::open(parent)?;
        parent_file.sync_all()?;
    }

    Ok(())
}

impl ManifestEntry {
    /// Create a ManifestEntry from a byte payload (auto-computes sha256).
    pub fn from_payload(
        name: impl Into<String>,
        kind: Kind,
        schema_version: u32,
        producer: Producer,
        phase: Option<String>,
        attempt: Option<u32>,
        payload: &[u8],
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            schema_version,
            sha256: sha256_hex(payload),
            producer,
            phase,
            attempt,
            created_at: Some(chrono::Utc::now()),
        }
    }
}

impl Manifest {
    /// Create an empty manifest for a given run id.
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            schema_version: 1,
            entries: Vec::new(),
        }
    }

    /// Serialize to a JSON string (pretty-printed).
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// Deserialize from a JSON string.
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }

    /// Load a manifest file from disk, validate schema version, run orphan sweep.
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let content = std::fs::read_to_string(path)?;
        let mut manifest: Manifest = Self::from_json(&content)?;

        if manifest.schema_version != 1 {
            return Err(PhaseError::ArtefactSchemaMismatch {
                detail: format!(
                    "manifest schema_version is {}; only v1 is supported",
                    manifest.schema_version
                ),
            }
            .into());
        }

        // Orphan sweep: drop entries whose sha256 is not referenced by any
        // markers/<phase>.completed file under the run directory.
        // Only sweep when the markers directory exists; if it doesn't, this is
        // a normal load (not crash recovery) and all entries are valid.
        let run_dir = path.parent().unwrap_or(Path::new("."));
        let markers_dir = run_dir.join("markers");
        if markers_dir.is_dir() {
            let mut referenced = std::collections::HashSet::<String>::new();
            for entry in std::fs::read_dir(&markers_dir)? {
                let entry = entry?;
                let file_name = entry.file_name();
                let Some(name) = file_name.to_str() else {
                    continue;
                };
                if !name.ends_with(".completed") {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(entry.path()) else {
                    continue;
                };
                let Ok(marker) = serde_json::from_str::<CompletedMarker>(&text) else {
                    continue;
                };
                referenced.insert(marker.manifest_entry_sha256);
            }

            let original_len = manifest.entries.len();
            manifest.entries.retain(|e| {
                if referenced.contains(&e.sha256) {
                    true
                } else {
                    // TODO(T-029): replace eprintln with injected log sink once trace writer lands
                    eprintln!("orphan manifest entry dropped: {} ({})", e.name, e.sha256);
                    false

 succeeded in 0ms:
     1	use std::collections::{HashMap, HashSet};
     2	use std::path::{Path, PathBuf};
     3	
     4	use chrono::{DateTime, Duration, Utc};
     5	use serde::Deserialize;
     6	
     7	use crate::manifest::{dir_digest, Kind, Manifest, ManifestEntry};
     8	
     9	/// Per-phase resume status inferred from marker presence.
    10	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    11	pub enum PhaseStatus {
    12	    Started,
    13	    Completed,
    14	    Failed,
    15	    None,
    16	}
    17	
    18	/// Indicates whether the run is currently being written to or stale.
    19	#[derive(Debug, Clone, PartialEq, Eq)]
    20	pub enum HeartbeatStatus {
    21	    /// Writer heartbeat is present and within TTL.
    22	    Live(Heartbeat),
    23	    /// Writer heartbeat is present but older than TTL.
    24	    Stale {
    25	        /// Last heartbeat tick timestamp.
    26	        last_tick: DateTime<Utc>,
    27	        /// TTL window used by the caller, in seconds.
    28	        ttl_seconds: u64,
    29	    },
    30	    /// No heartbeat file exists.
    31	    Missing,
    32	}
    33	
    34	/// Snapshot of `heartbeat.json` loaded from disk.
    35	#[derive(Debug, Clone, PartialEq, Eq)]
    36	pub struct Heartbeat {
    37	    /// OS process id that wrote the heartbeat.
    38	    pub writer_pid: i64,
    39	    /// Host name of the writer.
    40	    pub writer_host: String,
    41	    /// Timestamp of the last heartbeat tick.
    42	    pub tick_at: DateTime<Utc>,
    43	}
    44	
    45	/// Typed errors emitted while loading run state from a run directory.
    46	#[derive(Debug, thiserror::Error)]
    47	pub enum LoadError {
    48	    #[error("manifest schema mismatch: expected {expected}, found {found} at {path}")]
    49	    ArtefactSchemaMismatch {
    50	        expected: u32,
    51	        found: u32,
    52	        path: String,
    53	    },
    54	
    55	    #[error("artefact missing: {path}")]
    56	    ArtefactMissing { path: String },
    57	
    58	    #[error("artefact corrupt: {path} (expected {expected}, found {found})")]
    59	    ArtefactCorrupt {
    60	        path: String,
    61	        expected: String,
    62	        found: String,
    63	    },
    64	
    65	    #[error("stale writer heartbeat: last tick {last_tick} older than ttl {ttl_seconds}s")]
    66	    StaleWriter {
    67	        last_tick: DateTime<Utc>,
    68	        ttl_seconds: u64,
    69	    },
    70	
    71	    #[error("live writer heartbeat: pid={writer_pid}, host={writer_host}")]
    72	    LiveWriter {
    73	        writer_pid: i64,
    74	        writer_host: String,
    75	    },
    76	
    77	    #[error("io error: {0}")]
    78	    Io(#[from] std::io::Error),
    79	
    80	    #[error("json error: {0}")]
    81	    Json(#[from] serde_json::Error),
    82	}
    83	
    84	#[derive(Debug)]
    85	pub struct RunState {
    86	    pub run_id: String,
    87	    pub entries: Vec<ManifestEntry>,
    88	    pub dropped_orphans: Vec<ManifestEntry>,
    89	    pub phase_status: HashMap<String, PhaseStatus>,
    90	    pub heartbeat: Option<HeartbeatStatus>,
    91	}
    92	
    93	#[derive(Debug, Deserialize)]
    94	struct CompletedMarker {
    95	    manifest_entry_sha256: String,
    96	    #[serde(default)]
    97	    phase: Option<String>,
    98	}
    99	
   100	#[derive(Debug, Deserialize)]
   101	struct HeartbeatMarker {
   102	    writer_pid: i64,
   103	    writer_host: String,
   104	    tick_at: DateTime<Utc>,
   105	}
   106	
   107	#[derive(Debug)]
   108	struct MarkerScanState {
   109	    phase_status: HashMap<String, PhaseStatus>,
   110	    completed_hashes: HashSet<String>,
   111	    has_completed_markers: bool,
   112	}
   113	
   114	impl RunState {
   115	    /// Load and validate run state from `<run_dir>/manifest.json`.
   116	    ///
   117	    /// Resume contract:
   118	    /// - Load manifest and verify schema.
   119	    /// - Drop orphaned entries only when marker metadata exists.
   120	    /// - Verify kept entries' on-disk digests.
   121	    /// - Detect live/stale heartbeat state.
   122	    pub fn load(run_dir: &Path, heartbeat_ttl_seconds: u64) -> Result<Self, LoadError> {
   123	        let manifest = Self::load_manifest(run_dir)?;
   124	        let marker_scan = Self::load_markers(run_dir)?;
   125	
   126	        let (entries, dropped_orphans) = if marker_scan.has_completed_markers {
   127	            Self::orphan_sweep(manifest.entries, &marker_scan.completed_hashes)
   128	        } else {
   129	            (manifest.entries, Vec::new())
   130	        };
   131	
   132	        let phase_status = marker_scan.phase_status;
   133	
   134	        let heartbeat = Self::read_heartbeat(run_dir)?.map(|hb| {
   135	            let age = Utc::now().signed_duration_since(hb.tick_at);
   136	            if age > Duration::seconds(heartbeat_ttl_seconds as i64) {
   137	                HeartbeatStatus::Stale {
   138	                    last_tick: hb.tick_at,
   139	                    ttl_seconds: heartbeat_ttl_seconds,
   140	                }
   141	            } else {
   142	                HeartbeatStatus::Live(hb)
   143	            }
   144	        });
   145	
   146	        Self::verify_entries(run_dir, &entries)?;
   147	
   148	        Ok(Self {
   149	            run_id: manifest.run_id,
   150	            entries,
   151	            dropped_orphans,
   152	            phase_status,
   153	            heartbeat,
   154	        })
   155	    }
   156	
   157	    fn load_manifest(run_dir: &Path) -> Result<Manifest, LoadError> {
   158	        let manifest_path = manifest_path(run_dir);
   159	        let text = std::fs::read_to_string(&manifest_path)?;
   160	        let manifest: Manifest = Manifest::from_json(&text)?;
   161	
   162	        if manifest.schema_version != 1 {
   163	            return Err(LoadError::ArtefactSchemaMismatch {
   164	                expected: 1,
   165	                found: manifest.schema_version,
   166	                path: manifest_path.display().to_string(),
   167	            });
   168	        }
   169	
   170	        for entry in &manifest.entries {
   171	            if entry.schema_version != 1 {
   172	                return Err(LoadError::ArtefactSchemaMismatch {
   173	                    expected: 1,
   174	                    found: entry.schema_version,
   175	                    path: entry.name.clone(),
   176	                });
   177	            }
   178	        }
   179	
   180	        Ok(manifest)
   181	    }
   182	
   183	    fn load_markers(run_dir: &Path) -> Result<MarkerScanState, LoadError> {
   184	        let mut status = HashMap::new();
   185	        let mut completed = HashSet::new();
   186	        let mut has_completed_markers = false;
   187	        let markers_dir = run_dir.join("markers");
   188	
   189	        if !markers_dir.exists() {
   190	            return Ok(MarkerScanState {
   191	                phase_status: status,
   192	                completed_hashes: completed,
   193	                has_completed_markers: false,
   194	            });
   195	        }
   196	
   197	        for dir_entry in std::fs::read_dir(&markers_dir)? {
   198	            let dir_entry = dir_entry?;
   199	            let path = dir_entry.path();
   200	            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
   201	                continue;
   202	            };
   203	
   204	            let Some(phase) = marker_phase(file_name) else {
   205	                continue;
   206	            };
   207	            if file_name.ends_with(".completed") {
   208	                has_completed_markers = true;
   209	            }
   210	
   211	            if file_name.ends_with(".completed") {
   212	                if let Ok(text) = std::fs::read_to_string(&path) {
   213	                    if let Ok(marker) = serde_json::from_str::<CompletedMarker>(&text) {
   214	                        completed.insert(marker.manifest_entry_sha256);
   215	                    }
   216	                }
   217	                let _ = update_phase_status(&mut status, phase.clone(), PhaseStatus::Completed);
   218	                continue;
   219	            }
   220	
   221	            if file_name.ends_with(".failed") {
   222	                let _ = update_phase_status(&mut status, phase, PhaseStatus::Failed);
   223	                continue;
   224	            }
   225	
   226	            if file_name.ends_with(".started") {
   227	                let _ = update_phase_status(&mut status, phase, PhaseStatus::Started);
   228	            }
   229	        }
   230	
   231	        Ok(MarkerScanState {
   232	            phase_status: status,
   233	            completed_hashes: completed,
   234	            has_completed_markers,
   235	        })
   236	    }
   237	
   238	    fn verify_entries(run_dir: &Path, entries: &[ManifestEntry]) -> Result<(), LoadError> {
   239	        for entry in entries {
   240	            let path = run_dir.join(&entry.name);
   241	            match entry.kind {
   242	                Kind::ChangesDir => {
   243	                    if !path.exists() {
   244	                        return Err(LoadError::ArtefactMissing {
   245	                            path: path.display().to_string(),
   246	                        });
   247	                    }
   248	                    let computed = dir_digest(&path).map_err(LoadError::Io)?;
   249	                    if computed != entry.sha256 {
   250	                        return Err(LoadError::ArtefactCorrupt {
   251	                            path: entry.name.clone(),
   252	                            expected: entry.sha256.clone(),
   253	                            found: computed,
   254	                        });
   255	                    }
   256	                }
   257	                Kind::DesignMd
   258	                | Kind::ReviewMd
   259	                | Kind::VerifyJson
   260	                | Kind::PhaseResultJson

exec
/bin/zsh -lc "nl -ba tests/run_state_load.rs | sed -n '1,260p'" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
     1	use std::fs;
     2	use std::path::Path;
     3	
     4	use chrono::{Duration, Utc};
     5	use loker::manifest::{Kind, Manifest, Producer};
     6	use loker::run_state::{LoadError, PhaseStatus, RunState};
     7	
     8	fn marker_completed(run_dir: &Path, phase: &str, sha: &str) {
     9	    let markers_dir = run_dir.join("markers");
    10	    fs::create_dir_all(&markers_dir).unwrap();
    11	    let payload = serde_json::json!({
    12	        "phase": phase,
    13	        "attempt": 1,
    14	        "completed_at": Utc::now().to_rfc3339(),
    15	        "manifest_entry_sha256": sha,
    16	        "artefact_paths": [format!("{phase}/out")],
    17	    });
    18	    fs::write(
    19	        markers_dir.join(format!("{phase}.completed")),
    20	        payload.to_string(),
    21	    )
    22	    .unwrap();
    23	}
    24	
    25	fn marker_started(run_dir: &Path, phase: &str) {
    26	    let markers_dir = run_dir.join("markers");
    27	    fs::create_dir_all(&markers_dir).unwrap();
    28	    let payload = serde_json::json!({
    29	        "phase": phase,
    30	        "attempt": 1,
    31	        "started_at": Utc::now().to_rfc3339(),
    32	        "writer_pid": 123,
    33	        "writer_host": "localhost",
    34	        "heartbeat_ttl_seconds": 300,
    35	    });
    36	    fs::write(
    37	        markers_dir.join(format!("{phase}.started")),
    38	        payload.to_string(),
    39	    )
    40	    .unwrap();
    41	}
    42	
    43	fn build_entry_payload(name: &str, payload: &[u8]) -> (loker::manifest::ManifestEntry, Vec<u8>) {
    44	    (
    45	        loker::manifest::ManifestEntry::from_payload(
    46	            name.to_string(),
    47	            Kind::DesignMd,
    48	            1,
    49	            Producer::Single,
    50	            Some("design".to_string()),
    51	            Some(1),
    52	            payload,
    53	        ),
    54	        payload.to_vec(),
    55	    )
    56	}
    57	
    58	fn write_manifest_with_run_state(
    59	    tmp: &std::path::Path,
    60	    entries: Vec<(loker::manifest::ManifestEntry, Vec<u8>)>,
    61	    run_id: &str,
    62	) -> Manifest {
    63	    let mut manifest = Manifest::new(run_id);
    64	    for (entry, bytes) in entries {
    65	        let relpath = Path::new(&entry.name);
    66	        let abs = tmp.join(relpath);
    67	        if let Some(parent) = abs.parent() {
    68	            fs::create_dir_all(parent).unwrap();
    69	        }
    70	        fs::write(abs, &bytes).unwrap();
    71	        manifest.entries.push(entry);
    72	    }
    73	    let manifest_path = tmp.join("manifest.json");
    74	    fs::write(manifest_path, manifest.to_json().unwrap()).unwrap();
    75	    manifest
    76	}
    77	
    78	#[test]
    79	fn happy_path_load_returns_surviving_entries() {
    80	    let tmp = tempfile::tempdir().unwrap();
    81	    let run_id = "run-001";
    82	
    83	    let (entry1, payload1) = build_entry_payload("design/design.md", b"hello design");
    84	    let (entry2, payload2) = build_entry_payload("review/review.md", b"hello review");
    85	
    86	    let manifest = write_manifest_with_run_state(
    87	        tmp.path(),
    88	        vec![(entry1.clone(), payload1), (entry2.clone(), payload2)],
    89	        run_id,
    90	    );
    91	
    92	    marker_completed(tmp.path(), "design", &entry1.sha256);
    93	    marker_completed(tmp.path(), "review", &entry2.sha256);
    94	
    95	    let run_state = RunState::load(tmp.path(), 300).unwrap();
    96	    assert_eq!(run_state.run_id, run_id);
    97	    assert_eq!(run_state.entries.len(), 2);
    98	    assert_eq!(run_state.dropped_orphans.len(), 0);
    99	    assert!(run_state.entries.iter().any(|e| e.name == entry1.name));
   100	    assert!(run_state.entries.iter().any(|e| e.name == entry2.name));
   101	    assert_eq!(manifest.run_id, run_id);
   102	}
   103	
   104	#[test]
   105	fn schema_mismatch_returns_artefact_schema_mismatch() {
   106	    let tmp = tempfile::tempdir().unwrap();
   107	    let text = r#"{"loker.run_id":"run-002","schema_version":2,"entries":[]}"#;
   108	    fs::write(tmp.path().join("manifest.json"), text).unwrap();
   109	
   110	    let err = RunState::load(tmp.path(), 300).unwrap_err();
   111	    match err {
   112	        LoadError::ArtefactSchemaMismatch {
   113	            expected, found, ..
   114	        } => {
   115	            assert_eq!(expected, 1);
   116	            assert_eq!(found, 2);
   117	        }
   118	        other => panic!("unexpected error: {other}"),
   119	    }
   120	}
   121	
   122	#[test]
   123	fn corrupt_entry_returns_artefact_corrupt() {
   124	    let tmp = tempfile::tempdir().unwrap();
   125	    let (entry, bytes) = build_entry_payload("design/design.md", b"good bytes");
   126	    write_manifest_with_run_state(tmp.path(), vec![(entry.clone(), bytes)], "run-003");
   127	    marker_completed(tmp.path(), "design", &entry.sha256);
   128	
   129	    fs::write(tmp.path().join("design/design.md"), b"bad bytes").unwrap();
   130	
   131	    let err = RunState::load(tmp.path(), 300).unwrap_err();
   132	    match err {
   133	        LoadError::ArtefactCorrupt {
   134	            path,
   135	            expected,
   136	            found,
   137	        } => {
   138	            assert!(path.ends_with("design/design.md"));
   139	            assert_eq!(expected, entry.sha256);
   140	            assert_ne!(found, expected);
   141	        }
   142	        other => panic!("unexpected error: {other}"),
   143	    }
   144	}
   145	
   146	#[test]
   147	fn missing_entry_returns_artefact_missing() {
   148	    let tmp = tempfile::tempdir().unwrap();
   149	    let (entry, bytes) = build_entry_payload("design/design.md", b"exists first");
   150	    write_manifest_with_run_state(tmp.path(), vec![(entry.clone(), bytes)], "run-004");
   151	    marker_completed(tmp.path(), "design", &entry.sha256);
   152	
   153	    fs::remove_file(tmp.path().join("design/design.md")).unwrap();
   154	
   155	    let err = RunState::load(tmp.path(), 300).unwrap_err();
   156	    match err {
   157	        LoadError::ArtefactMissing { path } => assert!(path.ends_with("design/design.md")),
   158	        other => panic!("unexpected error: {other}"),
   159	    }
   160	}
   161	
   162	#[test]
   163	fn orphan_sweep_drops_non_completed_entries() {
   164	    let tmp = tempfile::tempdir().unwrap();
   165	    let run_id = "run-005";
   166	    let (entry1, payload1) = build_entry_payload("design/design.md", b"keep this");
   167	    let (entry2, payload2) = build_entry_payload("review/review.md", b"drop this");
   168	    write_manifest_with_run_state(
   169	        tmp.path(),
   170	        vec![(entry1.clone(), payload1), (entry2.clone(), payload2)],
   171	        run_id,
   172	    );
   173	
   174	    marker_completed(tmp.path(), "design", &entry1.sha256);
   175	
   176	    let run_state = RunState::load(tmp.path(), 300).unwrap();
   177	    assert_eq!(run_state.entries.len(), 1);
   178	    assert_eq!(run_state.entries[0].name, entry1.name);
   179	    assert_eq!(run_state.dropped_orphans.len(), 1);
   180	    assert_eq!(run_state.dropped_orphans[0].name, entry2.name);
   181	}
   182	
   183	#[test]
   184	fn stale_heartbeat_is_reported() {
   185	    let tmp = tempfile::tempdir().unwrap();
   186	    let (entry, payload) = build_entry_payload("design/design.md", b"heartbeat test");
   187	    write_manifest_with_run_state(tmp.path(), vec![(entry.clone(), payload)], "run-006");
   188	
   189	    let stale = Utc::now() - Duration::seconds(120);
   190	    let heartbeat = serde_json::json!({
   191	        "writer_pid": 42,
   192	        "writer_host": "test-host",
   193	        "tick_at": stale.to_rfc3339(),
   194	    });
   195	    fs::write(tmp.path().join("heartbeat.json"), heartbeat.to_string()).unwrap();
   196	
   197	    let run_state = RunState::load(tmp.path(), 60).unwrap();
   198	    match run_state.heartbeat {
   199	        Some(loker::run_state::HeartbeatStatus::Stale {
   200	            ttl_seconds,
   201	            last_tick,
   202	        }) => {
   203	            assert_eq!(ttl_seconds, 60);
   204	            assert!(last_tick <= stale);
   205	        }
   206	        other => panic!("unexpected heartbeat: {other:?}"),
   207	    }
   208	}
   209	
   210	#[test]
   211	fn live_heartbeat_is_reported() {
   212	    let tmp = tempfile::tempdir().unwrap();
   213	    let (entry, payload) = build_entry_payload("design/design.md", b"live heartbeat");
   214	    write_manifest_with_run_state(tmp.path(), vec![(entry.clone(), payload)], "run-007");
   215	
   216	    let heartbeat = serde_json::json!({
   217	        "writer_pid": 77,
   218	        "writer_host": "host-x",
   219	        "tick_at": Utc::now().to_rfc3339(),
   220	    });
   221	    fs::write(tmp.path().join("heartbeat.json"), heartbeat.to_string()).unwrap();
   222	
   223	    let run_state = RunState::load(tmp.path(), 300).unwrap();
   224	    match run_state.heartbeat {
   225	        Some(loker::run_state::HeartbeatStatus::Live(hb)) => {
   226	            assert_eq!(hb.writer_pid, 77);
   227	            assert_eq!(hb.writer_host, "host-x");
   228	        }
   229	        other => panic!("unexpected heartbeat: {other:?}"),
   230	    }
   231	}
   232	
   233	#[test]
   234	fn empty_manifest_loads_empty_runstate() {
   235	    let tmp = tempfile::tempdir().unwrap();
   236	    let manifest = Manifest::new("run-008");
   237	    fs::write(
   238	        tmp.path().join("manifest.json"),
   239	        manifest.to_json().unwrap(),
   240	    )
   241	    .unwrap();
   242	
   243	    let run_state = RunState::load(tmp.path(), 300).unwrap();
   244	    assert_eq!(run_state.entries.len(), 0);
   245	    assert_eq!(run_state.dropped_orphans.len(), 0);
   246	    assert_eq!(run_state.phase_status.len(), 0);
   247	}
   248	
   249	#[test]
   250	fn missing_markers_directory_keeps_all_entries() {
   251	    let tmp = tempfile::tempdir().unwrap();
   252	    let (entry1, payload1) = build_entry_payload("design/design.md", b"entry one");
   253	    let (entry2, payload2) = build_entry_payload("review/review.md", b"entry two");
   254	    write_manifest_with_run_state(
   255	        tmp.path(),
   256	        vec![(entry1.clone(), payload1), (entry2.clone(), payload2)],
   257	        "run-009",
   258	    );
   259	
   260	    let run_state = RunState::load(tmp.path(), 300).unwrap();

exec
/bin/zsh -lc 'cargo fmt --check' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 250ms:
exec
/bin/zsh -lc "nl -ba src/run_state/load.rs | sed -n '261,420p'" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
exec
/bin/zsh -lc "nl -ba tests/run_state_load.rs | sed -n '261,420p'" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
   261	                | Kind::PendingJson
   262	                | Kind::ResponseJson
   263	                | Kind::SummaryJson
   264	                | Kind::TraceJsonl => {
   265	                    if !path.exists() {
   266	                        return Err(LoadError::ArtefactMissing {
   267	                            path: path.display().to_string(),
   268	                        });
   269	                    }
   270	
   271	                    let bytes = std::fs::read(&path)?;
   272	                    let computed = crate::manifest::sha256_hex(&bytes);
   273	                    if computed != entry.sha256 {
   274	                        return Err(LoadError::ArtefactCorrupt {
   275	                            path: path.display().to_string(),
   276	                            expected: entry.sha256.clone(),
   277	                            found: computed,
   278	                        });
   279	                    }
   280	                }
   281	            }
   282	        }
   283	        Ok(())
   284	    }
   285	
   286	    fn orphan_sweep(
   287	        entries: Vec<ManifestEntry>,
   288	        completed_hashes: &HashSet<String>,
   289	    ) -> (Vec<ManifestEntry>, Vec<ManifestEntry>) {
   290	        let mut kept = Vec::new();
   291	        let mut dropped = Vec::new();
   292	
   293	        for entry in entries {
   294	            if completed_hashes.contains(&entry.sha256) {
   295	                kept.push(entry);
   296	            } else {
   297	                let phase = entry
   298	                    .phase
   299	                    .clone()
   300	                    .or_else(|| entry.name.split('/').next().map(ToString::to_string));
   301	                let phase = phase.unwrap_or_else(|| "unknown".to_string());
   302	                let kind = kind_name(&entry.kind);
   303	                eprintln!(
   304	                    "orphan manifest entry dropped: phase={phase}, kind={kind}, sha256={}",
   305	                    entry.sha256
   306	                );
   307	                dropped.push(entry);
   308	            }
   309	        }
   310	
   311	        (kept, dropped)
   312	    }
   313	
   314	    fn read_heartbeat(run_dir: &Path) -> Result<Option<Heartbeat>, LoadError> {
   315	        let heartbeat_path = run_dir.join("heartbeat.json");
   316	        if !heartbeat_path.exists() {
   317	            return Ok(None);
   318	        }
   319	        let text = std::fs::read_to_string(&heartbeat_path)?;
   320	        let heartbeat: HeartbeatMarker = serde_json::from_str(&text)?;
   321	        Ok(Some(Heartbeat {
   322	            writer_pid: heartbeat.writer_pid,
   323	            writer_host: heartbeat.writer_host,
   324	            tick_at: heartbeat.tick_at,
   325	        }))
   326	    }
   327	
   328	    #[allow(dead_code)]
   329	    pub fn status_from_heartbeat(heartbeat: &Heartbeat, ttl_seconds: u64) -> HeartbeatStatus {
   330	        let age = Utc::now().signed_duration_since(heartbeat.tick_at);
   331	        if age > Duration::seconds(ttl_seconds as i64) {
   332	            HeartbeatStatus::Stale {
   333	                last_tick: heartbeat.tick_at,
   334	                ttl_seconds,
   335	            }
   336	        } else {
   337	            HeartbeatStatus::Live(heartbeat.clone())
   338	        }
   339	    }
   340	}
   341	
   342	fn manifest_path(run_dir: &Path) -> PathBuf {
   343	    run_dir.join("manifest.json")
   344	}
   345	
   346	fn marker_phase(file_name: &str) -> Option<String> {
   347	    file_name
   348	        .strip_suffix(".completed")
   349	        .or_else(|| file_name.strip_suffix(".failed"))
   350	        .or_else(|| file_name.strip_suffix(".started"))
   351	        .map(ToString::to_string)
   352	}
   353	
   354	fn update_phase_status(
   355	    map: &mut HashMap<String, PhaseStatus>,
   356	    phase: String,
   357	    status: PhaseStatus,
   358	) -> bool {
   359	    let next_rank = status_rank(status);
   360	    match map.get(&phase).copied() {
   361	        Some(current) => {
   362	            if status_rank(current) < next_rank {
   363	                map.insert(phase, status);
   364	                true
   365	            } else {
   366	                false
   367	            }
   368	        }
   369	        None => {
   370	            map.insert(phase, status);
   371	            true
   372	        }
   373	    }
   374	}
   375	
   376	fn status_rank(status: PhaseStatus) -> u8 {
   377	    match status {
   378	        PhaseStatus::None => 0,
   379	        PhaseStatus::Started => 1,
   380	        PhaseStatus::Failed => 2,
   381	        PhaseStatus::Completed => 3,
   382	    }
   383	}
   384	
   385	fn kind_name(kind: &Kind) -> &'static str {
   386	    match kind {
   387	        Kind::DesignMd => "design.md",
   388	        Kind::ReviewMd => "review.md",
   389	        Kind::VerifyJson => "verify.json",
   390	        Kind::PhaseResultJson => "phase_result.json",
   391	        Kind::PendingJson => "pending.json",
   392	        Kind::ResponseJson => "response.json",
   393	        Kind::SummaryJson => "summary.json",
   394	        Kind::ChangesDir => "changes/",
   395	        Kind::TraceJsonl => "trace.jsonl",
   396	    }
   397	}

 succeeded in 0ms:
   261	    assert_eq!(run_state.entries.len(), 2);
   262	    assert_eq!(run_state.dropped_orphans.len(), 0);
   263	}
   264	
   265	#[test]
   266	fn markers_without_completed_hashes_keeps_all_entries() {
   267	    let tmp = tempfile::tempdir().unwrap();
   268	    let (entry1, payload1) = build_entry_payload("design/design.md", b"entry one");
   269	    let (entry2, payload2) = build_entry_payload("review/review.md", b"entry two");
   270	    write_manifest_with_run_state(
   271	        tmp.path(),
   272	        vec![(entry1.clone(), payload1), (entry2.clone(), payload2)],
   273	        "run-009b",
   274	    );
   275	
   276	    marker_started(tmp.path(), "design");
   277	    fs::write(
   278	        tmp.path().join("markers/design.failed"),
   279	        serde_json::json!({
   280	            "phase": "design",
   281	            "attempts_made": 1,
   282	            "failed_at": Utc::now().to_rfc3339(),
   283	        })
   284	        .to_string(),
   285	    )
   286	    .unwrap();
   287	
   288	    let run_state = RunState::load(tmp.path(), 300).unwrap();
   289	    assert_eq!(run_state.entries.len(), 2);
   290	    assert_eq!(run_state.dropped_orphans.len(), 0);
   291	}
   292	
   293	#[test]
   294	fn phase_status_is_derived_from_markers() {
   295	    let tmp = tempfile::tempdir().unwrap();
   296	    let (design_entry, design_payload) = build_entry_payload("design/design.md", b"done now");
   297	    write_manifest_with_run_state(
   298	        tmp.path(),
   299	        vec![(design_entry.clone(), design_payload)],
   300	        "run-010",
   301	    );
   302	
   303	    marker_completed(tmp.path(), "design", &design_entry.sha256);
   304	    marker_started(tmp.path(), "review");
   305	
   306	    let run_state = RunState::load(tmp.path(), 300).unwrap();
   307	    assert_eq!(
   308	        run_state.phase_status.get("design"),
   309	        Some(&PhaseStatus::Completed)
   310	    );
   311	    assert_eq!(
   312	        run_state.phase_status.get("review"),
   313	        Some(&PhaseStatus::Started)
   314	    );
   315	}
   316	
   317	#[test]
   318	fn changes_dir_entry_is_verified_with_digest() {
   319	    let tmp = tempfile::tempdir().unwrap();
   320	
   321	    let dir_entry_digest = {
   322	        let digest_root = tmp.path().join("changes");
   323	        fs::create_dir_all(digest_root.join("sub")).unwrap();
   324	        fs::write(digest_root.join("a.txt"), b"alpha").unwrap();
   325	        fs::write(digest_root.join("sub/b.txt"), b"beta").unwrap();
   326	        loker::manifest::dir_digest(&digest_root).unwrap()
   327	    };
   328	
   329	    let manifest_entry = loker::manifest::ManifestEntry {
   330	        name: "changes/".to_string(),
   331	        kind: Kind::ChangesDir,
   332	        schema_version: 1,
   333	        sha256: dir_entry_digest,
   334	        producer: Producer::Single,
   335	        phase: Some("design".to_string()),
   336	        attempt: Some(1),
   337	        created_at: None,
   338	    };
   339	
   340	    let manifest = Manifest {
   341	        run_id: "run-010".to_string(),
   342	        schema_version: 1,
   343	        entries: vec![manifest_entry.clone()],
   344	    };
   345	    fs::write(
   346	        tmp.path().join("manifest.json"),
   347	        manifest.to_json().unwrap(),
   348	    )
   349	    .unwrap();
   350	    marker_completed(tmp.path(), "design", &manifest_entry.sha256);
   351	
   352	    let run_state = RunState::load(tmp.path(), 300).unwrap();
   353	    assert_eq!(run_state.entries[0].kind, Kind::ChangesDir);
   354	}

exec
/bin/zsh -lc "sed -n '260,420p' src/manifest.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
                    false
                }
            });
            let dropped = original_len - manifest.entries.len();
            if dropped > 0 {
                // TODO(T-029): replace eprintln with injected log sink
                eprintln!(
                    "orphan sweep dropped {} entries from manifest {}",
                    dropped, manifest.run_id
                );
            }
        }

        // Validate per-entry schema_version
        for entry in &manifest.entries {
            if entry.schema_version != 1 {
                return Err(PhaseError::ArtefactSchemaMismatch {
                    detail: format!(
                        "entry '{}' has schema_version {}; only v1 is supported",
                        entry.name, entry.schema_version
                    ),
                }
                .into());
            }
        }

        Ok(manifest)
    }

    /// Append an entry and atomically rewrite the manifest file on disk.
    pub fn append(&mut self, entry: ManifestEntry, path: &Path) -> Result<(), ManifestError> {
        self.entries.push(entry);
        let json = self.to_json();
        match json {
            Ok(json_str) => match atomic_write(path, json_str.as_bytes()) {
                Ok(()) => Ok(()),
                Err(e) => {
                    self.entries.pop(); // rollback on write failure
                    Err(e.into())
                }
            },
            Err(e) => {
                self.entries.pop(); // rollback on serialization failure
                Err(e.into())
            }
        }
    }

    /// Look up the sha256 of an entry by its name.  O(N) — fine for v0 sizes.
    pub fn sha256_for(&self, name: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.sha256.as_str())
    }

    /// Content-addressed verification: does the payload match the recorded sha256?
    pub fn verify(&self, name: &str, payload: &[u8]) -> Result<(), PhaseError> {
        let recorded = self
            .sha256_for(name)
            .ok_or_else(|| PhaseError::ArtefactSchemaMismatch {
                detail: format!("entry '{}' not found in manifest", name),
            })?;
        let computed = sha256_hex(payload);
        if recorded != computed {
            return Err(PhaseError::ArtefactSchemaMismatch {
                detail: format!(
                    "sha256 mismatch for '{}': recorded {} vs computed {}",
                    name, recorded, computed
                ),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_manifest_roundtrips() {
        let manifest = Manifest::new("run-001");
        let json = manifest.to_json().unwrap();
        let loaded: Manifest = Manifest::from_json(&json).unwrap();
        assert_eq!(manifest.run_id, loaded.run_id);
        assert_eq!(manifest.schema_version, loaded.schema_version);
        assert_eq!(manifest.entries, loaded.entries);
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        let data = b"hello world";
        let got = sha256_hex(data);
        let expected = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9";
        assert_eq!(got, expected);
    }

    #[test]
    fn atomic_write_and_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("manifest.json");
        let contents = b"{\"loker.run_id\":\"r1\",\"schema_version\":1,\"entries\":[]}";
        atomic_write(&path, contents).unwrap();
        let read = std::fs::read_to_string(&path).unwrap();
        assert_eq!(read.as_bytes(), contents);
    }
}

codex
The loader code is narrow enough now that the main risk is behavioral mismatch rather than style. I’m checking the product-facing acceptance criteria next, because there are already a couple of places where the branch diverges from the literal design text and I need to separate harmless drift from real scope failure.
exec
/bin/zsh -lc "sed -n '1,240p' docs/prds/clo-285-manifest-load.md" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
# PRD: CLO-285 — Manifest-driven artefact load with orphan-entry sweep

| Field | Value |
|-------|-------|
| Author | pi |
| Status | Draft |
| Created | 2026-05-02 |
| Task | CLO-285 |
| Depends on | CLO-283 (manifest writer), CLO-284 (phase markers, heartbeat)
| Blocks | T-031 (resumability)

## 1. Goal

Implement a typed resume-ready loader for `runs/<id>/manifest.json` that validates artefacts, drops orphan manifest entries, derives phase status from marker files, and reports writer heartbeat state so restart logic can decide whether rerun is safe.

## 2. Scope

### In scope
- Parse `runs/<id>/manifest.json` and enforce manifest schema version compatibility.
- Return a public `RunState` object containing:
  - `run_id`
  - `entries: Vec<ManifestEntry>` (post-orphan sweep)
  - `dropped_orphans: Vec<ManifestEntry>`
  - `phase_status: HashMap<String, PhaseStatus>`
- Detect orphan entries by comparing manifest hashes to `runs/<id>/markers/*.completed` and split entries into surviving vs dropped.
- Verify each surviving entry path against stored `sha256`:
  - For file kinds: hash file bytes.
  - For `kind: changes/`: hash directory digest via deterministic `dir_digest` behavior.
- Return typed load errors for schema mismatch, missing files, and corrupt files.
- Add stale/writer heartbeat detection from `runs/<id>/heartbeat.json` with TTL check.
- Derive phase status (`Started | Completed | Failed | None`) from marker files (`<phase>.started`, `<phase>.completed`, `<phase>.failed`).
- Document how this loader is used by resume path in rustdoc.

### Out of scope
- Full resume orchestration and phase rerun logic.
- Mutating `manifest.json` to remove orphans on disk.
- Attempt-directory walk/cleanup.

## 3. Acceptance Criteria

1. `LoadError` enum exists with typed variants covering at least schema mismatch, corrupted artefact bytes, missing artefact, stale writer, and live writer.
2. `RunState` is returned from a typed loader method and includes both surviving entries and dropped-orphan list.
3. Loader verifies entries against actual artefacts and fails with typed error variants for missing/corrupt values.
4. Orphan sweep is deterministic and logs each dropped entry with phase/kind/sha256.
5. `cargo test` + `make check` pass on `tests/run_state_load.rs` and existing manifest tests.
6. Loader phase-status derives deterministically from marker files.

exec
/bin/zsh -lc "sed -n '1,280p' docs/discovery/clo-285-manifest-load.md" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
# Discovery Report — CLO-285: Manifest-driven artefact load with orphan-entry sweep

## Step 1 — Problem Framing

### Who is affected
The phase runner and resume logic in M5+ cannot trust manifest output yet because the loader path currently has no typed per-entry validation or run-level state summary. Callers need a trustworthy, deterministic `RunState` before deciding which phases can be skipped after a crash.

### Current behaviour vs desired behaviour
Current code in `src/manifest.rs` can parse and write a manifest, and performs orphan filtering, but it does not verify referenced artefacts against their manifest digests, does not report stale/live heartbeat state, and does not expose per-phase completion/started/failed status. We need a loader that returns `RunState` with surviving entries, dropped orphans, and typed status while distinguishing missing/corrupt artefacts.

### Why now
`CLO-283` is already merged and the next critical-path dependency (`T-031` resumability) needs a canonical load contract to safely restart phases. The issue `CLO-284` is in progress and provides marker/heartbeat producers, so implementing the downstream reader contract now reduces integration risk for resume path tests.

## Step 2 — Existing code

### What exists
- `src/manifest.rs` contains schema types (`Manifest`, `ManifestEntry`, `Kind`, `Producer`), atomic rewrite append, sha helpers, and marker-based orphan filtering.
- `src/family.rs` already has `PhaseError::ArtefactSchemaMismatch`.
- `tests/manifest.rs` covers loader/orphan/schemas and atomic rewrite behavior.
- `docs/schemas/manifest.schema.json` defines the on-disk manifest contract.
- `docs/run-state.md` defines marker names, heartbeat behavior, and stale-run semantics.

### What is missing for this task
- Typed `LoadError` variants for file load failures (`ArtefactSchemaMismatch`, `ArtefactCorrupt`, `ArtefactMissing`, `StaleWriter`, `LiveWriter`).
- Per-entry hash verification against actual artefacts (or directory digest for `changes/`).
- `RunState` return type with phase status derivation.
- Heartbeat freshness check and stale/live classification.

### Baseline score
**4 / 10.** Core manifest types and persistence are present, but the read path is insufficient for resumability safety and typed API needs a new surface.

## Step 3 — Approaches

### Approach A — Extend `src/manifest.rs` loader into a new typed load API
- **Summary**: Keep existing `Manifest` types and add `RunState`, `LoadError`, heartbeat + phase-status helpers in `manifest.rs` with a new `Manifest::load_state(run_dir)` API.
- **Pros**: Minimal churn; reuses existing `Kind`, `Producer`, `dir_digest`, and atomic helpers; fewer new module boundaries.
- **Cons**: Ties load responsibilities to manifest module (already has manifest-specific naming). 
- **Effort**: M
- **Risk**: Low

### Approach B — Add `src/run_state/load.rs` as a separate loader module
- **Summary**: Keep `manifest.rs` focused on write/index semantics and implement a separate `run_state` module that owns `RunState`, `LoadError`, marker scan, heartbeat check, and artefact verification.
- **Pros**: Clear separation between manifest persistence and resume-oriented read semantics; easier future extension to resume walk.
- **Cons**: More file/module wiring and duplicated imports.
- **Effort**: M
- **Risk**: Low-medium

## Step 4 — Choice

**Chosen: Approach B — add `src/run_state/load.rs` with a focused read API.**

It keeps `manifest.rs` aligned with its existing manifest-domain responsibilities while introducing a dedicated loader surface for resume consumers. This makes the eventual `T-031` integration cleaner and keeps responsibilities separated.

## Step 5 — Discovery debt

- `CLO-284` is in progress and may slightly alter marker JSON shape. The design above assumes the final marker format in `docs/run-state.md`.

## References

- `docs/run-state.md`
- `docs/schemas/manifest.schema.json`
- `docs/designs/CLO-283-manifest.md`
- `docs/plans/001-implementation-roadmap.md` (T-026)

exec
/bin/zsh -lc 'rg -n "AC|Acceptance|acceptance|criteria|heartbeat|orphan|schema" docs/prds/clo-285-manifest-load.md docs/discovery/clo-285-manifest-load.md docs/designs/clo-285-manifest-load.md docs/plans/clo-285-manifest-load.md' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
docs/plans/clo-285-manifest-load.md:1:# Plan: CLO-285 Manifest-driven artefact load with orphan-entry sweep
docs/plans/clo-285-manifest-load.md:6:- Linear: https://linear.app/cloud-ai/issue/CLO-285/implement-manifest-driven-artefact-load-with-orphan-entry-sweep
docs/plans/clo-285-manifest-load.md:13:- Add `RunState::load(run_dir, heartbeat_ttl_seconds)` API and re-export via `src/lib.rs` if needed.
docs/plans/clo-285-manifest-load.md:17:**Acceptance:** `cargo test --test run_state_load -- --nocapture` compiles due type-level assertions in test scaffolding (after tests are added).
docs/plans/clo-285-manifest-load.md:23:- Parse `manifest.json`, enforce manifest/schema version checks.
docs/plans/clo-285-manifest-load.md:25:- Split entries into `entries` and `dropped_orphans`.
docs/plans/clo-285-manifest-load.md:27:- Evaluate heartbeat freshness and produce `HeartbeatStatus`.
docs/plans/clo-285-manifest-load.md:28:- Log each dropped orphan entry with phase/kind/sha256.
docs/plans/clo-285-manifest-load.md:30:**Acceptance:** `cargo test --test run_state_load --run-only orphan_sweep_drops_orphans` passes.
docs/plans/clo-285-manifest-load.md:36:- Add tests for happy path, schema mismatch, corrupt entry, missing entry, orphan sweep, stale/live heartbeat, empty manifest, phase status derivation.
docs/plans/clo-285-manifest-load.md:39:**Acceptance:** `cargo test --test run_state_load -- --nocapture` passes.
docs/plans/clo-285-manifest-load.md:46:**Acceptance:** `cargo test` compiles all integration tests referencing `run_state` and `cargo test --test run_state_load`.
docs/plans/clo-285-manifest-load.md:52:**Acceptance:** `cargo test --test manifest` passes.
docs/plans/clo-285-manifest-load.md:57:**Acceptance:** `make check` (fmt + clippy + test) passes.
docs/plans/clo-285-manifest-load.md:63:- `tests/run_state_load.rs` needs a stable marker schema contract from `CLO-284`; the plan assumes fields in `docs/run-state.md`.
docs/plans/clo-285-manifest-load.md:64:- Heartbeat status semantics are conservative; resume orchestration must decide how to treat missing heartbeat files.
docs/prds/clo-285-manifest-load.md:1:# PRD: CLO-285 — Manifest-driven artefact load with orphan-entry sweep
docs/prds/clo-285-manifest-load.md:9:| Depends on | CLO-283 (manifest writer), CLO-284 (phase markers, heartbeat)
docs/prds/clo-285-manifest-load.md:14:Implement a typed resume-ready loader for `runs/<id>/manifest.json` that validates artefacts, drops orphan manifest entries, derives phase status from marker files, and reports writer heartbeat state so restart logic can decide whether rerun is safe.
docs/prds/clo-285-manifest-load.md:19:- Parse `runs/<id>/manifest.json` and enforce manifest schema version compatibility.
docs/prds/clo-285-manifest-load.md:22:  - `entries: Vec<ManifestEntry>` (post-orphan sweep)
docs/prds/clo-285-manifest-load.md:23:  - `dropped_orphans: Vec<ManifestEntry>`
docs/prds/clo-285-manifest-load.md:25:- Detect orphan entries by comparing manifest hashes to `runs/<id>/markers/*.completed` and split entries into surviving vs dropped.
docs/prds/clo-285-manifest-load.md:29:- Return typed load errors for schema mismatch, missing files, and corrupt files.
docs/prds/clo-285-manifest-load.md:30:- Add stale/writer heartbeat detection from `runs/<id>/heartbeat.json` with TTL check.
docs/prds/clo-285-manifest-load.md:36:- Mutating `manifest.json` to remove orphans on disk.
docs/prds/clo-285-manifest-load.md:39:## 3. Acceptance Criteria
docs/prds/clo-285-manifest-load.md:41:1. `LoadError` enum exists with typed variants covering at least schema mismatch, corrupted artefact bytes, missing artefact, stale writer, and live writer.
docs/prds/clo-285-manifest-load.md:42:2. `RunState` is returned from a typed loader method and includes both surviving entries and dropped-orphan list.
docs/discovery/clo-285-manifest-load.md:1:# Discovery Report — CLO-285: Manifest-driven artefact load with orphan-entry sweep
docs/discovery/clo-285-manifest-load.md:9:Current code in `src/manifest.rs` can parse and write a manifest, and performs orphan filtering, but it does not verify referenced artefacts against their manifest digests, does not report stale/live heartbeat state, and does not expose per-phase completion/started/failed status. We need a loader that returns `RunState` with surviving entries, dropped orphans, and typed status while distinguishing missing/corrupt artefacts.
docs/discovery/clo-285-manifest-load.md:12:`CLO-283` is already merged and the next critical-path dependency (`T-031` resumability) needs a canonical load contract to safely restart phases. The issue `CLO-284` is in progress and provides marker/heartbeat producers, so implementing the downstream reader contract now reduces integration risk for resume path tests.
docs/discovery/clo-285-manifest-load.md:17:- `src/manifest.rs` contains schema types (`Manifest`, `ManifestEntry`, `Kind`, `Producer`), atomic rewrite append, sha helpers, and marker-based orphan filtering.
docs/discovery/clo-285-manifest-load.md:19:- `tests/manifest.rs` covers loader/orphan/schemas and atomic rewrite behavior.
docs/discovery/clo-285-manifest-load.md:20:- `docs/schemas/manifest.schema.json` defines the on-disk manifest contract.
docs/discovery/clo-285-manifest-load.md:21:- `docs/run-state.md` defines marker names, heartbeat behavior, and stale-run semantics.
docs/discovery/clo-285-manifest-load.md:35:- **Summary**: Keep existing `Manifest` types and add `RunState`, `LoadError`, heartbeat + phase-status helpers in `manifest.rs` with a new `Manifest::load_state(run_dir)` API.
docs/discovery/clo-285-manifest-load.md:42:- **Summary**: Keep `manifest.rs` focused on write/index semantics and implement a separate `run_state` module that owns `RunState`, `LoadError`, marker scan, heartbeat check, and artefact verification.
docs/discovery/clo-285-manifest-load.md:61:- `docs/schemas/manifest.schema.json`
docs/designs/clo-285-manifest-load.md:1:# Design: CLO-285 — Manifest-driven artefact load with orphan-entry sweep
docs/designs/clo-285-manifest-load.md:5:`CLO-283` added manifest persistence, but the read path still performs only schema checks and a marker-based orphan drop. T-031 resumability needs a stricter loader that (a) verifies each manifest entry against current on-disk artefacts, (b) reports dropped orphans separately, and (c) exposes phase progress plus writer heartbeat state. Without this, resume logic cannot reliably choose whether to skip, rerun, or block on an active writer.
docs/designs/clo-285-manifest-load.md:12:- Add typed load errors (`LoadError`) that distinguish schema mismatch, missing artefacts, corrupt artefacts, and heartbeat state (`StaleWriter` / `LiveWriter`).
docs/designs/clo-285-manifest-load.md:13:- Keep orphan handling deterministic: only keep manifest entries whose sha256 appears in `markers/*.completed`.
docs/designs/clo-285-manifest-load.md:18:- Mutating `manifest.json` to delete orphan rows from disk.
docs/designs/clo-285-manifest-load.md:26:- `src/run_state/load.rs` (new): owns load-time verification, heartbeat and marker interpretation, and `RunState` output.
docs/designs/clo-285-manifest-load.md:33:runs/<id>/manifest.json  --> parse manifest + schema_version check --> parse heartbeat marker files --> parse markers --> per-entry verify --> orphan sweep
docs/designs/clo-285-manifest-load.md:35:                                   +--------------------> RunState(entries, dropped_orphans, phase_status, heartbeat)
docs/designs/clo-285-manifest-load.md:41:2. Enforce manifest-level schema version and per-entry `entry.schema_version == 1`.
docs/designs/clo-285-manifest-load.md:45:   - `dropped_orphans` (sha256 not present).
docs/designs/clo-285-manifest-load.md:47:6. Detect heartbeat (`runs/<id>/heartbeat.json`) freshness using `heartbeat_ttl_seconds`.
docs/designs/clo-285-manifest-load.md:48:   - missing heartbeat file -> no warning and continue as `NoHeartbeat`.
docs/designs/clo-285-manifest-load.md:64:    #[error("manifest schema mismatch: expected {expected}, found {found}")]
docs/designs/clo-285-manifest-load.md:93:    pub dropped_orphans: Vec<ManifestEntry>,
docs/designs/clo-285-manifest-load.md:95:    pub heartbeat: Option<HeartbeatStatus>,
docs/designs/clo-285-manifest-load.md:110:        heartbeat_ttl_seconds: u64,
docs/designs/clo-285-manifest-load.md:117:Each dropped orphan should log at `WARN` level with `phase`, `kind`, and `sha256` (use `eprintln!` with TODO comment until a logger exists).
docs/designs/clo-285-manifest-load.md:123:  2. manifest schema mismatch -> `ArtefactSchemaMismatch`
docs/designs/clo-285-manifest-load.md:126:  5. orphan sweep -> dropped entries listed
docs/designs/clo-285-manifest-load.md:127:  6. stale heartbeat -> `StaleWriter`
docs/designs/clo-285-manifest-load.md:128:  7. fresh heartbeat -> `LiveWriter`
docs/designs/clo-285-manifest-load.md:145:- **Heartbeat missing**: should missing heartbeat be treated as `LiveWriter`-safe or neutral? This design treats it as neutral (`None`)
docs/designs/clo-285-manifest-load.md:148:- **Directory ownership**: keep orphan logging as `eprintln!` until centralized logging exists (`CLO-029`/trace logger landings).

codex
I’ve confirmed the acceptance criteria wording. The next pass is mechanical: I’m running clippy, tests, and `make check` directly so the verdict is based on current branch state, not the workflow YAML’s cached note.
exec
/bin/zsh -lc 'cargo clippy --all-targets --all-features -- -D warnings' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 3466ms:
    Checking loker v20260427.0.0 (/Users/mk/Code/orchestrator/loker--feat-clo-285-manifest)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.43s

codex
Clippy is in flight. While that runs, the review concern that stands out so far is behavioral: the implementation exposes heartbeat status on `RunState`, but it never emits the `LoadError::{StaleWriter,LiveWriter}` variants that the spec explicitly requires. I’m waiting for the gate results before I decide whether that is just design drift or a release blocker.
exec
/bin/zsh -lc 'cargo clippy --tests -- -D warnings' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 2964ms:
    Checking loker v20260427.0.0 (/Users/mk/Code/orchestrator/loker--feat-clo-285-manifest)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.93s

exec
/bin/zsh -lc 'cargo test' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 exited 101 in 384ms:
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.13s
     Running unittests src/lib.rs (target/debug/deps/loker-e0bbd5653d27b3fc)

running 656 tests
test aggregator::concat::tests::concat_kind_maps_to_strategy_label ... ok
test aggregator::concat::tests::llm_judge_kind_maps_to_strategy_label ... ok
test aggregator::concat::tests::vote_kind_maps_to_strategy_label ... ok
test aggregator::llm_judge::tests::llm_judge_family_overlap_opt_out_warns ... ok
test aggregator::llm_judge::tests::llm_judge_family_diverse_ok ... ok
test aggregator::llm_judge::tests::llm_judge_family_overlap_blocks ... ok
test aggregator::llm_judge::tests::llm_judge_parse_markdown_fenced_ballot ... ok
test aggregator::llm_judge::tests::llm_judge_parse_malformed_json ... ok
test aggregator::llm_judge::tests::llm_judge_parse_missing_chosen_index ... ok
test aggregator::concat::tests::concat_empty_input_returns_sentinel ... ok
test aggregator::concat::tests::concat_preserves_unknown_placeholders ... ok
test aggregator::concat::tests::concat_renders_success_sections_in_input_order ... ok
test aggregator::concat::tests::concat_whitespace_only_success_output_keeps_newline_invariants ... ok
test aggregator::concat::tests::concat_does_not_reexpand_placeholders_inside_metadata ... ok
test aggregator::concat::tests::concat_preserves_braced_unknown_expressions_containing_known_tokens ... ok
test aggregator::llm_judge::tests::llm_judge_parse_missing_reason ... ok
test aggregator::llm_judge::tests::llm_judge_parse_negative_chosen_index ... ok
test aggregator::concat::tests::concat_normalizes_crlf_failure_reason ... ok
test aggregator::concat::tests::concat_counts_success_and_failure ... ok
test aggregator::concat::tests::concat_escapes_multiline_failure_reason ... ok
test aggregator::llm_judge::tests::llm_judge_parse_out_of_bounds_index ... ok
test aggregator::llm_judge::tests::llm_judge_parse_out_of_bounds_index_clamped ... ok
test aggregator::llm_judge::tests::llm_judge_parse_valid_ballot ... ok
test aggregator::llm_judge::tests::llm_judge_parse_within_bounds_index_clamped ... ok
test aggregator::llm_judge::tests::llm_judge_parse_zero_candidates_index ... ok
test aggregator::tests::empty_text ... ok
test aggregator::tests::extra_keys_ok ... ok
test aggregator::tests::markdown_fenced_fail ... ok
test aggregator::tests::markdown_fenced_json ... ok
test aggregator::llm_judge::tests::llm_judge_prompt_includes_phase_name ... ok
test aggregator::llm_judge::tests::llm_judge_prompt_renders_candidates ... ok
test aggregator::tests::missing_pass ... ok
test aggregator::tests::pass_false ... ok
test aggregator::tests::pass_true ... ok
test aggregator::tests::wrong_pass_type ... ok
test aggregator::vote::tests::all_abstain ... ok
test aggregator::vote::tests::abstain_backend_error ... ok
test aggregator::vote::tests::closest_family_multiple_buckets_match ... ok
test aggregator::vote::tests::closest_family_multiple_matching_buckets ... ok
test aggregator::vote::tests::closest_family_no_match_fallback ... ok
test aggregator::vote::tests::empty_ballot_counts_as_abstain ... ok
test aggregator::vote::tests::empty_input ... ok
test aggregator::vote::tests::free_text_clear_winner ... ok
test aggregator::vote::tests::free_text_tie_closest_family ... ok
test aggregator::vote::tests::free_text_tie_first_responder ... ok
test aggregator::vote::tests::normalise_ballot_basic ... ok
test aggregator::vote::tests::normalise_case ... ok
test aggregator::vote::tests::normalise_whitespace ... ok
test aggregator::vote::tests::quorum_lost ... ok
test aggregator::vote::tests::free_text_tie_random_deterministic ... ok
test aggregator::vote::tests::sanitize_comment_in_metadata ... ok
test aggregator::vote::tests::vote_counts_sorted_descending ... ok
test aggregator::vote::tests::whitespace_only_ballot_counts_as_abstain ... ok
test apply_verify::diff_applier::tests::test_apply_empty_file_path_is_invalid_edit ... ok
test apply_verify::diff_applier::tests::test_apply_empty_edits ... ok
test apply_verify::edit_parser::tests::test_crlf_normalization ... ok
test apply_verify::diff_applier::tests::test_apply_rejects_absolute_path ... ok
test apply_verify::edit_parser::tests::test_detect_diff ... ok
test apply_verify::edit_parser::tests::test_detect_full_file ... ok
test apply_verify::diff_applier::tests::test_apply_file_not_found ... ok
test apply_verify::edit_parser::tests::test_detect_with_lang_hint_diff ... ok
test apply_verify::edit_parser::tests::test_detect_json_object ... ok
test apply_verify::diff_applier::tests::test_apply_rejects_path_traversal ... ok
test apply_verify::edit_parser::tests::test_detect_json_array ... ok
test apply_verify::edit_parser::tests::test_detect_with_lang_hint_json ... ok
test apply_verify::edit_parser::tests::test_diff_no_hunks ... ok
test apply_verify::edit_parser::tests::test_diff_context_lines ... ok
test apply_verify::edit_parser::tests::test_diff_multi_file ... ok
test apply_verify::diff_applier::tests::test_apply_old_text_not_found ... ok
test apply_verify::edit_parser::tests::test_diff_no_newline_marker ... ok
test apply_verify::diff_applier::tests::test_apply_ambiguous_match ... ok
test apply_verify::edit_parser::tests::test_diff_single_file ... ok
test apply_verify::edit_parser::tests::test_empty_input ... ok
test apply_verify::edit_parser::tests::test_diff_strips_ab_prefix ... ok
test apply_verify::edit_parser::tests::test_full_file_empty_path ... ok
test apply_verify::edit_parser::tests::test_full_file ... ok
test apply_verify::edit_parser::tests::test_full_file_no_path ... ok
test apply_verify::edit_parser::tests::test_full_file_with_dash_header ... ok
test apply_verify::diff_applier::tests::test_apply_unified_diff_multi_hunk_fails ... ok
test apply_verify::diff_applier::tests::test_apply_partial_failure ... ok
test apply_verify::edit_parser::tests::test_json_agentic_output ... ok
test apply_verify::diff_applier::tests::test_apply_json_single_file ... ok
test apply_verify::diff_applier::tests::test_apply_empty_old_in_find_replace_is_invalid ... ok
test apply_verify::edit_parser::tests::test_json_bare_array ... ok
test apply_verify::edit_parser::tests::test_input_too_large ... ok
test apply_verify::edit_parser::tests::test_json_control_chars ... ok
test apply_verify::edit_parser::tests::test_json_empty_edits ... ok
test apply_verify::diff_applier::tests::test_apply_full_file_create_new ... ok
test apply_verify::edit_parser::tests::test_json_trailing_newlines_normalized ... ok
test apply_verify::edit_parser::tests::test_json_with_message_field ... ok
test apply_verify::edit_parser::tests::test_json_malformed ... ok
test apply_verify::diff_applier::tests::test_apply_full_file_overwrite ... ok
test apply_verify::edit_parser::tests::test_malformed_diff ... ok
test apply_verify::edit_parser::tests::test_markdown_backticks_in_content ... ok
test apply_verify::diff_applier::tests::test_apply_unified_diff_single_hunk ... ok
test apply_verify::edit_parser::tests::test_markdown_generic_block ... ok
test apply_verify::edit_parser::tests::test_markdown_diff_block ... ok
test apply_verify::edit_parser::tests::test_markdown_json_block ... ok
test apply_verify::edit_parser::tests::test_whitespace_only_input ... ok
test apply_verify::diff_applier::tests::test_apply_multi_file_success ... ok
test apply_verify::rollback::tests::test_is_fully_restored_false ... ok
test apply_verify::rollback::tests::test_is_fully_restored_true ... ok
test apply_verify::retry_loop::tests::test_parse_error_stop ... ok
test apply_verify::rollback::tests::test_rollback_delete_tolerates_already_missing ... ok
test apply_verify::rollback::tests::test_rollback_continues_on_failure ... ok
test apply_verify::retry_loop::tests::test_apply_partial_failure_rolls_back ... ok
test apply_verify::rollback::tests::test_rollback_empty_result_is_noop ... ok
test apply_verify::rollback::tests::test_rollback_deletes_new_file ... ok
test apply_verify::rollback::tests::test_rollback_mixed_restore_and_delete ... ok
test apply_verify::rollback::tests::test_rollback_single_file ... ok
test apply_verify::rollback::tests::test_rollback_reverse_order ... ok
test aggregator::concat::tests::concat_mixed_success_failure_snapshot ... ok
test aggregator::vote::tests::vote_snapshot ... ok
test apply_verify::verification::tests::test_verify_captures_both_streams ... ok
test apply_verify::retry_loop::tests::test_parse_error_retries ... ok
test apply_verify::verification::tests::test_verify_captures_stdout ... ok
test apply_verify::verification::tests::test_verify_captures_stderr ... ok
test apply_verify::retry_loop::tests::test_apply_error_triggers_rollback_and_retry ... ok
test apply_verify::retry_loop::tests::test_requester_error_surfaced ... ok
test apply_verify::retry_loop::tests::test_max_retries_zero_runs_once ... ok
test backend::claude::tests::capabilities_match_current_wiring ... ok
test apply_verify::retry_loop::tests::test_verify_failure_triggers_rollback ... ok
test backend::claude::tests::test_claude_response_deserialize_with_usage ... ok
test apply_verify::retry_loop::tests::test_success_first_attempt ... ok
test backend::claude::tests::test_claude_response_deserialize_without_usage ... ok
test backend::codex::tests::capabilities_match_current_wiring ... ok
test backend::gemini::tests::capabilities_match_current_wiring ... ok
test backend::genai_error::tests::classify_404_body_detects_unknown_function_fixture ... ok
test backend::genai_error::tests::classify_5xx_body_detects_anthropic_auth_fixture ... ok
test backend::genai_error::tests::classify_5xx_body_detects_rate_limit_signature ... ok
test backend::genai_error::tests::classify_5xx_body_returns_none_for_generic_5xx ... ok
test backend::genai_error::tests::contains_status_code_handles_punctuation_boundaries ... ok
test backend::genai_error::tests::map_status_401_to_auth ... ok
test apply_verify::retry_loop::tests::test_parse_error_on_last_retry_exits ... ok
test backend::genai_error::tests::map_status_403_to_auth ... ok
test backend::genai_error::tests::map_status_404_other_to_execution_failed ... ok
test backend::genai_error::tests::map_status_404_unknown_function_to_config ... ok
test backend::genai_error::tests::map_status_429_to_rate_limit_retryable ... ok
test backend::genai_error::tests::map_status_500_to_network_retryable ... ok
test backend::genai_error::tests::map_status_502_generic_to_network_retryable ... ok
test backend::genai_error::tests::map_status_502_upstream_auth_to_auth_not_retryable ... ok
test backend::genai_error::tests::map_status_502_upstream_rate_limit_to_rate_limit_retryable ... ok
test backend::genai_error::tests::map_status_503_to_network_retryable ... ok
test backend::genai_error::tests::map_status_unknown_to_execution_failed ... ok
test backend::ollama::tests::test_ollama_response_deserialize_partial_counts ... ok
test backend::ollama::tests::test_ollama_response_deserialize_with_counts ... ok
test backend::ollama::tests::test_ollama_response_deserialize_without_model ... ok
test backend::retry::tests::test_get_delay_attempt_zero_is_zero ... ok
test backend::retry::tests::test_get_delay_clamped_at_max ... ok
test backend::retry::tests::test_get_delay_grows_exponentially ... ok
test apply_verify::verification::tests::test_verify_failure_exit_code ... ok
test backend::retry::tests::test_retry_executor_does_not_retry_non_retryable ... ok
test backend::tensorzero::tests::canonicalize_wire_model_strips_to_canonical_on_wire ... ok
test backend::tensorzero::tests::capabilities_match_current_wiring ... ok
test backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime ... FAILED
test backend::tensorzero::tests::maps_401_to_auth_not_retryable ... FAILED
test backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable ... FAILED
test apply_verify::retry_loop::tests::test_integration_end_to_end ... ok
test backend::tensorzero::tests::maps_429_to_rate_limit_retryable ... FAILED
test backend::tensorzero::tests::maps_500_to_retryable_error ... FAILED
test backend::tensorzero::tests::maps_502_generic_to_network_retryable ... FAILED
test backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable ... FAILED
test backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable ... FAILED
test backend::tensorzero::tests::maps_malformed_json_to_parse_error ... FAILED
test backend::tensorzero::tests::maps_request_timeout_to_timeout_error ... FAILED
test backend::tensorzero::tests::normalize_endpoint_appends_when_missing ... ok
test backend::tensorzero::tests::normalize_endpoint_does_not_double_suffix ... ok
test backend::tensorzero::tests::returns_text_on_200_success ... FAILED
test backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model ... FAILED
test backend::tests::backend_capabilities_none_is_all_false ... ok
test backend::tests::capabilities_for_name_matches_static_expectations ... ok
test backend::tests::capabilities_for_name_unknown_returns_none ... ok
test backend::tests::default_capabilities_are_none ... ok
test backend::tests::tensorzero_adapter_allows_missing_api_key_env_field ... ok
test backend::tests::tensorzero_adapter_maps_endpoint_model_auth_timeout ... ok
test backend::tests::tensorzero_adapter_rejects_missing_endpoint_model_zero_timeout_and_bad_scheme ... ok
test backend::tests::tensorzero_create_backend_queries_wiremock_gateway ... FAILED
test backend::retry::tests::test_retry_success_after_failures ... ok
test backend::retry::tests::test_retry_exhausted ... ok
test backend::tests::test_backend_error_display ... ok
test backend::tests::test_backend_error_not_retryable ... ok
test backend::tests::test_backend_error_from_anyhow ... ok
test backend::tests::test_backend_error_retryable ... ok
test backend::tests::test_query_output_from_process_empty_stderr_normalized ... ok
test backend::tests::test_query_output_from_process_empty_stdout ... ok
test backend::tests::test_query_output_from_process_populates_backend_and_duration ... ok
test backend::tests::test_query_output_from_process_with_stderr ... ok
test backend::tests::test_query_output_from_text ... ok
test backend::tests::test_query_output_from_text_populates_backend_and_duration ... ok
test backend::tests::test_query_output_with_model_none ... ok
test backend::tests::test_query_output_with_structured_none ... ok
test backend::tests::test_query_output_with_model_some ... ok
test backend::tests::test_query_output_with_usage_none ... ok
test backend::tests::test_query_output_with_structured_some ... ok
test backend::tests::test_query_output_with_usage_some ... ok
test backend::tests::test_token_usage_default_zero ... ok
test backend::tests::test_token_usage_new_computes_total ... ok
test backend::tests::test_token_usage_new_saturates_on_overflow ... ok
test backend::tests::test_token_usage_saturating_add ... ok
test backend::tests::with_elapsed_is_idempotent_on_repeated_calls ... ok
test backend::tests::with_elapsed_is_noop_on_non_timeout_variants ... ok
test backend::tests::with_elapsed_overrides_timeout_elapsed_ms ... ok
test cache::tests::test_cache_key_deterministic ... ok
test cache::tests::test_cache_disabled ... ok
test cache::tests::test_cache_key_different_backends ... ok
test cache::tests::test_cache_key_different_prompts ... ok
test cache::tests::test_cache_warnings_on_parse_failure ... ok
test cache::tests::test_cache_warnings_deduplicated ... ok
test config::tests::test_backend_config_defaults ... ok
test config::tests::test_claude_backend_defaults ... ok
test config::tests::test_codex_backend_defaults ... ok
test config::tests::test_command_wrapper_config ... ok
test config::tests::test_command_wrapper_default_none ... ok
test config::tests::test_command_wrapper_docker_example ... ok
test config::tests::test_conductor_custom_config ... ok
test config::tests::test_conductor_defaults ... ok
test config::tests::test_deep_merge_boolean_override ... ok
test config::tests::test_deep_merge_empty_overlay ... ok
test config::tests::test_config_serialization_roundtrip ... ok
test config::tests::test_deep_merge_hashmap_add ... ok
test config::tests::test_deep_merge_hashmap_override ... ok
test config::tests::test_deep_merge_scalar_override ... ok
test config::tests::test_deep_merge_partial_config ... ok
test config::tests::test_default_config ... ok
test config::tests::test_deny_unknown_fields ... ok
test config::tests::test_deep_merge_vec_replace ... ok
test config::tests::test_gemini_backend_defaults ... ok
test config::tests::test_hunt_task_defaults ... ok
test config::tests::test_load_config_from_paths_no_files ... ok
test config::tests::test_load_config_from_paths_explicit_bypasses ... ok
test config::tests::test_load_config_from_paths_project_only ... ok
test config::tests::test_load_config_from_paths_three_layers ... ok
test apply_verify::verification::tests::test_verify_success ... ok
test config::tests::test_parse_custom_backend ... ok
test config::tests::test_parse_custom_task ... ok
test config::tests::test_parse_minimal_config ... ok
test apply_verify::verification::tests::test_verify_uses_passed_cwd ... ok
test apply_verify::verification::tests::test_verify_invalid_command_exits_127 ... ok
test config::tests::test_tensorzero_missing_endpoint_fails ... ok
test config::tests::test_tensorzero_invalid_url_fails ... ok
test apply_verify::retry_loop::tests::test_max_retries_exhausted ... ok
test consensus::tests::test_majority_vote_clear_winner ... ok
test config::tests::test_tensorzero_zero_timeout_fails ... ok
test consensus::tests::test_majority_vote_empty ... ok
test consensus::tests::test_majority_vote_tie_first_wins ... ok
test consensus::tests::test_weighted_vote ... ok
test consensus::tests::test_weighted_vote_clear_winner ... ok
test consensus::tests::test_whitespace_normalization ... ok
test config::tests::test_load_config_from_paths_user_parse_error ... ok
test family::tests::aggregator_rejected_display ... ok
test family::tests::as_str_openai ... ok
test family::tests::as_str_other ... ok
test family::tests::display_anthropic ... ok
test family::tests::display_other ... ok
test family::tests::enforce_all_anthropic_rejected ... ok
test config::tests::test_tensorzero_config_serialization_roundtrip ... ok
test family::tests::enforce_cross_family_deterministic ... ok
test family::tests::enforce_distinct_other_ok ... ok
test family::tests::enforce_empty_slice_ok ... ok
test family::tests::enforce_mixed_families_ok ... ok
test family::tests::enforce_same_other_rejected ... ok
test family::tests::enforce_single_backend_ok ... ok
test family::tests::enforce_three_same_family ... ok
test family::tests::enforce_two_distinct_others_ok ... ok
test family::tests::family_of_bedrock ... ok
test family::tests::family_of_claude ... ok
test family::tests::family_of_codex ... ok
test family::tests::family_of_empty_string ... ok
test family::tests::family_of_gemini ... ok
test family::tests::family_of_loker_no_suffix ... ok
test family::tests::family_of_loker_prefix_anthropic ... ok
test family::tests::family_of_loker_prefix_gemini ... ok
test family::tests::family_of_loker_prefix_google ... ok
test family::tests::family_of_loker_prefix_local ... ok
test family::tests::family_of_loker_prefix_ollama ... ok
test family::tests::family_of_loker_prefix_openai ... ok
test family::tests::family_of_loker_zhipu_suffix ... ok
test family::tests::family_of_ollama ... ok
test family::tests::family_of_openai ... ok
test family::tests::family_of_tensorzero ... ok
test family::tests::family_of_tensorzero_function_name ... ok
test family::tests::family_of_tensorzero_slash_only ... ok
test family::tests::family_of_tensorzero_unknown_suffix ... ok
test family::tests::family_of_tensorzero_zhipu_suffix ... ok
test family::tests::family_of_unknown ... ok
test family::tests::family_of_zhipu ... ok
test family::tests::judge_unavailable_display ... ok
test family::tests::quorum_lost_display ... ok
test context::tests::test_no_context ... ok
test context::tests::test_detect_rails_with_goldiloader ... ok
test manifest::tests::empty_manifest_roundtrips ... ok
test manifest::tests::sha256_hex_matches_known_vector ... ok
test context::tests::test_detect_typescript ... ok
test role::tests::test_resolution_builder ... ok
test role::tests::test_backend_filtering ... ok
test role::tests::test_resolution_is_empty ... ok
test role::tests::test_role_config_new ... ok
test role::tests::test_role_resolution_error_display ... ok
test role::tests::test_role_resolver_default_team ... ok
test role::tests::test_role_resolver_no_backends_available ... ok
test role::tests::test_role_resolver_resolve_global_role ... ok
test role::tests::test_role_config_serialization ... ok
test config::tests::test_tensorzero_to_backend_opts_resolves_env ... ok
test role::tests::test_role_resolver_role_not_found ... ok
test role::tests::test_role_resolver_team_can_define_custom_role ... ok
test role::tests::test_role_resolver_team_override ... ok
test role::tests::test_role_resolver_team_override_takes_precedence ... ok
test role::tests::test_routing_strategy_default_is_fallback ... ok
test git_agent::tests::test_is_initialized_false_for_nonexistent ... ok
test role::tests::test_team_config_default ... ok
test role::tests::test_valid_parallel_config ... ok
test role::tests::test_validation_parallel_min_success_exceeds_backends ... ok
test role::tests::test_validation_parallel_min_success_too_low ... ok
test role::tests::test_validation_unknown_backend ... ok
test role::tests::test_team_config_serialization ... ok
test strategy::escalating_retry::tests::config_default_false ... ok
test strategy::escalating_retry::tests::config_round_trip_false ... ok
test strategy::escalating_retry::tests::config_round_trip_true ... ok
test git_agent::tests::test_is_available_returns_bool ... ok
test apply_verify::verification::tests::test_verify_output_truncated ... ok
test apply_verify::retry_loop::tests::test_attempt_records ... ok
test backend::ollama::tests::capabilities_match_current_wiring ... FAILED
test backend::tensorzero::tests::name_is_tensorzero ... FAILED
test backend::tests::tensorzero_create_backend_supported_when_capability_supported ... FAILED
test strategy::escalating_retry::tests::envelope_backend_error_shows_null_response ... ok
test strategy::escalating_retry::tests::redaction_api_key_value ... ok
test strategy::escalating_retry::tests::envelope_verify_reason_only_when_no_response ... ok
test strategy::escalating_retry::tests::envelope_under_budget_no_truncation ... ok
test strategy::escalating_retry::tests::redaction_bearer_token ... ok
test strategy::escalating_retry::tests::envelope_hard_caps_when_body_alone_exceeds_budget ... ok
test strategy::escalating_retry::tests::redaction_aws_key ... ok
test strategy::escalating_retry::tests::truncate_exact_boundary ... ok
test strategy::escalating_retry::tests::redaction_does_not_false_positive_short_text ... ok
test strategy::escalating_retry::tests::truncate_multibyte_safe ... ok
test strategy::escalating_retry::tests::truncate_no_op_when_under_budget ... ok
test strategy::escalating_retry::tests::truncate_with_suffix_fits_within_budget ... ok
test strategy::future_variant_compiles::stub_fan_out_implements_strategy ... ok
test strategy::escalating_retry::tests::redaction_long_blob_heuristic ... ok
test strategy::parallel_fanout::tests::any_fail_all_pass ... ok
test strategy::parallel_fanout::tests::any_fail_markdown_fenced_json ... ok
test strategy::parallel_fanout::tests::any_fail_markdown_fenced_fail ... ok
test strategy::escalating_retry::tests::envelope_over_budget_truncates_excerpt ... ok
test strategy::parallel_fanout::tests::any_fail_valid_json_extra_keys ... ok
test strategy::parallel_fanout::tests::empty_targets_yields_no_backends ... ok
test strategy::parallel_fanout::tests::backend_not_found ... ok
test strategy::parallel_fanout::tests::any_fail_backend_error_treated_as_failure ... ok
test strategy::parallel_fanout::tests::floor_violation ... ok
test strategy::parallel_fanout::tests::any_fail_empty_query_text ... ok
test strategy::parallel_fanout::tests::happy_path_all_succeed ... ok
test strategy::parallel_fanout::tests::any_fail_all_fail ... ok
test strategy::parallel_fanout::tests::prompt_render_failure_no_dispatch ... ok
test strategy::verify::run_command::tests::run_command_builder_api ... ok
test strategy::verify::run_command::tests::run_command_default_values ... ok
test strategy::parallel_fanout::tests::any_fail_missing_pass_field ... ok
test strategy::parallel_fanout::tests::any_fail_wrong_pass_type ... ok
test strategy::parallel_fanout::tests::one_fails_floor_still_met ... ok
test strategy::verify::run_command::tests::verify_missing_command_fails ... ok
test strategy::verify::test_runner::tests::cargo_2_pass_1_fail ... ok
test strategy::parallel_fanout::tests::vote_quorum_lost ... ok
test strategy::verify::test_runner::tests::cargo_3_pass_0_fail ... ok
test strategy::verify::test_runner::tests::cargo_empty_no_tests ... ok
test strategy::verify::test_runner::tests::cargo_first_failure_preserves_stdout_excerpt ... ok
test strategy::verify::test_runner::tests::cargo_malformed_json_line_skipped ... ok
test strategy::verify::test_runner::tests::cargo_first_failure_truncates_utf8_excerpt_safely ... ok
test strategy::verify::test_runner::tests::cargo_skips_compiler_messages ... ok
test strategy::verify::test_runner::tests::pytest_4_pass_2_fail ... ok
test strategy::verify::test_runner::tests::pytest_5_pass_0_fail ... ok
test strategy::parallel_fanout::tests::any_fail_first_fails ... ok
test strategy::verify::test_runner::tests::pytest_empty_no_tests ... ok
test strategy::verify::test_runner::tests::pytest_missing_summary_field ... ok
test strategy::parallel_fanout::tests::any_fail_non_deterministic_offender ... ok
test strategy::verify::test_runner::tests::pytest_non_json_output ... ok
test strategy::verify::test_runner::tests::verify_result_from_passing_tests ... ok
test strategy::verify::test_runner::tests::verify_result_from_failing_tests ... ok
test strategy::verify::test_runner::tests::verify_result_timed_out ... ok
test strategy::verify::test_runner::tests::verify_result_killed_by_signal ... ok
test strategy::verify::test_runner::tests::verify_result_no_tests_ran ... ok
test strategy::verify::verify::tests::failure_reason_display ... ok
test strategy::verify::verify::tests::failure_reason_builder_api ... ok
test strategy::verify::verify::tests::reserved_repair_compiles_but_not_pass ... ok
test strategy::verify::verify::tests::stub_verify_hook_returns_error ... ok
test strategy::verify::verify::tests::reserved_score_compiles_but_not_pass ... ok
test strategy::verify::verify::tests::stub_verify_hook_returns_fail ... ok
test strategy::verify::verify::tests::stub_verify_hook_returns_fail_with_full_reason ... ok
test strategy::verify::verify::tests::stub_verify_hook_returns_pass ... ok
test strategy::verify::verify::tests::verify_context_from_query_output ... ok
test template::context::tests::test_arg_out_of_bounds ... ok
test template::context::tests::test_arg_access ... ok
test template::context::tests::test_arg_zero_undefined ... ok
test template::context::tests::test_env_lookup ... ok
test template::context::tests::test_env_missing ... ok
test template::context::tests::test_loop_vars_object_item ... ok
test template::context::tests::test_loop_vars_preserve_existing_namespaces ... ok
test template::context::tests::test_loop_vars_string_item ... ok
test template::context::tests::test_step_field_fallback_no_parsed_output ... ok
test template::context::tests::test_step_output ... ok
test template::context::tests::test_step_field_with_parsed_output ... ok
test template::context::tests::test_step_success_false ... ok
test template::context::tests::test_step_success_true ... ok
test template::context::tests::test_workflow_backends ... ok
test template::filters::tests::test_default_val_defined ... ok
test template::context::tests::test_workflow_backends_empty ... ok
test template::filters::tests::test_default_val_empty_string ... ok
test template::filters::tests::test_default_val_undefined ... ok
test template::filters::tests::test_first_empty ... ok
test template::filters::tests::test_first_normal ... ok
test template::filters::tests::test_join_default_separator ... ok
test template::filters::tests::test_first_single ... ok
test template::filters::tests::test_join_empty ... ok
test template::filters::tests::test_join_with_separator ... ok
test template::filters::tests::test_json_encode_nested ... ok
test template::filters::tests::test_json_encode_number ... ok
test template::filters::tests::test_json_encode_string ... ok
test template::filters::tests::test_last_empty ... ok
test template::filters::tests::test_last_normal ... ok
test template::filters::tests::test_last_single ... ok
test template::filters::tests::test_lines_empty ... ok
test template::filters::tests::test_lines_multiline ... ok
test template::filters::tests::test_shell_escape_backticks_and_dollar ... ok
test template::filters::tests::test_shell_escape_basic ... ok
test template::filters::tests::test_lines_single ... ok
test template::filters::tests::test_shell_escape_injection ... ok
test template::filters::tests::test_shell_escape_newlines ... ok
test template::filters::tests::test_shell_escape_null_bytes ... ok
test template::filters::tests::test_shell_escape_single_quotes ... ok
test template::filters::tests::test_shell_escape_unicode ... ok
test template::filters::tests::test_trim_already_trimmed ... ok
test template::filters::tests::test_trim_newlines ... ok
test template::filters::tests::test_trim_whitespace ... ok
test template::tests::test_combined_env_arg_step ... ok
test template::tests::test_eval_expression_falsy ... ok
test template::tests::test_eval_expression_truthy ... ok
test template::tests::test_eval_expression_undefined ... ok
test template::tests::test_no_reexpansion_of_braces_in_output ... ok
test template::tests::test_parse_error ... ok
test template::tests::test_undefined_variable ... ok
test utils::tests::test_backend_error_kind_from_typed ... ok
test template::tests::test_render_mixed ... ok
test utils::tests::test_classify_auth_401 ... ok
test utils::tests::test_classify_capacity_exhausted ... ok
test utils::tests::test_classify_auth_invalid_key ... ok
test utils::tests::test_classify_network_refused ... ok
test utils::tests::test_classify_not_installed ... ok
test utils::tests::test_classify_rate_limit_429 ... ok
test utils::tests::test_classify_rate_limit_quota ... ok
test utils::tests::test_classify_resource_exhausted ... ok
test utils::tests::test_classify_unknown ... ok
test utils::tests::test_summarize_capacity ... ok
test utils::tests::test_summarize_rate_limit ... ok
test utils::tests::test_summarize_typed_backend_error ... ok
test utils::tests::test_summarize_unknown_truncates ... ok
test utils::tests::test_redact_secrets_aws_key ... ok
test utils::tests::test_truncate_exact_length ... ok
test utils::tests::test_redact_secrets_bearer_token ... ok
test utils::tests::test_truncate_long_string ... ok
test utils::tests::test_truncate_short_string ... ok
test utils::tests::test_truncate_unicode ... ok
test utils::tests::test_truncate_utf8_ascii ... ok
test utils::tests::test_truncate_utf8_empty_string ... ok
test utils::tests::test_truncate_utf8_exact_boundary ... ok
test utils::tests::test_truncate_utf8_multibyte_boundary ... ok
test utils::tests::test_truncate_utf8_within_limit ... ok
test utils::tests::test_truncate_utf8_zero_cap ... ok
test workflow::tests::required_capabilities_returns_file_edit_for_apply_edits ... ok
test workflow::tests::required_capabilities_returns_empty_for_plain_step ... ok
test workflow::tests::test_apply_lenient_mode_empty_response_fails ... ok
test workflow::tests::test_apply_lenient_mode_non_empty_passes_with_cleaned_output ... ok
test utils::tests::test_redact_secrets_api_key_value ... ok
test workflow::tests::test_apply_lenient_mode_preserves_internal_whitespace ... ok
test workflow::tests::test_apply_lenient_mode_whitespace_only_fails ... ok
test apply_verify::retry_loop::tests::test_success_on_retry_after_verify_failure ... ok
test workflow::tests::test_apply_parse_error_policy_default_fails ... ok
test workflow::tests::test_apply_once_parse_error_returns_err ... ok
test manifest::tests::atomic_write_and_read ... ok
test workflow::tests::test_apply_parse_error_policy_explicit_fail_matches_default ... ok
test workflow::tests::test_apply_parse_error_policy_pass_succeeds_without_output ... ok
test workflow::tests::test_apply_parse_error_policy_skip_drops_validation ... ok
test workflow::tests::test_apply_once_apply_error_rolls_back ... ok
test workflow::tests::test_apply_parse_error_policy_unknown_value_falls_back_to_fail ... ok
test workflow::tests::test_build_apply_fix_prompt_includes_partial_paths ... ok
test workflow::tests::test_build_parse_fix_prompt_contains_previous_raw ... ok
test workflow::tests::test_build_verify_fix_prompt_with_exit_code ... ok
test workflow::tests::test_build_verify_fix_prompt_with_timeout_uses_timeout_string ... ok
test strategy::parallel_fanout::tests::vote_success ... ok
test workflow::tests::test_apply_once_success_without_format ... ok
test strategy::parallel_fanout::tests::any_fail_mid_list_fails ... ok
test strategy::parallel_fanout::tests::vote_tie_random_deterministic ... ok
test workflow::tests::test_continue_on_error_toml_parsing ... ok
test workflow::tests::test_duplicate_step_names_error ... ok
test workflow::tests::test_condition_unparseable_returns_true ... ok
test workflow::tests::test_evaluate_condition_error_recovery ... ok
test workflow::tests::test_extract_json_field_bool ... ok
test workflow::tests::test_extract_json_field_multiline ... ok
test workflow::tests::test_extract_json_field_not_found ... ok
test workflow::tests::test_condition_steps_success ... ok
test workflow::tests::test_extract_json_field_number ... ok
test workflow::tests::test_extract_json_field_string ... ok
test workflow::tests::test_extract_json_from_markdown_block ... ok
test workflow::tests::test_extract_json_from_plain_block ... ok
test workflow::tests::test_extract_json_raw ... ok
test workflow::tests::test_extract_json_with_text_before ... ok
test workflow::tests::test_extract_json_with_literal_newlines ... ok
test workflow::tests::test_find_closing_fence ... ok
test workflow::tests::test_condition_equals ... ok
test workflow::tests::test_condition_contains ... ok
test workflow::tests::test_heuristic_contains_double_quotes ... ok
test workflow::tests::test_heuristic_contains_empty_string_always_passes ... ok
test workflow::tests::test_heuristic_contains_fail ... ok
test workflow::tests::test_condition_legacy_syntax ... ok
test workflow::tests::test_heuristic_contains_pass ... ok
test workflow::tests::test_group_by_depth_forward_declared_dependency ... ok
test workflow::tests::test_heuristic_contains_single_quote_char ... ok
test workflow::tests::test_condition_not ... ok
test workflow::tests::test_heuristic_contains_special_chars ... ok
test workflow::tests::test_heuristic_empty_check_string ... ok
test workflow::tests::test_heuristic_min_length_fail ... ok
test workflow::tests::test_heuristic_min_length_invalid_arg ... ok
test workflow::tests::test_heuristic_min_length_pass ... ok
test workflow::tests::test_heuristic_min_length_unicode ... ok
test workflow::tests::test_heuristic_min_length_whitespace_counts ... ok
test workflow::tests::test_heuristic_min_length_zero_always_passes ... ok
test workflow::tests::test_heuristic_not_empty_fail_empty ... ok
test workflow::tests::test_heuristic_not_empty_fail_whitespace ... ok
test workflow::tests::test_heuristic_not_empty_pass ... ok
test workflow::tests::test_heuristic_unknown_check ... ok
test workflow::tests::test_condition_json_field_access ... ok
test workflow::tests::test_for_each_parsed_output_not_array ... ok
test workflow::tests::test_for_each_with_parsed_output ... ok
test workflow::tests::test_interpolate_loop_vars_index ... ok
test workflow::tests::test_interpolate_loop_vars_item_whole_object ... ok
test workflow::tests::test_interpolate_loop_vars_item_string ... ok
test workflow::tests::test_interpolate_validation_prompt_basic ... ok
test workflow::tests::test_interpolate_validation_prompt_injection_safety ... ok
test workflow::tests::test_interpolate_validation_prompt_no_stderr ... ok
test workflow::tests::test_interpolate_validation_prompt_no_truncation_when_under_limit ... ok
test workflow::tests::test_interpolate_loop_vars_combined ... ok
test workflow::tests::test_interpolate_loop_vars_missing_field ... ok
test workflow::tests::test_interpolate_validation_prompt_truncation ... ok
test workflow::tests::test_interpolate_validation_prompt_with_stderr ... ok
test workflow::tests::test_interpolate_loop_vars_multiple_fields_one_missing ... ok
test workflow::tests::test_interpolate_loop_vars_item_object ... ok
test workflow::tests::test_interpolate_parsed_output_none_fallback ... ok
test workflow::tests::test_jinja_missing_step_default_fallback ... ok
test workflow::tests::test_jinja_default_filter ... ok
test workflow::tests::test_jinja_if_block ... ok
test workflow::tests::test_jinja_join_filter ... ok
test workflow::tests::test_load_error_tracker_backoff_progression ... ok
test workflow::tests::test_jinja_inline_for_loop ... ok
test workflow::tests::test_jinja_chained_filters ... ok
test workflow::tests::test_load_error_tracker_bail_at_threshold ... ok
test workflow::tests::test_load_error_tracker_reset_on_success ... ok
test workflow::tests::test_interpolate_with_fields_json ... ok
test workflow::tests::test_load_error_tracker_success_with_no_prior_errors ... ok
test workflow::tests::test_map_retry_failure_apply_error_with_paths ... ok
test workflow::tests::test_map_retry_failure_apply_error_without_paths ... ok
test workflow::tests::test_map_retry_failure_attempt_count_from_retries ... ok
test workflow::tests::test_jinja_trim_filter ... ok
test workflow::tests::test_map_retry_failure_empty_attempts ... ok
test workflow::tests::test_map_retry_failure_parse_error ... ok
test workflow::tests::test_map_retry_failure_verify_exit_code ... ok
test workflow::tests::test_map_retry_failure_verify_has_priority_over_apply ... ok
test workflow::tests::test_jinja_shell_escape_filter ... ok
test workflow::tests::test_map_retry_failure_verify_timeout ... ok
test workflow::tests::test_map_retry_failure_stderr_truncated_to_1kb ... ok
test workflow::tests::test_parse_for_each_inline_array ... ok
test workflow::tests::test_map_template_error_reports_offending_variable_in_multi_expression ... ok
test workflow::tests::test_min_deps_success_without_depends_on_error ... ok
test workflow::tests::test_output_format_toml_parsing ... ok
test workflow::tests::test_parse_for_each_inline_array_objects ... ok
test workflow::tests::test_min_deps_success_validation_empty_deps ... ok
test workflow::tests::test_min_deps_success_validation_valid ... ok
test workflow::tests::test_parse_step_output_json ... ok
test workflow::tests::test_parse_step_output_lines ... ok
test workflow::tests::test_parse_step_output_none ... ok
test workflow::tests::test_parse_for_each_not_array ... ok
test workflow::tests::test_parse_for_each_step_not_found ... ok
test workflow::tests::test_min_deps_success_validation_exceeds_deps ... ok
test workflow::tests::test_parse_for_each_invalid_format ... ok
test workflow::tests::test_parse_step_output_text ... ok
test workflow::tests::test_parse_validation_response_empty_string_is_error ... ok
test workflow::tests::test_parse_validation_response_invalid_status ... ok
test workflow::tests::test_parse_for_each_step_reference ... ok
test workflow::tests::test_parse_validation_response_json_fail ... ok
test workflow::tests::test_parse_validation_response_json_in_fences ... ok
test workflow::tests::test_parse_validation_response_json_pass ... ok
test workflow::tests::test_parse_validation_response_json_pass_no_output ... ok
test workflow::tests::test_parse_validation_response_review_failed ... ok
test workflow::tests::test_parse_validation_response_unrecognized_is_error ... ok
test workflow::tests::test_sanitize_json_strings ... ok
test workflow::tests::test_step_failure_kind_copy_eq ... ok
test workflow::tests::test_step_failure_kind_display ... ok
test workflow::tests::test_step_for_each_inline_array_toml ... ok
test workflow::tests::test_step_for_each_toml_parsing ... ok
test workflow::tests::test_step_if_alias ... ok
test workflow::tests::test_step_result_error_backend_error ... ok
test workflow::tests::test_step_result_error_edit_failed ... ok
test workflow::tests::test_step_result_error_has_no_validation ... ok
test workflow::tests::test_step_result_error_output_matches_failure_message ... ok
test workflow::tests::test_step_result_error_produces_failure ... ok
test workflow::tests::test_step_result_error_skipped ... ok
test workflow::tests::test_step_result_error_verify_failed ... ok
test workflow::tests::test_strip_markdown_fences_json ... ok
test workflow::tests::test_strip_markdown_fences_none ... ok
test workflow::tests::test_strip_markdown_fences_plain ... ok
test workflow::tests::test_strip_markdown_fences_with_whitespace ... ok
test workflow::tests::test_success_step_has_no_failure ... ok
test workflow::tests::test_parse_validate_config_absent ... ok
test workflow::tests::test_parse_for_each_step_reference_with_code_block ... ok
test workflow::tests::test_translate_contains_call ... ok
test workflow::tests::test_parse_validate_config_from_toml ... ok
test workflow::tests::test_parse_validate_config_mixed_fields ... ok
test workflow::tests::test_translate_contains_with_single_quoted_literal_containing_double_quote ... ok
test workflow::tests::test_parse_for_each_field_access ... ok
test workflow::tests::test_translate_contains_with_steps_prefix ... ok
test workflow::tests::test_translate_contains_with_escaped_quotes ... ok
test workflow::tests::test_timeout_at_minimum_allowed ... ok
test workflow::tests::test_translate_equals_call ... ok
test workflow::tests::test_timeout_normal_value_allowed ... ok
test workflow::tests::test_timeout_too_small_validation ... ok
test workflow::tests::test_translate_equals_with_steps_prefix ... ok
test workflow::tests::test_timeout_zero_allowed ... ok
test workflow::tests::test_translate_fast_path_whitespace_variants ... ok
test workflow::tests::test_translate_multiple_contains ... ok
test workflow::tests::test_translate_mixed_legacy_new ... ok
test workflow::tests::test_translate_legacy_steps_output_contains ... ok
test workflow::tests::test_translate_passthrough_already_valid ... ok
test workflow::tests::test_translate_passthrough_empty ... ok
test workflow::tests::test_translate_nested_not ... ok
test workflow::tests::test_truncate_for_prompt_over_limit ... ok
test workflow::tests::test_truncate_for_prompt_under_limit ... ok
test workflow::tests::test_translate_legacy_double_quotes ... ok
test workflow::tests::test_validation_failure_has_no_step_failure ... ok
test workflow::tests::test_verify_command_composition_pattern ... ok
test workflow::tests::validate_accepts_apply_edits_on_claude ... ok
test workflow::tests::validate_rejects_apply_edits_on_ollama ... ok
test workflow::tests::validate_rejects_apply_edits_with_multiple_backends ... ok
test workflow::tests::validate_rejects_apply_edits_with_no_backend ... ok
test workflow::tests::test_workflow_level_continue_on_error ... ok
test workflow::tests::validate_skips_shell_only_steps ... ok
test workflow::tests::validate_treats_unknown_backend_as_none ... ok
test workflow::tests::validate_with_capabilities_handles_empty_steps ... ok
test workflows::tests::test_embedded_workflows_exist ... ok
test workflow::tests::test_validate_config_new_fields_default_to_none ... ok
test workflow::tests::test_validate_config_defaults ... ok
test workflow::tests::test_validate_config_parses_on_parse_error_field ... ok
test workflows::tests::test_embedded_workflows_parse ... ok
test workflow::tests::test_validate_config_parses_mode_lenient_field ... ok
test workflow::tests::test_validate_config_new_fields_parsing ... ok
test strategy::verify::run_command::tests::verify_false_fails_with_code ... ok
test strategy::verify::run_command::tests::verify_echo_passes ... ok
test workflow::tests::test_apply_once_with_format_runs_after_apply ... ok
test backend::retry::tests::test_retry_executor_honors_rate_limit_retry_after ... ok
test apply_verify::verification::tests::test_verify_elapsed_ms_nonzero ... ok
test strategy::verify::run_command::tests::verify_sleeps_timeout ... ok
test apply_verify::verification::tests::test_verify_timeout_real_elapsed ... ok
test apply_verify::verification::tests::test_verify_timeout_kills_process_group ... ok

failures:

---- backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime stdout ----

thread 'backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime' (48332700) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- backend::tensorzero::tests::maps_401_to_auth_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_401_to_auth_not_retryable' (48332754) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable' (48332776) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_429_to_rate_limit_retryable stdout ----

thread 'backend::tensorzero::tests::maps_429_to_rate_limit_retryable' (48332795) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_500_to_retryable_error stdout ----

thread 'backend::tensorzero::tests::maps_500_to_retryable_error' (48332810) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_generic_to_network_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_generic_to_network_retryable' (48332814) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable' (48332822) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable' (48332821) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_malformed_json_to_parse_error stdout ----

thread 'backend::tensorzero::tests::maps_malformed_json_to_parse_error' (48332826) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_request_timeout_to_timeout_error stdout ----

thread 'backend::tensorzero::tests::maps_request_timeout_to_timeout_error' (48332827) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::returns_text_on_200_success stdout ----

thread 'backend::tensorzero::tests::returns_text_on_200_success' (48332831) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model stdout ----

thread 'backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model' (48332832) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tests::tensorzero_create_backend_queries_wiremock_gateway stdout ----

thread 'backend::tests::tensorzero_create_backend_queries_wiremock_gateway' (48332840) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::ollama::tests::capabilities_match_current_wiring stdout ----

thread 'backend::ollama::tests::capabilities_match_current_wiring' (48332670) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
Attempted to create a NULL object.

---- backend::tensorzero::tests::name_is_tensorzero stdout ----

thread 'backend::tensorzero::tests::name_is_tensorzero' (48332828) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
Attempted to create a NULL object.

---- backend::tests::tensorzero_create_backend_supported_when_capability_supported stdout ----

thread 'backend::tests::tensorzero_create_backend_supported_when_capability_supported' (48332841) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
Attempted to create a NULL object.


failures:
    backend::ollama::tests::capabilities_match_current_wiring
    backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime
    backend::tensorzero::tests::maps_401_to_auth_not_retryable
    backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable
    backend::tensorzero::tests::maps_429_to_rate_limit_retryable
    backend::tensorzero::tests::maps_500_to_retryable_error
    backend::tensorzero::tests::maps_502_generic_to_network_retryable
    backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable
    backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable
    backend::tensorzero::tests::maps_malformed_json_to_parse_error
    backend::tensorzero::tests::maps_request_timeout_to_timeout_error
    backend::tensorzero::tests::name_is_tensorzero
    backend::tensorzero::tests::returns_text_on_200_success
    backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model
    backend::tests::tensorzero_create_backend_queries_wiremock_gateway
    backend::tests::tensorzero_create_backend_supported_when_capability_supported

test result: FAILED. 640 passed; 16 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s

error: test failed, to rerun pass `--lib`

codex
The repo-wide `cargo test` gate is red in this sandbox before we even get to branch-specific logic. The failures are all environment-bound: `wiremock` cannot bind local ports here, and a few backend tests panic inside `system-configuration` on this host. I’m still checking the branch-specific tests and the marker contract, because there are separate implementation issues in the new loader that matter even if the CI environment is healthier.
exec
/bin/zsh -lc "sed -n '1,280p' docs/run-state.md" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
# Atomic run-state write protocol (D3)

**Status**: decided. Closes PRD §11 D3, blocks T-024 (manifest writer),
T-025 (phase status markers), T-031 (resumability).
**Scope**: how loker writes everything under `runs/<id>/` so any crash
leaves the run resumable without manual cleanup. Read this once;
T-024/T-025/T-031 reference it instead of relitigating.

## Why

FR-21 mandates atomic per-phase status markers. FR-23b - FR-23e mandate
a manifest plus an `attempts/<phase>/<n>/` archive for failed retries.
Crash-injection tests (FR-23c) demand a single, well-defined order so
the reader can tell "completed" apart from "torn mid-write" without
guessing. Without a written protocol, every call site (manifest writer,
markers, per-phase artefacts, HITL pending/responses, summary) would
re-derive its own ordering and we would discover divergence the hard
way - in a half-resumed run.

## Run directory layout (recap from FR-22)

```
runs/<workflow>-<timestamp>-<short-uuid>/
├── manifest.json              # canonical artefact index (full rewrite via tmp+rename)
├── trace.jsonl                # append-only OTel GenAI events (D2 / T-002)
├── markers/
│   ├── <phase>.started        # one per phase that began
│   ├── <phase>.completed      # one per phase that produced a verified artefact
│   └── <phase>.failed         # one per phase that exhausted retries
├── heartbeat.json             # writer pid + monotonic clock tick (refreshed periodically)
├── <phase>/                   # canonical artefacts produced by the phase
│   └── <artefact>             # e.g. design/design.md, verify/verify.json
├── attempts/<phase>/<n>/      # debris from failed attempts (FR-23e)
│   └── <artefact>             # not loaded by downstream phases
└── pending/, responses/       # HITL only (M10 / D4)
```

Phase names are stable identifiers from the workflow definition
(`design`, `review`, `verify`, ...). `<n>` is a 1-indexed attempt
counter that increments per failure within the same phase invocation.

## Decision: tmp + rename + status marker (canonical at top-level)

Two candidates were considered, both compatible with FR-21 and FR-23e:

**A. tmp + rename + status marker.** Canonical artefact lives at
`<phase>/<artefact>`. Each write is `<artefact>.<rand>.tmp` -> fsync
file -> rename to canonical -> fsync parent dir. `phase.completed`
written *after* artefact and manifest entry are durable. Failed
attempts copied to `attempts/<phase>/<n>/` for postmortem.

**B. Attempt-directory canonical.** All attempts live under
`attempts/<phase>/<n>/`. Canonical is a symlink (or pointer file)
`<phase>/current` swung to the latest successful attempt directory.
Markers live next to the pointer.

We pick **A**. Trade-offs:

| Concern | A (chosen) | B (rejected) |
|---|---|---|
| Reader path | Stable: `runs/<id>/<phase>/<artefact>` | Two hops: read `current` then path-build |
| Manifest entries | Path field is canonical and stable | Path must include `<n>` or be re-resolved per read |
| Crash-mid-write detection | Hash check on canonical + missing marker | Pointer race window: pointer swung but `phase.completed` not yet written |
| Portability | Plain `rename(2)`; no symlink at all | Symlinks are unprivileged-blocked on Windows; pointer-file workaround adds indirection |
| File watch (`notify`, M5+ SSE) | Single canonical path per artefact | Watcher fires on every attempt write; UI must filter by current pointer |
| History inspection | `attempts/` dir always there | Native to layout, no extra dir |
| Concurrent writers (same artefact) | rename(2) is atomic; loser visible only via tmp filename collision (random suffix) | Pointer swing race; needs lock or compare-and-swap on pointer file |

A's only concession is that retry history lives in a sibling `attempts/`
dir rather than as the canonical layout itself. We accept that: the
canonical path is the load-bearing read path, and stable plain-file
semantics matter more than retro-aesthetic of "everything is an
attempt". B's pointer indirection breaks the simple `cat
runs/<id>/<phase>/<artefact>` mental model that operators will reach
for in production.

## Write protocol

### Atomic file commit primitive

For any file that must be readable as "either the old contents or the
new contents, never torn":

1. Open `<final>.<rand64>.tmp` in the **same directory** as `<final>`
   (rename across directories is not atomic on most POSIX filesystems).
2. Write payload.
3. `fsync(file_fd)` - commit data and metadata to disk.
4. `rename(<tmp>, <final>)` - POSIX-atomic replace.
5. `fsync(parent_dir_fd)` - commit the directory entry update.

In Rust, `tempfile::NamedTempFile::new_in(dir)` + `.persist(<final>)`
implements (1), (2), and (4) portably. Step 3 requires calling
`file.as_file().sync_all()?` before `persist()`, since `tempfile`
does not implicitly fsync the file. Step 5 needs a manual
`File::open(parent).sync_all()` because `tempfile` does not fsync the
directory either.

The random suffix (64 bits is enough) prevents collisions when two
processes - or a previously-crashed writer's leftover - attempt the
same final path. Reader sweeps any `*.tmp` files older than the
heartbeat TTL on resume (see "Stale tmp" below).

### Phase markers

Markers are tiny JSON files written via the atomic-commit primitive
into `runs/<id>/markers/`.

`<phase>.started`:

```json
{
  "phase": "design",
  "attempt": 1,
  "started_at": "2026-04-25T20:45:00Z",
  "writer_pid": 12345,
  "writer_host": "loker-runner-3",
  "heartbeat_ttl_seconds": 300
}
```

`<phase>.completed`:

```json
{
  "phase": "design",
  "attempt": 1,
  "completed_at": "2026-04-25T20:48:13Z",
  "manifest_entry_sha256": "ab12...",
  "artefact_paths": ["design/design.md"]
}
```

`<phase>.failed`:

```json
{
  "phase": "design",
  "attempts_made": 3,
  "failed_at": "2026-04-25T20:51:02Z",
  "error_class": "BackendTimeout",
  "last_attempt_path": "attempts/design/3/"
}
```

### Per-phase commit order

For each successful attempt of a phase, the writer MUST execute steps
in this order and complete each step's fsync before starting the next:

1. **Started**: write `markers/<phase>.started` via the atomic-commit
   primitive on **every attempt**, updating the body's `attempt`
   field to the current attempt number. The marker is small;
   rewriting it per attempt keeps it self-describing. The reader
   trusts the started marker's `attempt` value as authoritative for
   the in-flight attempt; on a fresh start of the same phase it
   verifies that value against `attempts_made + 1` from any
   `markers/<phase>.failed` and against the count of subdirectories
   under `attempts/<phase>/` (a tiebreaker when markers disagree
   after a partially-recovered crash).
2. **Artefact**: write each artefact file via the atomic-commit
   primitive into `<phase>/<artefact>`.
3. **Manifest**: rewrite `manifest.json` via the atomic-commit
   primitive with the new entries appended logically (full rewrite
   physically).
4. **Completed**: write `markers/<phase>.completed` via the
   atomic-commit primitive.

**Critical invariant**: `markers/<phase>.completed` is the single
source of truth for "phase done". Its presence implies (2) and (3)
are durable. Its absence implies the phase MUST be rerun, even if (2)
and (3) appear complete on disk.

### Manifest rewrite

The manifest is "append-only" in the sense that entries are only ever
added, never mutated or deleted. Physically, each phase commit
rewrites the entire file via tmp+rename. This is intentional:
line-append-with-fsync on JSON text invites partial-line corruption
on power loss (the OS may flush the rename but not the appended
bytes). The manifest is small enough (low hundreds of entries even
for large workflows) that full rewrite is cheap and avoids a class of
bugs.

Each entry is content-addressed by sha256 of the artefact bytes
(FR-23b/c). Resumption verifies hash before trusting the entry. A
mismatch (artefact tampered after marker written) surfaces as
`PhaseError::ArtefactSchemaMismatch` per FR-23d.

### Failed-attempt archive

On phase failure (retry budget exhausted, unrecoverable error):

1. Move (`rename`, same filesystem) the in-flight `<phase>/<artefact>`
   files - if any survived past their tmp - into
   `attempts/<phase>/<n>/<artefact>`.
2. Move (or copy + remove) any `<phase>/*.tmp` debris into the same
   `attempts/<phase>/<n>/` directory for postmortem.
3. Write `markers/<phase>.failed` via the atomic-commit primitive.

Archival is best-effort: failure to archive must not block the
`phase.failed` marker. If archival itself fails, log a warning to
`trace.jsonl` and proceed.

### Heartbeat

`heartbeat.json` is rewritten via the atomic-commit primitive every
`heartbeat_ttl_seconds / 3` (default 100s for a 300s TTL) by the
active writer. Body:

```json
{ "writer_pid": 12345, "writer_host": "loker-runner-3", "tick_at": "..." }
```

Stale heartbeats let the reader distinguish "writer is alive,
phase-in-progress" from "writer died, phase needs rerun" without
process-discovery RPC. TTL is conservative (5 min default) - tighter
TTLs trade false-positive reruns for faster crash recovery; the
default favors no-spurious-rerun.

## Read protocol

### Resume walk

For each phase declared in the workflow, in dependency order:

1. **`markers/<phase>.completed` present**:
   - Read the marker's `manifest_entry_sha256`. This digest is the
     authoritative lookup key for the completed phase.
   - Load that manifest entry; treat it as the sole source of truth
     for the phase's artefact set. The completed marker's
     `artefact_paths` is a denormalised hint for human-readable
     debugging only - the manifest entry is canonical.
   - sha256-verify each on-disk artefact referenced by that manifest
     entry against the digests recorded in the same entry.
   - On mismatch: log `ArtefactSchemaMismatch`, treat phase as failed,
     rerun. (The completion marker plus a tampered artefact means
     someone edited files between runs; we do not silently trust
     edits.)
   - On match: skip phase, advance to next.
2. **`markers/<phase>.failed` present, no completed**:
   - Read `attempts_made` from the failed marker; rerun starting from
     attempt `attempts_made + 1`.
3. **`markers/<phase>.started` present, no completed, no failed**:
   - Compare `heartbeat.json` `tick_at` to `now()`.
   - If `now - tick_at < heartbeat_ttl_seconds`: another writer holds
     the run. Abort the resume with a clear error, do not start a
     parallel writer.
   - Else (stale): treat as failed (writer died mid-phase). Move any
     `<phase>/<artefact>` and `<phase>/*.tmp` to
     `attempts/<phase>/<n>/`. Rerun from attempt `n+1`.
4. **No markers for `<phase>`**: phase is fresh; run normally.

### Stale tmp sweep

On resume, before any phase work begins, sweep every
`runs/<id>/**/*.tmp` whose mtime is older than `heartbeat_ttl_seconds`.
Move into `attempts/_orphan_tmp/<YYYYMMDDTHHMMSSZ>/` (compact UTC
timestamp, no `:` separator so the path is portable across
filesystems including Windows) rather than deleting - crash-debugging
value outweighs disk cost.

### Hash verification

Manifest entries carry `sha256` and `schema_version`. Readers MUST
verify both:

- sha256 mismatch -> `ArtefactSchemaMismatch` (treat as failed).
- schema_version mismatch -> `ArtefactSchemaMismatch` (FR-23d).

Hash verification on every resume is cheap (small artefacts) and
catches both filesystem corruption and manual edits.

## Fault-injection test plan

The protocol is only as strong as the tests. T-031 (resumability)
must include the following kill matrix. Each row is one test: kill the
writer at the named transition, then resume and assert the expected
reader behaviour.

| # | Kill point | On-disk state after kill | Expected reader behaviour |

exec
/bin/zsh -lc 'cargo test --test run_state_load' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
exec
/bin/zsh -lc 'cargo test --test manifest' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
exec
/bin/zsh -lc 'make check' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 135ms:
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.11s
     Running tests/run_state_load.rs (target/debug/deps/run_state_load-28127dea3923fba5)

running 12 tests
test empty_manifest_loads_empty_runstate ... ok
test schema_mismatch_returns_artefact_schema_mismatch ... ok
test stale_heartbeat_is_reported ... ok
test corrupt_entry_returns_artefact_corrupt ... ok
test missing_markers_directory_keeps_all_entries ... ok
test live_heartbeat_is_reported ... ok
test phase_status_is_derived_from_markers ... ok
test missing_entry_returns_artefact_missing ... ok
test changes_dir_entry_is_verified_with_digest ... ok
test markers_without_completed_hashes_keeps_all_entries ... ok
test happy_path_load_returns_surviving_entries ... ok
test orphan_sweep_drops_non_completed_entries ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s


 succeeded in 217ms:
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.14s
     Running tests/manifest.rs (target/debug/deps/manifest-884beac56b4eeb6d)

running 10 tests
test empty_manifest_roundtrips ... ok
test schema_version_mismatch_rejected ... ok
test sha256_mismatch_returns_schema_error ... ok
test atomic_crash_after_rename_before_parent_fsync ... ok
test atomic_crash_before_rename_leaves_tmp ... ok
test generated_manifest_validates_against_schema ... ok
test changes_dir_digest_is_deterministic ... ok
test changes_dir_digest_flattens_subdirs ... ok
test append_and_reload_preserves_entries ... ok
test orphan_sweep_drops_unreferenced_entries ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s


 succeeded in 4079ms:
cargo clippy -- -D warnings
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.12s
cargo test
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.12s
     Running unittests src/lib.rs (target/debug/deps/loker-e0bbd5653d27b3fc)

running 656 tests
test aggregator::concat::tests::concat_kind_maps_to_strategy_label ... ok
test aggregator::concat::tests::llm_judge_kind_maps_to_strategy_label ... ok
test aggregator::concat::tests::vote_kind_maps_to_strategy_label ... ok
test aggregator::llm_judge::tests::llm_judge_family_diverse_ok ... ok
test aggregator::llm_judge::tests::llm_judge_family_overlap_opt_out_warns ... ok
test aggregator::llm_judge::tests::llm_judge_family_overlap_blocks ... ok
test aggregator::concat::tests::concat_renders_success_sections_in_input_order ... ok
test aggregator::concat::tests::concat_preserves_braced_unknown_expressions_containing_known_tokens ... ok
test aggregator::concat::tests::concat_does_not_reexpand_placeholders_inside_metadata ... ok
test aggregator::concat::tests::concat_whitespace_only_success_output_keeps_newline_invariants ... ok
test aggregator::concat::tests::concat_empty_input_returns_sentinel ... ok
test aggregator::concat::tests::concat_escapes_multiline_failure_reason ... ok
test aggregator::concat::tests::concat_normalizes_crlf_failure_reason ... ok
test aggregator::concat::tests::concat_counts_success_and_failure ... ok
test aggregator::llm_judge::tests::llm_judge_parse_malformed_json ... ok
test aggregator::concat::tests::concat_preserves_unknown_placeholders ... ok
test aggregator::llm_judge::tests::llm_judge_parse_out_of_bounds_index ... ok
test aggregator::llm_judge::tests::llm_judge_parse_missing_chosen_index ... ok
test aggregator::llm_judge::tests::llm_judge_parse_missing_reason ... ok
test aggregator::llm_judge::tests::llm_judge_parse_markdown_fenced_ballot ... ok
test aggregator::llm_judge::tests::llm_judge_parse_negative_chosen_index ... ok
test aggregator::llm_judge::tests::llm_judge_parse_out_of_bounds_index_clamped ... ok
test aggregator::llm_judge::tests::llm_judge_parse_valid_ballot ... ok
test aggregator::llm_judge::tests::llm_judge_parse_within_bounds_index_clamped ... ok
test aggregator::llm_judge::tests::llm_judge_parse_zero_candidates_index ... ok
test aggregator::tests::extra_keys_ok ... ok
test aggregator::tests::empty_text ... ok
test aggregator::tests::markdown_fenced_fail ... ok
test aggregator::tests::markdown_fenced_json ... ok
test aggregator::tests::missing_pass ... ok
test aggregator::llm_judge::tests::llm_judge_prompt_includes_phase_name ... ok
test aggregator::tests::pass_false ... ok
test aggregator::llm_judge::tests::llm_judge_prompt_renders_candidates ... ok
test aggregator::tests::pass_true ... ok
test aggregator::tests::wrong_pass_type ... ok
test aggregator::vote::tests::all_abstain ... ok
test aggregator::vote::tests::abstain_backend_error ... ok
test aggregator::vote::tests::closest_family_multiple_matching_buckets ... ok
test aggregator::vote::tests::closest_family_no_match_fallback ... ok
test aggregator::vote::tests::empty_input ... ok
test aggregator::vote::tests::empty_ballot_counts_as_abstain ... ok
test aggregator::vote::tests::free_text_clear_winner ... ok
test aggregator::vote::tests::closest_family_multiple_buckets_match ... ok
test aggregator::vote::tests::normalise_ballot_basic ... ok
test aggregator::vote::tests::free_text_tie_closest_family ... ok
test aggregator::vote::tests::free_text_tie_first_responder ... ok
test aggregator::vote::tests::quorum_lost ... ok
test aggregator::vote::tests::normalise_whitespace ... ok
test aggregator::vote::tests::normalise_case ... ok
test aggregator::vote::tests::vote_counts_sorted_descending ... ok
test aggregator::vote::tests::free_text_tie_random_deterministic ... ok
test aggregator::vote::tests::sanitize_comment_in_metadata ... ok
test aggregator::vote::tests::whitespace_only_ballot_counts_as_abstain ... ok
test apply_verify::diff_applier::tests::test_apply_empty_edits ... ok
test apply_verify::diff_applier::tests::test_apply_empty_file_path_is_invalid_edit ... ok
test apply_verify::edit_parser::tests::test_crlf_normalization ... ok
test apply_verify::edit_parser::tests::test_detect_diff ... ok
test apply_verify::edit_parser::tests::test_detect_full_file ... ok
test apply_verify::edit_parser::tests::test_detect_json_array ... ok
test apply_verify::edit_parser::tests::test_detect_json_object ... ok
test apply_verify::diff_applier::tests::test_apply_file_not_found ... ok
test apply_verify::edit_parser::tests::test_detect_with_lang_hint_diff ... ok
test apply_verify::diff_applier::tests::test_apply_rejects_absolute_path ... ok
test apply_verify::edit_parser::tests::test_detect_with_lang_hint_json ... ok
test apply_verify::edit_parser::tests::test_diff_no_hunks ... ok
test apply_verify::edit_parser::tests::test_diff_context_lines ... ok
test apply_verify::edit_parser::tests::test_diff_multi_file ... ok
test apply_verify::edit_parser::tests::test_diff_no_newline_marker ... ok
test apply_verify::edit_parser::tests::test_diff_strips_ab_prefix ... ok
test apply_verify::edit_parser::tests::test_diff_single_file ... ok
test apply_verify::edit_parser::tests::test_empty_input ... ok
test apply_verify::edit_parser::tests::test_full_file ... ok
test apply_verify::edit_parser::tests::test_full_file_empty_path ... ok
test apply_verify::edit_parser::tests::test_full_file_no_path ... ok
test apply_verify::edit_parser::tests::test_full_file_with_dash_header ... ok
test apply_verify::edit_parser::tests::test_json_agentic_output ... ok
test apply_verify::edit_parser::tests::test_input_too_large ... ok
test apply_verify::diff_applier::tests::test_apply_rejects_path_traversal ... ok
test apply_verify::edit_parser::tests::test_json_bare_array ... ok
test apply_verify::edit_parser::tests::test_json_empty_edits ... ok
test apply_verify::edit_parser::tests::test_json_malformed ... ok
test apply_verify::edit_parser::tests::test_json_trailing_newlines_normalized ... ok
test apply_verify::edit_parser::tests::test_json_control_chars ... ok
test apply_verify::diff_applier::tests::test_apply_old_text_not_found ... ok
test apply_verify::edit_parser::tests::test_json_with_message_field ... ok
test apply_verify::edit_parser::tests::test_malformed_diff ... ok
test apply_verify::diff_applier::tests::test_apply_ambiguous_match ... ok
test apply_verify::diff_applier::tests::test_apply_empty_old_in_find_replace_is_invalid ... ok
test apply_verify::edit_parser::tests::test_markdown_backticks_in_content ... ok
test apply_verify::edit_parser::tests::test_markdown_diff_block ... ok
test apply_verify::edit_parser::tests::test_markdown_json_block ... ok
test apply_verify::edit_parser::tests::test_markdown_generic_block ... ok
test apply_verify::edit_parser::tests::test_whitespace_only_input ... ok
test apply_verify::diff_applier::tests::test_apply_json_single_file ... ok
test apply_verify::diff_applier::tests::test_apply_full_file_overwrite ... ok
test apply_verify::diff_applier::tests::test_apply_partial_failure ... ok
test apply_verify::diff_applier::tests::test_apply_unified_diff_multi_hunk_fails ... ok
test apply_verify::retry_loop::tests::test_parse_error_stop ... ok
test apply_verify::diff_applier::tests::test_apply_full_file_create_new ... ok
test apply_verify::diff_applier::tests::test_apply_unified_diff_single_hunk ... ok
test apply_verify::rollback::tests::test_is_fully_restored_false ... ok
test apply_verify::rollback::tests::test_is_fully_restored_true ... ok
test apply_verify::diff_applier::tests::test_apply_multi_file_success ... ok
test apply_verify::retry_loop::tests::test_apply_partial_failure_rolls_back ... ok
test apply_verify::rollback::tests::test_rollback_delete_tolerates_already_missing ... ok
test apply_verify::rollback::tests::test_rollback_continues_on_failure ... ok
test apply_verify::rollback::tests::test_rollback_deletes_new_file ... ok
test apply_verify::rollback::tests::test_rollback_empty_result_is_noop ... ok
test apply_verify::rollback::tests::test_rollback_single_file ... ok
test apply_verify::rollback::tests::test_rollback_mixed_restore_and_delete ... ok
test apply_verify::rollback::tests::test_rollback_reverse_order ... ok
test aggregator::concat::tests::concat_mixed_success_failure_snapshot ... ok
test aggregator::vote::tests::vote_snapshot ... ok
test apply_verify::retry_loop::tests::test_parse_error_on_last_retry_exits ... ok
test apply_verify::verification::tests::test_verify_captures_both_streams ... ok
test apply_verify::retry_loop::tests::test_parse_error_retries ... ok
test apply_verify::retry_loop::tests::test_success_first_attempt ... ok
test apply_verify::retry_loop::tests::test_requester_error_surfaced ... ok
test apply_verify::verification::tests::test_verify_captures_stderr ... ok
test apply_verify::retry_loop::tests::test_max_retries_zero_runs_once ... ok
test backend::claude::tests::capabilities_match_current_wiring ... ok
test backend::claude::tests::test_claude_response_deserialize_with_usage ... ok
test backend::claude::tests::test_claude_response_deserialize_without_usage ... ok
test backend::codex::tests::capabilities_match_current_wiring ... ok
test apply_verify::retry_loop::tests::test_apply_error_triggers_rollback_and_retry ... ok
test apply_verify::retry_loop::tests::test_verify_failure_triggers_rollback ... ok
test backend::genai_error::tests::classify_404_body_detects_unknown_function_fixture ... ok
test backend::gemini::tests::capabilities_match_current_wiring ... ok
test backend::genai_error::tests::classify_5xx_body_detects_anthropic_auth_fixture ... ok
test backend::genai_error::tests::classify_5xx_body_returns_none_for_generic_5xx ... ok
test backend::genai_error::tests::classify_5xx_body_detects_rate_limit_signature ... ok
test backend::genai_error::tests::contains_status_code_handles_punctuation_boundaries ... ok
test backend::genai_error::tests::map_status_401_to_auth ... ok
test backend::genai_error::tests::map_status_403_to_auth ... ok
test backend::genai_error::tests::map_status_404_other_to_execution_failed ... ok
test backend::genai_error::tests::map_status_404_unknown_function_to_config ... ok
test backend::genai_error::tests::map_status_429_to_rate_limit_retryable ... ok
test backend::genai_error::tests::map_status_500_to_network_retryable ... ok
test backend::genai_error::tests::map_status_502_upstream_auth_to_auth_not_retryable ... ok
test backend::genai_error::tests::map_status_502_generic_to_network_retryable ... ok
test backend::genai_error::tests::map_status_502_upstream_rate_limit_to_rate_limit_retryable ... ok
test backend::genai_error::tests::map_status_503_to_network_retryable ... ok
test backend::genai_error::tests::map_status_unknown_to_execution_failed ... ok
test backend::ollama::tests::test_ollama_response_deserialize_with_counts ... ok
test apply_verify::verification::tests::test_verify_captures_stdout ... ok
test backend::ollama::tests::test_ollama_response_deserialize_partial_counts ... ok
test backend::ollama::tests::test_ollama_response_deserialize_without_model ... ok
test backend::retry::tests::test_get_delay_attempt_zero_is_zero ... ok
test backend::retry::tests::test_get_delay_clamped_at_max ... ok
test backend::retry::tests::test_retry_executor_does_not_retry_non_retryable ... ok
test backend::retry::tests::test_get_delay_grows_exponentially ... ok
test apply_verify::verification::tests::test_verify_failure_exit_code ... ok
test backend::tensorzero::tests::canonicalize_wire_model_strips_to_canonical_on_wire ... ok
test backend::tensorzero::tests::capabilities_match_current_wiring ... ok
test apply_verify::retry_loop::tests::test_integration_end_to_end ... ok
test backend::retry::tests::test_retry_exhausted ... ok
test backend::retry::tests::test_retry_success_after_failures ... ok
test apply_verify::verification::tests::test_verify_uses_passed_cwd ... ok
test apply_verify::verification::tests::test_verify_invalid_command_exits_127 ... ok
test apply_verify::retry_loop::tests::test_max_retries_exhausted ... ok
test apply_verify::verification::tests::test_verify_success ... ok
test apply_verify::retry_loop::tests::test_success_on_retry_after_verify_failure ... ok
test apply_verify::verification::tests::test_verify_output_truncated ... ok
test apply_verify::retry_loop::tests::test_attempt_records ... ok
test backend::ollama::tests::capabilities_match_current_wiring ... ok
test backend::tensorzero::tests::name_is_tensorzero ... ok
test backend::tensorzero::tests::normalize_endpoint_appends_when_missing ... ok
test backend::tensorzero::tests::normalize_endpoint_does_not_double_suffix ... ok
test backend::tensorzero::tests::maps_502_generic_to_network_retryable ... ok
test backend::tensorzero::tests::maps_429_to_rate_limit_retryable ... ok
test backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime ... ok
test backend::tests::backend_capabilities_none_is_all_false ... ok
test backend::tensorzero::tests::maps_401_to_auth_not_retryable ... ok
test backend::tests::capabilities_for_name_matches_static_expectations ... ok
test backend::tests::capabilities_for_name_unknown_returns_none ... ok
test backend::tests::default_capabilities_are_none ... ok
test backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable ... ok
test backend::tests::tensorzero_adapter_allows_missing_api_key_env_field ... ok
test backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable ... ok
test backend::tests::tensorzero_adapter_maps_endpoint_model_auth_timeout ... ok
test backend::tensorzero::tests::maps_malformed_json_to_parse_error ... ok
test backend::tests::tensorzero_adapter_rejects_missing_endpoint_model_zero_timeout_and_bad_scheme ... ok
test backend::tests::test_backend_error_display ... ok
test backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable ... ok
test backend::tests::test_backend_error_from_anyhow ... ok
test backend::tensorzero::tests::maps_500_to_retryable_error ... ok
test backend::tests::test_backend_error_not_retryable ... ok
test backend::tests::test_backend_error_retryable ... ok
test backend::tests::test_query_output_from_process_empty_stderr_normalized ... ok
test backend::tests::test_query_output_from_process_empty_stdout ... ok
test backend::tests::test_query_output_from_process_populates_backend_and_duration ... ok
test backend::tests::test_query_output_from_process_with_stderr ... ok
test backend::tests::tensorzero_create_backend_supported_when_capability_supported ... ok
test backend::tests::test_query_output_from_text ... ok
test backend::tests::test_query_output_from_text_populates_backend_and_duration ... ok
test backend::tests::test_query_output_with_model_none ... ok
test backend::tests::test_query_output_with_model_some ... ok
test backend::tests::test_query_output_with_structured_none ... ok
test backend::tests::test_query_output_with_structured_some ... ok
test backend::tests::test_query_output_with_usage_none ... ok
test backend::tests::test_query_output_with_usage_some ... ok
test backend::tests::test_token_usage_default_zero ... ok
test backend::tests::test_token_usage_new_computes_total ... ok
test backend::tests::test_token_usage_new_saturates_on_overflow ... ok
test backend::tests::test_token_usage_saturating_add ... ok
test backend::tests::with_elapsed_is_idempotent_on_repeated_calls ... ok
test backend::tests::with_elapsed_is_noop_on_non_timeout_variants ... ok
test backend::tests::with_elapsed_overrides_timeout_elapsed_ms ... ok
test cache::tests::test_cache_disabled ... ok
test cache::tests::test_cache_key_deterministic ... ok
test cache::tests::test_cache_key_different_backends ... ok
test cache::tests::test_cache_key_different_prompts ... ok
test config::tests::test_claude_backend_defaults ... ok
test config::tests::test_codex_backend_defaults ... ok
test config::tests::test_command_wrapper_default_none ... ok
test config::tests::test_conductor_defaults ... ok
test config::tests::test_command_wrapper_docker_example ... ok
test config::tests::test_command_wrapper_config ... ok
test config::tests::test_backend_config_defaults ... ok
test config::tests::test_conductor_custom_config ... ok
test config::tests::test_deep_merge_boolean_override ... ok
test config::tests::test_deep_merge_empty_overlay ... ok
test config::tests::test_deep_merge_scalar_override ... ok
test backend::tensorzero::tests::returns_text_on_200_success ... ok
test backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model ... ok
test config::tests::test_default_config ... ok
test config::tests::test_deep_merge_partial_config ... ok
test config::tests::test_deep_merge_hashmap_override ... ok
test config::tests::test_deep_merge_hashmap_add ... ok
test cache::tests::test_cache_warnings_on_parse_failure ... ok
test config::tests::test_deny_unknown_fields ... ok
test config::tests::test_gemini_backend_defaults ... ok
test config::tests::test_hunt_task_defaults ... ok
test config::tests::test_deep_merge_vec_replace ... ok
test cache::tests::test_cache_warnings_deduplicated ... ok
test config::tests::test_parse_custom_backend ... ok
test config::tests::test_parse_minimal_config ... ok
test backend::tests::tensorzero_create_backend_queries_wiremock_gateway ... ok
test config::tests::test_parse_custom_task ... ok
test config::tests::test_tensorzero_to_backend_opts_resolves_env ... ok
test config::tests::test_tensorzero_missing_endpoint_fails ... ok
test config::tests::test_tensorzero_invalid_url_fails ... ok
test config::tests::test_tensorzero_zero_timeout_fails ... ok
test consensus::tests::test_majority_vote_clear_winner ... ok
test consensus::tests::test_majority_vote_empty ... ok
test consensus::tests::test_majority_vote_tie_first_wins ... ok
test consensus::tests::test_weighted_vote ... ok
test consensus::tests::test_weighted_vote_clear_winner ... ok
test consensus::tests::test_whitespace_normalization ... ok
test config::tests::test_load_config_from_paths_no_files ... ok
test family::tests::aggregator_rejected_display ... ok
test family::tests::as_str_other ... ok
test family::tests::as_str_openai ... ok
test family::tests::display_other ... ok
test family::tests::display_anthropic ... ok
test config::tests::test_config_serialization_roundtrip ... ok
test family::tests::enforce_all_anthropic_rejected ... ok
test family::tests::enforce_distinct_other_ok ... ok
test family::tests::enforce_empty_slice_ok ... ok
test family::tests::enforce_mixed_families_ok ... ok
test family::tests::enforce_cross_family_deterministic ... ok
test family::tests::enforce_same_other_rejected ... ok
test family::tests::enforce_single_backend_ok ... ok
test family::tests::enforce_three_same_family ... ok
test family::tests::enforce_two_distinct_others_ok ... ok
test family::tests::family_of_bedrock ... ok
test family::tests::family_of_codex ... ok
test family::tests::family_of_claude ... ok
test family::tests::family_of_loker_no_suffix ... ok
test family::tests::family_of_empty_string ... ok
test family::tests::family_of_gemini ... ok
test family::tests::family_of_loker_prefix_anthropic ... ok
test family::tests::family_of_loker_prefix_google ... ok
test family::tests::family_of_loker_prefix_gemini ... ok
test family::tests::family_of_loker_prefix_local ... ok
test family::tests::family_of_loker_prefix_ollama ... ok
test config::tests::test_load_config_from_paths_explicit_bypasses ... ok
test family::tests::family_of_loker_prefix_openai ... ok
test family::tests::family_of_ollama ... ok
test config::tests::test_tensorzero_config_serialization_roundtrip ... ok
test family::tests::family_of_openai ... ok
test family::tests::family_of_loker_zhipu_suffix ... ok
test family::tests::family_of_tensorzero ... ok
test family::tests::family_of_tensorzero_function_name ... ok
test family::tests::family_of_tensorzero_slash_only ... ok
test family::tests::family_of_tensorzero_unknown_suffix ... ok
test family::tests::family_of_tensorzero_zhipu_suffix ... ok
test family::tests::family_of_unknown ... ok
test family::tests::family_of_zhipu ... ok
test family::tests::judge_unavailable_display ... ok
test family::tests::quorum_lost_display ... ok
test manifest::tests::empty_manifest_roundtrips ... ok
test manifest::tests::sha256_hex_matches_known_vector ... ok
test role::tests::test_backend_filtering ... ok
test role::tests::test_resolution_builder ... ok
test role::tests::test_resolution_is_empty ... ok
test role::tests::test_role_config_new ... ok
test role::tests::test_role_resolution_error_display ... ok
test role::tests::test_role_resolver_default_team ... ok
test role::tests::test_role_config_serialization ... ok
test role::tests::test_role_resolver_no_backends_available ... ok
test role::tests::test_role_resolver_resolve_global_role ... ok
test role::tests::test_role_resolver_role_not_found ... ok
test role::tests::test_role_resolver_team_override ... ok
test role::tests::test_role_resolver_team_override_takes_precedence ... ok
test role::tests::test_role_resolver_team_can_define_custom_role ... ok
test context::tests::test_no_context ... ok
test role::tests::test_routing_strategy_default_is_fallback ... ok
test role::tests::test_team_config_default ... ok
test config::tests::test_load_config_from_paths_project_only ... ok
test role::tests::test_valid_parallel_config ... ok
test role::tests::test_validation_parallel_min_success_exceeds_backends ... ok
test role::tests::test_validation_parallel_min_success_too_low ... ok
test role::tests::test_validation_unknown_backend ... ok
test role::tests::test_team_config_serialization ... ok
test strategy::escalating_retry::tests::config_default_false ... ok
test strategy::escalating_retry::tests::config_round_trip_false ... ok
test strategy::escalating_retry::tests::config_round_trip_true ... ok
test git_agent::tests::test_is_initialized_false_for_nonexistent ... ok
test context::tests::test_detect_rails_with_goldiloader ... ok
test git_agent::tests::test_is_available_returns_bool ... ok
test context::tests::test_detect_typescript ... ok
test config::tests::test_load_config_from_paths_user_parse_error ... ok
test config::tests::test_load_config_from_paths_three_layers ... ok
test strategy::escalating_retry::tests::redaction_bearer_token ... ok
test strategy::escalating_retry::tests::redaction_aws_key ... ok
test strategy::escalating_retry::tests::redaction_api_key_value ... ok
test strategy::escalating_retry::tests::envelope_verify_reason_only_when_no_response ... ok
test strategy::escalating_retry::tests::envelope_backend_error_shows_null_response ... ok
test strategy::escalating_retry::tests::envelope_under_budget_no_truncation ... ok
test strategy::escalating_retry::tests::truncate_exact_boundary ... ok
test strategy::escalating_retry::tests::envelope_hard_caps_when_body_alone_exceeds_budget ... ok
test strategy::escalating_retry::tests::truncate_multibyte_safe ... ok
test strategy::escalating_retry::tests::truncate_no_op_when_under_budget ... ok
test strategy::escalating_retry::tests::truncate_with_suffix_fits_within_budget ... ok
test strategy::escalating_retry::tests::redaction_does_not_false_positive_short_text ... ok
test strategy::future_variant_compiles::stub_fan_out_implements_strategy ... ok
test strategy::escalating_retry::tests::redaction_long_blob_heuristic ... ok
test strategy::parallel_fanout::tests::any_fail_markdown_fenced_json ... ok
test strategy::parallel_fanout::tests::any_fail_markdown_fenced_fail ... ok
test strategy::parallel_fanout::tests::any_fail_all_pass ... ok
test strategy::parallel_fanout::tests::any_fail_valid_json_extra_keys ... ok
test strategy::escalating_retry::tests::envelope_over_budget_truncates_excerpt ... ok
test strategy::parallel_fanout::tests::backend_not_found ... ok
test strategy::parallel_fanout::tests::empty_targets_yields_no_backends ... ok
test strategy::parallel_fanout::tests::floor_violation ... ok
test strategy::parallel_fanout::tests::happy_path_all_succeed ... ok
test strategy::parallel_fanout::tests::one_fails_floor_still_met ... ok
test strategy::parallel_fanout::tests::any_fail_first_fails ... ok
test strategy::parallel_fanout::tests::any_fail_all_fail ... ok
test strategy::parallel_fanout::tests::any_fail_backend_error_treated_as_failure ... ok
test strategy::verify::run_command::tests::run_command_builder_api ... ok
test strategy::parallel_fanout::tests::any_fail_missing_pass_field ... ok
test strategy::verify::run_command::tests::run_command_default_values ... ok
test strategy::parallel_fanout::tests::prompt_render_failure_no_dispatch ... ok
test strategy::parallel_fanout::tests::any_fail_empty_query_text ... ok
test strategy::parallel_fanout::tests::any_fail_wrong_pass_type ... ok
test strategy::parallel_fanout::tests::vote_quorum_lost ... ok
test strategy::parallel_fanout::tests::any_fail_non_deterministic_offender ... ok
test strategy::verify::test_runner::tests::cargo_2_pass_1_fail ... ok
test strategy::verify::test_runner::tests::cargo_3_pass_0_fail ... ok
test strategy::verify::test_runner::tests::cargo_empty_no_tests ... ok
test strategy::verify::test_runner::tests::cargo_first_failure_preserves_stdout_excerpt ... ok
test strategy::verify::run_command::tests::verify_missing_command_fails ... ok
test strategy::verify::test_runner::tests::cargo_first_failure_truncates_utf8_excerpt_safely ... ok
test strategy::verify::test_runner::tests::cargo_malformed_json_line_skipped ... ok
test strategy::verify::test_runner::tests::cargo_skips_compiler_messages ... ok
test strategy::verify::test_runner::tests::pytest_4_pass_2_fail ... ok
test strategy::verify::test_runner::tests::pytest_5_pass_0_fail ... ok
test strategy::verify::test_runner::tests::pytest_empty_no_tests ... ok
test strategy::verify::test_runner::tests::pytest_missing_summary_field ... ok
test strategy::verify::test_runner::tests::pytest_non_json_output ... ok
test strategy::verify::test_runner::tests::verify_result_from_passing_tests ... ok
test strategy::verify::test_runner::tests::verify_result_from_failing_tests ... ok
test strategy::verify::test_runner::tests::verify_result_no_tests_ran ... ok
test strategy::verify::test_runner::tests::verify_result_timed_out ... ok
test strategy::verify::verify::tests::failure_reason_builder_api ... ok
test strategy::verify::test_runner::tests::verify_result_killed_by_signal ... ok
test strategy::verify::verify::tests::failure_reason_display ... ok
test strategy::verify::verify::tests::reserved_repair_compiles_but_not_pass ... ok
test strategy::verify::verify::tests::reserved_score_compiles_but_not_pass ... ok
test strategy::verify::verify::tests::stub_verify_hook_returns_error ... ok
test strategy::verify::verify::tests::stub_verify_hook_returns_fail ... ok
test strategy::verify::verify::tests::stub_verify_hook_returns_fail_with_full_reason ... ok
test strategy::verify::verify::tests::stub_verify_hook_returns_pass ... ok
test strategy::verify::verify::tests::verify_context_from_query_output ... ok
test template::context::tests::test_arg_out_of_bounds ... ok
test template::context::tests::test_arg_access ... ok
test template::context::tests::test_arg_zero_undefined ... ok
test template::context::tests::test_env_lookup ... ok
test template::context::tests::test_env_missing ... ok
test template::context::tests::test_loop_vars_object_item ... ok
test template::context::tests::test_loop_vars_string_item ... ok
test template::context::tests::test_loop_vars_preserve_existing_namespaces ... ok
test template::context::tests::test_step_field_fallback_no_parsed_output ... ok
test template::context::tests::test_step_field_with_parsed_output ... ok
test template::context::tests::test_step_output ... ok
test template::context::tests::test_step_success_false ... ok
test template::context::tests::test_step_success_true ... ok
test template::context::tests::test_workflow_backends ... ok
test template::filters::tests::test_default_val_defined ... ok
test template::context::tests::test_workflow_backends_empty ... ok
test template::filters::tests::test_default_val_empty_string ... ok
test template::filters::tests::test_default_val_undefined ... ok
test template::filters::tests::test_first_empty ... ok
test template::filters::tests::test_first_normal ... ok
test template::filters::tests::test_first_single ... ok
test template::filters::tests::test_join_default_separator ... ok
test template::filters::tests::test_join_empty ... ok
test template::filters::tests::test_join_with_separator ... ok
test template::filters::tests::test_json_encode_nested ... ok
test template::filters::tests::test_json_encode_number ... ok
test template::filters::tests::test_json_encode_string ... ok
test template::filters::tests::test_last_empty ... ok
test template::filters::tests::test_last_normal ... ok
test template::filters::tests::test_last_single ... ok
test template::filters::tests::test_lines_empty ... ok
test template::filters::tests::test_lines_multiline ... ok
test template::filters::tests::test_lines_single ... ok
test template::filters::tests::test_shell_escape_backticks_and_dollar ... ok
test template::filters::tests::test_shell_escape_basic ... ok
test template::filters::tests::test_shell_escape_injection ... ok
test template::filters::tests::test_shell_escape_newlines ... ok
test template::filters::tests::test_shell_escape_null_bytes ... ok
test template::filters::tests::test_shell_escape_single_quotes ... ok
test template::filters::tests::test_shell_escape_unicode ... ok
test template::filters::tests::test_trim_already_trimmed ... ok
test template::filters::tests::test_trim_newlines ... ok
test template::filters::tests::test_trim_whitespace ... ok
test template::tests::test_eval_expression_falsy ... ok
test template::tests::test_combined_env_arg_step ... ok
test template::tests::test_eval_expression_truthy ... ok
test template::tests::test_eval_expression_undefined ... ok
test template::tests::test_no_reexpansion_of_braces_in_output ... ok
test template::tests::test_parse_error ... ok
test utils::tests::test_backend_error_kind_from_typed ... ok
test template::tests::test_undefined_variable ... ok
test utils::tests::test_classify_auth_401 ... ok
test template::tests::test_render_mixed ... ok
test utils::tests::test_classify_auth_invalid_key ... ok
test utils::tests::test_classify_capacity_exhausted ... ok
test utils::tests::test_classify_network_refused ... ok
test utils::tests::test_classify_not_installed ... ok
test utils::tests::test_classify_rate_limit_429 ... ok
test utils::tests::test_classify_rate_limit_quota ... ok
test utils::tests::test_classify_resource_exhausted ... ok
test utils::tests::test_classify_unknown ... ok
test utils::tests::test_summarize_capacity ... ok
test utils::tests::test_redact_secrets_aws_key ... ok
test utils::tests::test_summarize_rate_limit ... ok
test utils::tests::test_summarize_typed_backend_error ... ok
test utils::tests::test_truncate_exact_length ... ok
test utils::tests::test_redact_secrets_bearer_token ... ok
test utils::tests::test_summarize_unknown_truncates ... ok
test utils::tests::test_truncate_long_string ... ok
test utils::tests::test_truncate_short_string ... ok
test utils::tests::test_truncate_unicode ... ok
test utils::tests::test_truncate_utf8_ascii ... ok
test utils::tests::test_truncate_utf8_empty_string ... ok
test utils::tests::test_truncate_utf8_exact_boundary ... ok
test utils::tests::test_truncate_utf8_multibyte_boundary ... ok
test utils::tests::test_truncate_utf8_within_limit ... ok
test utils::tests::test_truncate_utf8_zero_cap ... ok
test workflow::tests::required_capabilities_returns_empty_for_plain_step ... ok
test workflow::tests::required_capabilities_returns_file_edit_for_apply_edits ... ok
test workflow::tests::test_apply_lenient_mode_empty_response_fails ... ok
test utils::tests::test_redact_secrets_api_key_value ... ok
test workflow::tests::test_apply_lenient_mode_non_empty_passes_with_cleaned_output ... ok
test workflow::tests::test_apply_lenient_mode_preserves_internal_whitespace ... ok
test workflow::tests::test_apply_lenient_mode_whitespace_only_fails ... ok
test strategy::parallel_fanout::tests::any_fail_mid_list_fails ... ok
test workflow::tests::test_apply_parse_error_policy_default_fails ... ok
test workflow::tests::test_apply_parse_error_policy_explicit_fail_matches_default ... ok
test workflow::tests::test_apply_parse_error_policy_pass_succeeds_without_output ... ok
test workflow::tests::test_apply_parse_error_policy_skip_drops_validation ... ok
test workflow::tests::test_apply_parse_error_policy_unknown_value_falls_back_to_fail ... ok
test workflow::tests::test_build_apply_fix_prompt_includes_partial_paths ... ok
test workflow::tests::test_build_parse_fix_prompt_contains_previous_raw ... ok
test workflow::tests::test_build_verify_fix_prompt_with_exit_code ... ok
test workflow::tests::test_build_verify_fix_prompt_with_timeout_uses_timeout_string ... ok
test workflow::tests::test_apply_once_parse_error_returns_err ... ok
test manifest::tests::atomic_write_and_read ... ok
test workflow::tests::test_apply_once_apply_error_rolls_back ... ok
test strategy::parallel_fanout::tests::vote_success ... ok
test strategy::verify::run_command::tests::verify_false_fails_with_code ... ok
test workflow::tests::test_apply_once_success_without_format ... ok
test strategy::verify::run_command::tests::verify_echo_passes ... ok
test workflow::tests::test_continue_on_error_toml_parsing ... ok
test strategy::parallel_fanout::tests::vote_tie_random_deterministic ... ok
test workflow::tests::test_duplicate_step_names_error ... ok
test workflow::tests::test_extract_json_field_bool ... ok
test workflow::tests::test_extract_json_field_multiline ... ok
test workflow::tests::test_extract_json_field_not_found ... ok
test workflow::tests::test_extract_json_field_number ... ok
test workflow::tests::test_extract_json_field_string ... ok
test workflow::tests::test_extract_json_from_markdown_block ... ok
test workflow::tests::test_extract_json_from_plain_block ... ok
test workflow::tests::test_extract_json_raw ... ok
test workflow::tests::test_extract_json_with_literal_newlines ... ok
test workflow::tests::test_extract_json_with_text_before ... ok
test workflow::tests::test_find_closing_fence ... ok
test workflow::tests::test_evaluate_condition_error_recovery ... ok
test workflow::tests::test_condition_unparseable_returns_true ... ok
test workflow::tests::test_condition_steps_success ... ok
test workflow::tests::test_group_by_depth_forward_declared_dependency ... ok
test workflow::tests::test_heuristic_contains_double_quotes ... ok
test workflow::tests::test_heuristic_contains_empty_string_always_passes ... ok
test workflow::tests::test_heuristic_contains_fail ... ok
test workflow::tests::test_heuristic_contains_pass ... ok
test workflow::tests::test_heuristic_contains_single_quote_char ... ok
test workflow::tests::test_condition_equals ... ok
test workflow::tests::test_condition_contains ... ok
test workflow::tests::test_heuristic_contains_special_chars ... ok
test workflow::tests::test_heuristic_empty_check_string ... ok
test workflow::tests::test_condition_legacy_syntax ... ok
test workflow::tests::test_heuristic_min_length_fail ... ok
test workflow::tests::test_heuristic_min_length_invalid_arg ... ok
test workflow::tests::test_heuristic_min_length_pass ... ok
test workflow::tests::test_heuristic_min_length_unicode ... ok
test workflow::tests::test_heuristic_min_length_whitespace_counts ... ok
test workflow::tests::test_heuristic_min_length_zero_always_passes ... ok
test workflow::tests::test_condition_not ... ok
test workflow::tests::test_heuristic_not_empty_fail_empty ... ok
test workflow::tests::test_heuristic_not_empty_fail_whitespace ... ok
test workflow::tests::test_heuristic_not_empty_pass ... ok
test workflow::tests::test_heuristic_unknown_check ... ok
test workflow::tests::test_for_each_parsed_output_not_array ... ok
test workflow::tests::test_for_each_with_parsed_output ... ok
test workflow::tests::test_condition_json_field_access ... ok
test workflow::tests::test_interpolate_validation_prompt_basic ... ok
test workflow::tests::test_interpolate_validation_prompt_injection_safety ... ok
test workflow::tests::test_interpolate_validation_prompt_no_stderr ... ok
test workflow::tests::test_interpolate_validation_prompt_no_truncation_when_under_limit ... ok
test workflow::tests::test_interpolate_validation_prompt_truncation ... ok
test workflow::tests::test_interpolate_validation_prompt_with_stderr ... ok
test workflow::tests::test_interpolate_loop_vars_index ... ok
test workflow::tests::test_interpolate_loop_vars_item_string ... ok
test workflow::tests::test_interpolate_loop_vars_item_whole_object ... ok
test workflow::tests::test_interpolate_loop_vars_missing_field ... ok
test workflow::tests::test_interpolate_loop_vars_combined ... ok
test workflow::tests::test_interpolate_loop_vars_multiple_fields_one_missing ... ok
test workflow::tests::test_interpolate_loop_vars_item_object ... ok
test workflow::tests::test_interpolate_parsed_output_none_fallback ... ok
test workflow::tests::test_interpolate_with_fields_json ... ok
test workflow::tests::test_load_error_tracker_backoff_progression ... ok
test workflow::tests::test_jinja_chained_filters ... ok
test workflow::tests::test_jinja_default_filter ... ok
test workflow::tests::test_load_error_tracker_bail_at_threshold ... ok
test workflow::tests::test_load_error_tracker_reset_on_success ... ok
test workflow::tests::test_jinja_if_block ... ok
test workflow::tests::test_load_error_tracker_success_with_no_prior_errors ... ok
test workflow::tests::test_map_retry_failure_apply_error_with_paths ... ok
test workflow::tests::test_jinja_missing_step_default_fallback ... ok
test workflow::tests::test_jinja_join_filter ... ok
test workflow::tests::test_jinja_trim_filter ... ok
test workflow::tests::test_map_retry_failure_apply_error_without_paths ... ok
test workflow::tests::test_jinja_inline_for_loop ... ok
test workflow::tests::test_map_retry_failure_attempt_count_from_retries ... ok
test workflow::tests::test_map_retry_failure_empty_attempts ... ok
test workflow::tests::test_jinja_shell_escape_filter ... ok
test workflow::tests::test_map_retry_failure_parse_error ... ok
test workflow::tests::test_map_retry_failure_verify_exit_code ... ok
test workflow::tests::test_map_retry_failure_verify_has_priority_over_apply ... ok
test workflow::tests::test_map_retry_failure_stderr_truncated_to_1kb ... ok
test workflow::tests::test_map_retry_failure_verify_timeout ... ok
test workflow::tests::test_map_template_error_reports_offending_variable_in_multi_expression ... ok
test workflow::tests::test_parse_for_each_inline_array ... ok
test workflow::tests::test_output_format_toml_parsing ... ok
test workflow::tests::test_min_deps_success_without_depends_on_error ... ok
test workflow::tests::test_parse_for_each_inline_array_objects ... ok
test workflow::tests::test_min_deps_success_validation_empty_deps ... ok
test workflow::tests::test_parse_step_output_json ... ok
test workflow::tests::test_min_deps_success_validation_valid ... ok
test workflow::tests::test_min_deps_success_validation_exceeds_deps ... ok
test workflow::tests::test_parse_step_output_lines ... ok
test workflow::tests::test_parse_step_output_none ... ok
test workflow::tests::test_parse_step_output_text ... ok
test workflow::tests::test_parse_for_each_not_array ... ok
test workflow::tests::test_parse_for_each_step_not_found ... ok
test workflow::tests::test_parse_for_each_invalid_format ... ok
test workflow::tests::test_parse_validation_response_empty_string_is_error ... ok
test workflow::tests::test_parse_validation_response_invalid_status ... ok
test workflow::tests::test_parse_for_each_step_reference ... ok
test workflow::tests::test_parse_for_each_step_reference_with_code_block ... ok
test workflow::tests::test_parse_validation_response_json_fail ... ok
test workflow::tests::test_parse_validation_response_json_in_fences ... ok
test workflow::tests::test_parse_validation_response_json_pass ... ok
test workflow::tests::test_parse_validation_response_json_pass_no_output ... ok
test workflow::tests::test_parse_validation_response_review_failed ... ok
test workflow::tests::test_parse_validation_response_unrecognized_is_error ... ok
test workflow::tests::test_sanitize_json_strings ... ok
test workflow::tests::test_step_failure_kind_copy_eq ... ok
test workflow::tests::test_step_failure_kind_display ... ok
test workflow::tests::test_step_for_each_inline_array_toml ... ok
test workflow::tests::test_step_for_each_toml_parsing ... ok
test workflow::tests::test_step_result_error_edit_failed ... ok
test workflow::tests::test_step_result_error_backend_error ... ok
test workflow::tests::test_step_result_error_has_no_validation ... ok
test workflow::tests::test_step_result_error_output_matches_failure_message ... ok
test workflow::tests::test_step_if_alias ... ok
test workflow::tests::test_step_result_error_produces_failure ... ok
test workflow::tests::test_step_result_error_skipped ... ok
test workflow::tests::test_step_result_error_verify_failed ... ok
test workflow::tests::test_strip_markdown_fences_json ... ok
test workflow::tests::test_strip_markdown_fences_none ... ok
test workflow::tests::test_strip_markdown_fences_plain ... ok
test workflow::tests::test_strip_markdown_fences_with_whitespace ... ok
test workflow::tests::test_success_step_has_no_failure ... ok
test workflow::tests::test_parse_validate_config_absent ... ok
test workflow::tests::test_parse_validate_config_from_toml ... ok
test workflow::tests::test_parse_validate_config_mixed_fields ... ok
test workflow::tests::test_translate_contains_with_single_quoted_literal_containing_double_quote ... ok
test workflow::tests::test_translate_contains_with_escaped_quotes ... ok
test workflow::tests::test_translate_contains_call ... ok
test workflow::tests::test_parse_for_each_field_access ... ok
test workflow::tests::test_translate_equals_with_steps_prefix ... ok
test workflow::tests::test_translate_fast_path_whitespace_variants ... ok
test workflow::tests::test_translate_equals_call ... ok
test workflow::tests::test_translate_legacy_steps_output_contains ... ok
test workflow::tests::test_translate_legacy_double_quotes ... ok
test workflow::tests::test_translate_multiple_contains ... ok
test workflow::tests::test_translate_passthrough_already_valid ... ok
test workflow::tests::test_translate_passthrough_empty ... ok
test workflow::tests::test_translate_mixed_legacy_new ... ok
test workflow::tests::test_translate_contains_with_steps_prefix ... ok
test workflow::tests::test_truncate_for_prompt_over_limit ... ok
test workflow::tests::test_timeout_normal_value_allowed ... ok
test workflow::tests::test_truncate_for_prompt_under_limit ... ok
test workflow::tests::test_translate_nested_not ... ok
test workflow::tests::test_timeout_zero_allowed ... ok
test workflow::tests::test_validation_failure_has_no_step_failure ... ok
test workflow::tests::test_verify_command_composition_pattern ... ok
test workflow::tests::validate_accepts_apply_edits_on_claude ... ok
test workflow::tests::test_workflow_level_continue_on_error ... ok
test workflow::tests::validate_rejects_apply_edits_on_ollama ... ok
test workflow::tests::validate_rejects_apply_edits_with_multiple_backends ... ok
test workflow::tests::validate_rejects_apply_edits_with_no_backend ... ok
test workflow::tests::validate_skips_shell_only_steps ... ok
test workflow::tests::validate_treats_unknown_backend_as_none ... ok
test workflow::tests::test_timeout_at_minimum_allowed ... ok
test workflow::tests::validate_with_capabilities_handles_empty_steps ... ok
test workflows::tests::test_embedded_workflows_exist ... ok
test workflow::tests::test_timeout_too_small_validation ... ok
test workflow::tests::test_validate_config_defaults ... ok
test workflow::tests::test_validate_config_new_fields_parsing ... ok
test workflow::tests::test_validate_config_parses_on_parse_error_field ... ok
test workflow::tests::test_validate_config_new_fields_default_to_none ... ok
test workflows::tests::test_embedded_workflows_parse ... ok
test workflow::tests::test_validate_config_parses_mode_lenient_field ... ok
test workflow::tests::test_apply_once_with_format_runs_after_apply ... ok
test backend::retry::tests::test_retry_executor_honors_rate_limit_retry_after ... ok
test apply_verify::verification::tests::test_verify_elapsed_ms_nonzero ... ok
test strategy::verify::run_command::tests::verify_sleeps_timeout ... ok
test backend::tensorzero::tests::maps_request_timeout_to_timeout_error ... ok
test apply_verify::verification::tests::test_verify_timeout_kills_process_group ... ok
test apply_verify::verification::tests::test_verify_timeout_real_elapsed ... ok

test result: ok. 656 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s

     Running unittests src/main.rs (target/debug/deps/loker-00bca742a2ba89a7)

running 532 tests
test apply_verify::edit_parser::tests::test_crlf_normalization ... ok
test apply_verify::edit_parser::tests::test_detect_diff ... ok
test apply_verify::edit_parser::tests::test_detect_full_file ... ok
test apply_verify::edit_parser::tests::test_detect_json_array ... ok
test apply_verify::diff_applier::tests::test_apply_empty_file_path_is_invalid_edit ... ok
test apply_verify::diff_applier::tests::test_apply_rejects_path_traversal ... ok
test apply_verify::edit_parser::tests::test_detect_json_object ... ok
test apply_verify::edit_parser::tests::test_detect_with_lang_hint_diff ... ok
test apply_verify::edit_parser::tests::test_detect_with_lang_hint_json ... ok
test apply_verify::edit_parser::tests::test_diff_context_lines ... ok
test apply_verify::diff_applier::tests::test_apply_rejects_absolute_path ... ok
test apply_verify::edit_parser::tests::test_diff_multi_file ... ok
test apply_verify::edit_parser::tests::test_diff_no_hunks ... ok
test apply_verify::edit_parser::tests::test_diff_strips_ab_prefix ... ok
test apply_verify::edit_parser::tests::test_diff_no_newline_marker ... ok
test apply_verify::edit_parser::tests::test_diff_single_file ... ok
test apply_verify::edit_parser::tests::test_empty_input ... ok
test apply_verify::edit_parser::tests::test_full_file_no_path ... ok
test apply_verify::edit_parser::tests::test_full_file_empty_path ... ok
test apply_verify::edit_parser::tests::test_full_file ... ok
test apply_verify::edit_parser::tests::test_full_file_with_dash_header ... ok
test apply_verify::edit_parser::tests::test_json_bare_array ... ok
test apply_verify::edit_parser::tests::test_json_agentic_output ... ok
test apply_verify::edit_parser::tests::test_input_too_large ... ok
test apply_verify::edit_parser::tests::test_json_control_chars ... ok
test apply_verify::edit_parser::tests::test_json_empty_edits ... ok
test apply_verify::diff_applier::tests::test_apply_file_not_found ... ok
test apply_verify::edit_parser::tests::test_json_malformed ... ok
test apply_verify::edit_parser::tests::test_json_trailing_newlines_normalized ... ok
test apply_verify::edit_parser::tests::test_json_with_message_field ... ok
test apply_verify::edit_parser::tests::test_malformed_diff ... ok
test apply_verify::edit_parser::tests::test_markdown_backticks_in_content ... ok
test apply_verify::edit_parser::tests::test_markdown_diff_block ... ok
test apply_verify::edit_parser::tests::test_markdown_generic_block ... ok
test apply_verify::diff_applier::tests::test_apply_empty_edits ... ok
test apply_verify::edit_parser::tests::test_whitespace_only_input ... ok
test apply_verify::edit_parser::tests::test_markdown_json_block ... ok
test apply_verify::diff_applier::tests::test_apply_empty_old_in_find_replace_is_invalid ... ok
test apply_verify::diff_applier::tests::test_apply_unified_diff_multi_hunk_fails ... ok
test apply_verify::diff_applier::tests::test_apply_old_text_not_found ... ok
test apply_verify::retry_loop::tests::test_parse_error_stop ... ok
test apply_verify::diff_applier::tests::test_apply_json_single_file ... ok
test apply_verify::diff_applier::tests::test_apply_ambiguous_match ... ok
test apply_verify::diff_applier::tests::test_apply_full_file_overwrite ... ok
test apply_verify::diff_applier::tests::test_apply_partial_failure ... ok
test apply_verify::diff_applier::tests::test_apply_full_file_create_new ... ok
test apply_verify::diff_applier::tests::test_apply_unified_diff_single_hunk ... ok
test apply_verify::rollback::tests::test_is_fully_restored_false ... ok
test apply_verify::rollback::tests::test_is_fully_restored_true ... ok
test apply_verify::rollback::tests::test_rollback_delete_tolerates_already_missing ... ok
test apply_verify::retry_loop::tests::test_apply_partial_failure_rolls_back ... ok
test apply_verify::rollback::tests::test_rollback_deletes_new_file ... ok
test apply_verify::diff_applier::tests::test_apply_multi_file_success ... ok
test apply_verify::rollback::tests::test_rollback_empty_result_is_noop ... ok
test apply_verify::rollback::tests::test_rollback_continues_on_failure ... ok
test apply_verify::rollback::tests::test_rollback_single_file ... ok
test apply_verify::rollback::tests::test_rollback_mixed_restore_and_delete ... ok
test apply_verify::rollback::tests::test_rollback_reverse_order ... ok
test apply_verify::verification::tests::test_verify_captures_stderr ... ok
test apply_verify::verification::tests::test_verify_captures_both_streams ... ok
test apply_verify::retry_loop::tests::test_parse_error_retries ... ok
test apply_verify::retry_loop::tests::test_max_retries_zero_runs_once ... ok
test apply_verify::retry_loop::tests::test_requester_error_surfaced ... ok
test apply_verify::retry_loop::tests::test_apply_error_triggers_rollback_and_retry ... ok
test apply_verify::verification::tests::test_verify_captures_stdout ... ok
test apply_verify::verification::tests::test_verify_failure_exit_code ... ok
test backend::claude::tests::capabilities_match_current_wiring ... ok
test apply_verify::retry_loop::tests::test_parse_error_on_last_retry_exits ... ok
test apply_verify::retry_loop::tests::test_success_first_attempt ... ok
test backend::claude::tests::test_claude_response_deserialize_with_usage ... ok
test backend::claude::tests::test_claude_response_deserialize_without_usage ... ok
test backend::codex::tests::capabilities_match_current_wiring ... ok
test backend::gemini::tests::capabilities_match_current_wiring ... ok
test apply_verify::retry_loop::tests::test_verify_failure_triggers_rollback ... ok
test backend::genai_error::tests::classify_404_body_detects_unknown_function_fixture ... ok
test backend::genai_error::tests::contains_status_code_handles_punctuation_boundaries ... ok
test backend::genai_error::tests::classify_5xx_body_detects_anthropic_auth_fixture ... ok
test backend::genai_error::tests::classify_5xx_body_detects_rate_limit_signature ... ok
test backend::genai_error::tests::classify_5xx_body_returns_none_for_generic_5xx ... ok
test backend::genai_error::tests::map_status_401_to_auth ... ok
test backend::genai_error::tests::map_status_403_to_auth ... ok
test backend::genai_error::tests::map_status_404_other_to_execution_failed ... ok
test backend::genai_error::tests::map_status_404_unknown_function_to_config ... ok
test backend::genai_error::tests::map_status_429_to_rate_limit_retryable ... ok
test backend::genai_error::tests::map_status_500_to_network_retryable ... ok
test backend::genai_error::tests::map_status_502_generic_to_network_retryable ... ok
test backend::genai_error::tests::map_status_502_upstream_auth_to_auth_not_retryable ... ok
test backend::genai_error::tests::map_status_503_to_network_retryable ... ok
test backend::genai_error::tests::map_status_502_upstream_rate_limit_to_rate_limit_retryable ... ok
test backend::genai_error::tests::map_status_unknown_to_execution_failed ... ok
test backend::ollama::tests::test_ollama_response_deserialize_with_counts ... ok
test backend::ollama::tests::test_ollama_response_deserialize_without_model ... ok
test backend::ollama::tests::test_ollama_response_deserialize_partial_counts ... ok
test backend::retry::tests::test_get_delay_attempt_zero_is_zero ... ok
test backend::retry::tests::test_get_delay_clamped_at_max ... ok
test backend::retry::tests::test_get_delay_grows_exponentially ... ok
test backend::retry::tests::test_retry_executor_does_not_retry_non_retryable ... ok
test apply_verify::retry_loop::tests::test_integration_end_to_end ... ok
test backend::tensorzero::tests::canonicalize_wire_model_strips_to_canonical_on_wire ... ok
test backend::tensorzero::tests::capabilities_match_current_wiring ... ok
test backend::retry::tests::test_retry_success_after_failures ... ok
test backend::retry::tests::test_retry_exhausted ... ok
test apply_verify::verification::tests::test_verify_success ... ok
test apply_verify::verification::tests::test_verify_invalid_command_exits_127 ... ok
test apply_verify::retry_loop::tests::test_max_retries_exhausted ... ok
test apply_verify::verification::tests::test_verify_uses_passed_cwd ... ok
test apply_verify::verification::tests::test_verify_output_truncated ... ok
test apply_verify::retry_loop::tests::test_success_on_retry_after_verify_failure ... ok
test apply_verify::retry_loop::tests::test_attempt_records ... ok
test backend::tensorzero::tests::name_is_tensorzero ... ok
test backend::tensorzero::tests::normalize_endpoint_appends_when_missing ... ok
test backend::ollama::tests::capabilities_match_current_wiring ... ok
test backend::tensorzero::tests::normalize_endpoint_does_not_double_suffix ... ok
test backend::tensorzero::tests::maps_401_to_auth_not_retryable ... ok
test backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable ... ok
test backend::tensorzero::tests::maps_502_generic_to_network_retryable ... ok
test backend::tests::backend_capabilities_none_is_all_false ... ok
test backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable ... ok
test backend::tests::capabilities_for_name_matches_static_expectations ... ok
test backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime ... ok
test backend::tests::capabilities_for_name_unknown_returns_none ... ok
test backend::tests::default_capabilities_are_none ... ok
test backend::tests::tensorzero_adapter_allows_missing_api_key_env_field ... ok
test backend::tests::tensorzero_adapter_maps_endpoint_model_auth_timeout ... ok
test backend::tests::tensorzero_adapter_rejects_missing_endpoint_model_zero_timeout_and_bad_scheme ... ok
test backend::tensorzero::tests::maps_malformed_json_to_parse_error ... ok
test backend::tests::test_backend_error_display ... ok
test backend::tests::test_backend_error_from_anyhow ... ok
test backend::tensorzero::tests::maps_429_to_rate_limit_retryable ... ok
test backend::tests::test_backend_error_not_retryable ... ok
test backend::tests::test_backend_error_retryable ... ok
test backend::tests::test_query_output_from_process_empty_stderr_normalized ... ok
test backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable ... ok
test backend::tests::test_query_output_from_process_empty_stdout ... ok
test backend::tensorzero::tests::maps_500_to_retryable_error ... ok
test backend::tests::test_query_output_from_process_populates_backend_and_duration ... ok
test backend::tests::test_query_output_from_process_with_stderr ... ok
test backend::tests::tensorzero_create_backend_supported_when_capability_supported ... ok
test backend::tests::test_query_output_from_text ... ok
test backend::tests::test_query_output_from_text_populates_backend_and_duration ... ok
test backend::tests::test_query_output_with_model_none ... ok
test backend::tests::test_query_output_with_model_some ... ok
test backend::tests::test_query_output_with_structured_none ... ok
test backend::tests::test_query_output_with_structured_some ... ok
test backend::tests::test_query_output_with_usage_none ... ok
test backend::tests::test_query_output_with_usage_some ... ok
test backend::tests::test_token_usage_default_zero ... ok
test backend::tests::test_token_usage_new_computes_total ... ok
test backend::tests::test_token_usage_new_saturates_on_overflow ... ok
test backend::tests::test_token_usage_saturating_add ... ok
test backend::tests::with_elapsed_is_idempotent_on_repeated_calls ... ok
test backend::tests::with_elapsed_is_noop_on_non_timeout_variants ... ok
test backend::tests::with_elapsed_overrides_timeout_elapsed_ms ... ok
test cache::tests::test_cache_disabled ... ok
test cache::tests::test_cache_key_deterministic ... ok
test cache::tests::test_cache_key_different_backends ... ok
test cache::tests::test_cache_key_different_prompts ... ok
test config::tests::test_codex_backend_defaults ... ok
test config::tests::test_claude_backend_defaults ... ok
test config::tests::test_command_wrapper_default_none ... ok
test config::tests::test_command_wrapper_config ... ok
test config::tests::test_command_wrapper_docker_example ... ok
test config::tests::test_conductor_defaults ... ok
test config::tests::test_backend_config_defaults ... ok
test config::tests::test_conductor_custom_config ... ok
test config::tests::test_deep_merge_boolean_override ... ok
test config::tests::test_deep_merge_empty_overlay ... ok
test config::tests::test_deep_merge_scalar_override ... ok
test config::tests::test_deep_merge_hashmap_add ... ok
test config::tests::test_deep_merge_partial_config ... ok
test config::tests::test_deep_merge_hashmap_override ... ok
test config::tests::test_default_config ... ok
test config::tests::test_deny_unknown_fields ... ok
test config::tests::test_gemini_backend_defaults ... ok
test config::tests::test_hunt_task_defaults ... ok
test config::tests::test_deep_merge_vec_replace ... ok
test cache::tests::test_cache_warnings_on_parse_failure ... ok
test backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model ... ok
test backend::tensorzero::tests::returns_text_on_200_success ... ok
test cache::tests::test_cache_warnings_deduplicated ... ok
test config::tests::test_parse_minimal_config ... ok
test config::tests::test_parse_custom_backend ... ok
test config::tests::test_parse_custom_task ... ok
test config::tests::test_tensorzero_to_backend_opts_resolves_env ... ok
test config::tests::test_tensorzero_invalid_url_fails ... ok
test config::tests::test_tensorzero_missing_endpoint_fails ... ok
test backend::tests::tensorzero_create_backend_queries_wiremock_gateway ... ok
test config::tests::test_tensorzero_zero_timeout_fails ... ok
test consensus::tests::test_majority_vote_empty ... ok
test consensus::tests::test_majority_vote_tie_first_wins ... ok
test consensus::tests::test_majority_vote_clear_winner ... ok
test config::tests::test_config_serialization_roundtrip ... ok
test consensus::tests::test_weighted_vote ... ok
test consensus::tests::test_weighted_vote_clear_winner ... ok
test config::tests::test_load_config_from_paths_no_files ... ok
test consensus::tests::test_whitespace_normalization ... ok
test delegation::tests::test_backend_profiles_exist ... ok
test delegation::tests::test_case_insensitive_matching ... ok
test delegation::tests::test_classify_dead_code ... ok
test delegation::tests::test_classify_architecture ... ok
test delegation::tests::test_classify_multiple_categories ... ok
test delegation::tests::test_classify_general_fallback ... ok
test delegation::tests::test_classify_n1 ... ok
test delegation::tests::test_classify_performance ... ok
test delegation::tests::test_explain_contains_categories ... ok
test delegation::tests::test_classify_security ... ok
test delegation::tests::test_delegator_default ... ok
test config::tests::test_load_config_from_paths_explicit_bypasses ... ok
test delegation::tests::test_explain_contains_recommendations ... ok
test delegation::tests::test_recommend_architecture ... ok
test delegation::tests::test_recommend_general_returns_backend ... ok
test delegation::tests::test_recommend_dead_code ... ok
test delegation::tests::test_recommend_returns_multiple ... ok
test delegation::tests::test_recommend_security ... ok
test delegation::tests::test_recommend_n1 ... ok
test role::tests::test_resolution_builder ... ok
test role::tests::test_backend_filtering ... ok
test role::tests::test_resolution_is_empty ... ok
test role::tests::test_role_config_new ... ok
test config::tests::test_tensorzero_config_serialization_roundtrip ... ok
test role::tests::test_role_resolver_default_team ... ok
test role::tests::test_role_resolution_error_display ... ok
test role::tests::test_role_config_serialization ... ok
test role::tests::test_role_resolver_no_backends_available ... ok
test role::tests::test_role_resolver_resolve_global_role ... ok
test role::tests::test_role_resolver_role_not_found ... ok
test role::tests::test_role_resolver_team_can_define_custom_role ... ok
test role::tests::test_role_resolver_team_override ... ok
test role::tests::test_routing_strategy_default_is_fallback ... ok
test role::tests::test_role_resolver_team_override_takes_precedence ... ok
test role::tests::test_team_config_default ... ok
test role::tests::test_valid_parallel_config ... ok
test role::tests::test_validation_parallel_min_success_too_low ... ok
test role::tests::test_validation_unknown_backend ... ok
test role::tests::test_team_config_serialization ... ok
test context::tests::test_no_context ... ok
test role::tests::test_validation_parallel_min_success_exceeds_backends ... ok
test tasks::hunt::tests::test_truncate_title_81_chars_truncates ... ok
test git_agent::tests::test_is_initialized_false_for_nonexistent ... ok
test tasks::hunt::tests::test_truncate_title_combined ... ok
test tasks::hunt::tests::test_truncate_title_exactly_80_chars ... ok
test config::tests::test_load_config_from_paths_project_only ... ok
test tasks::hunt::tests::test_truncate_title_long_string_truncates ... ok
test tasks::hunt::tests::test_truncate_title_mixed_ascii_utf8 ... ok
test tasks::hunt::tests::test_truncate_title_removes_markdown_bold ... ok
test tasks::hunt::tests::test_truncate_title_removes_markdown_heading ... ok
test tasks::hunt::tests::test_truncate_title_short_string_unchanged ... ok
test tasks::hunt::tests::test_truncate_title_trims_whitespace ... ok
test tasks::hunt::tests::test_truncate_title_utf8_emoji ... ok
test tasks::hunt::tests::test_truncate_title_utf8_emoji_truncates ... ok
test template::context::tests::test_arg_zero_undefined ... ok
test template::context::tests::test_arg_out_of_bounds ... ok
test template::context::tests::test_arg_access ... ok
test template::context::tests::test_loop_vars_object_item ... ok
test context::tests::test_detect_rails_with_goldiloader ... ok
test context::tests::test_detect_typescript ... ok
test template::context::tests::test_loop_vars_preserve_existing_namespaces ... ok
test template::context::tests::test_loop_vars_string_item ... ok
test template::context::tests::test_step_field_fallback_no_parsed_output ... ok
test template::context::tests::test_step_field_with_parsed_output ... ok
test template::context::tests::test_step_output ... ok
test template::context::tests::test_step_success_false ... ok
test template::context::tests::test_step_success_true ... ok
test template::filters::tests::test_default_val_defined ... ok
test template::context::tests::test_workflow_backends ... ok
test template::context::tests::test_workflow_backends_empty ... ok
test template::filters::tests::test_default_val_empty_string ... ok
test template::filters::tests::test_default_val_undefined ... ok
test template::filters::tests::test_first_empty ... ok
test template::filters::tests::test_first_normal ... ok
test template::filters::tests::test_first_single ... ok
test template::filters::tests::test_join_default_separator ... ok
test template::filters::tests::test_join_empty ... ok
test template::filters::tests::test_join_with_separator ... ok
test config::tests::test_load_config_from_paths_user_parse_error ... ok
test template::filters::tests::test_json_encode_nested ... ok
test template::filters::tests::test_json_encode_number ... ok
test template::filters::tests::test_json_encode_string ... ok
test template::filters::tests::test_last_empty ... ok
test template::filters::tests::test_last_normal ... ok
test template::filters::tests::test_last_single ... ok
test template::filters::tests::test_lines_empty ... ok
test template::filters::tests::test_lines_multiline ... ok
test template::filters::tests::test_lines_single ... ok
test template::filters::tests::test_shell_escape_backticks_and_dollar ... ok
test template::filters::tests::test_shell_escape_basic ... ok
test template::filters::tests::test_shell_escape_injection ... ok
test template::filters::tests::test_shell_escape_newlines ... ok
test template::filters::tests::test_shell_escape_null_bytes ... ok
test git_agent::tests::test_is_available_returns_bool ... ok
test template::filters::tests::test_shell_escape_single_quotes ... ok
test template::context::tests::test_env_lookup ... ok
test template::context::tests::test_env_missing ... ok
test template::filters::tests::test_shell_escape_unicode ... ok
test template::filters::tests::test_trim_already_trimmed ... ok
test template::filters::tests::test_trim_newlines ... ok
test template::filters::tests::test_trim_whitespace ... ok
test config::tests::test_load_config_from_paths_three_layers ... ok
test template::tests::test_combined_env_arg_step ... ok
test template::tests::test_eval_expression_falsy ... ok
test template::tests::test_eval_expression_undefined ... ok
test template::tests::test_eval_expression_truthy ... ok
test template::tests::test_parse_error ... ok
test tests::test_parse_pr_github_standard ... ok
test template::tests::test_no_reexpansion_of_braces_in_output ... ok
test tests::test_parse_pr_github_with_files_suffix ... ok
test template::tests::test_undefined_variable ... ok
test tests::test_parse_pr_github_with_fragment ... ok
test template::tests::test_render_mixed ... ok
test tests::test_parse_pr_github_with_query_params ... ok
test tests::test_parse_pr_github_with_trailing_slash ... ok
test tests::test_parse_pr_gitlab_self_hosted ... ok
test tests::test_parse_pr_gitlab_standard ... ok
test tests::test_parse_pr_gitlab_with_diffs_suffix ... ok
test tests::test_parse_pr_invalid_host ... ok
test tests::test_parse_pr_missing_pr_number ... ok
test tests::test_parse_pr_owner_repo_hash_format ... ok
test tests::test_parse_pr_non_numeric ... ok
test tests::test_parse_pr_spoofed_host ... ok
test tests::test_parse_pr_with_explicit_repo ... ok
test utils::tests::test_backend_error_kind_from_typed ... ok
test utils::tests::test_classify_auth_401 ... ok
test utils::tests::test_classify_auth_invalid_key ... ok
test utils::tests::test_classify_capacity_exhausted ... ok
test utils::tests::test_classify_network_refused ... ok
test utils::tests::test_classify_not_installed ... ok
test utils::tests::test_classify_rate_limit_429 ... ok
test utils::tests::test_classify_rate_limit_quota ... ok
test utils::tests::test_classify_resource_exhausted ... ok
test utils::tests::test_classify_unknown ... ok
test utils::tests::test_summarize_capacity ... ok
test utils::tests::test_summarize_rate_limit ... ok
test utils::tests::test_summarize_typed_backend_error ... ok
test utils::tests::test_truncate_exact_length ... ok
test utils::tests::test_truncate_long_string ... ok
test utils::tests::test_summarize_unknown_truncates ... ok
test utils::tests::test_truncate_short_string ... ok
test utils::tests::test_truncate_unicode ... ok
test utils::tests::test_truncate_utf8_ascii ... ok
test utils::tests::test_truncate_utf8_empty_string ... ok
test utils::tests::test_truncate_utf8_exact_boundary ... ok
test utils::tests::test_truncate_utf8_multibyte_boundary ... ok
test utils::tests::test_truncate_utf8_within_limit ... ok
test utils::tests::test_truncate_utf8_zero_cap ... ok
test workflow::tests::required_capabilities_returns_empty_for_plain_step ... ok
test workflow::tests::required_capabilities_returns_file_edit_for_apply_edits ... ok
test workflow::tests::test_apply_lenient_mode_empty_response_fails ... ok
test workflow::tests::test_apply_lenient_mode_non_empty_passes_with_cleaned_output ... ok
test workflow::tests::test_apply_lenient_mode_preserves_internal_whitespace ... ok
test workflow::tests::test_apply_lenient_mode_whitespace_only_fails ... ok
test workflow::tests::test_apply_parse_error_policy_default_fails ... ok
test workflow::tests::test_apply_parse_error_policy_explicit_fail_matches_default ... ok
test workflow::tests::test_apply_parse_error_policy_pass_succeeds_without_output ... ok
test workflow::tests::test_apply_parse_error_policy_skip_drops_validation ... ok
test workflow::tests::test_apply_parse_error_policy_unknown_value_falls_back_to_fail ... ok
test workflow::tests::test_build_apply_fix_prompt_includes_partial_paths ... ok
test workflow::tests::test_build_parse_fix_prompt_contains_previous_raw ... ok
test workflow::tests::test_build_verify_fix_prompt_with_exit_code ... ok
test workflow::tests::test_build_verify_fix_prompt_with_timeout_uses_timeout_string ... ok
test workflow::tests::test_apply_once_parse_error_returns_err ... ok
test workflow::tests::test_apply_once_apply_error_rolls_back ... ok
test workflow::tests::test_apply_once_success_without_format ... ok
test workflow::tests::test_condition_unparseable_returns_true ... ok
test workflow::tests::test_condition_steps_success ... ok
test workflow::tests::test_continue_on_error_toml_parsing ... ok
test workflow::tests::test_duplicate_step_names_error ... ok
test workflow::tests::test_condition_equals ... ok
test workflow::tests::test_extract_json_field_bool ... ok
test workflow::tests::test_condition_contains ... ok
test workflow::tests::test_extract_json_field_multiline ... ok
test workflow::tests::test_extract_json_field_not_found ... ok
test workflow::tests::test_condition_legacy_syntax ... ok
test workflow::tests::test_extract_json_field_number ... ok
test workflow::tests::test_extract_json_field_string ... ok
test workflow::tests::test_evaluate_condition_error_recovery ... ok
test workflow::tests::test_extract_json_from_markdown_block ... ok
test workflow::tests::test_extract_json_from_plain_block ... ok
test workflow::tests::test_condition_not ... ok
test workflow::tests::test_extract_json_raw ... ok
test workflow::tests::test_extract_json_with_text_before ... ok
test workflow::tests::test_extract_json_with_literal_newlines ... ok
test workflow::tests::test_find_closing_fence ... ok
test workflow::tests::test_heuristic_contains_double_quotes ... ok
test workflow::tests::test_heuristic_contains_empty_string_always_passes ... ok
test workflow::tests::test_heuristic_contains_fail ... ok
test workflow::tests::test_heuristic_contains_pass ... ok
test workflow::tests::test_heuristic_contains_single_quote_char ... ok
test workflow::tests::test_group_by_depth_forward_declared_dependency ... ok
test workflow::tests::test_heuristic_contains_special_chars ... ok
test workflow::tests::test_heuristic_empty_check_string ... ok
test workflow::tests::test_heuristic_min_length_fail ... ok
test workflow::tests::test_heuristic_min_length_invalid_arg ... ok
test workflow::tests::test_heuristic_min_length_pass ... ok
test workflow::tests::test_heuristic_min_length_unicode ... ok
test workflow::tests::test_heuristic_min_length_whitespace_counts ... ok
test workflow::tests::test_heuristic_min_length_zero_always_passes ... ok
test workflow::tests::test_heuristic_not_empty_fail_empty ... ok
test workflow::tests::test_heuristic_not_empty_fail_whitespace ... ok
test workflow::tests::test_heuristic_not_empty_pass ... ok
test workflow::tests::test_heuristic_unknown_check ... ok
test workflow::tests::test_condition_json_field_access ... ok
test workflow::tests::test_for_each_with_parsed_output ... ok
test workflow::tests::test_for_each_parsed_output_not_array ... ok
test workflow::tests::test_interpolate_loop_vars_item_whole_object ... ok
test workflow::tests::test_interpolate_loop_vars_item_string ... ok
test workflow::tests::test_interpolate_loop_vars_index ... ok
test workflow::tests::test_interpolate_validation_prompt_basic ... ok
test workflow::tests::test_interpolate_loop_vars_missing_field ... ok
test workflow::tests::test_interpolate_validation_prompt_no_stderr ... ok
test workflow::tests::test_interpolate_loop_vars_multiple_fields_one_missing ... ok
test workflow::tests::test_interpolate_validation_prompt_injection_safety ... ok
test workflow::tests::test_interpolate_loop_vars_combined ... ok
test workflow::tests::test_interpolate_loop_vars_item_object ... ok
test workflow::tests::test_interpolate_validation_prompt_no_truncation_when_under_limit ... ok
test workflow::tests::test_interpolate_validation_prompt_truncation ... ok
test workflow::tests::test_interpolate_validation_prompt_with_stderr ... ok
test workflow::tests::test_jinja_if_block ... ok
test workflow::tests::test_jinja_join_filter ... ok
test workflow::tests::test_jinja_default_filter ... ok
test workflow::tests::test_jinja_chained_filters ... ok
test workflow::tests::test_interpolate_parsed_output_none_fallback ... ok
test workflow::tests::test_jinja_inline_for_loop ... ok
test workflow::tests::test_interpolate_with_fields_json ... ok
test workflow::tests::test_load_error_tracker_backoff_progression ... ok
test workflow::tests::test_load_error_tracker_bail_at_threshold ... ok
test workflow::tests::test_load_error_tracker_reset_on_success ... ok
test workflow::tests::test_load_error_tracker_success_with_no_prior_errors ... ok
test workflow::tests::test_map_retry_failure_apply_error_with_paths ... ok
test workflow::tests::test_map_retry_failure_apply_error_without_paths ... ok
test workflow::tests::test_map_retry_failure_attempt_count_from_retries ... ok
test workflow::tests::test_jinja_trim_filter ... ok
test workflow::tests::test_jinja_missing_step_default_fallback ... ok
test workflow::tests::test_map_retry_failure_parse_error ... ok
test workflow::tests::test_jinja_shell_escape_filter ... ok
test workflow::tests::test_map_retry_failure_empty_attempts ... ok
test workflow::tests::test_map_retry_failure_verify_exit_code ... ok
test workflow::tests::test_map_retry_failure_verify_has_priority_over_apply ... ok
test workflow::tests::test_map_retry_failure_stderr_truncated_to_1kb ... ok
test workflow::tests::test_map_retry_failure_verify_timeout ... ok
test workflow::tests::test_output_format_toml_parsing ... ok
test workflow::tests::test_min_deps_success_without_depends_on_error ... ok
test workflow::tests::test_apply_once_with_format_runs_after_apply ... ok
test workflow::tests::test_map_template_error_reports_offending_variable_in_multi_expression ... ok
test workflow::tests::test_parse_for_each_inline_array ... ok
test workflow::tests::test_parse_for_each_inline_array_objects ... ok
test workflow::tests::test_min_deps_success_validation_empty_deps ... ok
test workflow::tests::test_min_deps_success_validation_valid ... ok
test workflow::tests::test_min_deps_success_validation_exceeds_deps ... ok
test workflow::tests::test_parse_step_output_json ... ok
test workflow::tests::test_parse_step_output_lines ... ok
test workflow::tests::test_parse_step_output_none ... ok
test workflow::tests::test_parse_step_output_text ... ok
test workflow::tests::test_parse_for_each_invalid_format ... ok
test workflow::tests::test_parse_for_each_step_not_found ... ok
test workflow::tests::test_parse_for_each_not_array ... ok
test workflow::tests::test_parse_for_each_step_reference ... ok
test workflow::tests::test_parse_validation_response_empty_string_is_error ... ok
test workflow::tests::test_parse_validation_response_json_fail ... ok
test workflow::tests::test_parse_validation_response_invalid_status ... ok
test workflow::tests::test_parse_validation_response_json_in_fences ... ok
test workflow::tests::test_parse_validation_response_json_pass ... ok
test workflow::tests::test_parse_validation_response_json_pass_no_output ... ok
test workflow::tests::test_parse_validation_response_review_failed ... ok
test workflow::tests::test_parse_validation_response_unrecognized_is_error ... ok
test workflow::tests::test_sanitize_json_strings ... ok
test workflow::tests::test_step_failure_kind_copy_eq ... ok
test workflow::tests::test_step_failure_kind_display ... ok
test workflow::tests::test_step_for_each_inline_array_toml ... ok
test workflow::tests::test_step_for_each_toml_parsing ... ok
test workflow::tests::test_step_if_alias ... ok
test workflow::tests::test_parse_validate_config_absent ... ok
test workflow::tests::test_step_result_error_backend_error ... ok
test workflow::tests::test_step_result_error_edit_failed ... ok
test workflow::tests::test_step_result_error_has_no_validation ... ok
test workflow::tests::test_parse_for_each_step_reference_with_code_block ... ok
test workflow::tests::test_step_result_error_output_matches_failure_message ... ok
test workflow::tests::test_step_result_error_produces_failure ... ok
test workflow::tests::test_parse_validate_config_from_toml ... ok
test workflow::tests::test_step_result_error_skipped ... ok
test workflow::tests::test_step_result_error_verify_failed ... ok
test workflow::tests::test_strip_markdown_fences_json ... ok
test workflow::tests::test_strip_markdown_fences_none ... ok
test workflow::tests::test_strip_markdown_fences_plain ... ok
test workflow::tests::test_parse_validate_config_mixed_fields ... ok
test workflow::tests::test_strip_markdown_fences_with_whitespace ... ok
test workflow::tests::test_success_step_has_no_failure ... ok
test workflow::tests::test_translate_contains_call ... ok
test workflow::tests::test_translate_contains_with_single_quoted_literal_containing_double_quote ... ok
test workflow::tests::test_translate_contains_with_escaped_quotes ... ok
test workflow::tests::test_timeout_normal_value_allowed ... ok
test workflow::tests::test_parse_for_each_field_access ... ok
test workflow::tests::test_timeout_at_minimum_allowed ... ok
test workflow::tests::test_translate_contains_with_steps_prefix ... ok
test workflow::tests::test_translate_equals_call ... ok
test workflow::tests::test_translate_fast_path_whitespace_variants ... ok
test workflow::tests::test_translate_equals_with_steps_prefix ... ok
test workflow::tests::test_translate_passthrough_already_valid ... ok
test workflow::tests::test_timeout_zero_allowed ... ok
test workflow::tests::test_translate_nested_not ... ok
test workflow::tests::test_translate_legacy_double_quotes ... ok
test workflow::tests::test_translate_passthrough_empty ... ok
test workflow::tests::test_translate_multiple_contains ... ok
test workflow::tests::test_timeout_too_small_validation ... ok
test workflow::tests::test_translate_mixed_legacy_new ... ok
test workflow::tests::test_truncate_for_prompt_over_limit ... ok
test workflow::tests::test_truncate_for_prompt_under_limit ... ok
test workflow::tests::test_translate_legacy_steps_output_contains ... ok
test workflow::tests::test_validation_failure_has_no_step_failure ... ok
test workflow::tests::test_verify_command_composition_pattern ... ok
test workflow::tests::validate_accepts_apply_edits_on_claude ... ok
test workflow::tests::validate_rejects_apply_edits_on_ollama ... ok
test workflow::tests::validate_rejects_apply_edits_with_multiple_backends ... ok
test workflow::tests::validate_rejects_apply_edits_with_no_backend ... ok
test workflow::tests::test_workflow_level_continue_on_error ... ok
test workflow::tests::validate_skips_shell_only_steps ... ok
test workflow::tests::validate_treats_unknown_backend_as_none ... ok
test workflow::tests::validate_with_capabilities_handles_empty_steps ... ok
test workflows::tests::test_embedded_workflows_exist ... ok
test workflow::tests::test_validate_config_new_fields_default_to_none ... ok
test utils::tests::test_redact_secrets_aws_key ... ok
test utils::tests::test_redact_secrets_bearer_token ... ok
test utils::tests::test_redact_secrets_api_key_value ... ok
test workflow::tests::test_validate_config_defaults ... ok
test workflow::tests::test_validate_config_new_fields_parsing ... ok
test workflow::tests::test_validate_config_parses_mode_lenient_field ... ok
test workflows::tests::test_embedded_workflows_parse ... ok
test workflow::tests::test_validate_config_parses_on_parse_error_field ... ok
test backend::retry::tests::test_retry_executor_honors_rate_limit_retry_after ... ok
test apply_verify::verification::tests::test_verify_elapsed_ms_nonzero ... ok
test backend::tensorzero::tests::maps_request_timeout_to_timeout_error ... ok
test apply_verify::verification::tests::test_verify_timeout_real_elapsed ... ok
test apply_verify::verification::tests::test_verify_timeout_kills_process_group ... ok

test result: ok. 532 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s

     Running tests/aggregator_llm_judge.rs (target/debug/deps/aggregator_llm_judge-574dff9c8a43cc0a)

running 7 tests
test llm_judge_backend_error_maps_to_judge_unavailable ... ok
test llm_judge_family_overlap_refused ... ok
test llm_judge_malformed_json ... ok
test llm_judge_family_overlap_opt_out ... ok
test llm_judge_success ... ok
test llm_judge_snapshot ... ok
test llm_judge_waits_for_full_candidate_set_even_if_min_responses_is_met ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

     Running tests/integration.rs (target/debug/deps/integration-1bef9132c4010787)

running 6 tests
test test_llm_validate_workflow ... ok
test test_interpolation_workflow ... ok
test test_validate_workflow ... ok
test test_conditionals_workflow ... ok
test test_parallel_workflow ... ok
test test_retry_workflow ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.42s

     Running tests/manifest.rs (target/debug/deps/manifest-884beac56b4eeb6d)

running 10 tests
test empty_manifest_roundtrips ... ok
test schema_version_mismatch_rejected ... ok
test sha256_mismatch_returns_schema_error ... ok
test atomic_crash_after_rename_before_parent_fsync ... ok
test atomic_crash_before_rename_leaves_tmp ... ok
test changes_dir_digest_flattens_subdirs ... ok
test changes_dir_digest_is_deterministic ... ok
test generated_manifest_validates_against_schema ... ok
test append_and_reload_preserves_entries ... ok
test orphan_sweep_drops_unreferenced_entries ... ok

test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s

     Running tests/run_state_load.rs (target/debug/deps/run_state_load-28127dea3923fba5)

running 12 tests
test schema_mismatch_returns_artefact_schema_mismatch ... ok
test empty_manifest_loads_empty_runstate ... ok
test live_heartbeat_is_reported ... ok
test missing_markers_directory_keeps_all_entries ... ok
test stale_heartbeat_is_reported ... ok
test missing_entry_returns_artefact_missing ... ok
test corrupt_entry_returns_artefact_corrupt ... ok
test phase_status_is_derived_from_markers ... ok
test orphan_sweep_drops_non_completed_entries ... ok
test happy_path_load_returns_surviving_entries ... ok
test markers_without_completed_hashes_keeps_all_entries ... ok
test changes_dir_entry_is_verified_with_digest ... ok

test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests/schema_validation.rs (target/debug/deps/schema_validation-be7796b47267c147)

running 1 test
test run_artefact_schemas_validate_their_fixtures ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests/strategy_escalating_retry.rs (target/debug/deps/strategy_escalating_retry-441e7543ffb2c60d)

running 15 tests
test pass_failure_context_defaults_false ... ok
test mid_list_pass_returns_winner_and_captures_earlier_attempts ... ok
test full_exhaustion_returns_exhausted_error_with_all_attempts ... ok
test first_pass_success_returns_immediately ... ok
test pass_failure_context_off_passes_bare_prompt ... ok
test empty_rungs_yields_no_backends_error ... ok
test non_retryable_backend_error_does_not_skip_subsequent_backends ... ok
test missing_backend_in_pool_returns_backend_not_found ... ok
test phase_result_json_validates_against_escalating_schema ... ok
test exhausted_payload_validates_against_schema ... ok
test pass_failure_context_on_after_backend_error ... ok
test pass_failure_context_on_after_verify_fail ... ok
test pass_failure_context_three_rung_chain ... ok
test pass_failure_context_redacts_secrets ... ok
test pass_failure_context_truncates_large_body ... ok

test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests/strategy_parallel_fanout.rs (target/debug/deps/strategy_parallel_fanout-fbe75629fd302652)

running 8 tests
test one_fails_min_responses_still_satisfied ... ok
test fast_targets_cancel_slow ... ok
test happy_path_all_targets_succeed ... ok
test too_many_failures_returns_floor_violation ... ok
test phase_result_validates_against_parallel_schema ... ok
test floor_violation_payload_validates_against_schema ... ok
test vote_success_integration ... ok
test outcomes_contain_all_backends ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s

     Running tests/strategy_single_model.rs (target/debug/deps/strategy_single_model-d90dc4569dd1691e)

running 9 tests
test backend_not_found ... ok
test prompt_render_failure_surfaces_template_error ... ok
test no_aggregation_when_multiple_backends_present ... ok
test happy_path_emits_one_attempt ... ok
test empty_backends_yields_no_backends_error ... ok
test prompt_model_override_falls_through_to_attempt ... ok
test no_retry_on_backend_error ... ok
test missing_usage_serialises_zeroes ... ok
test output_validates_against_d2_schema ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

     Running tests/tensorzero_backend.rs (target/debug/deps/tensorzero_backend-15fa3cbffc08936a)

running 7 tests
test malformed_json_returns_parse_error ... ok
test success_200_returns_text ... ok
test auth_failure_401_is_not_retryable ... ok
test auth_failure_403_is_not_retryable ... ok
test server_error_500_is_retryable ... ok
test rate_limit_429_is_retryable ... ok
test request_timeout_returns_timeout_error ... ok

test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s

     Running tests/tensorzero_integration.rs (target/debug/deps/tensorzero_integration-82d44af9b5ec7b25)

running 1 test
test tz_integration_round_trip_via_loker_d1_openai ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/verify_llm_verifier.rs (target/debug/deps/verify_llm_verifier-a03db12985c7e48d)

running 9 tests
test no_is_fail ... ok
test yes_is_pass ... ok
test candidate_substitution_and_prompt_params ... ok
test yes_variants_pass ... ok
test backend_error_is_fail ... ok
test deterministic_param_sorting_respects_key_length ... ok
test unparseable_response_fails ... ok
test non_candidate_braces_passthrough ... ok
test forwards_system_prompt ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/verify_run_command.rs (target/debug/deps/verify_run_command-fd7b8d7d142dc8a4)

running 8 tests
test runcommand_echo_passes ... ok
test runcommand_env_allowlist ... ok
test runcommand_output_truncation_markers ... ok
test runcommand_false_fails_with_exit_code ... ok
test runcommand_secret_redaction_in_output ... ok
test runcommand_sleeps_timeout ... ok
test runcommand_process_group_killed_on_timeout ... ok
test runcommand_cpu_limit_forced_signal ... ok

test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.00s

     Running tests/verify_test_runner.rs (target/debug/deps/verify_test_runner-356223ae2df0f208)

running 9 tests
test cargo_malformed_json_lines ... ok
test pytest_noisy_multiline_output_is_parsed ... ok
test cargo_empty_no_tests ... ok
test verify_result_non_zero_exit_with_passing_tests_fails ... ok
test cargo_2_pass_1_fail ... ok
test pytest_5_pass_0_fail ... ok
test cargo_3_pass_0_fail ... ok
test pytest_4_pass_2_fail ... ok
test pytest_non_json_exit ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s

   Doc-tests loker

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


codex
The targeted branch tests are green, and `make check` is still running. One interesting datapoint: the repo-wide plain `cargo test` failed immediately in this environment, but `make check` is progressing much further, so I’m treating that as a signal that the project’s canonical gate may be setting up the test runtime differently. I need the final `make check` exit before I lock the checklist.
codex
`make check` completed green, including the new `run_state_load` suite. There’s still a reproducibility mismatch with plain `cargo test`, so I’m checking the Make target definition before I write that up, and I’m finishing the code-level findings now.
exec
/bin/zsh -lc "sed -n '1,220p' Makefile" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
.PHONY: help build test check clippy fmt clean sync feature release install pi-init

# Auto-generate version from today's date with auto-incrementing patch
# Format: YYYYMMDD.0.X where X increments if releasing multiple times per day
define get_next_version
$(shell \
	TODAY=$$(date +%Y%m%d); \
	LATEST=$$(git tag -l "v$$TODAY.*" 2>/dev/null | sort -V | tail -1); \
	if [ -z "$$LATEST" ]; then \
		echo "$$TODAY.0.0"; \
	else \
		PATCH=$$(echo "$$LATEST" | sed 's/.*\.0\.\([0-9]*\)/\1/'); \
		echo "$$TODAY.0.$$((PATCH + 1))"; \
	fi \
)
endef

VERSION := $(get_next_version)
BRANCH := $(shell git rev-parse --abbrev-ref HEAD)

help:
	@echo "Lok Makefile"
	@echo ""
	@echo "Development:"
	@echo "  make build                         - Build release binary"
	@echo "  make install                       - Build and install to cargo bin"
	@echo "  make test                          - Run tests"
	@echo "  make check                         - Run fmt check + clippy + test"
	@echo "  make clippy                        - Run clippy"
	@echo "  make fmt                           - Format code"
	@echo "  make clean                         - Clean build artifacts"
	@echo ""
	@echo "Pi:"
	@echo "  make pi-init                       - Install npm deps for all .pi/extensions/*"
	@echo ""
	@echo "Workflow:"
	@echo "  make sync                          - Pull latest from upstream into main"
	@echo "  make feature NAME=my-feature       - Create feature branch from main"
	@echo "  make merge                         - Merge current feature branch into main"
	@echo "  make pi-init                       - Install npm deps for .pi extensions"
	@echo ""
	@echo "Release:"
	@echo "  make release                       - Auto-version release ($(VERSION))"
	@echo "  make release VERSION=20260329.0.0  - Release with specific version"
	@echo ""
	@echo "Current branch: $(BRANCH)"
	@echo "Next version:   $(VERSION)"

# --- Development ---

build:
	cargo build --release

install: check
	cargo install --path .

test:
	cargo test

clippy:
	cargo clippy -- -D warnings

fmt:
	cargo fmt

check: fmt
	cargo clippy -- -D warnings
	cargo test

clean:
	cargo clean

pi-init:
	@for ext in .pi/extensions/*/package.json; do \
		[ -f "$$ext" ] || continue; \
		dir=$$(dirname $$ext); \
		echo "Installing deps in $$dir..."; \
		if [ -f "$$dir/package-lock.json" ]; then \
			(cd $$dir && npm ci --silent); \
		else \
			(cd $$dir && npm install --silent); \
		fi; \
	done
	@echo "Pi extensions ready"

# --- Workflow ---

sync:
	@git checkout main
	@git fetch upstream
	@git merge upstream/main
	@git push origin main
	@echo "main synced with upstream and pushed to origin"

feature:
ifndef NAME
	$(error Usage: make feature NAME=my-feature)
endif
	@git checkout main
	@git checkout -b feature/$(NAME)
	@echo "Created feature/$(NAME) from main"

merge:
	@if [ "$(BRANCH)" = "main" ]; then echo "Already on main - switch to a feature branch first"; exit 1; fi
	@echo "Merging $(BRANCH) into main..."
	@git checkout main
	@git merge --no-ff $(BRANCH) -m "Merge $(BRANCH)"
	@echo "Merged. Run 'git push origin main' when ready."

# --- Release ---

release:
	@echo "Running checks before release..."
	@cargo fmt -- --check
	@cargo clippy -- -D warnings
	@cargo test
	@echo ""
	@echo "Creating release v$(VERSION)..."
	@git checkout -b release/v$(VERSION)
	@sed -i '' 's/^version = .*/version = "$(VERSION)"/' Cargo.toml
	@cargo check --quiet 2>/dev/null || true
	@git add Cargo.toml Cargo.lock
	@git commit -m "chore: bump version to $(VERSION)"
	@git checkout main
	@git merge --no-ff release/v$(VERSION) -m "Merge branch 'release/v$(VERSION)'"
	@git tag -a v$(VERSION) -m "Release v$(VERSION)"
	@git branch -d release/v$(VERSION)
	@cargo build --release
	@cp target/release/loker /usr/local/bin/loker
	@git push origin main
	@git push origin v$(VERSION)
	@echo ""
	@echo "Released v$(VERSION)"
	@echo "  - Tagged v$(VERSION)"
	@echo "  - Pushed to origin"
	@echo "  - Installed to /usr/local/bin/loker"
	@loker --version

exec
/bin/zsh -lc 'rg -n "''^check:|cargo test|cargo clippy|fmt" Makefile makefile GNUmakefile' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 exited 2 in 0ms:
rg: GNUmakefile: No such file or directory (os error 2)
Makefile:1:.PHONY: help build test check clippy fmt clean sync feature release install pi-init
Makefile:28:	@echo "  make check                         - Run fmt check + clippy + test"
Makefile:30:	@echo "  make fmt                           - Format code"
Makefile:58:	cargo test
Makefile:61:	cargo clippy -- -D warnings
Makefile:63:fmt:
Makefile:64:	cargo fmt
Makefile:66:check: fmt
Makefile:67:	cargo clippy -- -D warnings
Makefile:68:	cargo test
Makefile:114:	@cargo fmt -- --check
Makefile:115:	@cargo clippy -- -D warnings
Makefile:116:	@cargo test
makefile:1:.PHONY: help build test check clippy fmt clean sync feature release install pi-init
makefile:28:	@echo "  make check                         - Run fmt check + clippy + test"
makefile:30:	@echo "  make fmt                           - Format code"
makefile:58:	cargo test
makefile:61:	cargo clippy -- -D warnings
makefile:63:fmt:
makefile:64:	cargo fmt
makefile:66:check: fmt
makefile:67:	cargo clippy -- -D warnings
makefile:68:	cargo test
makefile:114:	@cargo fmt -- --check
makefile:115:	@cargo clippy -- -D warnings
makefile:116:	@cargo test

exec
/bin/zsh -lc "rg -n \"LoadError::StaleWriter|LoadError::LiveWriter|HeartbeatStatus::Missing|PhaseStatus::None|status_from_heartbeat|manifest_entry_sha256|artefact_paths|phase_status\" src tests docs -g '"'!target'"'" in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 succeeded in 0ms:
src/run_state/load.rs:89:    pub phase_status: HashMap<String, PhaseStatus>,
src/run_state/load.rs:95:    manifest_entry_sha256: String,
src/run_state/load.rs:109:    phase_status: HashMap<String, PhaseStatus>,
src/run_state/load.rs:132:        let phase_status = marker_scan.phase_status;
src/run_state/load.rs:152:            phase_status,
src/run_state/load.rs:191:                phase_status: status,
src/run_state/load.rs:214:                        completed.insert(marker.manifest_entry_sha256);
src/run_state/load.rs:217:                let _ = update_phase_status(&mut status, phase.clone(), PhaseStatus::Completed);
src/run_state/load.rs:222:                let _ = update_phase_status(&mut status, phase, PhaseStatus::Failed);
src/run_state/load.rs:227:                let _ = update_phase_status(&mut status, phase, PhaseStatus::Started);
src/run_state/load.rs:232:            phase_status: status,
src/run_state/load.rs:329:    pub fn status_from_heartbeat(heartbeat: &Heartbeat, ttl_seconds: u64) -> HeartbeatStatus {
src/run_state/load.rs:354:fn update_phase_status(
src/run_state/load.rs:378:        PhaseStatus::None => 0,
docs/reviews/clo-283-design-gemini.md:25:**What:** The text says "entries whose `sha256` is present in that set, **or whose phase/attempt marker values are otherwise referenced**." The marker files contain `manifest_entry_sha256` — they do NOT reference entries by phase/attempt. The "or" clause introduces incorrect filtering logic.
docs/reviews/clo-285-codex-validation.md:170:                                   +--------------------> RunState(entries, dropped_orphans, phase_status, heartbeat)
docs/reviews/clo-285-codex-validation.md:229:    pub phase_status: std::collections::HashMap<String, PhaseStatus>,
docs/reviews/clo-285-codex-validation.md:678:+    pub phase_status: HashMap<String, PhaseStatus>,
docs/reviews/clo-285-codex-validation.md:684:+    manifest_entry_sha256: String,
docs/reviews/clo-285-codex-validation.md:698:+    phase_status: HashMap<String, PhaseStatus>,
docs/reviews/clo-285-codex-validation.md:721:+        let phase_status = marker_scan.phase_status;
docs/reviews/clo-285-codex-validation.md:741:+            phase_status,
docs/reviews/clo-285-codex-validation.md:780:+                phase_status: status,
docs/reviews/clo-285-codex-validation.md:803:+                        completed.insert(marker.manifest_entry_sha256);
docs/reviews/clo-285-codex-validation.md:806:+                let _ = update_phase_status(&mut status, phase.clone(), PhaseStatus::Completed);
docs/reviews/clo-285-codex-validation.md:811:+                let _ = update_phase_status(&mut status, phase, PhaseStatus::Failed);
docs/reviews/clo-285-codex-validation.md:816:+                let _ = update_phase_status(&mut status, phase, PhaseStatus::Started);
docs/reviews/clo-285-codex-validation.md:821:+            phase_status: status,
docs/reviews/clo-285-codex-validation.md:918:+    pub fn status_from_heartbeat(heartbeat: &Heartbeat, ttl_seconds: u64) -> HeartbeatStatus {
docs/reviews/clo-285-codex-validation.md:943:+fn update_phase_status(
docs/reviews/clo-285-codex-validation.md:967:+        PhaseStatus::None => 0,
docs/reviews/clo-285-codex-validation.md:1016:+        "manifest_entry_sha256": sha,
docs/reviews/clo-285-codex-validation.md:1017:+        "artefact_paths": [format!("{phase}/out")],
docs/reviews/clo-285-codex-validation.md:1247:+    assert_eq!(run_state.phase_status.len(), 0);
docs/reviews/clo-285-codex-validation.md:1295:+fn phase_status_is_derived_from_markers() {
docs/reviews/clo-285-codex-validation.md:1309:+        run_state.phase_status.get("design"),
docs/reviews/clo-285-codex-validation.md:1313:+        run_state.phase_status.get("review"),
docs/reviews/clo-285-codex-validation.md:1377:    manifest_entry_sha256: String,
docs/reviews/clo-285-codex-validation.md:1613:                referenced.insert(marker.manifest_entry_sha256);
docs/reviews/clo-285-codex-validation.md:1714:    89	    pub phase_status: HashMap<String, PhaseStatus>,
docs/reviews/clo-285-codex-validation.md:1720:    95	    manifest_entry_sha256: String,
docs/reviews/clo-285-codex-validation.md:1734:   109	    phase_status: HashMap<String, PhaseStatus>,
docs/reviews/clo-285-codex-validation.md:1757:   132	        let phase_status = marker_scan.phase_status;
docs/reviews/clo-285-codex-validation.md:1777:   152	            phase_status,
docs/reviews/clo-285-codex-validation.md:1816:   191	                phase_status: status,
docs/reviews/clo-285-codex-validation.md:1839:   214	                        completed.insert(marker.manifest_entry_sha256);
docs/reviews/clo-285-codex-validation.md:1842:   217	                let _ = update_phase_status(&mut status, phase.clone(), PhaseStatus::Completed);
docs/reviews/clo-285-codex-validation.md:1847:   222	                let _ = update_phase_status(&mut status, phase, PhaseStatus::Failed);
docs/reviews/clo-285-codex-validation.md:1852:   227	                let _ = update_phase_status(&mut status, phase, PhaseStatus::Started);
docs/reviews/clo-285-codex-validation.md:1857:   232	            phase_status: status,
docs/reviews/clo-285-codex-validation.md:1904:    15	        "manifest_entry_sha256": sha,
docs/reviews/clo-285-codex-validation.md:1905:    16	        "artefact_paths": [format!("{phase}/out")],
docs/reviews/clo-285-codex-validation.md:2135:   246	    assert_eq!(run_state.phase_status.len(), 0);
docs/reviews/clo-285-codex-validation.md:2227:   329	    pub fn status_from_heartbeat(heartbeat: &Heartbeat, ttl_seconds: u64) -> HeartbeatStatus {
docs/reviews/clo-285-codex-validation.md:2252:   354	fn update_phase_status(
docs/reviews/clo-285-codex-validation.md:2276:   378	        PhaseStatus::None => 0,
docs/reviews/clo-285-codex-validation.md:2331:   294	fn phase_status_is_derived_from_markers() {
docs/reviews/clo-285-codex-validation.md:2345:   308	        run_state.phase_status.get("design"),
docs/reviews/clo-285-codex-validation.md:2349:   312	        run_state.phase_status.get("review"),
docs/reviews/clo-285-codex-validation.md:2533:  - `phase_status: HashMap<String, PhaseStatus>`
docs/reviews/clo-285-codex-validation.md:2673:docs/designs/clo-285-manifest-load.md:35:                                   +--------------------> RunState(entries, dropped_orphans, phase_status, heartbeat)
docs/reviews/clo-285-codex-validation.md:3608:  "manifest_entry_sha256": "ab12...",
docs/reviews/clo-285-codex-validation.md:3609:  "artefact_paths": ["design/design.md"]
docs/reviews/clo-285-codex-validation.md:3707:   - Read the marker's `manifest_entry_sha256`. This digest is the
docs/reviews/clo-285-codex-validation.md:3711:     `artefact_paths` is a denormalised hint for human-readable
docs/reviews/clo-285-codex-validation.md:3779:test phase_status_is_derived_from_markers ... ok
docs/reviews/clo-285-codex-validation.md:5064:test phase_status_is_derived_from_markers ... ok
tests/manifest.rs:178:        "manifest_entry_sha256": entry1.sha256,
tests/manifest.rs:179:        "artefact_paths": ["design/design.md"]
tests/run_state_load.rs:15:        "manifest_entry_sha256": sha,
tests/run_state_load.rs:16:        "artefact_paths": [format!("{phase}/out")],
tests/run_state_load.rs:246:    assert_eq!(run_state.phase_status.len(), 0);
tests/run_state_load.rs:294:fn phase_status_is_derived_from_markers() {
tests/run_state_load.rs:308:        run_state.phase_status.get("design"),
tests/run_state_load.rs:312:        run_state.phase_status.get("review"),
src/manifest.rs:14:    manifest_entry_sha256: String,
src/manifest.rs:250:                referenced.insert(marker.manifest_entry_sha256);
docs/prds/clo-283-manifest.md:130:- Orphan sweep reads `markers/` dir for `*.completed` files, extracts their `manifest_entry_sha256` fields, drops entries whose sha256 is not present.
docs/status/clo-245-workflow.yaml:55:      - "Manifest is full-rewrite-via-tmp-rename, not text-append. Avoids partial-line corruption. Cheap because manifests are small. manifest_entry_sha256 is the authoritative key for completed phases; completed marker's artefact_paths is a denormalised debug hint."
docs/status/clo-245-workflow.yaml:120:    details: "Addressed 8 inline review comments (2 gemini-code-assist + 6 copilot-pull-request-reviewer) in commit 0dc8007. Clarifications: NamedTempFile::persist fsync requirements (file + parent dir explicit); started marker rewrite-every-attempt (resolves contradiction); manifest_entry_sha256 authoritative; markers/<phase>.* notation standardised; kill matrix scoped to process-crash semantics with rows 6/11 called out as power-loss; YYYYMMDDTHHMMSSZ portable stale-tmp timestamp. All 8 replies posted; gemini auto-re-validated via /gemini review trigger; Copilot auto-re-reviewed on push. make check green."
docs/designs/CLO-283-manifest.md:207:1. Collect all `manifest_entry_sha256` values from `markers/*.completed` files under the run directory.
docs/designs/CLO-283-manifest.md:211:Note: The marker file format is defined in `docs/run-state.md` §Phase markers.  The sweep code deserialises each marker with serde and extracts the `manifest_entry_sha256` field.
docs/designs/clo-285-manifest-load.md:35:                                   +--------------------> RunState(entries, dropped_orphans, phase_status, heartbeat)
docs/designs/clo-285-manifest-load.md:94:    pub phase_status: std::collections::HashMap<String, PhaseStatus>,
docs/plans/clo-283-manifest.md:45:- Implement orphan sweep: resolve `path.parent()/markers`, read `*.completed` marker files, extract `manifest_entry_sha256` fields, drop unreferenced entries with `eprintln!` log + `// TODO(T-029):` comment.
docs/prds/clo-285-manifest-load.md:24:  - `phase_status: HashMap<String, PhaseStatus>`
docs/run-state.md:128:  "manifest_entry_sha256": "ab12...",
docs/run-state.md:129:  "artefact_paths": ["design/design.md"]
docs/run-state.md:227:   - Read the marker's `manifest_entry_sha256`. This digest is the
docs/run-state.md:231:     `artefact_paths` is a denormalised hint for human-readable

codex
There’s one more gate question: plain `cargo test` and `make check` disagreed. I’m rerunning plain `cargo test` once to determine whether that was a transient flake or a persistent reproducibility problem, because pre-PR validation should treat flakiness as a real risk.
exec
/bin/zsh -lc 'cargo test' in /Users/mk/Code/orchestrator/loker--feat-clo-285-manifest
 exited 101 in 354ms:
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.10s
     Running unittests src/lib.rs (target/debug/deps/loker-e0bbd5653d27b3fc)

running 656 tests
test aggregator::concat::tests::concat_kind_maps_to_strategy_label ... ok
test aggregator::concat::tests::llm_judge_kind_maps_to_strategy_label ... ok
test aggregator::concat::tests::vote_kind_maps_to_strategy_label ... ok
test aggregator::llm_judge::tests::llm_judge_family_overlap_opt_out_warns ... ok
test aggregator::llm_judge::tests::llm_judge_family_diverse_ok ... ok
test aggregator::llm_judge::tests::llm_judge_family_overlap_blocks ... ok
test aggregator::llm_judge::tests::llm_judge_parse_malformed_json ... ok
test aggregator::llm_judge::tests::llm_judge_parse_markdown_fenced_ballot ... ok
test aggregator::concat::tests::concat_preserves_unknown_placeholders ... ok
test aggregator::concat::tests::concat_renders_success_sections_in_input_order ... ok
test aggregator::concat::tests::concat_whitespace_only_success_output_keeps_newline_invariants ... ok
test aggregator::concat::tests::concat_counts_success_and_failure ... ok
test aggregator::concat::tests::concat_escapes_multiline_failure_reason ... ok
test aggregator::concat::tests::concat_does_not_reexpand_placeholders_inside_metadata ... ok
test aggregator::llm_judge::tests::llm_judge_parse_missing_reason ... ok
test aggregator::llm_judge::tests::llm_judge_parse_negative_chosen_index ... ok
test aggregator::concat::tests::concat_preserves_braced_unknown_expressions_containing_known_tokens ... ok
test aggregator::llm_judge::tests::llm_judge_parse_out_of_bounds_index ... ok
test aggregator::concat::tests::concat_normalizes_crlf_failure_reason ... ok
test aggregator::llm_judge::tests::llm_judge_parse_valid_ballot ... ok
test aggregator::concat::tests::concat_empty_input_returns_sentinel ... ok
test aggregator::llm_judge::tests::llm_judge_parse_out_of_bounds_index_clamped ... ok
test aggregator::llm_judge::tests::llm_judge_parse_missing_chosen_index ... ok
test aggregator::llm_judge::tests::llm_judge_parse_within_bounds_index_clamped ... ok
test aggregator::llm_judge::tests::llm_judge_parse_zero_candidates_index ... ok
test aggregator::tests::empty_text ... ok
test aggregator::tests::extra_keys_ok ... ok
test aggregator::tests::markdown_fenced_fail ... ok
test aggregator::tests::markdown_fenced_json ... ok
test aggregator::tests::missing_pass ... ok
test aggregator::llm_judge::tests::llm_judge_prompt_includes_phase_name ... ok
test aggregator::llm_judge::tests::llm_judge_prompt_renders_candidates ... ok
test aggregator::tests::pass_false ... ok
test aggregator::tests::wrong_pass_type ... ok
test aggregator::tests::pass_true ... ok
test aggregator::vote::tests::all_abstain ... ok
test aggregator::vote::tests::abstain_backend_error ... ok
test aggregator::vote::tests::closest_family_multiple_buckets_match ... ok
test aggregator::vote::tests::closest_family_multiple_matching_buckets ... ok
test aggregator::vote::tests::empty_ballot_counts_as_abstain ... ok
test aggregator::vote::tests::empty_input ... ok
test aggregator::vote::tests::closest_family_no_match_fallback ... ok
test aggregator::vote::tests::free_text_clear_winner ... ok
test aggregator::vote::tests::free_text_tie_closest_family ... ok
test aggregator::vote::tests::free_text_tie_first_responder ... ok
test aggregator::vote::tests::normalise_ballot_basic ... ok
test aggregator::vote::tests::normalise_case ... ok
test aggregator::vote::tests::quorum_lost ... ok
test aggregator::vote::tests::normalise_whitespace ... ok
test aggregator::vote::tests::free_text_tie_random_deterministic ... ok
test aggregator::vote::tests::sanitize_comment_in_metadata ... ok
test aggregator::vote::tests::vote_counts_sorted_descending ... ok
test aggregator::vote::tests::whitespace_only_ballot_counts_as_abstain ... ok
test apply_verify::diff_applier::tests::test_apply_empty_file_path_is_invalid_edit ... ok
test apply_verify::diff_applier::tests::test_apply_empty_edits ... ok
test apply_verify::diff_applier::tests::test_apply_rejects_path_traversal ... ok
test apply_verify::edit_parser::tests::test_crlf_normalization ... ok
test apply_verify::edit_parser::tests::test_detect_diff ... ok
test apply_verify::diff_applier::tests::test_apply_rejects_absolute_path ... ok
test apply_verify::edit_parser::tests::test_detect_full_file ... ok
test apply_verify::edit_parser::tests::test_detect_json_array ... ok
test apply_verify::edit_parser::tests::test_detect_json_object ... ok
test apply_verify::edit_parser::tests::test_detect_with_lang_hint_diff ... ok
test apply_verify::edit_parser::tests::test_detect_with_lang_hint_json ... ok
test apply_verify::edit_parser::tests::test_diff_context_lines ... ok
test apply_verify::edit_parser::tests::test_diff_multi_file ... ok
test apply_verify::edit_parser::tests::test_diff_no_hunks ... ok
test apply_verify::edit_parser::tests::test_diff_no_newline_marker ... ok
test apply_verify::edit_parser::tests::test_diff_single_file ... ok
test apply_verify::edit_parser::tests::test_empty_input ... ok
test apply_verify::edit_parser::tests::test_diff_strips_ab_prefix ... ok
test apply_verify::edit_parser::tests::test_full_file ... ok
test apply_verify::edit_parser::tests::test_full_file_no_path ... ok
test apply_verify::edit_parser::tests::test_full_file_empty_path ... ok
test apply_verify::edit_parser::tests::test_full_file_with_dash_header ... ok
test apply_verify::edit_parser::tests::test_json_agentic_output ... ok
test apply_verify::edit_parser::tests::test_input_too_large ... ok
test apply_verify::edit_parser::tests::test_json_empty_edits ... ok
test apply_verify::diff_applier::tests::test_apply_file_not_found ... ok
test apply_verify::edit_parser::tests::test_json_bare_array ... ok
test apply_verify::edit_parser::tests::test_json_control_chars ... ok
test apply_verify::edit_parser::tests::test_json_malformed ... ok
test apply_verify::edit_parser::tests::test_json_with_message_field ... ok
test apply_verify::edit_parser::tests::test_malformed_diff ... ok
test apply_verify::edit_parser::tests::test_json_trailing_newlines_normalized ... ok
test apply_verify::edit_parser::tests::test_markdown_diff_block ... ok
test apply_verify::edit_parser::tests::test_markdown_backticks_in_content ... ok
test apply_verify::edit_parser::tests::test_markdown_generic_block ... ok
test apply_verify::edit_parser::tests::test_markdown_json_block ... ok
test apply_verify::diff_applier::tests::test_apply_ambiguous_match ... ok
test apply_verify::edit_parser::tests::test_whitespace_only_input ... ok
test apply_verify::diff_applier::tests::test_apply_empty_old_in_find_replace_is_invalid ... ok
test apply_verify::diff_applier::tests::test_apply_unified_diff_multi_hunk_fails ... ok
test apply_verify::diff_applier::tests::test_apply_full_file_overwrite ... ok
test apply_verify::diff_applier::tests::test_apply_partial_failure ... ok
test apply_verify::diff_applier::tests::test_apply_json_single_file ... ok
test apply_verify::diff_applier::tests::test_apply_old_text_not_found ... ok
test apply_verify::retry_loop::tests::test_parse_error_stop ... ok
test apply_verify::diff_applier::tests::test_apply_unified_diff_single_hunk ... ok
test apply_verify::diff_applier::tests::test_apply_full_file_create_new ... ok
test apply_verify::rollback::tests::test_is_fully_restored_false ... ok
test apply_verify::rollback::tests::test_is_fully_restored_true ... ok
test apply_verify::retry_loop::tests::test_apply_partial_failure_rolls_back ... ok
test apply_verify::diff_applier::tests::test_apply_multi_file_success ... ok
test apply_verify::rollback::tests::test_rollback_continues_on_failure ... ok
test apply_verify::rollback::tests::test_rollback_delete_tolerates_already_missing ... ok
test apply_verify::rollback::tests::test_rollback_empty_result_is_noop ... ok
test apply_verify::rollback::tests::test_rollback_deletes_new_file ... ok
test apply_verify::rollback::tests::test_rollback_single_file ... ok
test apply_verify::rollback::tests::test_rollback_mixed_restore_and_delete ... ok
test apply_verify::rollback::tests::test_rollback_reverse_order ... ok
test aggregator::concat::tests::concat_mixed_success_failure_snapshot ... ok
test aggregator::vote::tests::vote_snapshot ... ok
test apply_verify::retry_loop::tests::test_parse_error_retries ... ok
test apply_verify::retry_loop::tests::test_parse_error_on_last_retry_exits ... ok
test apply_verify::retry_loop::tests::test_max_retries_zero_runs_once ... ok
test apply_verify::retry_loop::tests::test_apply_error_triggers_rollback_and_retry ... ok
test apply_verify::retry_loop::tests::test_verify_failure_triggers_rollback ... ok
test apply_verify::retry_loop::tests::test_success_first_attempt ... ok
test apply_verify::retry_loop::tests::test_requester_error_surfaced ... ok
test backend::claude::tests::capabilities_match_current_wiring ... ok
test backend::claude::tests::test_claude_response_deserialize_with_usage ... ok
test backend::claude::tests::test_claude_response_deserialize_without_usage ... ok
test backend::codex::tests::capabilities_match_current_wiring ... ok
test backend::gemini::tests::capabilities_match_current_wiring ... ok
test apply_verify::verification::tests::test_verify_captures_stdout ... ok
test apply_verify::verification::tests::test_verify_captures_both_streams ... ok
test backend::genai_error::tests::classify_5xx_body_detects_anthropic_auth_fixture ... ok
test backend::genai_error::tests::classify_404_body_detects_unknown_function_fixture ... ok
test backend::genai_error::tests::classify_5xx_body_detects_rate_limit_signature ... ok
test backend::genai_error::tests::classify_5xx_body_returns_none_for_generic_5xx ... ok
test backend::genai_error::tests::map_status_403_to_auth ... ok
test backend::genai_error::tests::contains_status_code_handles_punctuation_boundaries ... ok
test backend::genai_error::tests::map_status_401_to_auth ... ok
test apply_verify::verification::tests::test_verify_captures_stderr ... ok
test backend::genai_error::tests::map_status_404_other_to_execution_failed ... ok
test backend::genai_error::tests::map_status_404_unknown_function_to_config ... ok
test backend::genai_error::tests::map_status_429_to_rate_limit_retryable ... ok
test backend::genai_error::tests::map_status_500_to_network_retryable ... ok
test backend::genai_error::tests::map_status_502_generic_to_network_retryable ... ok
test backend::genai_error::tests::map_status_502_upstream_auth_to_auth_not_retryable ... ok
test backend::genai_error::tests::map_status_503_to_network_retryable ... ok
test backend::genai_error::tests::map_status_502_upstream_rate_limit_to_rate_limit_retryable ... ok
test backend::genai_error::tests::map_status_unknown_to_execution_failed ... ok
test backend::ollama::tests::test_ollama_response_deserialize_partial_counts ... ok
test backend::ollama::tests::test_ollama_response_deserialize_with_counts ... ok
test backend::ollama::tests::test_ollama_response_deserialize_without_model ... ok
test backend::retry::tests::test_get_delay_attempt_zero_is_zero ... ok
test backend::retry::tests::test_get_delay_clamped_at_max ... ok
test backend::retry::tests::test_get_delay_grows_exponentially ... ok
test backend::retry::tests::test_retry_executor_does_not_retry_non_retryable ... ok
test apply_verify::retry_loop::tests::test_integration_end_to_end ... ok
test apply_verify::verification::tests::test_verify_failure_exit_code ... ok
test backend::tensorzero::tests::canonicalize_wire_model_strips_to_canonical_on_wire ... ok
test backend::tensorzero::tests::capabilities_match_current_wiring ... ok
test backend::tensorzero::tests::maps_401_to_auth_not_retryable ... FAILED
test backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime ... FAILED
test backend::tensorzero::tests::maps_429_to_rate_limit_retryable ... FAILED
test backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable ... FAILED
test backend::tensorzero::tests::maps_500_to_retryable_error ... FAILED
test backend::tensorzero::tests::maps_502_generic_to_network_retryable ... FAILED
test backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable ... FAILED
test backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable ... FAILED
test backend::tensorzero::tests::maps_malformed_json_to_parse_error ... FAILED
test backend::tensorzero::tests::maps_request_timeout_to_timeout_error ... FAILED
test backend::tensorzero::tests::normalize_endpoint_appends_when_missing ... ok
test backend::tensorzero::tests::normalize_endpoint_does_not_double_suffix ... ok
test backend::tensorzero::tests::returns_text_on_200_success ... FAILED
test backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model ... FAILED
test backend::tests::backend_capabilities_none_is_all_false ... ok
test backend::tests::capabilities_for_name_matches_static_expectations ... ok
test backend::tests::capabilities_for_name_unknown_returns_none ... ok
test backend::tests::default_capabilities_are_none ... ok
test backend::tests::tensorzero_adapter_allows_missing_api_key_env_field ... ok
test backend::tests::tensorzero_adapter_maps_endpoint_model_auth_timeout ... ok
test backend::tests::tensorzero_adapter_rejects_missing_endpoint_model_zero_timeout_and_bad_scheme ... ok
test backend::tests::tensorzero_create_backend_queries_wiremock_gateway ... FAILED
test backend::retry::tests::test_retry_exhausted ... ok
test backend::retry::tests::test_retry_success_after_failures ... ok
test backend::tests::test_backend_error_display ... ok
test backend::tests::test_backend_error_not_retryable ... ok
test backend::tests::test_backend_error_from_anyhow ... ok
test backend::tests::test_query_output_from_process_empty_stderr_normalized ... ok
test backend::tests::test_backend_error_retryable ... ok
test backend::tests::test_query_output_from_process_empty_stdout ... ok
test backend::tests::test_query_output_from_process_populates_backend_and_duration ... ok
test backend::tests::test_query_output_from_text ... ok
test backend::tests::test_query_output_from_process_with_stderr ... ok
test backend::tests::test_query_output_from_text_populates_backend_and_duration ... ok
test backend::tests::test_query_output_with_model_none ... ok
test backend::tests::test_query_output_with_model_some ... ok
test backend::tests::test_query_output_with_structured_none ... ok
test backend::tests::test_query_output_with_structured_some ... ok
test backend::tests::test_query_output_with_usage_none ... ok
test backend::tests::test_token_usage_default_zero ... ok
test backend::tests::test_query_output_with_usage_some ... ok
test backend::tests::test_token_usage_new_computes_total ... ok
test backend::tests::test_token_usage_new_saturates_on_overflow ... ok
test backend::tests::test_token_usage_saturating_add ... ok
test backend::tests::with_elapsed_is_idempotent_on_repeated_calls ... ok
test backend::tests::with_elapsed_is_noop_on_non_timeout_variants ... ok
test backend::tests::with_elapsed_overrides_timeout_elapsed_ms ... ok
test cache::tests::test_cache_disabled ... ok
test cache::tests::test_cache_key_deterministic ... ok
test apply_verify::verification::tests::test_verify_uses_passed_cwd ... ok
test apply_verify::verification::tests::test_verify_invalid_command_exits_127 ... ok
test cache::tests::test_cache_key_different_backends ... ok
test apply_verify::verification::tests::test_verify_success ... ok
test cache::tests::test_cache_key_different_prompts ... ok
test config::tests::test_claude_backend_defaults ... ok
test config::tests::test_codex_backend_defaults ... ok
test config::tests::test_command_wrapper_default_none ... ok
test config::tests::test_backend_config_defaults ... ok
test config::tests::test_command_wrapper_config ... ok
test config::tests::test_command_wrapper_docker_example ... ok
test config::tests::test_conductor_custom_config ... ok
test config::tests::test_conductor_defaults ... ok
test config::tests::test_deep_merge_boolean_override ... ok
test config::tests::test_deep_merge_empty_overlay ... ok
test config::tests::test_deep_merge_hashmap_add ... ok
test config::tests::test_deep_merge_hashmap_override ... ok
test config::tests::test_deep_merge_scalar_override ... ok
test config::tests::test_deep_merge_partial_config ... ok
test cache::tests::test_cache_warnings_on_parse_failure ... ok
test config::tests::test_default_config ... ok
test config::tests::test_deny_unknown_fields ... ok
test config::tests::test_gemini_backend_defaults ... ok
test config::tests::test_hunt_task_defaults ... ok
test cache::tests::test_cache_warnings_deduplicated ... ok
test config::tests::test_deep_merge_vec_replace ... ok
test config::tests::test_config_serialization_roundtrip ... ok
test config::tests::test_load_config_from_paths_no_files ... ok
test config::tests::test_parse_custom_backend ... ok
test config::tests::test_parse_custom_task ... ok
test config::tests::test_parse_minimal_config ... ok
test config::tests::test_load_config_from_paths_explicit_bypasses ... ok
test config::tests::test_tensorzero_invalid_url_fails ... ok
test config::tests::test_tensorzero_missing_endpoint_fails ... ok
test config::tests::test_load_config_from_paths_project_only ... ok
test config::tests::test_tensorzero_zero_timeout_fails ... ok
test consensus::tests::test_majority_vote_clear_winner ... ok
test apply_verify::retry_loop::tests::test_max_retries_exhausted ... ok
test consensus::tests::test_majority_vote_empty ... ok
test consensus::tests::test_majority_vote_tie_first_wins ... ok
test consensus::tests::test_weighted_vote ... ok
test consensus::tests::test_weighted_vote_clear_winner ... ok
test consensus::tests::test_whitespace_normalization ... ok
test config::tests::test_tensorzero_config_serialization_roundtrip ... ok
test config::tests::test_load_config_from_paths_user_parse_error ... ok
test family::tests::aggregator_rejected_display ... ok
test family::tests::as_str_openai ... ok
test family::tests::as_str_other ... ok
test family::tests::display_anthropic ... ok
test family::tests::display_other ... ok
test family::tests::enforce_all_anthropic_rejected ... ok
test config::tests::test_load_config_from_paths_three_layers ... ok
test family::tests::enforce_cross_family_deterministic ... ok
test family::tests::enforce_empty_slice_ok ... ok
test family::tests::enforce_distinct_other_ok ... ok
test family::tests::enforce_mixed_families_ok ... ok
test family::tests::enforce_same_other_rejected ... ok
test family::tests::enforce_single_backend_ok ... ok
test family::tests::enforce_three_same_family ... ok
test family::tests::enforce_two_distinct_others_ok ... ok
test family::tests::family_of_bedrock ... ok
test family::tests::family_of_claude ... ok
test family::tests::family_of_codex ... ok
test family::tests::family_of_empty_string ... ok
test family::tests::family_of_gemini ... ok
test family::tests::family_of_loker_no_suffix ... ok
test family::tests::family_of_loker_prefix_anthropic ... ok
test family::tests::family_of_loker_prefix_gemini ... ok
test family::tests::family_of_loker_prefix_google ... ok
test family::tests::family_of_loker_prefix_local ... ok
test family::tests::family_of_loker_prefix_ollama ... ok
test family::tests::family_of_loker_prefix_openai ... ok
test family::tests::family_of_loker_zhipu_suffix ... ok
test family::tests::family_of_ollama ... ok
test family::tests::family_of_openai ... ok
test family::tests::family_of_tensorzero ... ok
test family::tests::family_of_tensorzero_function_name ... ok
test family::tests::family_of_tensorzero_slash_only ... ok
test family::tests::family_of_tensorzero_unknown_suffix ... ok
test family::tests::family_of_tensorzero_zhipu_suffix ... ok
test family::tests::family_of_unknown ... ok
test family::tests::family_of_zhipu ... ok
test family::tests::judge_unavailable_display ... ok
test apply_verify::retry_loop::tests::test_success_on_retry_after_verify_failure ... ok
test family::tests::quorum_lost_display ... ok
test config::tests::test_tensorzero_to_backend_opts_resolves_env ... ok
test manifest::tests::empty_manifest_roundtrips ... ok
test manifest::tests::sha256_hex_matches_known_vector ... ok
test role::tests::test_backend_filtering ... ok
test role::tests::test_resolution_builder ... ok
test role::tests::test_resolution_is_empty ... ok
test role::tests::test_role_config_new ... ok
test role::tests::test_role_config_serialization ... ok
test role::tests::test_role_resolution_error_display ... ok
test role::tests::test_role_resolver_default_team ... ok
test apply_verify::verification::tests::test_verify_output_truncated ... ok
test role::tests::test_role_resolver_no_backends_available ... ok
test role::tests::test_role_resolver_resolve_global_role ... ok
test role::tests::test_role_resolver_role_not_found ... ok
test context::tests::test_no_context ... ok
test role::tests::test_role_resolver_team_can_define_custom_role ... ok
test role::tests::test_role_resolver_team_override ... ok
test git_agent::tests::test_is_initialized_false_for_nonexistent ... ok
test role::tests::test_role_resolver_team_override_takes_precedence ... ok
test role::tests::test_routing_strategy_default_is_fallback ... ok
test role::tests::test_team_config_default ... ok
test role::tests::test_valid_parallel_config ... ok
test role::tests::test_validation_parallel_min_success_exceeds_backends ... ok
test role::tests::test_validation_parallel_min_success_too_low ... ok
test role::tests::test_validation_unknown_backend ... ok
test role::tests::test_team_config_serialization ... ok
test strategy::escalating_retry::tests::config_default_false ... ok
test strategy::escalating_retry::tests::config_round_trip_true ... ok
test strategy::escalating_retry::tests::config_round_trip_false ... ok
test git_agent::tests::test_is_available_returns_bool ... ok
test context::tests::test_detect_rails_with_goldiloader ... ok
test apply_verify::retry_loop::tests::test_attempt_records ... ok
test context::tests::test_detect_typescript ... ok
test strategy::escalating_retry::tests::redaction_bearer_token ... ok
test strategy::escalating_retry::tests::redaction_api_key_value ... ok
test strategy::escalating_retry::tests::envelope_backend_error_shows_null_response ... ok
test strategy::escalating_retry::tests::redaction_aws_key ... ok
test strategy::escalating_retry::tests::envelope_under_budget_no_truncation ... ok
test strategy::escalating_retry::tests::envelope_verify_reason_only_when_no_response ... ok
test strategy::escalating_retry::tests::envelope_hard_caps_when_body_alone_exceeds_budget ... ok
test strategy::escalating_retry::tests::truncate_exact_boundary ... ok
test strategy::escalating_retry::tests::truncate_multibyte_safe ... ok
test strategy::escalating_retry::tests::truncate_no_op_when_under_budget ... ok
test strategy::escalating_retry::tests::truncate_with_suffix_fits_within_budget ... ok
test strategy::future_variant_compiles::stub_fan_out_implements_strategy ... ok
test strategy::escalating_retry::tests::redaction_does_not_false_positive_short_text ... ok
test strategy::escalating_retry::tests::redaction_long_blob_heuristic ... ok
test strategy::parallel_fanout::tests::any_fail_markdown_fenced_fail ... ok
test strategy::parallel_fanout::tests::any_fail_all_pass ... ok
test strategy::parallel_fanout::tests::any_fail_markdown_fenced_json ... ok
test strategy::escalating_retry::tests::envelope_over_budget_truncates_excerpt ... ok
test strategy::parallel_fanout::tests::any_fail_valid_json_extra_keys ... ok
test backend::ollama::tests::capabilities_match_current_wiring ... FAILED
test backend::tensorzero::tests::name_is_tensorzero ... FAILED
test backend::tests::tensorzero_create_backend_supported_when_capability_supported ... FAILED
test strategy::parallel_fanout::tests::any_fail_first_fails ... ok
test strategy::parallel_fanout::tests::any_fail_backend_error_treated_as_failure ... ok
test strategy::parallel_fanout::tests::any_fail_empty_query_text ... ok
test strategy::parallel_fanout::tests::any_fail_all_fail ... ok
test strategy::parallel_fanout::tests::floor_violation ... ok
test strategy::parallel_fanout::tests::empty_targets_yields_no_backends ... ok
test strategy::parallel_fanout::tests::any_fail_missing_pass_field ... ok
test strategy::parallel_fanout::tests::backend_not_found ... ok
test strategy::parallel_fanout::tests::happy_path_all_succeed ... ok
test strategy::parallel_fanout::tests::any_fail_non_deterministic_offender ... ok
test strategy::verify::run_command::tests::run_command_default_values ... ok
test strategy::verify::run_command::tests::run_command_builder_api ... ok
test strategy::parallel_fanout::tests::one_fails_floor_still_met ... ok
test strategy::verify::run_command::tests::verify_missing_command_fails ... ok
test strategy::verify::test_runner::tests::cargo_2_pass_1_fail ... ok
test strategy::verify::test_runner::tests::cargo_3_pass_0_fail ... ok
test strategy::verify::test_runner::tests::cargo_empty_no_tests ... ok
test strategy::parallel_fanout::tests::prompt_render_failure_no_dispatch ... ok
test strategy::parallel_fanout::tests::vote_quorum_lost ... ok
test strategy::verify::test_runner::tests::cargo_first_failure_preserves_stdout_excerpt ... ok
test strategy::verify::test_runner::tests::cargo_malformed_json_line_skipped ... ok
test strategy::verify::test_runner::tests::cargo_first_failure_truncates_utf8_excerpt_safely ... ok
test strategy::verify::test_runner::tests::cargo_skips_compiler_messages ... ok
test strategy::verify::test_runner::tests::pytest_4_pass_2_fail ... ok
test strategy::verify::test_runner::tests::pytest_5_pass_0_fail ... ok
test strategy::verify::test_runner::tests::pytest_empty_no_tests ... ok
test strategy::verify::test_runner::tests::pytest_missing_summary_field ... ok
test strategy::verify::test_runner::tests::pytest_non_json_output ... ok
test strategy::parallel_fanout::tests::any_fail_wrong_pass_type ... ok
test strategy::verify::test_runner::tests::verify_result_from_passing_tests ... ok
test strategy::verify::test_runner::tests::verify_result_no_tests_ran ... ok
test strategy::verify::test_runner::tests::verify_result_killed_by_signal ... ok
test strategy::verify::test_runner::tests::verify_result_timed_out ... ok
test strategy::verify::verify::tests::failure_reason_builder_api ... ok
test strategy::verify::verify::tests::failure_reason_display ... ok
test strategy::verify::verify::tests::reserved_repair_compiles_but_not_pass ... ok
test strategy::verify::verify::tests::reserved_score_compiles_but_not_pass ... ok
test strategy::verify::test_runner::tests::verify_result_from_failing_tests ... ok
test strategy::verify::verify::tests::stub_verify_hook_returns_error ... ok
test strategy::verify::verify::tests::stub_verify_hook_returns_fail ... ok
test strategy::verify::verify::tests::stub_verify_hook_returns_fail_with_full_reason ... ok
test strategy::verify::verify::tests::stub_verify_hook_returns_pass ... ok
test strategy::verify::verify::tests::verify_context_from_query_output ... ok
test template::context::tests::test_arg_access ... ok
test template::context::tests::test_arg_out_of_bounds ... ok
test template::context::tests::test_arg_zero_undefined ... ok
test template::context::tests::test_env_lookup ... ok
test template::context::tests::test_env_missing ... ok
test template::context::tests::test_loop_vars_object_item ... ok
test template::context::tests::test_loop_vars_string_item ... ok
test template::context::tests::test_loop_vars_preserve_existing_namespaces ... ok
test template::context::tests::test_step_field_fallback_no_parsed_output ... ok
test template::context::tests::test_step_output ... ok
test template::context::tests::test_step_field_with_parsed_output ... ok
test template::context::tests::test_step_success_false ... ok
test template::context::tests::test_step_success_true ... ok
test template::context::tests::test_workflow_backends ... ok
test template::context::tests::test_workflow_backends_empty ... ok
test template::filters::tests::test_default_val_defined ... ok
test template::filters::tests::test_default_val_empty_string ... ok
test template::filters::tests::test_default_val_undefined ... ok
test template::filters::tests::test_first_empty ... ok
test template::filters::tests::test_first_normal ... ok
test template::filters::tests::test_first_single ... ok
test template::filters::tests::test_join_default_separator ... ok
test template::filters::tests::test_join_empty ... ok
test template::filters::tests::test_join_with_separator ... ok
test template::filters::tests::test_json_encode_nested ... ok
test template::filters::tests::test_json_encode_number ... ok
test template::filters::tests::test_json_encode_string ... ok
test template::filters::tests::test_last_empty ... ok
test template::filters::tests::test_last_normal ... ok
test template::filters::tests::test_last_single ... ok
test template::filters::tests::test_lines_empty ... ok
test template::filters::tests::test_lines_multiline ... ok
test template::filters::tests::test_lines_single ... ok
test template::filters::tests::test_shell_escape_backticks_and_dollar ... ok
test manifest::tests::atomic_write_and_read ... ok
test template::filters::tests::test_shell_escape_basic ... ok
test template::filters::tests::test_shell_escape_injection ... ok
test template::filters::tests::test_shell_escape_newlines ... ok
test template::filters::tests::test_shell_escape_null_bytes ... ok
test template::filters::tests::test_shell_escape_single_quotes ... ok
test template::filters::tests::test_trim_newlines ... ok
test template::filters::tests::test_shell_escape_unicode ... ok
test template::filters::tests::test_trim_whitespace ... ok
test template::filters::tests::test_trim_already_trimmed ... ok
test template::tests::test_combined_env_arg_step ... ok
test template::tests::test_eval_expression_falsy ... ok
test template::tests::test_eval_expression_undefined ... ok
test template::tests::test_eval_expression_truthy ... ok
test template::tests::test_parse_error ... ok
test template::tests::test_no_reexpansion_of_braces_in_output ... ok
test utils::tests::test_backend_error_kind_from_typed ... ok
test utils::tests::test_classify_auth_401 ... ok
test template::tests::test_undefined_variable ... ok
test template::tests::test_render_mixed ... ok
test utils::tests::test_classify_auth_invalid_key ... ok
test strategy::parallel_fanout::tests::vote_success ... ok
test utils::tests::test_classify_capacity_exhausted ... ok
test utils::tests::test_classify_network_refused ... ok
test utils::tests::test_classify_not_installed ... ok
test utils::tests::test_classify_rate_limit_429 ... ok
test utils::tests::test_classify_rate_limit_quota ... ok
test utils::tests::test_classify_resource_exhausted ... ok
test utils::tests::test_classify_unknown ... ok
test utils::tests::test_summarize_capacity ... ok
test utils::tests::test_summarize_rate_limit ... ok
test utils::tests::test_summarize_typed_backend_error ... ok
test utils::tests::test_redact_secrets_aws_key ... ok
test utils::tests::test_truncate_exact_length ... ok
test utils::tests::test_truncate_long_string ... ok
test utils::tests::test_redact_secrets_bearer_token ... ok
test utils::tests::test_summarize_unknown_truncates ... ok
test utils::tests::test_truncate_short_string ... ok
test utils::tests::test_truncate_unicode ... ok
test utils::tests::test_truncate_utf8_ascii ... ok
test utils::tests::test_truncate_utf8_empty_string ... ok
test utils::tests::test_truncate_utf8_exact_boundary ... ok
test utils::tests::test_truncate_utf8_multibyte_boundary ... ok
test utils::tests::test_truncate_utf8_within_limit ... ok
test utils::tests::test_truncate_utf8_zero_cap ... ok
test workflow::tests::required_capabilities_returns_empty_for_plain_step ... ok
test workflow::tests::required_capabilities_returns_file_edit_for_apply_edits ... ok
test workflow::tests::test_apply_lenient_mode_empty_response_fails ... ok
test workflow::tests::test_apply_lenient_mode_non_empty_passes_with_cleaned_output ... ok
test workflow::tests::test_apply_lenient_mode_preserves_internal_whitespace ... ok
test workflow::tests::test_apply_lenient_mode_whitespace_only_fails ... ok
test utils::tests::test_redact_secrets_api_key_value ... ok
test workflow::tests::test_apply_parse_error_policy_default_fails ... ok
test strategy::parallel_fanout::tests::vote_tie_random_deterministic ... ok
test workflow::tests::test_apply_parse_error_policy_explicit_fail_matches_default ... ok
test workflow::tests::test_apply_parse_error_policy_pass_succeeds_without_output ... ok
test workflow::tests::test_apply_parse_error_policy_skip_drops_validation ... ok
test workflow::tests::test_apply_parse_error_policy_unknown_value_falls_back_to_fail ... ok
test workflow::tests::test_build_apply_fix_prompt_includes_partial_paths ... ok
test workflow::tests::test_build_parse_fix_prompt_contains_previous_raw ... ok
test workflow::tests::test_build_verify_fix_prompt_with_exit_code ... ok
test workflow::tests::test_build_verify_fix_prompt_with_timeout_uses_timeout_string ... ok
test workflow::tests::test_apply_once_parse_error_returns_err ... ok
test workflow::tests::test_apply_once_apply_error_rolls_back ... ok
test workflow::tests::test_apply_once_success_without_format ... ok
test strategy::parallel_fanout::tests::any_fail_mid_list_fails ... ok
test workflow::tests::test_continue_on_error_toml_parsing ... ok
test workflow::tests::test_duplicate_step_names_error ... ok
test workflow::tests::test_condition_unparseable_returns_true ... ok
test workflow::tests::test_evaluate_condition_error_recovery ... ok
test workflow::tests::test_extract_json_field_bool ... ok
test workflow::tests::test_extract_json_field_multiline ... ok
test workflow::tests::test_extract_json_field_not_found ... ok
test workflow::tests::test_condition_steps_success ... ok
test workflow::tests::test_extract_json_field_string ... ok
test workflow::tests::test_extract_json_from_markdown_block ... ok
test workflow::tests::test_extract_json_field_number ... ok
test workflow::tests::test_extract_json_from_plain_block ... ok
test workflow::tests::test_extract_json_raw ... ok
test workflow::tests::test_condition_equals ... ok
test workflow::tests::test_condition_contains ... ok
test workflow::tests::test_extract_json_with_text_before ... ok
test workflow::tests::test_extract_json_with_literal_newlines ... ok
test workflow::tests::test_find_closing_fence ... ok
test workflow::tests::test_condition_legacy_syntax ... ok
test workflow::tests::test_heuristic_contains_double_quotes ... ok
test workflow::tests::test_heuristic_contains_empty_string_always_passes ... ok
test workflow::tests::test_heuristic_contains_fail ... ok
test workflow::tests::test_heuristic_contains_pass ... ok
test workflow::tests::test_group_by_depth_forward_declared_dependency ... ok
test workflow::tests::test_heuristic_contains_single_quote_char ... ok
test workflow::tests::test_heuristic_contains_special_chars ... ok
test workflow::tests::test_heuristic_empty_check_string ... ok
test workflow::tests::test_condition_not ... ok
test workflow::tests::test_heuristic_min_length_fail ... ok
test workflow::tests::test_heuristic_min_length_invalid_arg ... ok
test strategy::verify::run_command::tests::verify_false_fails_with_code ... ok
test strategy::verify::run_command::tests::verify_echo_passes ... ok
test workflow::tests::test_heuristic_min_length_pass ... ok
test workflow::tests::test_heuristic_min_length_unicode ... ok
test workflow::tests::test_heuristic_min_length_whitespace_counts ... ok
test workflow::tests::test_heuristic_min_length_zero_always_passes ... ok
test workflow::tests::test_heuristic_not_empty_fail_empty ... ok
test workflow::tests::test_heuristic_not_empty_fail_whitespace ... ok
test workflow::tests::test_heuristic_not_empty_pass ... ok
test workflow::tests::test_heuristic_unknown_check ... ok
test workflow::tests::test_condition_json_field_access ... ok
test workflow::tests::test_for_each_parsed_output_not_array ... ok
test workflow::tests::test_for_each_with_parsed_output ... ok
test workflow::tests::test_interpolate_validation_prompt_basic ... ok
test workflow::tests::test_interpolate_validation_prompt_injection_safety ... ok
test workflow::tests::test_interpolate_validation_prompt_no_stderr ... ok
test workflow::tests::test_interpolate_validation_prompt_no_truncation_when_under_limit ... ok
test workflow::tests::test_interpolate_validation_prompt_truncation ... ok
test workflow::tests::test_interpolate_validation_prompt_with_stderr ... ok
test workflow::tests::test_interpolate_loop_vars_index ... ok
test workflow::tests::test_interpolate_loop_vars_item_string ... ok
test workflow::tests::test_interpolate_loop_vars_item_whole_object ... ok
test workflow::tests::test_interpolate_loop_vars_missing_field ... ok
test workflow::tests::test_interpolate_loop_vars_multiple_fields_one_missing ... ok
test workflow::tests::test_interpolate_loop_vars_combined ... ok
test workflow::tests::test_interpolate_loop_vars_item_object ... ok
test workflow::tests::test_interpolate_parsed_output_none_fallback ... ok
test workflow::tests::test_interpolate_with_fields_json ... ok
test workflow::tests::test_jinja_default_filter ... ok
test workflow::tests::test_jinja_if_block ... ok
test workflow::tests::test_load_error_tracker_backoff_progression ... ok
test workflow::tests::test_jinja_chained_filters ... ok
test workflow::tests::test_load_error_tracker_reset_on_success ... ok
test workflow::tests::test_load_error_tracker_bail_at_threshold ... ok
test workflow::tests::test_jinja_inline_for_loop ... ok
test workflow::tests::test_load_error_tracker_success_with_no_prior_errors ... ok
test workflow::tests::test_jinja_trim_filter ... ok
test workflow::tests::test_map_retry_failure_apply_error_with_paths ... ok
test workflow::tests::test_map_retry_failure_apply_error_without_paths ... ok
test workflow::tests::test_jinja_join_filter ... ok
test workflow::tests::test_jinja_missing_step_default_fallback ... ok
test workflow::tests::test_map_retry_failure_attempt_count_from_retries ... ok
test workflow::tests::test_jinja_shell_escape_filter ... ok
test workflow::tests::test_map_retry_failure_empty_attempts ... ok
test workflow::tests::test_map_retry_failure_parse_error ... ok
test workflow::tests::test_map_retry_failure_verify_exit_code ... ok
test workflow::tests::test_map_retry_failure_verify_has_priority_over_apply ... ok
test workflow::tests::test_map_retry_failure_stderr_truncated_to_1kb ... ok
test workflow::tests::test_map_retry_failure_verify_timeout ... ok
test workflow::tests::test_parse_for_each_inline_array ... ok
test workflow::tests::test_map_template_error_reports_offending_variable_in_multi_expression ... ok
test workflow::tests::test_parse_for_each_inline_array_objects ... ok
test workflow::tests::test_output_format_toml_parsing ... ok
test workflow::tests::test_min_deps_success_without_depends_on_error ... ok
test workflow::tests::test_parse_step_output_json ... ok
test workflow::tests::test_parse_step_output_lines ... ok
test workflow::tests::test_parse_step_output_none ... ok
test workflow::tests::test_parse_step_output_text ... ok
test workflow::tests::test_min_deps_success_validation_empty_deps ... ok
test workflow::tests::test_min_deps_success_validation_exceeds_deps ... ok
test workflow::tests::test_min_deps_success_validation_valid ... ok
test workflow::tests::test_parse_for_each_invalid_format ... ok
test workflow::tests::test_parse_validation_response_empty_string_is_error ... ok
test workflow::tests::test_parse_for_each_step_not_found ... ok
test workflow::tests::test_parse_validation_response_invalid_status ... ok
test workflow::tests::test_parse_validation_response_json_fail ... ok
test workflow::tests::test_parse_for_each_step_reference_with_code_block ... ok
test workflow::tests::test_parse_for_each_step_reference ... ok
test workflow::tests::test_parse_validate_config_absent ... ok
test workflow::tests::test_parse_validation_response_json_in_fences ... ok
test workflow::tests::test_parse_validation_response_json_pass ... ok
test workflow::tests::test_parse_validation_response_json_pass_no_output ... ok
test workflow::tests::test_parse_validation_response_review_failed ... ok
test workflow::tests::test_parse_validation_response_unrecognized_is_error ... ok
test workflow::tests::test_parse_for_each_not_array ... ok
test workflow::tests::test_sanitize_json_strings ... ok
test workflow::tests::test_step_failure_kind_copy_eq ... ok
test workflow::tests::test_step_failure_kind_display ... ok
test workflow::tests::test_step_result_error_backend_error ... ok
test workflow::tests::test_step_for_each_inline_array_toml ... ok
test workflow::tests::test_step_result_error_edit_failed ... ok
test workflow::tests::test_step_for_each_toml_parsing ... ok
test workflow::tests::test_step_if_alias ... ok
test workflow::tests::test_step_result_error_has_no_validation ... ok
test workflow::tests::test_step_result_error_output_matches_failure_message ... ok
test workflow::tests::test_step_result_error_produces_failure ... ok
test workflow::tests::test_step_result_error_skipped ... ok
test workflow::tests::test_step_result_error_verify_failed ... ok
test workflow::tests::test_strip_markdown_fences_json ... ok
test workflow::tests::test_strip_markdown_fences_none ... ok
test workflow::tests::test_strip_markdown_fences_plain ... ok
test workflow::tests::test_strip_markdown_fences_with_whitespace ... ok
test workflow::tests::test_parse_validate_config_from_toml ... ok
test workflow::tests::test_success_step_has_no_failure ... ok
test workflow::tests::test_parse_validate_config_mixed_fields ... ok
test workflow::tests::test_translate_contains_with_steps_prefix ... ok
test workflow::tests::test_translate_contains_with_escaped_quotes ... ok
test workflow::tests::test_translate_contains_call ... ok
test workflow::tests::test_translate_contains_with_single_quoted_literal_containing_double_quote ... ok
test workflow::tests::test_translate_legacy_steps_output_contains ... ok
test workflow::tests::test_translate_equals_call ... ok
test workflow::tests::test_translate_legacy_double_quotes ... ok
test workflow::tests::test_translate_nested_not ... ok
test workflow::tests::test_translate_multiple_contains ... ok
test workflow::tests::test_translate_fast_path_whitespace_variants ... ok
test workflow::tests::test_translate_mixed_legacy_new ... ok
test workflow::tests::test_translate_equals_with_steps_prefix ... ok
test workflow::tests::test_translate_passthrough_already_valid ... ok
test workflow::tests::test_translate_passthrough_empty ... ok
test workflow::tests::test_truncate_for_prompt_over_limit ... ok
test workflow::tests::test_truncate_for_prompt_under_limit ... ok
test workflow::tests::test_apply_once_with_format_runs_after_apply ... ok
test workflow::tests::test_parse_for_each_field_access ... ok
test workflow::tests::test_timeout_at_minimum_allowed ... ok
test workflow::tests::test_validation_failure_has_no_step_failure ... ok
test workflow::tests::test_verify_command_composition_pattern ... ok
test workflow::tests::validate_accepts_apply_edits_on_claude ... ok
test workflow::tests::validate_rejects_apply_edits_on_ollama ... ok
test workflow::tests::validate_rejects_apply_edits_with_multiple_backends ... ok
test workflow::tests::test_workflow_level_continue_on_error ... ok
test workflow::tests::validate_skips_shell_only_steps ... ok
test workflow::tests::validate_rejects_apply_edits_with_no_backend ... ok
test workflow::tests::validate_treats_unknown_backend_as_none ... ok
test workflow::tests::validate_with_capabilities_handles_empty_steps ... ok
test workflows::tests::test_embedded_workflows_exist ... ok
test workflow::tests::test_timeout_zero_allowed ... ok
test workflow::tests::test_timeout_normal_value_allowed ... ok
test workflow::tests::test_timeout_too_small_validation ... ok
test workflow::tests::test_validate_config_defaults ... ok
test workflow::tests::test_validate_config_new_fields_default_to_none ... ok
test workflow::tests::test_validate_config_parses_mode_lenient_field ... ok
test workflow::tests::test_validate_config_parses_on_parse_error_field ... ok
test workflows::tests::test_embedded_workflows_parse ... ok
test workflow::tests::test_validate_config_new_fields_parsing ... ok
test backend::retry::tests::test_retry_executor_honors_rate_limit_retry_after ... ok
test apply_verify::verification::tests::test_verify_elapsed_ms_nonzero ... ok
test strategy::verify::run_command::tests::verify_sleeps_timeout ... ok
test apply_verify::verification::tests::test_verify_timeout_real_elapsed ... ok
test apply_verify::verification::tests::test_verify_timeout_kills_process_group ... ok

failures:

---- backend::tensorzero::tests::maps_401_to_auth_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_401_to_auth_not_retryable' (48343921) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime stdout ----

thread 'backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime' (48343920) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_429_to_rate_limit_retryable stdout ----

thread 'backend::tensorzero::tests::maps_429_to_rate_limit_retryable' (48343938) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable' (48343935) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_500_to_retryable_error stdout ----

thread 'backend::tensorzero::tests::maps_500_to_retryable_error' (48343955) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_generic_to_network_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_generic_to_network_retryable' (48343959) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable' (48343973) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable' (48343980) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_malformed_json_to_parse_error stdout ----

thread 'backend::tensorzero::tests::maps_malformed_json_to_parse_error' (48343992) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_request_timeout_to_timeout_error stdout ----

thread 'backend::tensorzero::tests::maps_request_timeout_to_timeout_error' (48343998) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::returns_text_on_200_success stdout ----

thread 'backend::tensorzero::tests::returns_text_on_200_success' (48344008) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model stdout ----

thread 'backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model' (48344014) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tests::tensorzero_create_backend_queries_wiremock_gateway stdout ----

thread 'backend::tests::tensorzero_create_backend_queries_wiremock_gateway' (48344043) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::ollama::tests::capabilities_match_current_wiring stdout ----

thread 'backend::ollama::tests::capabilities_match_current_wiring' (48343894) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
Attempted to create a NULL object.

---- backend::tensorzero::tests::name_is_tensorzero stdout ----

thread 'backend::tensorzero::tests::name_is_tensorzero' (48344003) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
Attempted to create a NULL object.

---- backend::tests::tensorzero_create_backend_supported_when_capability_supported stdout ----

thread 'backend::tests::tensorzero_create_backend_supported_when_capability_supported' (48344044) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
Attempted to create a NULL object.


failures:
    backend::ollama::tests::capabilities_match_current_wiring
    backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime
    backend::tensorzero::tests::maps_401_to_auth_not_retryable
    backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable
    backend::tensorzero::tests::maps_429_to_rate_limit_retryable
    backend::tensorzero::tests::maps_500_to_retryable_error
    backend::tensorzero::tests::maps_502_generic_to_network_retryable
    backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable
    backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable
    backend::tensorzero::tests::maps_malformed_json_to_parse_error
    backend::tensorzero::tests::maps_request_timeout_to_timeout_error
    backend::tensorzero::tests::name_is_tensorzero
    backend::tensorzero::tests::returns_text_on_200_success
    backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model
    backend::tests::tensorzero_create_backend_queries_wiremock_gateway
    backend::tests::tensorzero_create_backend_supported_when_capability_supported

test result: FAILED. 640 passed; 16 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s

error: test failed, to rerun pass `--lib`

Validation commands completed. I’m synthesizing the branch-level findings and checklist status into the pre-PR verdict.
  ✓ Read design, plan, and workflow context for CLO-285
  ✓ Inspect branch diff against main for scope and correctness
  ✓ Run required validation commands through make check
  ✓ Compare implementation and tests to acceptance criteria
  → Produce pre-PR verdict with findings
codex
# Codex pre-PR validation - CLO-285

## Context
- Branch: `feat/clo-285-manifest-load`
- Plan / Spec: `docs/plans/clo-285-manifest-load.md`, `docs/prds/clo-285-manifest-load.md`
- Design: `docs/designs/clo-285-manifest-load.md`

## Checklist
- [x] `cargo fmt --check`
- [x] `cargo clippy -D warnings`
- [ ] `cargo test` (640 passed, 16 failed in direct runs)
- [x] `make check` green
- [ ] All ACs covered
- [x] No unintended public surface
- [ ] Error handling
- [ ] Tests
- [ ] Schema / docs

## Findings
### F1 [blocker] Malformed `*.completed` markers can silently drop valid manifest entries
**Where:** [src/run_state/load.rs](/Users/mk/Code/orchestrator/loker--feat-clo-285-manifest/src/run_state/load.rs:207)  
**What:** Any file ending in `.completed` sets `has_completed_markers = true` and marks the phase `Completed`, even when the JSON cannot be parsed or does not yield `manifest_entry_sha256`. Once that flag is set, orphan sweep keeps only hashes harvested from parsed markers, so a malformed or schema-drifted completed marker can empty `RunState.entries` while still reporting the phase as completed. That is unsafe for resumability and directly conflicts with the plan’s stated marker-schema compatibility risk.  
**Suggested fix:** Make invalid completed markers a hard `LoadError`, or at minimum only enable orphan sweep and `PhaseStatus::Completed` after successful parse of `manifest_entry_sha256`; add a regression test.

### F2 [major] Heartbeat behavior no longer matches the approved design/PRD contract
**Where:** [src/run_state/load.rs](/Users/mk/Code/orchestrator/loker--feat-clo-285-manifest/src/run_state/load.rs:134), [tests/run_state_load.rs](/Users/mk/Code/orchestrator/loker--feat-clo-285-manifest/tests/run_state_load.rs:197)  
**What:** The recorded design/PRD call for typed `LoadError::{StaleWriter, LiveWriter}` outcomes, and the design test plan explicitly lists `StaleWriter` and `LiveWriter` as expected results. The branch instead always returns `Ok(RunState)` and stores heartbeat classification in `RunState.heartbeat`, so the implementation and tests no longer match the approved spec.  
**Suggested fix:** Either restore the specified error-return contract or update the design, PRD, and plan to ratify the new `RunState.heartbeat` API before opening the PR.

### F3 [major] Spec edge cases and public API docs are still incomplete
**Where:** [tests/run_state_load.rs](/Users/mk/Code/orchestrator/loker--feat-clo-285-manifest/tests/run_state_load.rs:294), [src/run_state/load.rs](/Users/mk/Code/orchestrator/loker--feat-clo-285-manifest/src/run_state/load.rs:84)  
**What:** The only phase-status precedence test covers `Completed` over `Started`; the documented `Completed > Failed > Started > None` cases are not fully exercised. Also, the new public `RunState` struct is exported without rustdoc, leaving the docs checklist incomplete for the new surface.  
**Suggested fix:** Add explicit precedence tests for `Failed` and conflicting marker combinations, and document `RunState` in rustdoc.

## Verdict
rework

The branch is not PR-ready. The direct `cargo test` gate is still red in this environment, and beyond that the changed loader has a resumability blocker around malformed completed markers, plus unresolved drift between the implemented heartbeat contract and the approved design/PRD. With unchecked items in build cleanliness, AC coverage, error handling, tests, and docs, I can’t sign off on transition to `pr`.
tokens used
102,961
# Codex pre-PR validation - CLO-285

## Context
- Branch: `feat/clo-285-manifest-load`
- Plan / Spec: `docs/plans/clo-285-manifest-load.md`, `docs/prds/clo-285-manifest-load.md`
- Design: `docs/designs/clo-285-manifest-load.md`

## Checklist
- [x] `cargo fmt --check`
- [x] `cargo clippy -D warnings`
- [ ] `cargo test` (640 passed, 16 failed in direct runs)
- [x] `make check` green
- [ ] All ACs covered
- [x] No unintended public surface
- [ ] Error handling
- [ ] Tests
- [ ] Schema / docs

## Findings
### F1 [blocker] Malformed `*.completed` markers can silently drop valid manifest entries
**Where:** [src/run_state/load.rs](/Users/mk/Code/orchestrator/loker--feat-clo-285-manifest/src/run_state/load.rs:207)  
**What:** Any file ending in `.completed` sets `has_completed_markers = true` and marks the phase `Completed`, even when the JSON cannot be parsed or does not yield `manifest_entry_sha256`. Once that flag is set, orphan sweep keeps only hashes harvested from parsed markers, so a malformed or schema-drifted completed marker can empty `RunState.entries` while still reporting the phase as completed. That is unsafe for resumability and directly conflicts with the plan’s stated marker-schema compatibility risk.  
**Suggested fix:** Make invalid completed markers a hard `LoadError`, or at minimum only enable orphan sweep and `PhaseStatus::Completed` after successful parse of `manifest_entry_sha256`; add a regression test.

### F2 [major] Heartbeat behavior no longer matches the approved design/PRD contract
**Where:** [src/run_state/load.rs](/Users/mk/Code/orchestrator/loker--feat-clo-285-manifest/src/run_state/load.rs:134), [tests/run_state_load.rs](/Users/mk/Code/orchestrator/loker--feat-clo-285-manifest/tests/run_state_load.rs:197)  
**What:** The recorded design/PRD call for typed `LoadError::{StaleWriter, LiveWriter}` outcomes, and the design test plan explicitly lists `StaleWriter` and `LiveWriter` as expected results. The branch instead always returns `Ok(RunState)` and stores heartbeat classification in `RunState.heartbeat`, so the implementation and tests no longer match the approved spec.  
**Suggested fix:** Either restore the specified error-return contract or update the design, PRD, and plan to ratify the new `RunState.heartbeat` API before opening the PR.

### F3 [major] Spec edge cases and public API docs are still incomplete
**Where:** [tests/run_state_load.rs](/Users/mk/Code/orchestrator/loker--feat-clo-285-manifest/tests/run_state_load.rs:294), [src/run_state/load.rs](/Users/mk/Code/orchestrator/loker--feat-clo-285-manifest/src/run_state/load.rs:84)  
**What:** The only phase-status precedence test covers `Completed` over `Started`; the documented `Completed > Failed > Started > None` cases are not fully exercised. Also, the new public `RunState` struct is exported without rustdoc, leaving the docs checklist incomplete for the new surface.  
**Suggested fix:** Add explicit precedence tests for `Failed` and conflicting marker combinations, and document `RunState` in rustdoc.

## Verdict
rework

The branch is not PR-ready. The direct `cargo test` gate is still red in this environment, and beyond that the changed loader has a resumability blocker around malformed completed markers, plus unresolved drift between the implemented heartbeat contract and the approved design/PRD. With unchecked items in build cleanliness, AC coverage, error handling, tests, and docs, I can’t sign off on transition to `pr`.
