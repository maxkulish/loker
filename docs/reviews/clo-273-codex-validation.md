Reading prompt from stdin...
OpenAI Codex v0.128.0 (research preview)
--------
workdir: /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
model: gpt-5.4
provider: openai
approval: never
sandbox: workspace-write [workdir, /tmp, $TMPDIR, /Users/mk/.codex/memories]
reasoning effort: high
reasoning summaries: none
session id: 019de779-5158-7810-a021-46acb6289096
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
# Codex pre-PR validation - CLO-273

## Context
- Branch: feat/clo-273-test-runner
- Plan / Spec: docs/plans/clo-273-test-runner.md
- Design: docs/designs/clo-273-test-runner.md

## Checklist
- [x] cargo fmt --check
- [x] cargo clippy -D warnings
- [x] cargo test (n passed)
- [x] make check green
- [x] All ACs covered
- [x] No unintended public surface
- [x] Error handling
- [x] Tests
- [x] Schema / docs

## Findings
### F1 [severity] one-line
**Where:** file:line
**What:** 2-3 sentences
**Suggested fix:** concrete

## Verdict
approve | approve_with_changes | rework

one-paragraph rationale
```

Severity: blocker, major, minor, nit.

The verdict line MUST appear verbatim and must be one of the three
canonical strings - the orchestrator parses it.

## Hard rules

- The verdict is binding. If you write `approve`, you are signing off
  on the change being PR-ready.
- Never recommend bypassing pre-commit hooks (`--no-verify`) or signing
  (`--no-gpg-sign`).
- Never recommend force-pushing an existing PR branch without warning.
- Never approve while any item in the checklist is `[ ]`.

---

You are a senior code reviewer. Review all changes on this branch against
this task's design document and implementation plan.

Inputs:
- Branch: ${BRANCH}
- Design: docs/designs/clo-273-test-runner.md
- Plan: docs/plans/clo-273-test-runner.md
- Diff: git diff main...HEAD

Check for correctness, completeness, regressions, code quality, security,
schema/API compatibility, and scope creep.

Output markdown with findings grouped by severity. End with:
## Verdict
approve | approve_with_changes | rework

codex
Reviewing the branch against the design, plan, and diff first, then I’ll run the validation commands in the same order as the pre-PR checklist.
exec
/bin/zsh -lc 'git branch --show-current' in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
exec
/bin/zsh -lc 'git status --short' in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
 M docs/reviews/clo-273-codex-validation.md
 M docs/reviews/clo-273-gemini-validation.md
 M docs/status/clo-273-workflow.yaml

 succeeded in 0ms:
feat/clo-273-test-runner

exec
/bin/zsh -lc "rg --files docs | rg 'clo-273-(test-runner|workflow)\\.(md|yaml)"'$'"'" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
docs/status/clo-273-workflow.yaml
docs/prds/clo-273-test-runner.md
docs/plans/clo-273-test-runner.md
docs/designs/clo-273-test-runner.md

exec
/bin/zsh -lc 'git diff --stat main...HEAD' in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
 docs/designs/clo-273-test-runner.md                | 179 +++++
 docs/discovery/clo-273.md                          |  78 +++
 docs/plans/clo-273-test-runner.md                  |  74 +++
 docs/prds/clo-273-test-runner.md                   |  40 ++
 docs/reviews/clo-273-codex-validation.md           |  21 +
 docs/reviews/clo-273-gemini-validation.md          |  45 ++
 docs/reviews/clo-273-validation-synthesis.md       |  24 +
 docs/status/clo-273-workflow.yaml                  | 164 +++++
 src/strategy/verify/mod.rs                         |   3 +
 src/strategy/verify/run_command.rs                 |  26 +-
 src/strategy/verify/test_runner.rs                 | 718 +++++++++++++++++++++
 tests/fixtures/test_runner/cargo_2pass_1fail.jsonl |   3 +
 tests/fixtures/test_runner/cargo_3pass_0fail.jsonl |   3 +
 tests/fixtures/test_runner/cargo_empty.jsonl       |   3 +
 tests/fixtures/test_runner/cargo_malformed.jsonl   |   4 +
 tests/fixtures/test_runner/pytest_4pass_2fail.json |  17 +
 tests/fixtures/test_runner/pytest_5pass_0fail.json |  16 +
 tests/verify_test_runner.rs                        | 258 ++++++++
 18 files changed, 1663 insertions(+), 13 deletions(-)

codex
I’ve confirmed the target branch and the change set. Next I’m checking the design/plan requirements against the actual Rust diff before I spend time on the full gate commands.
  → Read the CLO-273 design, plan, and workflow context to extract acceptance criteria and planned tasks
  • Inspect the branch diff for correctness, scope, public surface, error handling, and test coverage
  • Run the required validation commands through make check and capture the first failing gate, if any
  • Produce the pre-PR validation report with findings and binding verdict
exec
/bin/zsh -lc "sed -n '1,220p' docs/plans/clo-273-test-runner.md" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
exec
/bin/zsh -lc "sed -n '1,220p' docs/designs/clo-273-test-runner.md" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
# Design: CLO-273 — Implement TestRunner verify hook parsing cargo + pytest JSON

| Field | Value |
|-------|-------|
| Task | CLO-273 |
| Date | 2026-05-01 |
| Phase | design |
| Discovery | docs/discovery/clo-273.md |
| PRD | docs/prds/clo-273-test-runner.md |

## 1. Problem

`VerifyHook` is now available (`CLO-270`) and `RunCommand` already exists (`CLO-271`) as a reusable process-based verifier, but there is still no dedicated test-hook implementation. Without a test runner verifier, strategy retries are gated only by LLM-style checks and cannot reliably depend on deterministic pass/fail counts from project tests. This leaves the phase runner path (`T-029`) without a full production-quality test verification primitive.

CLO-273 requires a hook that can execute project test commands and convert structured test output into a binary gate signal (`Pass`/`Fail`) with enough context for downstream feedback. The issue explicitly requires cargo and pytest support with fixture-based parser contracts, so the implementation should be mostly deterministic and parser-first.

## 2. Goals & Non-goals

### Goals

1. **Implement `TestRunner` verify hook** in `src/strategy/verify/test_runner.rs` with `TestRunnerKind` enum (`Cargo`, `Pytest`) and builder-style configuration (`runner`, `cwd`, `extra_args`, `sandbox`).
2. **Execute tests via `RunCommand` internals**, reusing existing `cwd`, env allowlist, output caps, timeouts, signal handling, and `FailureReason` mapping.
3. **Parse structured output** into `TestResult` for:
   - cargo JSON-lines (`type: "test"`, `event: "ok"/"failed"`, etc.),
   - pytest JSON-report summary (`summary.passed`, `summary.failed`).
4. **Map outcomes to verify results**:
   - `failed > 0` ⇒ `Fail` with first-failure name/excerpt,
   - `failed == 0 && passed > 0` ⇒ `Pass`,
   - `passed == 0 && failed == 0` ⇒ `Fail { summary: "no tests ran" }`.
5. **Keep parsing robust** by skipping malformed cargo lines and handling pytest missing/unparseable output without crashing the hook.
6. **Add/retain fixture-driven parser tests** in `tests/verify_test_runner.rs` for the contract in ACs.

### Non-goals

- No new backend/protocol support beyond `cargo` and `pytest` in v0.
- No historical trend tracking; gate only current run.
- No attempt to implement external language-specific reporters outside pytest JSON output.
- No phase-runner wiring changes in this task (that is handled in `T-029`).

## 3. Architecture

### 3.1 Module layout

```
src/strategy/verify/
  mod.rs            # new re-export: TestRunner, TestRunnerKind, SandboxOpts, TestResult
  verify.rs         # VerifyHook trait + FailureReason + shared hook types
  run_command.rs    # reusable process verifier, used by both RunCommand and TestRunner
  test_runner.rs    # NEW: cargo + pytest parser and TestRunner::verify impl

tests/
  verify_test_runner.rs # fixture-driven unit/integration parser and mapping tests
  fixtures/test_runner/* # deterministic parser fixtures
```

### 3.2 Data flow

```
TestRunner::verify(ctx) [ctx unused for now]
  ├─ build RunCommand based on TestRunnerKind
  │   ├─ cargo => cargo test --message-format=json --no-fail-fast
  │   └─ pytest => pytest --json-report --json-report-file=-
  ├─ apply sandbox opts (cwd, allowlist, timeouts, caps)
  ├─ run command (inherited from RunCommand)
  ├─ parse stdout into TestResult
  │   ├─ cargo parser: line-by-line JSON events
  │   └─ pytest parser: full-json parse, fallback to line scan
  ├─ translate CommandRun + TestResult into VerifyResult
  └─ return Pass/Fail with structured FailureReason
```

### 3.3 Types

- `TestRunnerKind`: `Cargo | Pytest`.
- `SandboxOpts`: execution policy fields currently needed by tests (`env_allowlist`, `wall_timeout`, `stdout_cap`, `stderr_cap`).
- `TestRunner`: `{ runner, cwd, extra_args, sandbox }`.
- `TestResult`: `{ passed, failed, first_failure_name, first_failure_excerpt }`.
- `CommandRun` and `VerifyResult` are reused from `run_command` / verify module.

## 4. Public API and behaviors

### 4.1 Core structs

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestRunnerKind {
    Cargo,
    Pytest,
}

#[derive(Debug, Clone)]
pub struct SandboxOpts {
    pub env_allowlist: Vec<String>,
    pub wall_timeout: Duration,
    pub stdout_cap: usize,
    pub stderr_cap: usize,
}

#[derive(Debug, Clone)]
pub struct TestRunner {
    pub runner: TestRunnerKind,
    pub cwd: PathBuf,
    pub extra_args: Vec<String>,
    pub sandbox: SandboxOpts,
}
```

### 4.2 Hook contract

```rust
#[async_trait]
impl VerifyHook for TestRunner {
    fn name(&self) -> &str;
    async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError>;
}
```

`verify()` builds a `RunCommand`, executes tests, parses output, and maps to:
- `VerifyResult::Pass` (no failures, at least one pass),
- `VerifyResult::Fail` otherwise.

### 4.3 Parsing rules

#### Cargo (`parse_cargo_output`)

- Consume JSON-lines.
- Count only objects where `type == "test"`.
- `event == "ok"` increments `passed`.
- `event == "failed"` increments `failed` and captures first failure name + excerpt.
- Ignore other events (`ignored`, benchmarks, etc.).
- Malformed lines are ignored (not fatal).
- Ignore lines where test event is `ok` and `ignored == true`.

#### Pytest (`parse_pytest_output`)

- Parse full stdout as JSON; fallback to line-by-line JSON candidate parsing.
- Read `summary.passed` and `summary.failed`.
- Locate first failed test in `tests[]` (`outcome == "failed"`) for name and `longrepr` excerpt.
- On parse or missing-summary failure, return zero counts with descriptive non-empty excerpt in result.

### 4.4 Result translation

`to_verify_result(run, parsed)`:
1. If timed out/signaled, produce fail with sandbox violation metadata.
2. If parsed yields zero/zero → fail "no tests ran" + captured output.
3. If failed > 0 → include first-failure details in summary and exit code.
4. Else pass.

## 5. Implementation plan (v0)

1. **Parser-only stability:** keep `parse_cargo_output` / `parse_pytest_output` deterministic and independent of actual command execution.
2. **Map runner assembly:** ensure `build_run_command()` maps both kinds and forwards `extra_args`.
3. **Failure mapping:** ensure `to_verify_result()` handles timeout/signal/non-zero/empty output consistently with existing `FailureReason` patterns.
4. **Fixtures and tests:** add fixtures for pass/fail/malformed/no-output fixtures in `tests/fixtures/test_runner` and assertions in `tests/verify_test_runner.rs`.
5. **Run full checks:** `cargo test --test verify_test_runner`, then at least `cargo test --test verify_run_command` and full `cargo test`.

## 6. Validation plan (from discovery/PRD)

- `cargo test --test verify_test_runner` must pass.
- Full test suite remains green.
- `cargo clippy --all-targets` must pass with no warnings.
- Result mapping should preserve deterministic, parseable outputs for retry gating.

## 7. Risks and open questions

### Risks

- Pytest output schema can vary by plugin/version; we intentionally scope to json-report schema in issue contract.
- Some command failures may produce noisy non-JSON output; parser should fail-soft with safe `no tests ran`/partial context rather than panic.

### Open questions

- Should we add a future fallback parser (`pytest -q` style output) under a feature flag in a follow-up task once schema mismatches are observed in real projects?

## 8. Security considerations

- Execution remains within `RunCommand` sandbox boundaries (env allowlist, wall timeout, byte caps, process cleanup).
- `VerifyResult::FailureReason` keeps raw stdout/stderr for downstream redaction to remain explicit about trust boundaries.
- No credentials are passed through fixtures or test harnesses.

 succeeded in 0ms:
# Plan: CLO-273 Implement TestRunner verify hook parsing cargo + pytest JSON

## Context
- Design: `docs/designs/clo-273-test-runner.md`
- Discovery: `docs/discovery/clo-273.md`
- PRD: `docs/prds/clo-273-test-runner.md`
- Linear: https://linear.app/cloud-ai/issue/CLO-273/implement-testrunner-verify-hook-parsing-cargo-pytest-json
- Dependency: CLO-271 (RunCommand verify hook), CLO-270 (VerifyHook/FailureReason)

## Sub-tasks

### ST1 Consolidate TestRunner construction and public API
**Files:** `src/strategy/verify/test_runner.rs`
**Acceptance:** `cargo test --test verify_test_runner cargo_3_pass_0_fail -- --exact`
**Estimate:** S

Make `TestRunner`, `TestRunnerKind`, `SandboxOpts`, and `TestResult` APIs stable and explicit for execution wiring:
- confirm default behavior for `cwd`, `extra_args`, and `SandboxOpts` values,
- ensure command assembly for cargo/pytest is deterministic and unit-testable,
- keep parser entry points (`parse_cargo_output`, `parse_pytest_output`) publicly testable and documented.

### ST2 Lock parser behavior to contract fixtures
**Files:** `src/strategy/verify/test_runner.rs`, `tests/fixtures/test_runner/*.json`, `tests/verify_test_runner.rs`
**Acceptance:** `cargo test --test verify_test_runner cargo_2_pass_1_fail cargo_empty_no_tests pytest_4_pass_2_fail -- --exact`
**Estimate:** M

Harden and finalize parsing semantics:
- cargo JSON-lines: count `type == "test"`, handle `ignored == true` as not passed, capture first failure name/excerpt, skip malformed lines,
- pytest JSON-report: extract `summary.passed/failed`, first failed `nodeid` + `longrepr`, parse/line-fallback on malformed stream,
- keep all parser failures soft (return `0/0` with diagnostics).

### ST3 Complete `to_verify_result()` mapping and sandbox violation paths
**Files:** `src/strategy/verify/test_runner.rs`
**Acceptance:** `cargo test --test verify_test_runner verify_result_no_tests_ran -- --exact`
**Estimate:** M

Map command execution outcomes + parsed counts into `VerifyResult`:
- timeout → `Fail` with sandbox violation timeout,
- signal/non-zero status → `Fail` with structured reason and signal/non-zero context,
- `failed > 0` → first-failure summary + stdout/stderr passthrough,
- `passed > 0 && failed == 0` → `Pass`,
- `passed == 0 && failed == 0` → `Fail { reason: "no tests ran" }`.

### ST4 Finish `VerifyHook` implementation and verify integration path
**Files:** `src/strategy/verify/test_runner.rs`, `src/strategy/verify/mod.rs`
**Acceptance:** `cargo test --test verify_test_runner pytest_non_json_exit -- --exact`
**Estimate:** M

Implement/retain `VerifyHook` execution path using `RunCommand::run()`:
- execute via resolved cargo/pytest command,
- pass parsed output through conversion path,
- verify `mod.rs` exports include `TestRunner`, `TestRunnerKind`, `SandboxOpts`, `TestResult`.

### ST5 Sanity gate for changed modules
**Files:** `src/strategy/verify/test_runner.rs`, `src/strategy/verify/mod.rs`, `tests/verify_test_runner.rs`
**Acceptance:** `cargo test --test verify_test_runner`
**Estimate:** S

Run the focused test module in CI-like mode, update any remaining fixture/parser gaps, and prepare for full-suite pre-merge.

### ST6 Pre-merge gate
**Files:** Entire workspace
**Acceptance:** `make check`
**Estimate:** S

Run project-wide pre-merge checks.

## Pre-merge gate
- `make check`

## Risks
- Pytest schema drift across plugin versions can change `summary` shape; parser is bounded to contract and guarded by parse-fail fallback.
- Non-zero command exits with malformed output require conservative failure semantics (`no tests ran`) to avoid false pass.
- Fixture-only tests can miss shell/runtime behavior; pre-merge full test gate remains required.

exec
/bin/zsh -lc "sed -n '1,260p' docs/status/clo-273-workflow.yaml" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
task_id: clo-273
task_type: development
classification_reason: Title starts with "Implement" with mostly mechanical ACs.
task_profile:
  has_backend: false
  has_frontend: false
  has_data_model: false
  has_external_deps: false
  skip_probe: false
pending_human_action: null
linear:
  team: Cloud-ai
  project: Loker
  status_at_start: Backlog
  blocks: []
  blocked_by:
    - CLO-271
    - CLO-270
  priority: Medium
  branch_suggested: kmamemo/clo-273-implement-testrunner-verify-hook-parsing-cargo-pytest-json
  branch_actual: feat/clo-273-test-runner
workflow:
  current_phase: implement
  status: in_progress
  created_at: '2026-05-01T14:40:59.518Z'
  updated_at: '2026-05-02T06:31:08.740Z'
phases:
  discovery:
    status: complete
    approved: true
    problem_framed: true
    prd_exists: true
    prd_file: docs/prds/clo-273-test-runner.md
    prd_created: true
    discovery_report: docs/discovery/clo-273.md
    discovery_debt: []
    baseline_score: 7
    approaches_identified: 3
    approach_chosen: Approach A
  design:
    status: complete
    design_doc: docs/designs/clo-273-test-runner.md
    discovery_context_used: true
    draft_ready: true
    review_completed: false
    review_gemini: null
    review_ollama: null
    review_synthesis: null
    review_verdict: null
    review_skip_reason: AI review tooling unavailable in environment; design manually approved by maintainer.
    review_applied: false
    applied_suggestions: []
    flagged_suggestions: []
    finalized: true
    human_review_completed: true
  plan:
    status: complete
    plan_file: docs/plans/clo-273-test-runner.md
    approved: true
  implement:
    status: validating
    commits:
      - d10d83b
      - 5c6651b
      - 72b5c11
  pr:
    status: pending
  complete:
    status: pending
history:
  - timestamp: '2026-05-01T14:40:59.518Z'
    action: workflow_started
    phase: init
    details: Workflow initialized for clo-273 as development
  - timestamp: '2026-05-01T15:04:21.915Z'
    action: init_classified
    phase: init
    details: 'Classified as development: title is an implementation task with mechanical acceptance-criteria scope.'
  - timestamp: '2026-05-01T15:04:37.188Z'
    action: project_sync_skipped
    phase: init
    details: No PROJECT.md/ROADMAP.md/DEPENDENCIES.md exist in this repo.
  - timestamp: '2026-05-01T15:04:38.653Z'
    action: phase_transition
    phase: init
    details: Transitioned from init to discovery
  - timestamp: '2026-05-01T15:05:31.008Z'
    action: branch_created
    phase: discovery
    details: Branch feat/clo-273-test-runner checked out (existing local branch pre-created)
  - timestamp: '2026-05-01T15:05:50.773Z'
    action: discovery_approved
    phase: discovery
    details: >-
      Approach: dedicated TestRunner hook module using RunCommand + structured parsers. Baseline 7/10. 3 approaches
      considered.
  - timestamp: '2026-05-01T15:05:52.123Z'
    action: phase_transition
    phase: discovery
    details: Transitioned from discovery to design
  - timestamp: '2026-05-01T15:06:45.335Z'
    action: design_draft_ready
    phase: design
    details: Draft created at docs/designs/clo-273-test-runner.md using discovery+PRD context; discovery_context_used=true.
  - timestamp: '2026-05-01T15:06:48.025Z'
    action: design_review_complete
    phase: design
    details: AI review unavailable in this environment. Proceeding with manual checkpoint; no auto-applicable suggestions.
  - timestamp: '2026-05-01T15:06:50.097Z'
    action: design_finalized
    phase: design
    details: Design approved for implementation. No conflicts detected. Proceeding to plan phase.
  - timestamp: '2026-05-01T15:07:03.345Z'
    action: design_human_review_complete
    phase: design
    details: >-
      Manual review path used; no conflicts. Review artifacts recorded as unavailable/skipped in workflow for
      environment constraints.
  - timestamp: '2026-05-01T15:09:12.620Z'
    action: plan_created
    phase: plan
    details: Plan with 6 sub-tasks at docs/plans/clo-273-test-runner.md
  - timestamp: '2026-05-01T15:09:17.126Z'
    action: plan_approved
    phase: plan
    details: Plan approved. 6 sub-tasks queued.
  - timestamp: '2026-05-01T15:09:18.460Z'
    action: phase_transition
    phase: plan
    details: Transitioned from plan to implement
  - timestamp: '2026-05-01T15:10:53.238Z'
    action: subtask_complete
    phase: implement
    details: >-
      ST1 landed: consolidated TestRunner construction/API and added core test_runner module + exports + parser tests.
      Commit d10d83b
  - timestamp: '2026-05-01T15:10:57.884Z'
    action: subtask_complete
    phase: implement
    details: 'ST2 landed: parser behavior locked to fixtures and soft-fail malformed/no-summary lines. Commit d10d83b.'
  - timestamp: '2026-05-01T15:11:00.532Z'
    action: subtask_complete
    phase: implement
    details: >-
      ST3 landed: `to_verify_result` maps timeout/signal/non-zero/no-tests/pass/fail semantics per contract. Commit
      d10d83b.
  - timestamp: '2026-05-01T15:11:01.926Z'
    action: subtask_complete
    phase: implement
    details: 'ST4 landed: VerifyHook integration via RunCommand and exports complete. Commit d10d83b.'
  - timestamp: '2026-05-01T15:11:03.344Z'
    action: subtask_complete
    phase: implement
    details: >-
      ST5 landed: full focused test suite (`cargo test --test verify_test_runner`) is green on current code. Commit
      d10d83b.
  - timestamp: '2026-05-01T15:11:04.899Z'
    action: subtask_complete
    phase: implement
    details: 'ST6 pre-merge sanity run complete: make check green. Commit d10d83b.'
  - timestamp: '2026-05-01T15:11:06.835Z'
    action: implementation_complete
    phase: implement
    details: All sub-tasks landed. make check green. 1 commit.
  - timestamp: '2026-05-02T06:31:07.577Z'
    action: subtask_complete
    phase: implement
    details: >-
      Minor follow-up: clippy-safe string assertions and explicit sandbox-violation assertion style in tests. Commit
      72b5c11.
  - timestamp: '2026-05-02T06:31:08.740Z'
    action: implementation_complete
    phase: implement
    details: All sub-tasks landed. make check green. 3 commits.
task_title: Implement TestRunner verify hook parsing cargo + pytest JSON
task_url: https://linear.app/cloud-ai/issue/CLO-273/implement-testrunner-verify-hook-parsing-cargo-pytest-json

exec
/bin/zsh -lc "sed -n '1,220p' docs/prds/clo-273-test-runner.md" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
# PRD: CLO-273 — Implement TestRunner verify hook parsing cargo + pytest JSON

| Field | Value |
|-------|-------|
| Source | Linear issue [CLO-273](https://linear.app/cloud-ai/issue/CLO-273/implement-testrunner-verify-hook-parsing-cargo-pytest-json) |
| PRD Reference | FR-16 — test-runner verify hook gates retries |
| Design doc | See `docs/plans/001-implementation-roadmap.md` and `docs/designs/clo-270-hook.md` for verify-hook architecture |
| Security | Follow existing FR-14 sandboxing expectations in `docs/prd/2026-04-25-loker.md` when invoking test commands (`cwd`, env allowlist, output caps, timeouts, process cleanup). |

## Scope

- Implement `TestRunner` verify hook under `src/strategy/verify/` with enum `TestRunnerKind` for supported runners.
- Support at least:
  - `Cargo` -> `cargo test --message-format=json --no-fail-fast`
  - `Pytest` -> `pytest --json-report --json-report-file=-`
- Reuse `RunCommand` internals for execution and sandboxing instead of inventing new process code.
- Parse structured output to produce `{failed, passed, first_failure_name, first_failure_excerpt}` metrics.
- Map verdicts:
  - `failed > 0` → `Fail` with structured `FailureReason`
  - `failed == 0 && passed > 0` → `Pass`
  - `passed == 0 && failed == 0` → `Fail` with reason `no tests ran`
- Tests-first implementation:
  - cargo: 3 pass / 0 fail → `Pass`
  - cargo: 2 pass / 1 fail → `Fail` with first failure details
  - cargo empty (0/0) → `Fail { "no tests ran" }`
  - cargo malformed JSON line in stream is skipped
  - pytest summary path with `summary.failed` / `summary.passed` -> expected pass/fail
  - pytest non-zero exit without JSON still fails with raw stderr summary.

## Acceptance criteria

- `cargo test --test verify_test_runner` is green.
- `cargo test` overall remains green after integration.
- `cargo clippy --all-targets` reports no warnings for the new module.
- PRD FR-16 requirement is satisfied: test-runner verify hook gates retries from pass/fail counts.

## Dependencies / blocks

- Blocked by: CLO-270 (VerifyHook trait), CLO-271 (RunCommand hook + sandboxing internals) — both required.
- Blocks: T-029 (phase runner)

codex
The design and plan are consistent with the workflow record, including the commit-level subtask mapping. I’m moving to the implementation now: diff review first, then the actual gate commands.
exec
/bin/zsh -lc "sed -n '1,260p' src/strategy/verify/test_runner.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
//! Test runner verify hook.
//!
//! Runs a project's test suite via [`RunCommand`] internals, parses
//! structured output for pass/fail counts, and returns `Pass` only when
//! zero tests failed and at least one ran.
//!
//! # Supported runners
//!
//! | Kind | Command | Parser |
//! |------|---------|--------|
//! | `Cargo` | `cargo test --message-format=json --no-fail-fast` | JSON‑lines per [`cargo::test` message format](https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages) |
//! | `Pytest` | `pytest --json-report --json-report-file=-` | [pytest-json-report](https://pypi.org/project/pytest-json-report/) summary output |

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde::de::Deserialize;

use super::run_command::RunCommand;
use super::{FailureReason, VerifyContext, VerifyError, VerifyHook, VerifyResult};

/// Supported test runner kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestRunnerKind {
    Cargo,
    Pytest,
}

/// Sandboxing configuration for test runner execution.
///
/// Maps to [`RunCommand`] fields but is kept as a separate struct so the
/// `TestRunner` API doesn't expose `RunCommand` internals directly.
#[derive(Debug, Clone)]
pub struct SandboxOpts {
    /// Environment variable names allowed in child process.
    /// Default: empty (default‑deny).
    pub env_allowlist: Vec<String>,
    /// Wall‑clock timeout before process‑group SIGKILL.
    pub wall_timeout: Duration,
    /// Max bytes captured from stdout.
    pub stdout_cap: usize,
    /// Max bytes captured from stderr.
    pub stderr_cap: usize,
}

impl Default for SandboxOpts {
    fn default() -> Self {
        Self {
            env_allowlist: Vec::new(),
            wall_timeout: Duration::from_secs(120),
            stdout_cap: 8192,
            stderr_cap: 8192,
        }
    }
}

/// Test runner verify hook.
///
/// Executes a test suite via the configured [`TestRunnerKind`], parses the
/// structured output, and returns:
///
/// - `Pass` when `failed == 0 && passed > 0`.
/// - `Fail` when `failed > 0`, carrying `{failed, passed, first_failure_name, first_failure_excerpt}`.
/// - `Fail { reason: "no tests ran" }` when `passed == 0 && failed == 0`.
#[derive(Debug, Clone)]
pub struct TestRunner {
    /// Which test runner to use.
    pub runner: TestRunnerKind,
    /// Working directory for the command.
    pub cwd: PathBuf,
    /// Extra arguments passed through to the test command (after the
    /// runner‑specific base args).
    pub extra_args: Vec<String>,
    /// Sandboxing options (timeouts, caps, env allowlist).
    pub sandbox: SandboxOpts,
}

impl TestRunner {
    /// Construct a new test runner.
    pub fn new(runner: TestRunnerKind, cwd: impl Into<PathBuf>) -> Self {
        Self {
            runner,
            cwd: cwd.into(),
            extra_args: Vec::new(),
            sandbox: SandboxOpts::default(),
        }
    }

    /// Append extra arguments passed to the test command.
    pub fn with_extra_args(mut self, args: impl IntoIterator<Item: AsRef<str>>) -> Self {
        self.extra_args = args.into_iter().map(|s| s.as_ref().to_string()).collect();
        self
    }

    /// Override sandbox options.
    pub fn with_sandbox(mut self, opts: SandboxOpts) -> Self {
        self.sandbox = opts;
        self
    }

    // ── internal helpers ────────────────────────────────────

    fn build_run_command(&self) -> RunCommand {
        let mut rc = match self.runner {
            TestRunnerKind::Cargo => {
                let mut args = vec![
                    "test".to_string(),
                    "--message-format=json".to_string(),
                    "--no-fail-fast".to_string(),
                ];
                args.extend(self.extra_args.clone());
                RunCommand::new("cargo").with_args(args)
            }
            TestRunnerKind::Pytest => {
                let mut args = vec![
                    "--json-report".to_string(),
                    "--json-report-file=-".to_string(),
                ];
                args.extend(self.extra_args.clone());
                RunCommand::new("pytest").with_args(args)
            }
        };

        rc = rc
            .with_cwd(self.cwd.clone())
            .with_env_allowlist(
                &self
                    .sandbox
                    .env_allowlist
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>(),
            )
            .with_wall_timeout(self.sandbox.wall_timeout)
            .with_stdout_cap(self.sandbox.stdout_cap)
            .with_stderr_cap(self.sandbox.stderr_cap);

        rc
    }

    /// Parse cargo test JSON‑lines output into pass/fail counts.
    ///
    /// Each line is a JSON object per cargo's `--message-format=json`
    /// spec. We look for messages with `"type":"test"` and examine
    /// the `event` field (`"ok"`, `"failed"`, `"ignored"`, etc.).
    /// Malformed JSON lines are silently skipped.
    pub fn parse_cargo_output(stdout: &str) -> TestResult {
        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut first_failure_name: Option<String> = None;
        let mut first_failure_excerpt: Option<String> = None;

        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let value: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue, // skip malformed lines
            };

            // Only process test events
            if value.get("type").and_then(|v| v.as_str()) != Some("test") {
                continue;
            }

            let event = value.get("event").and_then(|v| v.as_str()).unwrap_or("");

            // Ignored tests have `"ignored": true` alongside `"event": "ok"`.
            // Skip them — they didn't actually run.
            let ignored = value
                .get("ignored")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            match (event, ignored) {
                ("ok", false) => {
                    passed += 1;
                }
                ("ok", true) => {
                    // ignored test — not counted
                }
                ("failed", _) => {
                    failed += 1;
                    if first_failure_name.is_none() {
                        first_failure_name = value
                            .get("name")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());
                        // Cargo puts the failure stdout in the `stdout` field
                        first_failure_excerpt =
                            value.get("stdout").and_then(|v| v.as_str()).map(|s| {
                                let s = s.trim();
                                // Truncate to first 200 chars for a concise excerpt
                                truncate_excerpt(s, 200)
                            });
                    }
                }
                _ => {
                    // "ignored", "measured" (benchmarks) — not counted
                }
            }
        }

        TestResult {
            passed,
            failed,
            first_failure_name,
            first_failure_excerpt,
        }
    }

    /// Parse pytest JSON report output.
    ///
    /// Expects a single JSON object with a `summary` field containing
    /// `passed` and `failed` integers (from `pytest-json-report`).
    pub fn parse_pytest_output(stdout: &str) -> TestResult {
        // Try parsing the entire stdout as JSON first (handles multi-line reports).
        // Fall back to line-by-line search if that fails.
        let maybe_value: Option<serde_json::Value> =
            serde_json::from_str(stdout).ok().or_else(|| {
                stdout.find('{').and_then(|start| {
                    let mut de = serde_json::Deserializer::from_str(&stdout[start..]);
                    serde_json::Value::deserialize(&mut de).ok()
                })
            });

        let value = match maybe_value {
            Some(v) => v,
            None => {
                return TestResult {
                    passed: 0,
                    failed: 0,
                    first_failure_name: None,
                    first_failure_excerpt: Some(
                        "could not parse pytest JSON report from stdout".to_string(),
                    ),
                };
            }
        };

        let summary = match value.get("summary") {
            Some(s) => s,
            None => {
                return TestResult {
                    passed: 0,
                    failed: 0,
                    first_failure_name: None,
                    first_failure_excerpt: Some(
                        "pytest report missing `summary` field".to_string(),
                    ),
                };
            }
        };

        let passed = summary.get("passed").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let failed = summary.get("failed").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

exec
/bin/zsh -lc 'git diff -- src/strategy/verify/mod.rs src/strategy/verify/run_command.rs src/strategy/verify/test_runner.rs tests/verify_test_runner.rs' in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
exec
/bin/zsh -lc "sed -n '1,260p' tests/verify_test_runner.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
//! Integration tests for the `TestRunner` verify hook.
//!
//! Drives the JSON parsers from canned fixture files — no actual `cargo`
//! or `pytest` invocation in unit tests. Parser output is then fed
//! through `TestRunner::to_verify_result` to exercise the
//! `VerifyHook::verify` logic without subprocess overhead.

use std::path::Path;

use loker::strategy::verify::run_command::{CapturedOutput, CommandRun};
use loker::strategy::verify::{TestRunner, VerifyResult};

// ── helpers ──────────────────────────────────────────────────

/// Read a fixture file as a string.
fn read_fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("test_runner")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("failed to read fixture {name}: {e}");
    })
}

/// Convert fixture content to a JSON‑lines string that the cargo parser
/// would see on stdout. `.jsonl` files contain a JSON array — we flatten
/// each element to one JSON-per-line.
fn fixture_to_cargo_stdout(fixture: &str) -> String {
    // Cargo JSON-lines fixtures are raw JSON-per-line content.
    fixture.to_string()
}

/// Read a pytest fixture file as a string.
fn fake_captured_output(data: &str) -> CapturedOutput {
    CapturedOutput {
        data: data.as_bytes().to_vec(),
        truncated: false,
        elided_bytes: 0,
    }
}

fn exit_status(code: i32) -> std::process::ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code << 8)
    }

    #[cfg(not(unix))]
    {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code as u32)
    }
}

fn fake_command_run_with_code(stdout_data: &str, exit_code: i32) -> CommandRun {
    CommandRun {
        status: exit_status(exit_code),
        timed_out: false,
        stdout: fake_captured_output(stdout_data),
        stderr: fake_captured_output(""),
        secret_values: vec![],
        elapsed_ms: 10,
    }
}

fn fake_command_run(stdout_data: &str) -> CommandRun {
    fake_command_run_with_code(stdout_data, 0)
}

/// Parse cargo fixture, then convert to VerifyResult.
fn cargo_fixture_verify(name: &str) -> VerifyResult {
    let raw = read_fixture(name);
    let stdout = fixture_to_cargo_stdout(&raw);
    let result = TestRunner::parse_cargo_output(&stdout);
    let run = fake_command_run(&stdout);
    TestRunner::to_verify_result(run, result)
}

/// Parse pytest fixture, then convert to VerifyResult.
fn pytest_fixture_verify(name: &str) -> VerifyResult {
    let stdout = read_fixture(name);
    let result = TestRunner::parse_pytest_output(&stdout);
    let run = fake_command_run(&stdout);
    TestRunner::to_verify_result(run, result)
}

// ── Cargo tests ─────────────────────────────────────────────

#[test]
fn cargo_3_pass_0_fail() {
    let result = cargo_fixture_verify("cargo_3pass_0fail.jsonl");
    assert!(
        matches!(result, VerifyResult::Pass),
        "expected Pass, got {result:?}"
    );
}

#[test]
fn cargo_2_pass_1_fail() {
    let result = cargo_fixture_verify("cargo_2pass_1fail.jsonl");
    match result {
        VerifyResult::Fail { reason } => {
            assert!(
                reason.summary.contains("1 test(s) failed"),
                "summary should mention 1 failure: {}",
                reason.summary
            );
            assert!(
                reason.summary.contains("test_bad_divide"),
                "summary should contain failure name: {}",
                reason.summary
            );
            assert!(
                reason.summary.contains("assertion"),
                "summary should contain failure excerpt: {}",
                reason.summary
            );
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[test]
fn cargo_empty_no_tests() {
    let result = cargo_fixture_verify("cargo_empty.jsonl");
    match result {
        VerifyResult::Fail { reason } => {
            assert!(
                reason.summary.contains("no tests ran"),
                "expected 'no tests ran', got: {}",
                reason.summary
            );
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[test]
fn cargo_malformed_json_lines() {
    // The malformed fixture has a non-JSON line mixed with valid JSON lines.
    // The parser should skip the bad line and still count the valid ones.
    let raw = read_fixture("cargo_malformed.jsonl");
    let stdout = fixture_to_cargo_stdout(&raw);
    let result = TestRunner::parse_cargo_output(&stdout);
    assert_eq!(result.passed, 2, "should count 2 passing tests");
    assert_eq!(result.failed, 1, "should count 1 failing test");
}

// ── Pytest tests ────────────────────────────────────────────

#[test]
fn pytest_5_pass_0_fail() {
    let result = pytest_fixture_verify("pytest_5pass_0fail.json");
    assert!(
        matches!(result, VerifyResult::Pass),
        "expected Pass, got {result:?}"
    );
}

#[test]
fn pytest_4_pass_2_fail() {
    let result = pytest_fixture_verify("pytest_4pass_2fail.json");
    match result {
        VerifyResult::Fail { reason } => {
            assert!(
                reason.summary.contains("2 test(s) failed"),
                "summary should mention 2 failures: {}",
                reason.summary
            );
            assert!(
                reason.summary.contains("test_bar.py::test_bad"),
                "summary should contain first failure name: {}",
                reason.summary
            );
            assert!(
                reason.summary.contains("AssertionError"),
                "summary should contain failure excerpt: {}",
                reason.summary
            );
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[test]
fn pytest_noisy_multiline_output_is_parsed() {
    let stdout = r#"INFO: running tests
{
  "created": 1234567890,
  "duration": 0.1,
  "exitcode": 0,
  "root": "/tmp",
  "summary": {"passed": 1, "failed": 0, "total": 1, "collected": 1},
  "tests": [{"nodeid": "test.py::test_ok", "outcome": "passed"}]
}
INFO: done
"#;
    let result = TestRunner::parse_pytest_output(stdout);
    assert_eq!(result.passed, 1);
    assert_eq!(result.failed, 0);
}

#[test]
fn pytest_non_json_exit() {
    // Simulate process exits non-zero with no JSON written
    let stdout = "pytest: error: no tests found in test_runner/\n";
    let result = TestRunner::parse_pytest_output(stdout);
    assert_eq!(result.passed, 0);
    assert_eq!(result.failed, 0);

    // Build a command run with non-zero exit
    let run = CommandRun {
        status: exit_status(1),
        timed_out: false,
        stdout: fake_captured_output(stdout),
        stderr: fake_captured_output("ERROR: no tests collected\n"),
        secret_values: vec![],
        elapsed_ms: 15,
    };
    let vr = TestRunner::to_verify_result(run, result);
    match vr {
        VerifyResult::Fail { reason } => {
            assert!(
                reason.summary.contains("no tests ran"),
                "expected 'no tests ran', got: {}",
                reason.summary
            );
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

#[test]
fn verify_result_non_zero_exit_with_passing_tests_fails() {
    let stdout = r#"{"type":"test","event":"ok","name":"test_a","test_type":"unit"}
{"type":"test","event":"ok","name":"test_b","test_type":"unit"}
"#;
    let result = TestRunner::parse_cargo_output(stdout);
    let run = fake_command_run_with_code(stdout, 1);
    let vr = TestRunner::to_verify_result(run, result);
    match vr {
        VerifyResult::Fail { reason } => {
            assert!(
                reason.summary.contains("test runner exited with status 1"),
                "expected non-zero status summary, got: {}",
                reason.summary
            );
            assert!(
                reason.sandbox_violation.is_some(),
                "expected sandbox violation for non-zero exit"
            );
        }
        other => panic!("expected Fail, got {other:?}"),
    }
}

exec
/bin/zsh -lc "rg -n \"pub |unwrap\\(|expect\\(|TODO|FIXME|json-report|message-format=json|no-fail-fast|no tests ran|SandboxOpts|TestRunnerKind\" src/strategy/verify/test_runner.rs tests/verify_test_runner.rs src/strategy/verify/mod.rs src/strategy/verify/run_command.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
src/strategy/verify/mod.rs:12:pub mod llm_verifier;
src/strategy/verify/mod.rs:13:pub mod run_command;
src/strategy/verify/mod.rs:14:pub mod test_runner;
src/strategy/verify/mod.rs:15:pub mod verify;
src/strategy/verify/mod.rs:18:pub use verify::{
src/strategy/verify/mod.rs:23:pub use llm_verifier::LLMVerifier;
src/strategy/verify/mod.rs:24:pub use run_command::RunCommand;
src/strategy/verify/mod.rs:25:pub use test_runner::{SandboxOpts, TestResult, TestRunner, TestRunnerKind};
src/strategy/verify/test_runner.rs:11://! | `Cargo` | `cargo test --message-format=json --no-fail-fast` | JSON‑lines per [`cargo::test` message format](https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages) |
src/strategy/verify/test_runner.rs:12://! | `Pytest` | `pytest --json-report --json-report-file=-` | [pytest-json-report](https://pypi.org/project/pytest-json-report/) summary output |
src/strategy/verify/test_runner.rs:25:pub enum TestRunnerKind {
src/strategy/verify/test_runner.rs:35:pub struct SandboxOpts {
src/strategy/verify/test_runner.rs:38:    pub env_allowlist: Vec<String>,
src/strategy/verify/test_runner.rs:40:    pub wall_timeout: Duration,
src/strategy/verify/test_runner.rs:42:    pub stdout_cap: usize,
src/strategy/verify/test_runner.rs:44:    pub stderr_cap: usize,
src/strategy/verify/test_runner.rs:47:impl Default for SandboxOpts {
src/strategy/verify/test_runner.rs:60:/// Executes a test suite via the configured [`TestRunnerKind`], parses the
src/strategy/verify/test_runner.rs:65:/// - `Fail { reason: "no tests ran" }` when `passed == 0 && failed == 0`.
src/strategy/verify/test_runner.rs:67:pub struct TestRunner {
src/strategy/verify/test_runner.rs:69:    pub runner: TestRunnerKind,
src/strategy/verify/test_runner.rs:71:    pub cwd: PathBuf,
src/strategy/verify/test_runner.rs:74:    pub extra_args: Vec<String>,
src/strategy/verify/test_runner.rs:76:    pub sandbox: SandboxOpts,
src/strategy/verify/test_runner.rs:81:    pub fn new(runner: TestRunnerKind, cwd: impl Into<PathBuf>) -> Self {
src/strategy/verify/test_runner.rs:86:            sandbox: SandboxOpts::default(),
src/strategy/verify/test_runner.rs:91:    pub fn with_extra_args(mut self, args: impl IntoIterator<Item: AsRef<str>>) -> Self {
src/strategy/verify/test_runner.rs:97:    pub fn with_sandbox(mut self, opts: SandboxOpts) -> Self {
src/strategy/verify/test_runner.rs:106:            TestRunnerKind::Cargo => {
src/strategy/verify/test_runner.rs:109:                    "--message-format=json".to_string(),
src/strategy/verify/test_runner.rs:110:                    "--no-fail-fast".to_string(),
src/strategy/verify/test_runner.rs:115:            TestRunnerKind::Pytest => {
src/strategy/verify/test_runner.rs:117:                    "--json-report".to_string(),
src/strategy/verify/test_runner.rs:118:                    "--json-report-file=-".to_string(),
src/strategy/verify/test_runner.rs:144:    /// Each line is a JSON object per cargo's `--message-format=json`
src/strategy/verify/test_runner.rs:148:    pub fn parse_cargo_output(stdout: &str) -> TestResult {
src/strategy/verify/test_runner.rs:219:    /// `passed` and `failed` integers (from `pytest-json-report`).
src/strategy/verify/test_runner.rs:220:    pub fn parse_pytest_output(stdout: &str) -> TestResult {
src/strategy/verify/test_runner.rs:299:            TestRunnerKind::Cargo => Self::parse_cargo_output(stdout),
src/strategy/verify/test_runner.rs:300:            TestRunnerKind::Pytest => Self::parse_pytest_output(stdout),
src/strategy/verify/test_runner.rs:304:    pub fn to_verify_result(
src/strategy/verify/test_runner.rs:356:                        reason: FailureReason::new("no tests ran")
src/strategy/verify/test_runner.rs:382:                reason: FailureReason::new("no tests ran")
src/strategy/verify/test_runner.rs:424:            TestRunnerKind::Cargo => "TestRunner(cargo)",
src/strategy/verify/test_runner.rs:425:            TestRunnerKind::Pytest => "TestRunner(pytest)",
src/strategy/verify/test_runner.rs:439:pub struct TestResult {
src/strategy/verify/test_runner.rs:440:    pub passed: u32,
src/strategy/verify/test_runner.rs:441:    pub failed: u32,
src/strategy/verify/test_runner.rs:442:    pub first_failure_name: Option<String>,
src/strategy/verify/test_runner.rs:443:    pub first_failure_excerpt: Option<String>,
src/strategy/verify/test_runner.rs:569:        let excerpt = result.first_failure_excerpt.unwrap();
src/strategy/verify/test_runner.rs:584:            .expect("expected excerpt to be captured");
src/strategy/verify/test_runner.rs:713:                assert!(reason.summary.contains("no tests ran"));
tests/verify_test_runner.rs:132:                reason.summary.contains("no tests ran"),
tests/verify_test_runner.rs:133:                "expected 'no tests ran', got: {}",
tests/verify_test_runner.rs:227:                reason.summary.contains("no tests ran"),
tests/verify_test_runner.rs:228:                "expected 'no tests ran', got: {}",
src/strategy/verify/run_command.rs:31:pub struct RunCommand {
src/strategy/verify/run_command.rs:33:    pub cmd: String,
src/strategy/verify/run_command.rs:35:    pub args: Vec<String>,
src/strategy/verify/run_command.rs:38:    pub env_allowlist: Vec<String>,
src/strategy/verify/run_command.rs:40:    pub cwd: Option<PathBuf>,
src/strategy/verify/run_command.rs:42:    pub wall_timeout: Duration,
src/strategy/verify/run_command.rs:44:    pub cpu_timeout: Option<Duration>,
src/strategy/verify/run_command.rs:46:    pub stdout_cap: usize,
src/strategy/verify/run_command.rs:48:    pub stderr_cap: usize,
src/strategy/verify/run_command.rs:68:    pub fn new(cmd: impl Into<String>) -> Self {
src/strategy/verify/run_command.rs:76:    pub fn with_args(mut self, args: impl Into<Vec<String>>) -> Self {
src/strategy/verify/run_command.rs:82:    pub fn with_env_allowlist(mut self, vars: &[&str]) -> Self {
src/strategy/verify/run_command.rs:88:    pub fn with_cwd(mut self, path: impl Into<PathBuf>) -> Self {
src/strategy/verify/run_command.rs:94:    pub fn with_wall_timeout(mut self, timeout: Duration) -> Self {
src/strategy/verify/run_command.rs:100:    pub fn with_cpu_timeout(mut self, timeout: Option<Duration>) -> Self {
src/strategy/verify/run_command.rs:106:    pub fn with_stdout_cap(mut self, cap: usize) -> Self {
src/strategy/verify/run_command.rs:112:    pub fn with_stderr_cap(mut self, cap: usize) -> Self {
src/strategy/verify/run_command.rs:144:    pub async fn run(&self) -> Result<CommandRun, VerifyError> {
src/strategy/verify/run_command.rs:251:pub struct CommandRun {
src/strategy/verify/run_command.rs:252:    pub status: std::process::ExitStatus,
src/strategy/verify/run_command.rs:253:    pub timed_out: bool,
src/strategy/verify/run_command.rs:254:    pub stdout: CapturedOutput,
src/strategy/verify/run_command.rs:255:    pub stderr: CapturedOutput,
src/strategy/verify/run_command.rs:256:    pub secret_values: Vec<String>,
src/strategy/verify/run_command.rs:257:    pub elapsed_ms: u64,
src/strategy/verify/run_command.rs:261:pub struct CapturedOutput {
src/strategy/verify/run_command.rs:262:    pub data: Vec<u8>,
src/strategy/verify/run_command.rs:263:    pub truncated: bool,
src/strategy/verify/run_command.rs:264:    pub elided_bytes: usize,
src/strategy/verify/run_command.rs:268:    pub fn to_reason_text(&self) -> String {
src/strategy/verify/run_command.rs:498:        let result = rc.verify(&context()).await.unwrap();
src/strategy/verify/run_command.rs:507:        let result = rc.verify(&context()).await.unwrap();
src/strategy/verify/run_command.rs:537:        let result = rc.verify(&context()).await.unwrap();

exec
/bin/zsh -lc "sed -n '521,760p' src/strategy/verify/test_runner.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
exec
/bin/zsh -lc "sed -n '261,520p' src/strategy/verify/test_runner.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:

        let first_failure_name = value
            .get("tests")
            .and_then(|v| v.as_array())
            .and_then(|tests| {
                tests
                    .iter()
                    .find(|t| t.get("outcome").and_then(|o| o.as_str()) == Some("failed"))
            })
            .and_then(|t| t.get("nodeid").and_then(|n| n.as_str()))
            .map(|s| s.to_string());

        let first_failure_excerpt = value
            .get("tests")
            .and_then(|v| v.as_array())
            .and_then(|tests| {
                tests
                    .iter()
                    .find(|t| t.get("outcome").and_then(|o| o.as_str()) == Some("failed"))
            })
            .and_then(|t| {
                t.get("call")
                    .and_then(|c| c.get("longrepr"))
                    .or_else(|| t.get("longrepr"))
                    .and_then(|l| l.as_str())
            })
            .map(|s| truncate_excerpt(s, 200));

        TestResult {
            passed,
            failed,
            first_failure_name,
            first_failure_excerpt,
        }
    }

    fn parse_output(&self, stdout: &str) -> TestResult {
        match self.runner {
            TestRunnerKind::Cargo => Self::parse_cargo_output(stdout),
            TestRunnerKind::Pytest => Self::parse_pytest_output(stdout),
        }
    }

    pub fn to_verify_result(
        run_command_run: super::run_command::CommandRun,
        result: TestResult,
    ) -> VerifyResult {
        let runner_stdout = run_command_run.stdout.to_reason_text();
        let runner_stderr = run_command_run.stderr.to_reason_text();
        let truncated = run_command_run.stdout.truncated || run_command_run.stderr.truncated;
        let exit_code = run_command_run.status.code();
        let signal = run_command_run
            .status
            .code()
            .is_none()
            .then(|| {
                #[cfg(unix)]
                {
                    std::os::unix::process::ExitStatusExt::signal(&run_command_run.status)
                }
                #[cfg(not(unix))]
                {
                    None::<i32>
                }
            })
            .flatten();

        // Check for sandbox violations first
        if run_command_run.timed_out {
            return VerifyResult::Fail {
                reason: FailureReason::new("test runner timed out")
                    .with_stdout(runner_stdout)
                    .with_stderr(runner_stderr)
                    .with_truncated(truncated)
                    .with_sandbox_violation(crate::strategy::verify::SandboxViolation::Timeout),
            };
        }

        if let Some(sig) = signal {
            return VerifyResult::Fail {
                reason: FailureReason::new(format!("test runner killed by signal {sig}"))
                    .with_stdout(runner_stdout)
                    .with_stderr(runner_stderr)
                    .with_truncated(truncated)
                    .with_sandbox_violation(crate::strategy::verify::SandboxViolation::Signal {
                        signal: sig,
                    }),
            };
        }

        // Parse test output
        if let Some(code) = exit_code {
            if code != 0 && result.failed == 0 {
                if result.passed == 0 {
                    return VerifyResult::Fail {
                        reason: FailureReason::new("no tests ran")
                            .with_stdout(runner_stdout)
                            .with_stderr(runner_stderr)
                            .with_truncated(truncated)
                            .with_exit_code(code)
                            .with_sandbox_violation(
                                crate::strategy::verify::SandboxViolation::NonZeroExit { code },
                            ),
                    };
                }

                return VerifyResult::Fail {
                    reason: FailureReason::new(format!("test runner exited with status {code}"))
                        .with_stdout(runner_stdout)
                        .with_stderr(runner_stderr)
                        .with_truncated(truncated)
                        .with_exit_code(code)
                        .with_sandbox_violation(
                            crate::strategy::verify::SandboxViolation::NonZeroExit { code },
                        ),
                };
            }
        }

        if result.passed == 0 && result.failed == 0 {
            return VerifyResult::Fail {
                reason: FailureReason::new("no tests ran")
                    .with_stdout(runner_stdout)
                    .with_stderr(runner_stderr)
                    .with_truncated(truncated)
                    .with_exit_code(exit_code.unwrap_or(1)),
            };
        }

        if result.failed > 0 {
            let summary = match (&result.first_failure_name, &result.first_failure_excerpt) {
                (Some(name), Some(excerpt)) => {
                    format!(
                        "{} test(s) failed (first: {name}: {excerpt})",
                        result.failed
                    )
                }
                (Some(name), None) => {
                    format!("{} test(s) failed (first: {name})", result.failed)
                }
                (None, _) => {
                    format!("{} test(s) failed", result.failed)
                }
            };

            return VerifyResult::Fail {
                reason: FailureReason::new(summary)
                    .with_stdout(runner_stdout)
                    .with_stderr(runner_stderr)
                    .with_truncated(truncated)
                    .with_exit_code(exit_code.unwrap_or(1)),
            };
        }

        // All passed
        VerifyResult::Pass
    }
}

#[async_trait]
impl VerifyHook for TestRunner {
    fn name(&self) -> &str {
        match self.runner {
            TestRunnerKind::Cargo => "TestRunner(cargo)",
            TestRunnerKind::Pytest => "TestRunner(pytest)",
        }
    }

    async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
        let rc = self.build_run_command();
        let run = rc.run().await?;
        let parsed = self.parse_output(&run.stdout.to_reason_text());
        Ok(Self::to_verify_result(run, parsed))
    }
}

/// Structured result from parsing test runner output.
#[derive(Debug, Clone)]
pub struct TestResult {
    pub passed: u32,
    pub failed: u32,
    pub first_failure_name: Option<String>,
    pub first_failure_excerpt: Option<String>,
}

fn truncate_excerpt(text: &str, max_chars: usize) -> String {
    let normalized = text.trim();

    let char_count = normalized.chars().count();
    if char_count <= max_chars {
        return normalized.to_string();
    }

    normalized
        .chars()
        .take(max_chars)
        .collect::<String>()
        .chars()
        .chain(Some('…'))
        .collect()
}

#[cfg(test)]
fn exit_status_from_code(code: i32) -> std::process::ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code << 8)
    }

    #[cfg(not(unix))]
    {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code as u32)
    }
}

// ── unit tests (parser logic only, no subprocess) ───────────

#[cfg(test)]
mod tests {
    use super::*;

    fn cargo_test_result(fixture: &str) -> TestResult {
        TestRunner::parse_cargo_output(fixture)
    }

    fn pytest_test_result(fixture: &str) -> TestResult {
        TestRunner::parse_pytest_output(fixture)
    }

    // ── Cargo parser ────────────────────────────────────────

    #[test]
    fn cargo_3_pass_0_fail() {
        let output = r#"{"type":"test","event":"ok","name":"it_works","test_type":"unit"}
{"type":"test","event":"ok","name":"test_add","test_type":"unit"}
{"type":"test","event":"ok","name":"test_subtract","test_type":"unit"}
"#;
        let result = cargo_test_result(output);
        assert_eq!(result.passed, 3);
        assert_eq!(result.failed, 0);
        assert!(result.first_failure_name.is_none());
        assert!(result.first_failure_excerpt.is_none());
    }

    #[test]
    fn cargo_2_pass_1_fail() {
        let output = r#"{"type":"test","event":"ok","name":"it_works","test_type":"unit"}
{"type":"test","event":"failed","name":"test_bad_add","test_type":"unit","stdout":"assertion `left == right` failed\n  left: 3\n right: 5\n"}
{"type":"test","event":"ok","name":"test_good_add","test_type":"unit"}
"#;
        let result = cargo_test_result(output);
        assert_eq!(result.passed, 2);
        assert_eq!(result.failed, 1);
        assert_eq!(result.first_failure_name.as_deref(), Some("test_bad_add"));
        assert!(result
            .first_failure_excerpt
            .as_deref()
            .unwrap_or("")

 succeeded in 0ms:
            .contains("left == right"));
    }

    #[test]
    fn cargo_empty_no_tests() {
        let output = r#"{"type":"test","event":"ok","name":"test_dummy","test_type":"unit","ignored":true}
"#;
        // Only ignored tests — no actual tests ran
        let result = cargo_test_result(output);
        assert_eq!(result.passed, 0);
        assert_eq!(result.failed, 0);
    }

    #[test]
    fn cargo_malformed_json_line_skipped() {
        let output = r#"{"type":"test","event":"ok","name":"good_test","test_type":"unit"}
this is not json at all
{"type":"test","event":"ok","name":"another_test","test_type":"unit"}
{"type":"test","event":"failed","name":"bad_test","test_type":"unit","stdout":"failure!"}
"#;
        let result = cargo_test_result(output);
        assert_eq!(result.passed, 2);
        assert_eq!(result.failed, 1);
        assert_eq!(result.first_failure_name.as_deref(), Some("bad_test"));
    }

    #[test]
    fn cargo_skips_compiler_messages() {
        let output = r#"{"type":"compiler","message":"compiling foo"}
{"type":"test","event":"ok","name":"test_foo","test_type":"unit"}
{"type":"artifact","profile":"test"}
{"type":"test","event":"ok","name":"test_bar","test_type":"unit"}
"#;
        let result = cargo_test_result(output);
        assert_eq!(result.passed, 2);
        assert_eq!(result.failed, 0);
    }

    #[test]
    fn cargo_first_failure_preserves_stdout_excerpt() {
        let output = r#"{"type":"test","event":"ok","name":"passing","test_type":"unit"}
{"type":"test","event":"failed","name":"failing_test","test_type":"unit","stdout":"thread 'failing_test' panicked at src/main.rs:42:\nassertion `left == right` failed\n  left: 1\n right: 2\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n"}
{"type":"test","event":"failed","name":"also_failing","test_type":"unit","stdout":"another failure"}
"#;
        let result = cargo_test_result(output);
        assert_eq!(result.passed, 1);
        assert_eq!(result.failed, 2);
        assert_eq!(result.first_failure_name.as_deref(), Some("failing_test"));
        let excerpt = result.first_failure_excerpt.unwrap();
        assert!(excerpt.contains("panicked"));
        assert!(excerpt.contains("left == right"));
    }

    #[test]
    fn cargo_first_failure_truncates_utf8_excerpt_safely() {
        let unicode_excerpt = "😀".repeat(260);
        let output = format!(
            "{{\"type\":\"test\",\"event\":\"ok\",\"name\":\"passing\",\"test_type\":\"unit\"}}\n{{\"type\":\"test\",\"event\":\"failed\",\"name\":\"failing_test\",\"test_type\":\"unit\",\"stdout\":\"{unicode_excerpt}\"}}",
        );
        let result = cargo_test_result(&output);

        let excerpt = result
            .first_failure_excerpt
            .expect("expected excerpt to be captured");

        assert_eq!(excerpt.chars().count(), 201);
        assert!(excerpt.ends_with('…'));
        assert!(excerpt.len() > 200);
        assert!(excerpt.len() < 810);
    }

    // ── Pytest parser ───────────────────────────────────────

    #[test]
    fn pytest_5_pass_0_fail() {
        let output = r#"{"created": 1234567890, "duration": 0.15, "exitcode": 0, "root": "/tmp", "summary": {"passed": 5, "failed": 0, "total": 5, "collected": 5}, "tests": [{"nodeid": "test_foo.py::test_a", "outcome": "passed"}, {"nodeid": "test_foo.py::test_b", "outcome": "passed"}]}"#;
        let result = pytest_test_result(output);
        assert_eq!(result.passed, 5);
        assert_eq!(result.failed, 0);
        assert!(result.first_failure_name.is_none());
    }

    #[test]
    fn pytest_4_pass_2_fail() {
        let output = r#"{"created": 1234567890, "duration": 0.3, "exitcode": 1, "root": "/tmp", "summary": {"passed": 4, "failed": 2, "total": 6, "collected": 6}, "tests": [{"nodeid": "test_foo.py::test_a", "outcome": "passed"}, {"nodeid": "test_bar.py::test_bad", "outcome": "failed", "call": {"longrepr": "AssertionError: expected 5 got 3"}}, {"nodeid": "test_baz.py::test_bad2", "outcome": "failed", "call": {"longrepr": "TypeError: unsupported operand"}}]}"#;
        let result = pytest_test_result(output);
        assert_eq!(result.passed, 4);
        assert_eq!(result.failed, 2);
        assert_eq!(
            result.first_failure_name.as_deref(),
            Some("test_bar.py::test_bad")
        );
        assert!(result
            .first_failure_excerpt
            .as_deref()
            .unwrap_or("")
            .contains("AssertionError"));
    }

    #[test]
    fn pytest_empty_no_tests() {
        let output = r#"{"created": 1234567890, "duration": 0.01, "exitcode": 0, "root": "/tmp", "summary": {"passed": 0, "failed": 0, "total": 0, "collected": 0}, "tests": []}"#;
        let result = pytest_test_result(output);
        assert_eq!(result.passed, 0);
        assert_eq!(result.failed, 0);
    }

    #[test]
    fn pytest_missing_summary_field() {
        let output = r#"{"created": 1234567890, "duration": 0.01, "exitcode": 0}"#;
        let result = pytest_test_result(output);
        assert_eq!(result.passed, 0);
        assert_eq!(result.failed, 0);
        assert!(result
            .first_failure_excerpt
            .as_deref()
            .unwrap_or("")
            .contains("missing `summary` field"));
    }

    #[test]
    fn pytest_non_json_output() {
        let output = "pytest: error: no tests found";
        let result = pytest_test_result(output);
        assert_eq!(result.passed, 0);
        assert_eq!(result.failed, 0);
        assert!(result
            .first_failure_excerpt
            .as_deref()
            .unwrap_or("")
            .contains("could not parse"));
    }

    // ── VerifyResult conversion ─────────────────────────────

    fn fake_captured_output(data: &str) -> super::super::run_command::CapturedOutput {
        super::super::run_command::CapturedOutput {
            data: data.as_bytes().to_vec(),
            truncated: false,
            elided_bytes: 0,
        }
    }

    fn fake_command_run(stdout_data: &str) -> super::super::run_command::CommandRun {
        super::super::run_command::CommandRun {
            status: exit_status_from_code(0),
            timed_out: false,
            stdout: fake_captured_output(stdout_data),
            stderr: fake_captured_output(""),
            secret_values: vec![],
            elapsed_ms: 10,
        }
    }

    #[test]
    fn verify_result_from_passing_tests() {
        let stdout = r#"{"type":"test","event":"ok","name":"test_a","test_type":"unit"}
{"type":"test","event":"ok","name":"test_b","test_type":"unit"}
"#;
        let result = TestRunner::parse_cargo_output(stdout);
        let run = fake_command_run(stdout);
        let vr = TestRunner::to_verify_result(run, result);
        assert!(matches!(vr, VerifyResult::Pass));
    }

    #[test]
    fn verify_result_from_failing_tests() {
        let stdout = r#"{"type":"test","event":"ok","name":"test_a","test_type":"unit"}
{"type":"test","event":"failed","name":"test_bad","test_type":"unit","stdout":"assertion failed!"}
"#;
        let result = TestRunner::parse_cargo_output(stdout);
        let run = fake_command_run(stdout);
        let vr = TestRunner::to_verify_result(run, result);
        match vr {
            VerifyResult::Fail { reason } => {
                assert!(reason.summary.contains("1 test(s) failed"));
                assert!(reason.summary.contains("test_bad"));
                assert!(reason.summary.contains("assertion failed!"));
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn verify_result_no_tests_ran() {
        let stdout = r#"{"type":"test","event":"ignored","name":"ignored_test","test_type":"unit"}
"#;
        let result = TestRunner::parse_cargo_output(stdout);
        let run = fake_command_run(stdout);
        let vr = TestRunner::to_verify_result(run, result);
        match vr {
            VerifyResult::Fail { reason } => {
                assert!(reason.summary.contains("no tests ran"));
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }
}

exec
/bin/zsh -lc 'git diff main...HEAD -- src/strategy/verify/test_runner.rs src/strategy/verify/mod.rs src/strategy/verify/run_command.rs tests/verify_test_runner.rs' in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
diff --git a/src/strategy/verify/mod.rs b/src/strategy/verify/mod.rs
index 5ba44a5..7dfca55 100644
--- a/src/strategy/verify/mod.rs
+++ b/src/strategy/verify/mod.rs
@@ -7,9 +7,11 @@
 //!
 //! - [`LLMVerifier`] — delegates to an LLM backend and parses yes/no.
 //! - [`RunCommand`] — shells out and maps exit status (CLO-271).
+//! - [`TestRunner`] — parses structured test output (CLO-273).
 
 pub mod llm_verifier;
 pub mod run_command;
+pub mod test_runner;
 pub mod verify;
 
 // Re-export the core types so `strategy::verify::FailureReason` etc. work.
@@ -20,3 +22,4 @@ pub use verify::{
 // Re-export concrete implementations.
 pub use llm_verifier::LLMVerifier;
 pub use run_command::RunCommand;
+pub use test_runner::{SandboxOpts, TestResult, TestRunner, TestRunnerKind};
diff --git a/src/strategy/verify/run_command.rs b/src/strategy/verify/run_command.rs
index 0a55aaf..d63aaab 100644
--- a/src/strategy/verify/run_command.rs
+++ b/src/strategy/verify/run_command.rs
@@ -141,7 +141,7 @@ impl RunCommand {
             .collect()
     }
 
-    async fn run(&self) -> Result<CommandRun, VerifyError> {
+    pub async fn run(&self) -> Result<CommandRun, VerifyError> {
         let command_path = self.resolve_command()?;
 
         let mut command = Command::new(&command_path);
@@ -248,24 +248,24 @@ impl RunCommand {
 }
 
 #[derive(Debug)]
-struct CommandRun {
-    status: std::process::ExitStatus,
-    timed_out: bool,
-    stdout: CapturedOutput,
-    stderr: CapturedOutput,
-    secret_values: Vec<String>,
-    elapsed_ms: u64,
+pub struct CommandRun {
+    pub status: std::process::ExitStatus,
+    pub timed_out: bool,
+    pub stdout: CapturedOutput,
+    pub stderr: CapturedOutput,
+    pub secret_values: Vec<String>,
+    pub elapsed_ms: u64,
 }
 
 #[derive(Debug)]
-struct CapturedOutput {
-    data: Vec<u8>,
-    truncated: bool,
-    elided_bytes: usize,
+pub struct CapturedOutput {
+    pub data: Vec<u8>,
+    pub truncated: bool,
+    pub elided_bytes: usize,
 }
 
 impl CapturedOutput {
-    fn to_reason_text(&self) -> String {
+    pub fn to_reason_text(&self) -> String {
         let mut text = String::from_utf8_lossy(&self.data).into_owned();
         if self.truncated {
             text.push_str(&format!(
diff --git a/src/strategy/verify/test_runner.rs b/src/strategy/verify/test_runner.rs
new file mode 100644
index 0000000..e7c0314
--- /dev/null
+++ b/src/strategy/verify/test_runner.rs
@@ -0,0 +1,718 @@
+//! Test runner verify hook.
+//!
+//! Runs a project's test suite via [`RunCommand`] internals, parses
+//! structured output for pass/fail counts, and returns `Pass` only when
+//! zero tests failed and at least one ran.
+//!
+//! # Supported runners
+//!
+//! | Kind | Command | Parser |
+//! |------|---------|--------|
+//! | `Cargo` | `cargo test --message-format=json --no-fail-fast` | JSON‑lines per [`cargo::test` message format](https://doc.rust-lang.org/cargo/reference/external-tools.html#json-messages) |
+//! | `Pytest` | `pytest --json-report --json-report-file=-` | [pytest-json-report](https://pypi.org/project/pytest-json-report/) summary output |
+
+use std::path::PathBuf;
+use std::time::Duration;
+
+use async_trait::async_trait;
+use serde::de::Deserialize;
+
+use super::run_command::RunCommand;
+use super::{FailureReason, VerifyContext, VerifyError, VerifyHook, VerifyResult};
+
+/// Supported test runner kinds.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub enum TestRunnerKind {
+    Cargo,
+    Pytest,
+}
+
+/// Sandboxing configuration for test runner execution.
+///
+/// Maps to [`RunCommand`] fields but is kept as a separate struct so the
+/// `TestRunner` API doesn't expose `RunCommand` internals directly.
+#[derive(Debug, Clone)]
+pub struct SandboxOpts {
+    /// Environment variable names allowed in child process.
+    /// Default: empty (default‑deny).
+    pub env_allowlist: Vec<String>,
+    /// Wall‑clock timeout before process‑group SIGKILL.
+    pub wall_timeout: Duration,
+    /// Max bytes captured from stdout.
+    pub stdout_cap: usize,
+    /// Max bytes captured from stderr.
+    pub stderr_cap: usize,
+}
+
+impl Default for SandboxOpts {
+    fn default() -> Self {
+        Self {
+            env_allowlist: Vec::new(),
+            wall_timeout: Duration::from_secs(120),
+            stdout_cap: 8192,
+            stderr_cap: 8192,
+        }
+    }
+}
+
+/// Test runner verify hook.
+///
+/// Executes a test suite via the configured [`TestRunnerKind`], parses the
+/// structured output, and returns:
+///
+/// - `Pass` when `failed == 0 && passed > 0`.
+/// - `Fail` when `failed > 0`, carrying `{failed, passed, first_failure_name, first_failure_excerpt}`.
+/// - `Fail { reason: "no tests ran" }` when `passed == 0 && failed == 0`.
+#[derive(Debug, Clone)]
+pub struct TestRunner {
+    /// Which test runner to use.
+    pub runner: TestRunnerKind,
+    /// Working directory for the command.
+    pub cwd: PathBuf,
+    /// Extra arguments passed through to the test command (after the
+    /// runner‑specific base args).
+    pub extra_args: Vec<String>,
+    /// Sandboxing options (timeouts, caps, env allowlist).
+    pub sandbox: SandboxOpts,
+}
+
+impl TestRunner {
+    /// Construct a new test runner.
+    pub fn new(runner: TestRunnerKind, cwd: impl Into<PathBuf>) -> Self {
+        Self {
+            runner,
+            cwd: cwd.into(),
+            extra_args: Vec::new(),
+            sandbox: SandboxOpts::default(),
+        }
+    }
+
+    /// Append extra arguments passed to the test command.
+    pub fn with_extra_args(mut self, args: impl IntoIterator<Item: AsRef<str>>) -> Self {
+        self.extra_args = args.into_iter().map(|s| s.as_ref().to_string()).collect();
+        self
+    }
+
+    /// Override sandbox options.
+    pub fn with_sandbox(mut self, opts: SandboxOpts) -> Self {
+        self.sandbox = opts;
+        self
+    }
+
+    // ── internal helpers ────────────────────────────────────
+
+    fn build_run_command(&self) -> RunCommand {
+        let mut rc = match self.runner {
+            TestRunnerKind::Cargo => {
+                let mut args = vec![
+                    "test".to_string(),
+                    "--message-format=json".to_string(),
+                    "--no-fail-fast".to_string(),
+                ];
+                args.extend(self.extra_args.clone());
+                RunCommand::new("cargo").with_args(args)
+            }
+            TestRunnerKind::Pytest => {
+                let mut args = vec![
+                    "--json-report".to_string(),
+                    "--json-report-file=-".to_string(),
+                ];
+                args.extend(self.extra_args.clone());
+                RunCommand::new("pytest").with_args(args)
+            }
+        };
+
+        rc = rc
+            .with_cwd(self.cwd.clone())
+            .with_env_allowlist(
+                &self
+                    .sandbox
+                    .env_allowlist
+                    .iter()
+                    .map(|s| s.as_str())
+                    .collect::<Vec<_>>(),
+            )
+            .with_wall_timeout(self.sandbox.wall_timeout)
+            .with_stdout_cap(self.sandbox.stdout_cap)
+            .with_stderr_cap(self.sandbox.stderr_cap);
+
+        rc
+    }
+
+    /// Parse cargo test JSON‑lines output into pass/fail counts.
+    ///
+    /// Each line is a JSON object per cargo's `--message-format=json`
+    /// spec. We look for messages with `"type":"test"` and examine
+    /// the `event` field (`"ok"`, `"failed"`, `"ignored"`, etc.).
+    /// Malformed JSON lines are silently skipped.
+    pub fn parse_cargo_output(stdout: &str) -> TestResult {
+        let mut passed = 0u32;
+        let mut failed = 0u32;
+        let mut first_failure_name: Option<String> = None;
+        let mut first_failure_excerpt: Option<String> = None;
+
+        for line in stdout.lines() {
+            let line = line.trim();
+            if line.is_empty() {
+                continue;
+            }
+
+            let value: serde_json::Value = match serde_json::from_str(line) {
+                Ok(v) => v,
+                Err(_) => continue, // skip malformed lines
+            };
+
+            // Only process test events
+            if value.get("type").and_then(|v| v.as_str()) != Some("test") {
+                continue;
+            }
+
+            let event = value.get("event").and_then(|v| v.as_str()).unwrap_or("");
+
+            // Ignored tests have `"ignored": true` alongside `"event": "ok"`.
+            // Skip them — they didn't actually run.
+            let ignored = value
+                .get("ignored")
+                .and_then(|v| v.as_bool())
+                .unwrap_or(false);
+
+            match (event, ignored) {
+                ("ok", false) => {
+                    passed += 1;
+                }
+                ("ok", true) => {
+                    // ignored test — not counted
+                }
+                ("failed", _) => {
+                    failed += 1;
+                    if first_failure_name.is_none() {
+                        first_failure_name = value
+                            .get("name")
+                            .and_then(|v| v.as_str())
+                            .map(|s| s.to_string());
+                        // Cargo puts the failure stdout in the `stdout` field
+                        first_failure_excerpt =
+                            value.get("stdout").and_then(|v| v.as_str()).map(|s| {
+                                let s = s.trim();
+                                // Truncate to first 200 chars for a concise excerpt
+                                truncate_excerpt(s, 200)
+                            });
+                    }
+                }
+                _ => {
+                    // "ignored", "measured" (benchmarks) — not counted
+                }
+            }
+        }
+
+        TestResult {
+            passed,
+            failed,
+            first_failure_name,
+            first_failure_excerpt,
+        }
+    }
+
+    /// Parse pytest JSON report output.
+    ///
+    /// Expects a single JSON object with a `summary` field containing
+    /// `passed` and `failed` integers (from `pytest-json-report`).
+    pub fn parse_pytest_output(stdout: &str) -> TestResult {
+        // Try parsing the entire stdout as JSON first (handles multi-line reports).
+        // Fall back to line-by-line search if that fails.
+        let maybe_value: Option<serde_json::Value> =
+            serde_json::from_str(stdout).ok().or_else(|| {
+                stdout.find('{').and_then(|start| {
+                    let mut de = serde_json::Deserializer::from_str(&stdout[start..]);
+                    serde_json::Value::deserialize(&mut de).ok()
+                })
+            });
+
+        let value = match maybe_value {
+            Some(v) => v,
+            None => {
+                return TestResult {
+                    passed: 0,
+                    failed: 0,
+                    first_failure_name: None,
+                    first_failure_excerpt: Some(
+                        "could not parse pytest JSON report from stdout".to_string(),
+                    ),
+                };
+            }
+        };
+
+        let summary = match value.get("summary") {
+            Some(s) => s,
+            None => {
+                return TestResult {
+                    passed: 0,
+                    failed: 0,
+                    first_failure_name: None,
+                    first_failure_excerpt: Some(
+                        "pytest report missing `summary` field".to_string(),
+                    ),
+                };
+            }
+        };
+
+        let passed = summary.get("passed").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
+        let failed = summary.get("failed").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
+
+        let first_failure_name = value
+            .get("tests")
+            .and_then(|v| v.as_array())
+            .and_then(|tests| {
+                tests
+                    .iter()
+                    .find(|t| t.get("outcome").and_then(|o| o.as_str()) == Some("failed"))
+            })
+            .and_then(|t| t.get("nodeid").and_then(|n| n.as_str()))
+            .map(|s| s.to_string());
+
+        let first_failure_excerpt = value
+            .get("tests")
+            .and_then(|v| v.as_array())
+            .and_then(|tests| {
+                tests
+                    .iter()
+                    .find(|t| t.get("outcome").and_then(|o| o.as_str()) == Some("failed"))
+            })
+            .and_then(|t| {
+                t.get("call")
+                    .and_then(|c| c.get("longrepr"))
+                    .or_else(|| t.get("longrepr"))
+                    .and_then(|l| l.as_str())
+            })
+            .map(|s| truncate_excerpt(s, 200));
+
+        TestResult {
+            passed,
+            failed,
+            first_failure_name,
+            first_failure_excerpt,
+        }
+    }
+
+    fn parse_output(&self, stdout: &str) -> TestResult {
+        match self.runner {
+            TestRunnerKind::Cargo => Self::parse_cargo_output(stdout),
+            TestRunnerKind::Pytest => Self::parse_pytest_output(stdout),
+        }
+    }
+
+    pub fn to_verify_result(
+        run_command_run: super::run_command::CommandRun,
+        result: TestResult,
+    ) -> VerifyResult {
+        let runner_stdout = run_command_run.stdout.to_reason_text();
+        let runner_stderr = run_command_run.stderr.to_reason_text();
+        let truncated = run_command_run.stdout.truncated || run_command_run.stderr.truncated;
+        let exit_code = run_command_run.status.code();
+        let signal = run_command_run
+            .status
+            .code()
+            .is_none()
+            .then(|| {
+                #[cfg(unix)]
+                {
+                    std::os::unix::process::ExitStatusExt::signal(&run_command_run.status)
+                }
+                #[cfg(not(unix))]
+                {
+                    None::<i32>
+                }
+            })
+            .flatten();
+
+        // Check for sandbox violations first
+        if run_command_run.timed_out {
+            return VerifyResult::Fail {
+                reason: FailureReason::new("test runner timed out")
+                    .with_stdout(runner_stdout)
+                    .with_stderr(runner_stderr)
+                    .with_truncated(truncated)
+                    .with_sandbox_violation(crate::strategy::verify::SandboxViolation::Timeout),
+            };
+        }
+
+        if let Some(sig) = signal {
+            return VerifyResult::Fail {
+                reason: FailureReason::new(format!("test runner killed by signal {sig}"))
+                    .with_stdout(runner_stdout)
+                    .with_stderr(runner_stderr)
+                    .with_truncated(truncated)
+                    .with_sandbox_violation(crate::strategy::verify::SandboxViolation::Signal {
+                        signal: sig,
+                    }),
+            };
+        }
+
+        // Parse test output
+        if let Some(code) = exit_code {
+            if code != 0 && result.failed == 0 {
+                if result.passed == 0 {
+                    return VerifyResult::Fail {
+                        reason: FailureReason::new("no tests ran")
+                            .with_stdout(runner_stdout)
+                            .with_stderr(runner_stderr)
+                            .with_truncated(truncated)
+                            .with_exit_code(code)
+                            .with_sandbox_violation(
+                                crate::strategy::verify::SandboxViolation::NonZeroExit { code },
+                            ),
+                    };
+                }
+
+                return VerifyResult::Fail {
+                    reason: FailureReason::new(format!("test runner exited with status {code}"))
+                        .with_stdout(runner_stdout)
+                        .with_stderr(runner_stderr)
+                        .with_truncated(truncated)
+                        .with_exit_code(code)
+                        .with_sandbox_violation(
+                            crate::strategy::verify::SandboxViolation::NonZeroExit { code },
+                        ),
+                };
+            }
+        }
+
+        if result.passed == 0 && result.failed == 0 {
+            return VerifyResult::Fail {
+                reason: FailureReason::new("no tests ran")
+                    .with_stdout(runner_stdout)
+                    .with_stderr(runner_stderr)
+                    .with_truncated(truncated)
+                    .with_exit_code(exit_code.unwrap_or(1)),
+            };
+        }
+
+        if result.failed > 0 {
+            let summary = match (&result.first_failure_name, &result.first_failure_excerpt) {
+                (Some(name), Some(excerpt)) => {
+                    format!(
+                        "{} test(s) failed (first: {name}: {excerpt})",
+                        result.failed
+                    )
+                }
+                (Some(name), None) => {
+                    format!("{} test(s) failed (first: {name})", result.failed)
+                }
+                (None, _) => {
+                    format!("{} test(s) failed", result.failed)
+                }
+            };
+
+            return VerifyResult::Fail {
+                reason: FailureReason::new(summary)
+                    .with_stdout(runner_stdout)
+                    .with_stderr(runner_stderr)
+                    .with_truncated(truncated)
+                    .with_exit_code(exit_code.unwrap_or(1)),
+            };
+        }
+
+        // All passed
+        VerifyResult::Pass
+    }
+}
+
+#[async_trait]
+impl VerifyHook for TestRunner {
+    fn name(&self) -> &str {
+        match self.runner {
+            TestRunnerKind::Cargo => "TestRunner(cargo)",
+            TestRunnerKind::Pytest => "TestRunner(pytest)",
+        }
+    }
+
+    async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
+        let rc = self.build_run_command();
+        let run = rc.run().await?;
+        let parsed = self.parse_output(&run.stdout.to_reason_text());
+        Ok(Self::to_verify_result(run, parsed))
+    }
+}
+
+/// Structured result from parsing test runner output.
+#[derive(Debug, Clone)]
+pub struct TestResult {
+    pub passed: u32,
+    pub failed: u32,
+    pub first_failure_name: Option<String>,
+    pub first_failure_excerpt: Option<String>,
+}
+
+fn truncate_excerpt(text: &str, max_chars: usize) -> String {
+    let normalized = text.trim();
+
+    let char_count = normalized.chars().count();
+    if char_count <= max_chars {
+        return normalized.to_string();
+    }
+
+    normalized
+        .chars()
+        .take(max_chars)
+        .collect::<String>()
+        .chars()
+        .chain(Some('…'))
+        .collect()
+}
+
+#[cfg(test)]
+fn exit_status_from_code(code: i32) -> std::process::ExitStatus {
+    #[cfg(unix)]
+    {
+        use std::os::unix::process::ExitStatusExt;
+        std::process::ExitStatus::from_raw(code << 8)
+    }
+
+    #[cfg(not(unix))]
+    {
+        use std::os::windows::process::ExitStatusExt;
+        std::process::ExitStatus::from_raw(code as u32)
+    }
+}
+
+// ── unit tests (parser logic only, no subprocess) ───────────
+
+#[cfg(test)]
+mod tests {
+    use super::*;
+
+    fn cargo_test_result(fixture: &str) -> TestResult {
+        TestRunner::parse_cargo_output(fixture)
+    }
+
+    fn pytest_test_result(fixture: &str) -> TestResult {
+        TestRunner::parse_pytest_output(fixture)
+    }
+
+    // ── Cargo parser ────────────────────────────────────────
+
+    #[test]
+    fn cargo_3_pass_0_fail() {
+        let output = r#"{"type":"test","event":"ok","name":"it_works","test_type":"unit"}
+{"type":"test","event":"ok","name":"test_add","test_type":"unit"}
+{"type":"test","event":"ok","name":"test_subtract","test_type":"unit"}
+"#;
+        let result = cargo_test_result(output);
+        assert_eq!(result.passed, 3);
+        assert_eq!(result.failed, 0);
+        assert!(result.first_failure_name.is_none());
+        assert!(result.first_failure_excerpt.is_none());
+    }
+
+    #[test]
+    fn cargo_2_pass_1_fail() {
+        let output = r#"{"type":"test","event":"ok","name":"it_works","test_type":"unit"}
+{"type":"test","event":"failed","name":"test_bad_add","test_type":"unit","stdout":"assertion `left == right` failed\n  left: 3\n right: 5\n"}
+{"type":"test","event":"ok","name":"test_good_add","test_type":"unit"}
+"#;
+        let result = cargo_test_result(output);
+        assert_eq!(result.passed, 2);
+        assert_eq!(result.failed, 1);
+        assert_eq!(result.first_failure_name.as_deref(), Some("test_bad_add"));
+        assert!(result
+            .first_failure_excerpt
+            .as_deref()
+            .unwrap_or("")
+            .contains("left == right"));
+    }
+
+    #[test]
+    fn cargo_empty_no_tests() {
+        let output = r#"{"type":"test","event":"ok","name":"test_dummy","test_type":"unit","ignored":true}
+"#;
+        // Only ignored tests — no actual tests ran
+        let result = cargo_test_result(output);
+        assert_eq!(result.passed, 0);
+        assert_eq!(result.failed, 0);
+    }
+
+    #[test]
+    fn cargo_malformed_json_line_skipped() {
+        let output = r#"{"type":"test","event":"ok","name":"good_test","test_type":"unit"}
+this is not json at all
+{"type":"test","event":"ok","name":"another_test","test_type":"unit"}
+{"type":"test","event":"failed","name":"bad_test","test_type":"unit","stdout":"failure!"}
+"#;
+        let result = cargo_test_result(output);
+        assert_eq!(result.passed, 2);
+        assert_eq!(result.failed, 1);
+        assert_eq!(result.first_failure_name.as_deref(), Some("bad_test"));
+    }
+
+    #[test]
+    fn cargo_skips_compiler_messages() {
+        let output = r#"{"type":"compiler","message":"compiling foo"}
+{"type":"test","event":"ok","name":"test_foo","test_type":"unit"}
+{"type":"artifact","profile":"test"}
+{"type":"test","event":"ok","name":"test_bar","test_type":"unit"}
+"#;
+        let result = cargo_test_result(output);
+        assert_eq!(result.passed, 2);
+        assert_eq!(result.failed, 0);
+    }
+
+    #[test]
+    fn cargo_first_failure_preserves_stdout_excerpt() {
+        let output = r#"{"type":"test","event":"ok","name":"passing","test_type":"unit"}
+{"type":"test","event":"failed","name":"failing_test","test_type":"unit","stdout":"thread 'failing_test' panicked at src/main.rs:42:\nassertion `left == right` failed\n  left: 1\n right: 2\nnote: run with `RUST_BACKTRACE=1` environment variable to display a backtrace\n"}
+{"type":"test","event":"failed","name":"also_failing","test_type":"unit","stdout":"another failure"}
+"#;
+        let result = cargo_test_result(output);
+        assert_eq!(result.passed, 1);
+        assert_eq!(result.failed, 2);
+        assert_eq!(result.first_failure_name.as_deref(), Some("failing_test"));
+        let excerpt = result.first_failure_excerpt.unwrap();
+        assert!(excerpt.contains("panicked"));
+        assert!(excerpt.contains("left == right"));
+    }
+
+    #[test]
+    fn cargo_first_failure_truncates_utf8_excerpt_safely() {
+        let unicode_excerpt = "😀".repeat(260);
+        let output = format!(
+            "{{\"type\":\"test\",\"event\":\"ok\",\"name\":\"passing\",\"test_type\":\"unit\"}}\n{{\"type\":\"test\",\"event\":\"failed\",\"name\":\"failing_test\",\"test_type\":\"unit\",\"stdout\":\"{unicode_excerpt}\"}}",
+        );
+        let result = cargo_test_result(&output);
+
+        let excerpt = result
+            .first_failure_excerpt
+            .expect("expected excerpt to be captured");
+
+        assert_eq!(excerpt.chars().count(), 201);
+        assert!(excerpt.ends_with('…'));
+        assert!(excerpt.len() > 200);
+        assert!(excerpt.len() < 810);
+    }
+
+    // ── Pytest parser ───────────────────────────────────────
+
+    #[test]
+    fn pytest_5_pass_0_fail() {
+        let output = r#"{"created": 1234567890, "duration": 0.15, "exitcode": 0, "root": "/tmp", "summary": {"passed": 5, "failed": 0, "total": 5, "collected": 5}, "tests": [{"nodeid": "test_foo.py::test_a", "outcome": "passed"}, {"nodeid": "test_foo.py::test_b", "outcome": "passed"}]}"#;
+        let result = pytest_test_result(output);
+        assert_eq!(result.passed, 5);
+        assert_eq!(result.failed, 0);
+        assert!(result.first_failure_name.is_none());
+    }
+
+    #[test]
+    fn pytest_4_pass_2_fail() {
+        let output = r#"{"created": 1234567890, "duration": 0.3, "exitcode": 1, "root": "/tmp", "summary": {"passed": 4, "failed": 2, "total": 6, "collected": 6}, "tests": [{"nodeid": "test_foo.py::test_a", "outcome": "passed"}, {"nodeid": "test_bar.py::test_bad", "outcome": "failed", "call": {"longrepr": "AssertionError: expected 5 got 3"}}, {"nodeid": "test_baz.py::test_bad2", "outcome": "failed", "call": {"longrepr": "TypeError: unsupported operand"}}]}"#;
+        let result = pytest_test_result(output);
+        assert_eq!(result.passed, 4);
+        assert_eq!(result.failed, 2);
+        assert_eq!(
+            result.first_failure_name.as_deref(),
+            Some("test_bar.py::test_bad")
+        );
+        assert!(result
+            .first_failure_excerpt
+            .as_deref()
+            .unwrap_or("")
+            .contains("AssertionError"));
+    }
+
+    #[test]
+    fn pytest_empty_no_tests() {
+        let output = r#"{"created": 1234567890, "duration": 0.01, "exitcode": 0, "root": "/tmp", "summary": {"passed": 0, "failed": 0, "total": 0, "collected": 0}, "tests": []}"#;
+        let result = pytest_test_result(output);
+        assert_eq!(result.passed, 0);
+        assert_eq!(result.failed, 0);
+    }
+
+    #[test]
+    fn pytest_missing_summary_field() {
+        let output = r#"{"created": 1234567890, "duration": 0.01, "exitcode": 0}"#;
+        let result = pytest_test_result(output);
+        assert_eq!(result.passed, 0);
+        assert_eq!(result.failed, 0);
+        assert!(result
+            .first_failure_excerpt
+            .as_deref()
+            .unwrap_or("")
+            .contains("missing `summary` field"));
+    }
+
+    #[test]
+    fn pytest_non_json_output() {
+        let output = "pytest: error: no tests found";
+        let result = pytest_test_result(output);
+        assert_eq!(result.passed, 0);
+        assert_eq!(result.failed, 0);
+        assert!(result
+            .first_failure_excerpt
+            .as_deref()
+            .unwrap_or("")
+            .contains("could not parse"));
+    }
+
+    // ── VerifyResult conversion ─────────────────────────────
+
+    fn fake_captured_output(data: &str) -> super::super::run_command::CapturedOutput {
+        super::super::run_command::CapturedOutput {
+            data: data.as_bytes().to_vec(),
+            truncated: false,
+            elided_bytes: 0,
+        }
+    }
+
+    fn fake_command_run(stdout_data: &str) -> super::super::run_command::CommandRun {
+        super::super::run_command::CommandRun {
+            status: exit_status_from_code(0),
+            timed_out: false,
+            stdout: fake_captured_output(stdout_data),
+            stderr: fake_captured_output(""),
+            secret_values: vec![],
+            elapsed_ms: 10,
+        }
+    }
+
+    #[test]
+    fn verify_result_from_passing_tests() {
+        let stdout = r#"{"type":"test","event":"ok","name":"test_a","test_type":"unit"}
+{"type":"test","event":"ok","name":"test_b","test_type":"unit"}
+"#;
+        let result = TestRunner::parse_cargo_output(stdout);
+        let run = fake_command_run(stdout);
+        let vr = TestRunner::to_verify_result(run, result);
+        assert!(matches!(vr, VerifyResult::Pass));
+    }
+
+    #[test]
+    fn verify_result_from_failing_tests() {
+        let stdout = r#"{"type":"test","event":"ok","name":"test_a","test_type":"unit"}
+{"type":"test","event":"failed","name":"test_bad","test_type":"unit","stdout":"assertion failed!"}
+"#;
+        let result = TestRunner::parse_cargo_output(stdout);
+        let run = fake_command_run(stdout);
+        let vr = TestRunner::to_verify_result(run, result);
+        match vr {
+            VerifyResult::Fail { reason } => {
+                assert!(reason.summary.contains("1 test(s) failed"));
+                assert!(reason.summary.contains("test_bad"));
+                assert!(reason.summary.contains("assertion failed!"));
+            }
+            other => panic!("expected Fail, got {other:?}"),
+        }
+    }
+
+    #[test]
+    fn verify_result_no_tests_ran() {
+        let stdout = r#"{"type":"test","event":"ignored","name":"ignored_test","test_type":"unit"}
+"#;
+        let result = TestRunner::parse_cargo_output(stdout);
+        let run = fake_command_run(stdout);
+        let vr = TestRunner::to_verify_result(run, result);
+        match vr {
+            VerifyResult::Fail { reason } => {
+                assert!(reason.summary.contains("no tests ran"));
+            }
+            other => panic!("expected Fail, got {other:?}"),
+        }
+    }
+}
diff --git a/tests/verify_test_runner.rs b/tests/verify_test_runner.rs
new file mode 100644
index 0000000..76a46fb
--- /dev/null
+++ b/tests/verify_test_runner.rs
@@ -0,0 +1,258 @@
+//! Integration tests for the `TestRunner` verify hook.
+//!
+//! Drives the JSON parsers from canned fixture files — no actual `cargo`
+//! or `pytest` invocation in unit tests. Parser output is then fed
+//! through `TestRunner::to_verify_result` to exercise the
+//! `VerifyHook::verify` logic without subprocess overhead.
+
+use std::path::Path;
+
+use loker::strategy::verify::run_command::{CapturedOutput, CommandRun};
+use loker::strategy::verify::{TestRunner, VerifyResult};
+
+// ── helpers ──────────────────────────────────────────────────
+
+/// Read a fixture file as a string.
+fn read_fixture(name: &str) -> String {
+    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
+        .join("tests")
+        .join("fixtures")
+        .join("test_runner")
+        .join(name);
+    std::fs::read_to_string(&path).unwrap_or_else(|e| {
+        panic!("failed to read fixture {name}: {e}");
+    })
+}
+
+/// Convert fixture content to a JSON‑lines string that the cargo parser
+/// would see on stdout. `.jsonl` files contain a JSON array — we flatten
+/// each element to one JSON-per-line.
+fn fixture_to_cargo_stdout(fixture: &str) -> String {
+    // Cargo JSON-lines fixtures are raw JSON-per-line content.
+    fixture.to_string()
+}
+
+/// Read a pytest fixture file as a string.
+fn fake_captured_output(data: &str) -> CapturedOutput {
+    CapturedOutput {
+        data: data.as_bytes().to_vec(),
+        truncated: false,
+        elided_bytes: 0,
+    }
+}
+
+fn exit_status(code: i32) -> std::process::ExitStatus {
+    #[cfg(unix)]
+    {
+        use std::os::unix::process::ExitStatusExt;
+        std::process::ExitStatus::from_raw(code << 8)
+    }
+
+    #[cfg(not(unix))]
+    {
+        use std::os::windows::process::ExitStatusExt;
+        std::process::ExitStatus::from_raw(code as u32)
+    }
+}
+
+fn fake_command_run_with_code(stdout_data: &str, exit_code: i32) -> CommandRun {
+    CommandRun {
+        status: exit_status(exit_code),
+        timed_out: false,
+        stdout: fake_captured_output(stdout_data),
+        stderr: fake_captured_output(""),
+        secret_values: vec![],
+        elapsed_ms: 10,
+    }
+}
+
+fn fake_command_run(stdout_data: &str) -> CommandRun {
+    fake_command_run_with_code(stdout_data, 0)
+}
+
+/// Parse cargo fixture, then convert to VerifyResult.
+fn cargo_fixture_verify(name: &str) -> VerifyResult {
+    let raw = read_fixture(name);
+    let stdout = fixture_to_cargo_stdout(&raw);
+    let result = TestRunner::parse_cargo_output(&stdout);
+    let run = fake_command_run(&stdout);
+    TestRunner::to_verify_result(run, result)
+}
+
+/// Parse pytest fixture, then convert to VerifyResult.
+fn pytest_fixture_verify(name: &str) -> VerifyResult {
+    let stdout = read_fixture(name);
+    let result = TestRunner::parse_pytest_output(&stdout);
+    let run = fake_command_run(&stdout);
+    TestRunner::to_verify_result(run, result)
+}
+
+// ── Cargo tests ─────────────────────────────────────────────
+
+#[test]
+fn cargo_3_pass_0_fail() {
+    let result = cargo_fixture_verify("cargo_3pass_0fail.jsonl");
+    assert!(
+        matches!(result, VerifyResult::Pass),
+        "expected Pass, got {result:?}"
+    );
+}
+
+#[test]
+fn cargo_2_pass_1_fail() {
+    let result = cargo_fixture_verify("cargo_2pass_1fail.jsonl");
+    match result {
+        VerifyResult::Fail { reason } => {
+            assert!(
+                reason.summary.contains("1 test(s) failed"),
+                "summary should mention 1 failure: {}",
+                reason.summary
+            );
+            assert!(
+                reason.summary.contains("test_bad_divide"),
+                "summary should contain failure name: {}",
+                reason.summary
+            );
+            assert!(
+                reason.summary.contains("assertion"),
+                "summary should contain failure excerpt: {}",
+                reason.summary
+            );
+        }
+        other => panic!("expected Fail, got {other:?}"),
+    }
+}
+
+#[test]
+fn cargo_empty_no_tests() {
+    let result = cargo_fixture_verify("cargo_empty.jsonl");
+    match result {
+        VerifyResult::Fail { reason } => {
+            assert!(
+                reason.summary.contains("no tests ran"),
+                "expected 'no tests ran', got: {}",
+                reason.summary
+            );
+        }
+        other => panic!("expected Fail, got {other:?}"),
+    }
+}
+
+#[test]
+fn cargo_malformed_json_lines() {
+    // The malformed fixture has a non-JSON line mixed with valid JSON lines.
+    // The parser should skip the bad line and still count the valid ones.
+    let raw = read_fixture("cargo_malformed.jsonl");
+    let stdout = fixture_to_cargo_stdout(&raw);
+    let result = TestRunner::parse_cargo_output(&stdout);
+    assert_eq!(result.passed, 2, "should count 2 passing tests");
+    assert_eq!(result.failed, 1, "should count 1 failing test");
+}
+
+// ── Pytest tests ────────────────────────────────────────────
+
+#[test]
+fn pytest_5_pass_0_fail() {
+    let result = pytest_fixture_verify("pytest_5pass_0fail.json");
+    assert!(
+        matches!(result, VerifyResult::Pass),
+        "expected Pass, got {result:?}"
+    );
+}
+
+#[test]
+fn pytest_4_pass_2_fail() {
+    let result = pytest_fixture_verify("pytest_4pass_2fail.json");
+    match result {
+        VerifyResult::Fail { reason } => {
+            assert!(
+                reason.summary.contains("2 test(s) failed"),
+                "summary should mention 2 failures: {}",
+                reason.summary
+            );
+            assert!(
+                reason.summary.contains("test_bar.py::test_bad"),
+                "summary should contain first failure name: {}",
+                reason.summary
+            );
+            assert!(
+                reason.summary.contains("AssertionError"),
+                "summary should contain failure excerpt: {}",
+                reason.summary
+            );
+        }
+        other => panic!("expected Fail, got {other:?}"),
+    }
+}
+
+#[test]
+fn pytest_noisy_multiline_output_is_parsed() {
+    let stdout = r#"INFO: running tests
+{
+  "created": 1234567890,
+  "duration": 0.1,
+  "exitcode": 0,
+  "root": "/tmp",
+  "summary": {"passed": 1, "failed": 0, "total": 1, "collected": 1},
+  "tests": [{"nodeid": "test.py::test_ok", "outcome": "passed"}]
+}
+INFO: done
+"#;
+    let result = TestRunner::parse_pytest_output(stdout);
+    assert_eq!(result.passed, 1);
+    assert_eq!(result.failed, 0);
+}
+
+#[test]
+fn pytest_non_json_exit() {
+    // Simulate process exits non-zero with no JSON written
+    let stdout = "pytest: error: no tests found in test_runner/\n";
+    let result = TestRunner::parse_pytest_output(stdout);
+    assert_eq!(result.passed, 0);
+    assert_eq!(result.failed, 0);
+
+    // Build a command run with non-zero exit
+    let run = CommandRun {
+        status: exit_status(1),
+        timed_out: false,
+        stdout: fake_captured_output(stdout),
+        stderr: fake_captured_output("ERROR: no tests collected\n"),
+        secret_values: vec![],
+        elapsed_ms: 15,
+    };
+    let vr = TestRunner::to_verify_result(run, result);
+    match vr {
+        VerifyResult::Fail { reason } => {
+            assert!(
+                reason.summary.contains("no tests ran"),
+                "expected 'no tests ran', got: {}",
+                reason.summary
+            );
+        }
+        other => panic!("expected Fail, got {other:?}"),
+    }
+}
+
+#[test]
+fn verify_result_non_zero_exit_with_passing_tests_fails() {
+    let stdout = r#"{"type":"test","event":"ok","name":"test_a","test_type":"unit"}
+{"type":"test","event":"ok","name":"test_b","test_type":"unit"}
+"#;
+    let result = TestRunner::parse_cargo_output(stdout);
+    let run = fake_command_run_with_code(stdout, 1);
+    let vr = TestRunner::to_verify_result(run, result);
+    match vr {
+        VerifyResult::Fail { reason } => {
+            assert!(
+                reason.summary.contains("test runner exited with status 1"),
+                "expected non-zero status summary, got: {}",
+                reason.summary
+            );
+            assert!(
+                reason.sandbox_violation.is_some(),
+                "expected sandbox violation for non-zero exit"
+            );
+        }
+        other => panic!("expected Fail, got {other:?}"),
+    }
+}

exec
/bin/zsh -lc "sed -n '1,280p' src/strategy/verify/verify.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
exec
/bin/zsh -lc "sed -n '280,520p' src/strategy/verify/run_command.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
/// Redact known secret patterns and specific allowlisted secret values from text.
fn redact_output(text: &str, secret_values: &[String]) -> String {
    let mut result = redact_secrets(text);
    for secret in secret_values {
        if !secret.is_empty() && result.contains(secret.as_str()) {
            result = result.replace(secret.as_str(), "[REDACTED]");
        }
    }
    result
}

async fn read_stream_bounded<R: AsyncRead + Unpin + Send + 'static>(
    mut reader: R,
    max_bytes: usize,
) -> CapturedOutput {
    let mut buf = [0u8; 8192];
    let mut data = Vec::new();
    let mut truncated = false;
    let mut elided_bytes = 0usize;

    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let remaining = max_bytes.saturating_sub(data.len());
                if remaining == 0 {
                    truncated = true;
                    elided_bytes += n;
                    elided_bytes += drain_stream(&mut reader).await;
                    break;
                }

                let take = n.min(remaining);
                data.extend_from_slice(&buf[..take]);

                if take < n {
                    truncated = true;
                    elided_bytes += n - take;
                    elided_bytes += drain_stream(&mut reader).await;
                    break;
                }
            }
            Err(_) => break,
        }
    }

    CapturedOutput {
        data,
        truncated,
        elided_bytes,
    }
}

async fn drain_stream<R: AsyncRead + Unpin + Send>(reader: &mut R) -> usize {
    let mut buf = [0u8; 8192];
    let mut total = 0usize;

    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                total += n;
            }
        }
    }

    total
}

fn is_secret_like_env_key(key: &str) -> bool {
    let key = key.to_ascii_uppercase();
    key.contains("SECRET")
        || key.contains("TOKEN")
        || key.contains("PASSWORD")
        || key.contains("API_KEY")
        || key.contains("AUTH")
}

#[cfg(unix)]
fn status_signal(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn status_signal(_status: &std::process::ExitStatus) -> Option<i32> {
    None
}

#[cfg(unix)]
fn kill_process_group(pid: Option<u32>) {
    if let Some(pid) = pid {
        // SAFETY: this sends SIGKILL to a process group spawned by us and avoids
        // orphaned children. If the pid is stale or already dead, this is a
        // best-effort cleanup attempt.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
}

#[cfg(not(unix))]
fn kill_process_group(pid: Option<u32>) {
    let _ = pid;
}

#[async_trait]
impl VerifyHook for RunCommand {
    fn name(&self) -> &str {
        "RunCommand"
    }

    async fn verify(&self, _ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
        let run = self.run().await?;

        let stdout = redact_output(&run.stdout.to_reason_text(), &run.secret_values);
        let stderr = redact_output(&run.stderr.to_reason_text(), &run.secret_values);
        let truncated = run.stdout.truncated || run.stderr.truncated;
        let signal = status_signal(&run.status);

        if run.timed_out {
            return Ok(VerifyResult::Fail {
                reason: FailureReason::new(format!("command timed out: {}", self.cmd))
                    .with_stdout(stdout)
                    .with_stderr(stderr)
                    .with_truncated(truncated)
                    .with_sandbox_violation(SandboxViolation::Timeout),
            });
        }

        if let Some(code) = run.status.code() {
            if code == 0 {
                return Ok(VerifyResult::Pass);
            }

            return Ok(VerifyResult::Fail {
                reason: FailureReason::new(format!("command exited with status {code}"))
                    .with_exit_code(code)
                    .with_stdout(stdout)
                    .with_stderr(stderr)
                    .with_truncated(truncated)
                    .with_sandbox_violation(SandboxViolation::NonZeroExit { code }),
            });
        }

        if let Some(sig) = signal {
            return Ok(VerifyResult::Fail {
                reason: FailureReason::new(format!("command killed by signal {sig}"))
                    .with_stdout(stdout)
                    .with_stderr(stderr)
                    .with_truncated(truncated)
                    .with_sandbox_violation(SandboxViolation::Signal { signal: sig }),
            });
        }

        Ok(VerifyResult::Fail {
            reason: FailureReason::new(format!("command terminated unexpectedly: {}", self.cmd))
                .with_stdout(stdout)
                .with_stderr(stderr)
                .with_truncated(truncated),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> VerifyContext {
        VerifyContext {
            stdout: String::new(),
            stderr: None,
            exit_code: None,
            backend_name: "test".to_string(),
            model: None,
            structured: None,
            duration: Duration::ZERO,
        }
    }

    #[test]
    fn run_command_builder_api() {
        let rc = RunCommand::new("cargo")
            .with_args(vec!["test".to_string(), "--quiet".to_string()])
            .with_env_allowlist(&["PATH", "HOME"])
            .with_cwd("/tmp")
            .with_wall_timeout(Duration::from_secs(10))
            .with_cpu_timeout(Some(Duration::from_secs(5)))
            .with_stdout_cap(1024)
            .with_stderr_cap(2048);

        assert_eq!(rc.cmd, "cargo");
        assert_eq!(rc.args, vec!["test", "--quiet"]);
        assert_eq!(rc.env_allowlist, vec!["PATH", "HOME"]);
        assert_eq!(rc.cwd, Some(PathBuf::from("/tmp")));
        assert_eq!(rc.wall_timeout, Duration::from_secs(10));
        assert_eq!(rc.cpu_timeout, Some(Duration::from_secs(5)));
        assert_eq!(rc.stdout_cap, 1024);
        assert_eq!(rc.stderr_cap, 2048);
    }

    #[test]
    fn run_command_default_values() {
        let rc = RunCommand::new("echo");
        assert_eq!(rc.cmd, "echo");
        assert!(rc.args.is_empty());
        assert!(rc.env_allowlist.is_empty());
        assert!(rc.cwd.is_none());
        assert_eq!(rc.wall_timeout, DEFAULT_WALL_TIMEOUT);
        assert_eq!(rc.cpu_timeout, None);
        assert_eq!(rc.stdout_cap, DEFAULT_OUTPUT_CAP);
        assert_eq!(rc.stderr_cap, DEFAULT_OUTPUT_CAP);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn verify_echo_passes() {
        let rc = RunCommand::new("sh").with_args(vec!["-c".to_string(), "echo hello".to_string()]);
        let result = rc.verify(&context()).await.unwrap();
        assert!(matches!(result, VerifyResult::Pass));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn verify_false_fails_with_code() {
        let rc = RunCommand::new("sh")
            .with_args(vec!["-c".to_string(), "echo err >&2; exit 1".to_string()]);
        let result = rc.verify(&context()).await.unwrap();
        match result {
            VerifyResult::Fail { reason } => {
                assert_eq!(reason.exit_code, Some(1));
                assert!(reason.stderr.contains("err"));
                assert!(matches!(
                    reason.sandbox_violation,
                    Some(SandboxViolation::NonZeroExit { code: 1 })
                ));
            }
            other => panic!("expected fail, got {other:?}"),
        }
    }


 succeeded in 0ms:
//! Core verification types and trait.
//!
//! This module defines the `VerifyHook` trait and supporting types
//! (`VerifyResult`, `FailureReason`, `VerifyError`, `VerifyContext`).
//! Concrete implementations live in sibling modules (`llm_verifier`,
//! `run_command`).
//!
//! v0 hooks only need `Pass` and `Fail`. `Repair` and `Score` are reserved so
//! later hook implementations can evolve without changing the public enum.
//!
//! ## Security: redaction
//!
//! `FailureReason.stdout` and `FailureReason.stderr` carry raw output that may
//! contain secrets. Redaction is deferred to the consumer — CLO-260's
//! `pass_failure_context` path runs `redact_secrets()` before flowing the
//! reason into the next prompt. Hook implementations that log or persist
//! `FailureReason` fields directly must apply their own redaction.

/// Sandbox-level signal captured from command execution failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SandboxViolation {
    /// Wall-clock timeout expired while the command was running.
    Timeout,
    /// Process exited due to an OS signal (Unix-only).
    Signal { signal: i32 },
    /// Non-zero process exit code.
    NonZeroExit { code: i32 },
}

// ── FailureReason ────────────────────────────────────────────

use std::time::Duration;

use async_trait::async_trait;

use crate::backend::QueryOutput;

// ── FailureReason ────────────────────────────────────────────

/// Structured reason a verification hook returned `Fail`.
///
/// Carries enough detail to feed `pass_failure_context` (CLO-260).
/// Fields are `pub` so callers can extract individual signals without
/// parsing the combined `display()` string.
///
/// ## Security: Redaction
///
/// `stdout` and `stderr` carry raw output that may contain secrets
/// (API keys in LLM responses, stack traces with env vars). Redaction
/// is **deferred to the consumer** — CLO-260's `pass_failure_context`
/// path runs `redact_secrets()` on the reason before flowing it into
/// the next prompt. Hook implementations that log or persist
/// `FailureReason` fields directly must apply their own redaction.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct FailureReason {
    /// Human-readable summary (e.g. "test `it_adds` failed").
    pub summary: String,
    /// Captured stdout from the verification run (may be truncated).
    /// **Unredacted** — consumers must apply redaction before prompt injection.
    pub stdout: String,
    /// Captured stderr from the verification run (may be truncated).
    /// **Unredacted** — consumers must apply redaction before prompt injection.
    pub stderr: String,
    /// `true` iff stdout or stderr was truncated at `MAX_OUTPUT_BYTES`.
    pub truncated: bool,
    /// Exit code if the verifier ran as a process. `None` for in‑process
    /// verifiers (e.g. `LLMVerifier`).
    pub exit_code: Option<i32>,
    /// Optional sandbox signal that further explains process termination.
    pub sandbox_violation: Option<SandboxViolation>,
}

impl FailureReason {
    /// Create a new failure reason with a human-readable summary.
    /// All other fields default to empty / `None`.
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            stdout: String::new(),
            stderr: String::new(),
            truncated: false,
            exit_code: None,
            sandbox_violation: None,
        }
    }

    /// Attach captured stdout (builder-pattern).
    pub fn with_stdout(mut self, stdout: impl Into<String>) -> Self {
        self.stdout = stdout.into();
        self
    }

    /// Attach captured stderr (builder-pattern).
    pub fn with_stderr(mut self, stderr: impl Into<String>) -> Self {
        self.stderr = stderr.into();
        self
    }

    /// Mark the output as truncated (builder-pattern).
    pub fn with_truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }

    /// Attach an exit code (builder-pattern).
    pub fn with_exit_code(mut self, exit_code: i32) -> Self {
        self.exit_code = Some(exit_code);
        self
    }

    /// Attach sandbox metadata (builder-pattern).
    pub fn with_sandbox_violation(mut self, sandbox_violation: SandboxViolation) -> Self {
        self.sandbox_violation = Some(sandbox_violation);
        self
    }
}

impl std::fmt::Display for FailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.summary)?;
        if self.truncated {
            write!(f, " (truncated)")?;
        }
        Ok(())
    }
}

// ── VerifyResult ─────────────────────────────────────────────

/// Verdict returned by a `VerifyHook::verify()` call.
///
/// **Variant lifecycle:**
///
/// | Variant | v0 status | Notes |
/// |---------|-----------|-------|
/// | `Pass`  | **live** — emitted by v0 hooks | |
/// | `Fail { reason }` | **live** — `reason` is `FailureReason` | |
/// | `Repair { suggestion }` | **reserved** — compiles, no caller acts on it yet | M10 `HumanVerifier` will emit this |
/// | `Score(f32)` | **reserved** — compiles, no caller acts on it yet. Higher values = better quality. | Future cascadeflow‑style semantic gates |
///
/// Callers that pattern‑match MUST include arms for reserved variants;
/// the recommended pattern is a documented fallthrough (see
/// `escalating_retry.rs` for the reference consumer).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum VerifyResult {
    Pass,
    Fail { reason: FailureReason },
    Repair { suggestion: String },
    Score(f32),
}

impl VerifyResult {
    /// Convenience constructor for a `Pass` variant.
    pub fn pass() -> Self {
        Self::Pass
    }

    /// Convenience constructor for a `Fail` variant with a simple summary.
    /// Other `FailureReason` fields default to empty.
    pub fn fail(summary: impl Into<String>) -> Self {
        Self::Fail {
            reason: FailureReason::new(summary),
        }
    }

    /// Convenience constructor for a `Fail` variant with a fully populated reason.
    pub fn fail_with(reason: FailureReason) -> Self {
        Self::Fail { reason }
    }

    /// `true` iff this is a `Pass` variant.
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    /// `true` iff this is a `Fail` variant.
    pub fn is_fail(&self) -> bool {
        matches!(self, Self::Fail { .. })
    }
}

// ── VerifyError ──────────────────────────────────────────────

/// Error surfaced when a `VerifyHook` implementation itself fails.
///
/// Distinct from `VerifyResult::Fail`:
/// - `Fail` means the hook ran, decided "that output isn't good enough",
///   and produced a structured `FailureReason`.
/// - `VerifyError` means the hook could not run at all: sandbox crash,
///   backend unreachable, `make` missing from `$PATH`, etc.
///
/// ## Future: error source chain
/// For v0 the `message` string suffices. When CLO-271 (RunCommand)
/// introduces I/O errors and subprocess failures, `VerifyError` should
/// gain a `#[source]`-annotated field (e.g. `source: Option<Box<dyn std::error::Error + Send + Sync>>`)
/// to preserve the original error chain for debugging.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("verify hook failed: {message}")]
pub struct VerifyError {
    pub message: String,
}

impl VerifyError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

// ── VerifyContext ────────────────────────────────────────────

/// Input passed to every `VerifyHook::verify()` call.
///
/// Carries the output under verification plus metadata about the phase
/// and backend that produced it. Does **not** carry credentials
/// (API keys, tokens) — those live in `BackendConfig` and are never
/// exposed to verify hooks.
///
/// `#[non_exhaustive]` so the phase runner (T-029) can add fields
/// (manifest pointer, run‑dir paths) without breaking hook implementations.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct VerifyContext {
    /// Raw stdout from the backend call under verification.
    pub stdout: String,
    /// Raw stderr from the backend call, if any.
    pub stderr: Option<String>,
    /// Exit code if the backend ran as a subprocess.
    pub exit_code: Option<i32>,
    /// Name of the backend that produced this output (e.g. `"claude-3"`).
    pub backend_name: String,
    /// Model name reported by the backend, if known.
    pub model: Option<String>,
    /// Parsed JSON if the output was successfully deserialized.
    pub structured: Option<serde_json::Value>,
    /// Wall‑clock duration of the backend call, as measured by `Backend::query()`.
    pub duration: Duration,
}

impl VerifyContext {
    /// Build a `VerifyContext` from a `QueryOutput` plus a backend name.
    ///
    /// This is the EscalatingRetry call‑site constructor. When the phase
    /// runner (T-029) replaces EscalatingRetry as the direct caller, it
    /// can build `VerifyContext` from other sources (manifest, run dir)
    /// without touching hook implementations.
    pub fn from_query_output(query: &QueryOutput) -> Self {
        Self {
            stdout: query.stdout.clone(),
            stderr: query.stderr.clone(),
            exit_code: query.exit_code,
            backend_name: query.backend.clone(),
            model: query.model.clone(),
            structured: query.structured.clone(),
            duration: query.duration,
        }
    }
}

// ── VerifyHook trait ─────────────────────────────────────────

/// Verification hook that gates strategy progress.
///
/// Implementations are `Send + Sync` so they can be shared behind `Arc`
/// and driven across async tasks by the phase runner.
///
/// ## Method contract
///
/// - `name()` returns a stable, human‑readable label for trace output
///   (e.g. `"TestRunner"`, `"LLMVerifier"`).
/// - `verify(ctx)` inspects the backend output in `ctx` and returns a
///   verdict. `Err(VerifyError)` signals the hook itself failed;
///   `Ok(VerifyResult::Fail { .. })` signals the hook ran successfully
///   but judged the output insufficient.
///
/// ## Required context contract
///

codex
I have one likely API-surface concern already: `CommandRun` and `CapturedOutput` were widened to `pub` purely to support tests, and they are not part of the design’s intended surface. I’m validating the build gates now to see whether anything fails earlier than that review issue.
  ✓ Read the CLO-273 design, plan, and workflow context to extract acceptance criteria and planned tasks
  → Inspect the branch diff for correctness, scope, public surface, error handling, and test coverage
  → Run the required validation commands through make check and capture the first failing gate, if any
  • Produce the pre-PR validation report with findings and binding verdict
exec
/bin/zsh -lc 'cargo fmt --check' in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 390ms:
exec
/bin/zsh -lc 'cargo clippy --all-targets --all-features -- -D warnings' in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 426ms:
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.39s

exec
/bin/zsh -lc 'cargo clippy --tests' in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
codex
The first two gates are clean. `cargo clippy --tests` is still running; once that finishes I’ll continue directly into `cargo test` and `make check`.
 succeeded in 5211ms:
    Checking loker v20260427.0.0 (/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.17s

exec
/bin/zsh -lc 'cargo test' in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 exited 101 in 444ms:
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.17s
     Running unittests src/lib.rs (target/debug/deps/loker-e0bbd5653d27b3fc)

running 651 tests
test aggregator::concat::tests::vote_kind_maps_to_strategy_label ... ok
test aggregator::concat::tests::concat_kind_maps_to_strategy_label ... ok
test aggregator::concat::tests::llm_judge_kind_maps_to_strategy_label ... ok
test aggregator::llm_judge::tests::llm_judge_family_overlap_opt_out_warns ... ok
test aggregator::llm_judge::tests::llm_judge_family_diverse_ok ... ok
test aggregator::llm_judge::tests::llm_judge_parse_markdown_fenced_ballot ... ok
test aggregator::llm_judge::tests::llm_judge_parse_out_of_bounds_index ... ok
test aggregator::llm_judge::tests::llm_judge_parse_out_of_bounds_index_clamped ... ok
test aggregator::llm_judge::tests::llm_judge_parse_valid_ballot ... ok
test aggregator::llm_judge::tests::llm_judge_parse_within_bounds_index_clamped ... ok
test aggregator::llm_judge::tests::llm_judge_parse_negative_chosen_index ... ok
test aggregator::llm_judge::tests::llm_judge_parse_missing_chosen_index ... ok
test aggregator::llm_judge::tests::llm_judge_parse_zero_candidates_index ... ok
test aggregator::llm_judge::tests::llm_judge_parse_missing_reason ... ok
test aggregator::llm_judge::tests::llm_judge_family_overlap_blocks ... ok
test aggregator::llm_judge::tests::llm_judge_parse_malformed_json ... ok
test aggregator::tests::empty_text ... ok
test aggregator::tests::extra_keys_ok ... ok
test aggregator::tests::markdown_fenced_json ... ok
test aggregator::tests::pass_true ... ok
test aggregator::tests::missing_pass ... ok
test aggregator::tests::markdown_fenced_fail ... ok
test aggregator::tests::pass_false ... ok
test aggregator::tests::wrong_pass_type ... ok
test aggregator::vote::tests::all_abstain ... ok
test aggregator::vote::tests::closest_family_multiple_matching_buckets ... ok
test aggregator::vote::tests::empty_ballot_counts_as_abstain ... ok
test aggregator::vote::tests::empty_input ... ok
test aggregator::vote::tests::closest_family_no_match_fallback ... ok
test aggregator::vote::tests::free_text_tie_closest_family ... ok
test aggregator::vote::tests::closest_family_multiple_buckets_match ... ok
test aggregator::vote::tests::free_text_tie_first_responder ... ok
test aggregator::vote::tests::normalise_ballot_basic ... ok
test aggregator::vote::tests::normalise_case ... ok
test aggregator::vote::tests::normalise_whitespace ... ok
test aggregator::vote::tests::abstain_backend_error ... ok
test aggregator::vote::tests::quorum_lost ... ok
test aggregator::vote::tests::free_text_clear_winner ... ok
test aggregator::vote::tests::sanitize_comment_in_metadata ... ok
test aggregator::vote::tests::vote_counts_sorted_descending ... ok
test aggregator::vote::tests::whitespace_only_ballot_counts_as_abstain ... ok
test aggregator::vote::tests::free_text_tie_random_deterministic ... ok
test apply_verify::diff_applier::tests::test_apply_empty_edits ... ok
test apply_verify::diff_applier::tests::test_apply_empty_file_path_is_invalid_edit ... ok
test aggregator::concat::tests::concat_empty_input_returns_sentinel ... ok
test aggregator::concat::tests::concat_whitespace_only_success_output_keeps_newline_invariants ... ok
test aggregator::concat::tests::concat_renders_success_sections_in_input_order ... ok
test aggregator::concat::tests::concat_does_not_reexpand_placeholders_inside_metadata ... ok
test aggregator::concat::tests::concat_preserves_unknown_placeholders ... ok
test aggregator::concat::tests::concat_preserves_braced_unknown_expressions_containing_known_tokens ... ok
test aggregator::concat::tests::concat_escapes_multiline_failure_reason ... ok
test aggregator::concat::tests::concat_counts_success_and_failure ... ok
test aggregator::concat::tests::concat_normalizes_crlf_failure_reason ... ok
test aggregator::llm_judge::tests::llm_judge_prompt_includes_phase_name ... ok
test aggregator::llm_judge::tests::llm_judge_prompt_renders_candidates ... ok
test apply_verify::diff_applier::tests::test_apply_rejects_absolute_path ... ok
test apply_verify::diff_applier::tests::test_apply_rejects_path_traversal ... ok
test apply_verify::edit_parser::tests::test_detect_diff ... ok
test apply_verify::edit_parser::tests::test_detect_full_file ... ok
test apply_verify::edit_parser::tests::test_detect_json_array ... ok
test apply_verify::edit_parser::tests::test_detect_json_object ... ok
test apply_verify::edit_parser::tests::test_detect_with_lang_hint_diff ... ok
test apply_verify::edit_parser::tests::test_detect_with_lang_hint_json ... ok
test apply_verify::edit_parser::tests::test_crlf_normalization ... ok
test apply_verify::edit_parser::tests::test_diff_multi_file ... ok
test apply_verify::edit_parser::tests::test_diff_context_lines ... ok
test apply_verify::edit_parser::tests::test_diff_single_file ... ok
test apply_verify::edit_parser::tests::test_diff_no_hunks ... ok
test apply_verify::edit_parser::tests::test_diff_no_newline_marker ... ok
test apply_verify::edit_parser::tests::test_diff_strips_ab_prefix ... ok
test apply_verify::edit_parser::tests::test_empty_input ... ok
test apply_verify::edit_parser::tests::test_full_file_empty_path ... ok
test apply_verify::edit_parser::tests::test_full_file_no_path ... ok
test apply_verify::edit_parser::tests::test_input_too_large ... ok
test apply_verify::edit_parser::tests::test_full_file_with_dash_header ... ok
test apply_verify::edit_parser::tests::test_full_file ... ok
test apply_verify::diff_applier::tests::test_apply_file_not_found ... ok
test apply_verify::edit_parser::tests::test_json_empty_edits ... ok
test apply_verify::edit_parser::tests::test_json_bare_array ... ok
test apply_verify::edit_parser::tests::test_json_malformed ... ok
test apply_verify::edit_parser::tests::test_json_agentic_output ... ok
test apply_verify::edit_parser::tests::test_json_control_chars ... ok
test apply_verify::edit_parser::tests::test_json_trailing_newlines_normalized ... ok
test apply_verify::diff_applier::tests::test_apply_old_text_not_found ... ok
test apply_verify::edit_parser::tests::test_json_with_message_field ... ok
test apply_verify::edit_parser::tests::test_malformed_diff ... ok
test apply_verify::edit_parser::tests::test_markdown_backticks_in_content ... ok
test apply_verify::edit_parser::tests::test_markdown_diff_block ... ok
test apply_verify::edit_parser::tests::test_markdown_json_block ... ok
test apply_verify::edit_parser::tests::test_whitespace_only_input ... ok
test apply_verify::edit_parser::tests::test_markdown_generic_block ... ok
test apply_verify::diff_applier::tests::test_apply_ambiguous_match ... ok
test apply_verify::diff_applier::tests::test_apply_unified_diff_multi_hunk_fails ... ok
test apply_verify::diff_applier::tests::test_apply_empty_old_in_find_replace_is_invalid ... ok
test apply_verify::diff_applier::tests::test_apply_json_single_file ... ok
test apply_verify::diff_applier::tests::test_apply_full_file_overwrite ... ok
test apply_verify::diff_applier::tests::test_apply_partial_failure ... ok
test apply_verify::diff_applier::tests::test_apply_unified_diff_single_hunk ... ok
test apply_verify::diff_applier::tests::test_apply_full_file_create_new ... ok
test apply_verify::diff_applier::tests::test_apply_multi_file_success ... ok
test apply_verify::rollback::tests::test_is_fully_restored_false ... ok
test apply_verify::rollback::tests::test_is_fully_restored_true ... ok
test apply_verify::retry_loop::tests::test_parse_error_stop ... ok
test apply_verify::retry_loop::tests::test_apply_partial_failure_rolls_back ... ok
test apply_verify::rollback::tests::test_rollback_delete_tolerates_already_missing ... ok
test apply_verify::rollback::tests::test_rollback_continues_on_failure ... ok
test apply_verify::rollback::tests::test_rollback_empty_result_is_noop ... ok
test apply_verify::rollback::tests::test_rollback_deletes_new_file ... ok
test apply_verify::rollback::tests::test_rollback_mixed_restore_and_delete ... ok
test apply_verify::rollback::tests::test_rollback_single_file ... ok
test apply_verify::rollback::tests::test_rollback_reverse_order ... ok
test aggregator::vote::tests::vote_snapshot ... ok
test aggregator::concat::tests::concat_mixed_success_failure_snapshot ... ok
test apply_verify::retry_loop::tests::test_apply_error_triggers_rollback_and_retry ... ok
test apply_verify::retry_loop::tests::test_parse_error_retries ... ok
test apply_verify::verification::tests::test_verify_captures_both_streams ... ok
test apply_verify::verification::tests::test_verify_captures_stderr ... ok
test apply_verify::verification::tests::test_verify_captures_stdout ... ok
test apply_verify::retry_loop::tests::test_parse_error_on_last_retry_exits ... ok
test apply_verify::retry_loop::tests::test_requester_error_surfaced ... ok
test apply_verify::retry_loop::tests::test_max_retries_zero_runs_once ... ok
test backend::claude::tests::capabilities_match_current_wiring ... ok
test backend::claude::tests::test_claude_response_deserialize_without_usage ... ok
test backend::claude::tests::test_claude_response_deserialize_with_usage ... ok
test backend::codex::tests::capabilities_match_current_wiring ... ok
test backend::gemini::tests::capabilities_match_current_wiring ... ok
test backend::genai_error::tests::classify_404_body_detects_unknown_function_fixture ... ok
test backend::genai_error::tests::classify_5xx_body_detects_anthropic_auth_fixture ... ok
test backend::genai_error::tests::classify_5xx_body_detects_rate_limit_signature ... ok
test backend::genai_error::tests::classify_5xx_body_returns_none_for_generic_5xx ... ok
test backend::genai_error::tests::contains_status_code_handles_punctuation_boundaries ... ok
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
test apply_verify::retry_loop::tests::test_success_first_attempt ... ok
test backend::genai_error::tests::map_status_unknown_to_execution_failed ... ok
test apply_verify::retry_loop::tests::test_verify_failure_triggers_rollback ... ok
test backend::ollama::tests::test_ollama_response_deserialize_partial_counts ... ok
test backend::ollama::tests::test_ollama_response_deserialize_with_counts ... ok
test backend::ollama::tests::test_ollama_response_deserialize_without_model ... ok
test backend::retry::tests::test_get_delay_attempt_zero_is_zero ... ok
test backend::retry::tests::test_get_delay_clamped_at_max ... ok
test backend::retry::tests::test_retry_executor_does_not_retry_non_retryable ... ok
test backend::retry::tests::test_get_delay_grows_exponentially ... ok
test apply_verify::verification::tests::test_verify_failure_exit_code ... ok
test backend::tensorzero::tests::canonicalize_wire_model_strips_to_canonical_on_wire ... ok
test backend::tensorzero::tests::capabilities_match_current_wiring ... ok
test apply_verify::retry_loop::tests::test_integration_end_to_end ... ok
test apply_verify::retry_loop::tests::test_max_retries_exhausted ... ok
test apply_verify::verification::tests::test_verify_invalid_command_exits_127 ... ok
test backend::tensorzero::tests::maps_429_to_rate_limit_retryable ... FAILED
test backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable ... FAILED
test backend::tensorzero::tests::maps_401_to_auth_not_retryable ... FAILED
test backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime ... FAILED
test backend::tensorzero::tests::maps_500_to_retryable_error ... FAILED
test backend::tensorzero::tests::maps_502_generic_to_network_retryable ... FAILED
test backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable ... FAILED
test backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable ... FAILED
test backend::tensorzero::tests::normalize_endpoint_appends_when_missing ... ok
test backend::tensorzero::tests::normalize_endpoint_does_not_double_suffix ... ok
test backend::tensorzero::tests::maps_malformed_json_to_parse_error ... FAILED
test backend::tensorzero::tests::maps_request_timeout_to_timeout_error ... FAILED
test backend::retry::tests::test_retry_exhausted ... ok
test backend::retry::tests::test_retry_success_after_failures ... ok
test backend::tensorzero::tests::returns_text_on_200_success ... FAILED
test backend::tests::backend_capabilities_none_is_all_false ... ok
test backend::tests::capabilities_for_name_matches_static_expectations ... ok
test backend::tests::capabilities_for_name_unknown_returns_none ... ok
test backend::tests::default_capabilities_are_none ... ok
test backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model ... FAILED
test backend::tests::tensorzero_adapter_allows_missing_api_key_env_field ... ok
test backend::tests::tensorzero_adapter_maps_endpoint_model_auth_timeout ... ok
test backend::tests::tensorzero_adapter_rejects_missing_endpoint_model_zero_timeout_and_bad_scheme ... ok
test backend::tests::test_backend_error_display ... ok
test backend::tests::test_backend_error_not_retryable ... ok
test backend::tests::test_backend_error_retryable ... ok
test backend::tests::test_query_output_from_process_empty_stderr_normalized ... ok
test backend::tests::test_query_output_from_process_empty_stdout ... ok
test backend::tests::tensorzero_create_backend_queries_wiremock_gateway ... FAILED
test backend::tests::test_backend_error_from_anyhow ... ok
test backend::tests::test_query_output_from_process_with_stderr ... ok
test backend::tests::test_query_output_from_process_populates_backend_and_duration ... ok
test apply_verify::verification::tests::test_verify_uses_passed_cwd ... ok
test backend::tests::test_query_output_from_text ... ok
test backend::tests::test_query_output_from_text_populates_backend_and_duration ... ok
test backend::tests::test_query_output_with_model_none ... ok
test backend::tests::test_query_output_with_model_some ... ok
test backend::tests::test_query_output_with_structured_none ... ok
test backend::tests::test_query_output_with_usage_some ... ok
test apply_verify::verification::tests::test_verify_success ... ok
test backend::tests::test_query_output_with_structured_some ... ok
test backend::tests::test_query_output_with_usage_none ... ok
test backend::tests::test_token_usage_default_zero ... ok
test backend::tests::test_token_usage_new_computes_total ... ok
test backend::tests::test_token_usage_new_saturates_on_overflow ... ok
test backend::tests::test_token_usage_saturating_add ... ok
test backend::tests::with_elapsed_is_idempotent_on_repeated_calls ... ok
test backend::tests::with_elapsed_is_noop_on_non_timeout_variants ... ok
test backend::tests::with_elapsed_overrides_timeout_elapsed_ms ... ok
test cache::tests::test_cache_disabled ... ok
test cache::tests::test_cache_key_different_backends ... ok
test cache::tests::test_cache_key_different_prompts ... ok
test cache::tests::test_cache_key_deterministic ... ok
test cache::tests::test_cache_warnings_on_parse_failure ... ok
test cache::tests::test_cache_warnings_deduplicated ... ok
test config::tests::test_command_wrapper_default_none ... ok
test config::tests::test_codex_backend_defaults ... ok
test config::tests::test_claude_backend_defaults ... ok
test config::tests::test_conductor_defaults ... ok
test apply_verify::verification::tests::test_verify_output_truncated ... ok
test config::tests::test_deep_merge_boolean_override ... ok
test config::tests::test_deep_merge_empty_overlay ... ok
test config::tests::test_deep_merge_hashmap_add ... ok
test config::tests::test_deep_merge_hashmap_override ... ok
test config::tests::test_conductor_custom_config ... ok
test config::tests::test_command_wrapper_docker_example ... ok
test config::tests::test_backend_config_defaults ... ok
test config::tests::test_command_wrapper_config ... ok
test config::tests::test_default_config ... ok
test config::tests::test_deep_merge_partial_config ... ok
test config::tests::test_deep_merge_scalar_override ... ok
test config::tests::test_config_serialization_roundtrip ... ok
test config::tests::test_gemini_backend_defaults ... ok
test config::tests::test_hunt_task_defaults ... ok
test config::tests::test_deny_unknown_fields ... ok
test config::tests::test_deep_merge_vec_replace ... ok
test apply_verify::retry_loop::tests::test_success_on_retry_after_verify_failure ... ok
test config::tests::test_parse_custom_backend ... ok
test config::tests::test_parse_minimal_config ... ok
test config::tests::test_parse_custom_task ... ok
test config::tests::test_tensorzero_invalid_url_fails ... ok
test config::tests::test_tensorzero_missing_endpoint_fails ... ok
test config::tests::test_tensorzero_to_backend_opts_resolves_env ... ok
test consensus::tests::test_majority_vote_clear_winner ... ok
test config::tests::test_load_config_from_paths_no_files ... ok
test config::tests::test_tensorzero_zero_timeout_fails ... ok
test consensus::tests::test_majority_vote_empty ... ok
test consensus::tests::test_majority_vote_tie_first_wins ... ok
test consensus::tests::test_weighted_vote ... ok
test consensus::tests::test_weighted_vote_clear_winner ... ok
test consensus::tests::test_whitespace_normalization ... ok
test config::tests::test_load_config_from_paths_explicit_bypasses ... ok
test family::tests::aggregator_rejected_display ... ok
test family::tests::as_str_openai ... ok
test family::tests::as_str_other ... ok
test family::tests::display_anthropic ... ok
test config::tests::test_load_config_from_paths_project_only ... ok
test family::tests::display_other ... ok
test family::tests::enforce_all_anthropic_rejected ... ok
test config::tests::test_tensorzero_config_serialization_roundtrip ... ok
test family::tests::enforce_distinct_other_ok ... ok
test family::tests::enforce_empty_slice_ok ... ok
test family::tests::enforce_mixed_families_ok ... ok
test family::tests::enforce_cross_family_deterministic ... ok
test family::tests::enforce_same_other_rejected ... ok
test family::tests::enforce_single_backend_ok ... ok
test family::tests::enforce_three_same_family ... ok
test family::tests::enforce_two_distinct_others_ok ... ok
test family::tests::family_of_bedrock ... ok
test family::tests::family_of_claude ... ok
test family::tests::family_of_codex ... ok
test family::tests::family_of_empty_string ... ok
test family::tests::family_of_gemini ... ok
test context::tests::test_no_context ... ok
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
test context::tests::test_detect_rails_with_goldiloader ... ok
test role::tests::test_resolution_builder ... ok
test role::tests::test_backend_filtering ... ok
test role::tests::test_resolution_is_empty ... ok
test role::tests::test_role_config_new ... ok
test role::tests::test_role_resolver_default_team ... ok
test role::tests::test_role_resolver_no_backends_available ... ok
test role::tests::test_role_config_serialization ... ok
test role::tests::test_role_resolver_resolve_global_role ... ok
test role::tests::test_role_resolver_role_not_found ... ok
test role::tests::test_role_resolver_team_can_define_custom_role ... ok
test role::tests::test_role_resolver_team_override ... ok
test role::tests::test_role_resolution_error_display ... ok
test role::tests::test_role_resolver_team_override_takes_precedence ... ok
test role::tests::test_routing_strategy_default_is_fallback ... ok
test role::tests::test_team_config_default ... ok
test role::tests::test_valid_parallel_config ... ok
test role::tests::test_validation_parallel_min_success_exceeds_backends ... ok
test role::tests::test_team_config_serialization ... ok
test role::tests::test_validation_parallel_min_success_too_low ... ok
test role::tests::test_validation_unknown_backend ... ok
test strategy::escalating_retry::tests::config_default_false ... ok
test strategy::escalating_retry::tests::config_round_trip_true ... ok
test strategy::escalating_retry::tests::config_round_trip_false ... ok
test git_agent::tests::test_is_initialized_false_for_nonexistent ... ok
test context::tests::test_detect_typescript ... ok
test config::tests::test_load_config_from_paths_user_parse_error ... ok
test apply_verify::retry_loop::tests::test_attempt_records ... ok
test config::tests::test_load_config_from_paths_three_layers ... ok
test git_agent::tests::test_is_available_returns_bool ... ok
test strategy::escalating_retry::tests::redaction_api_key_value ... ok
test strategy::escalating_retry::tests::envelope_backend_error_shows_null_response ... ok
test strategy::escalating_retry::tests::redaction_aws_key ... ok
test strategy::escalating_retry::tests::redaction_bearer_token ... ok
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
test strategy::parallel_fanout::tests::any_fail_markdown_fenced_json ... ok
test strategy::parallel_fanout::tests::any_fail_all_pass ... ok
test strategy::parallel_fanout::tests::any_fail_markdown_fenced_fail ... ok
test strategy::escalating_retry::tests::envelope_over_budget_truncates_excerpt ... ok
test strategy::parallel_fanout::tests::any_fail_valid_json_extra_keys ... ok
test strategy::parallel_fanout::tests::backend_not_found ... ok
test strategy::parallel_fanout::tests::empty_targets_yields_no_backends ... ok
test strategy::parallel_fanout::tests::any_fail_empty_query_text ... ok
test strategy::parallel_fanout::tests::floor_violation ... ok
test strategy::parallel_fanout::tests::any_fail_first_fails ... ok
test strategy::parallel_fanout::tests::any_fail_all_fail ... ok
test strategy::parallel_fanout::tests::any_fail_backend_error_treated_as_failure ... ok
test strategy::parallel_fanout::tests::happy_path_all_succeed ... ok
test strategy::parallel_fanout::tests::one_fails_floor_still_met ... ok
test strategy::parallel_fanout::tests::any_fail_missing_pass_field ... ok
test strategy::verify::run_command::tests::run_command_builder_api ... ok
test strategy::verify::run_command::tests::run_command_default_values ... ok
test strategy::parallel_fanout::tests::any_fail_non_deterministic_offender ... ok
test strategy::parallel_fanout::tests::prompt_render_failure_no_dispatch ... ok
test strategy::parallel_fanout::tests::vote_success ... ok
test strategy::parallel_fanout::tests::vote_quorum_lost ... ok
test strategy::verify::run_command::tests::verify_missing_command_fails ... ok
test strategy::verify::test_runner::tests::cargo_3_pass_0_fail ... ok
test strategy::parallel_fanout::tests::any_fail_wrong_pass_type ... ok
test strategy::verify::test_runner::tests::cargo_2_pass_1_fail ... ok
test strategy::verify::test_runner::tests::cargo_empty_no_tests ... ok
test strategy::verify::test_runner::tests::cargo_first_failure_preserves_stdout_excerpt ... ok
test strategy::verify::test_runner::tests::cargo_malformed_json_line_skipped ... ok
test strategy::verify::test_runner::tests::cargo_first_failure_truncates_utf8_excerpt_safely ... ok
test strategy::verify::test_runner::tests::cargo_skips_compiler_messages ... ok
test strategy::verify::test_runner::tests::pytest_4_pass_2_fail ... ok
test strategy::verify::test_runner::tests::pytest_5_pass_0_fail ... ok
test strategy::verify::test_runner::tests::pytest_empty_no_tests ... ok
test strategy::verify::test_runner::tests::pytest_missing_summary_field ... ok
test strategy::verify::test_runner::tests::pytest_non_json_output ... ok
test strategy::verify::test_runner::tests::verify_result_from_failing_tests ... ok
test strategy::verify::test_runner::tests::verify_result_from_passing_tests ... ok
test strategy::verify::test_runner::tests::verify_result_no_tests_ran ... ok
test strategy::verify::verify::tests::failure_reason_display ... ok
test strategy::verify::verify::tests::failure_reason_builder_api ... ok
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
test template::filters::tests::test_default_val_defined ... ok
test template::context::tests::test_workflow_backends_empty ... ok
test template::context::tests::test_workflow_backends ... ok
test template::filters::tests::test_default_val_empty_string ... ok
test template::filters::tests::test_default_val_undefined ... ok
test template::filters::tests::test_first_empty ... ok
test template::filters::tests::test_first_normal ... ok
test template::filters::tests::test_first_single ... ok
test template::filters::tests::test_join_default_separator ... ok
test template::filters::tests::test_join_empty ... ok
test template::filters::tests::test_join_with_separator ... ok
test template::filters::tests::test_json_encode_number ... ok
test strategy::parallel_fanout::tests::vote_tie_random_deterministic ... ok
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
test template::filters::tests::test_json_encode_nested ... ok
test template::filters::tests::test_trim_newlines ... ok
test template::filters::tests::test_trim_whitespace ... ok
test template::tests::test_combined_env_arg_step ... ok
test template::tests::test_eval_expression_falsy ... ok
test template::tests::test_eval_expression_truthy ... ok
test template::tests::test_eval_expression_undefined ... ok
test template::tests::test_no_reexpansion_of_braces_in_output ... ok
test utils::tests::test_backend_error_kind_from_typed ... ok
test template::tests::test_parse_error ... ok
test utils::tests::test_classify_auth_401 ... ok
test template::tests::test_undefined_variable ... ok
test utils::tests::test_classify_auth_invalid_key ... ok
test template::tests::test_render_mixed ... ok
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
test utils::tests::test_summarize_unknown_truncates ... ok
test utils::tests::test_redact_secrets_aws_key ... ok
test utils::tests::test_truncate_long_string ... ok
test utils::tests::test_truncate_short_string ... ok
test utils::tests::test_truncate_unicode ... ok
test utils::tests::test_redact_secrets_bearer_token ... ok
test utils::tests::test_truncate_utf8_ascii ... ok
test utils::tests::test_truncate_utf8_empty_string ... ok
test utils::tests::test_truncate_utf8_exact_boundary ... ok
test utils::tests::test_truncate_utf8_multibyte_boundary ... ok
test utils::tests::test_truncate_utf8_within_limit ... ok
test utils::tests::test_truncate_utf8_zero_cap ... ok
test workflow::tests::required_capabilities_returns_empty_for_plain_step ... ok
test workflow::tests::required_capabilities_returns_file_edit_for_apply_edits ... ok
test workflow::tests::test_apply_lenient_mode_non_empty_passes_with_cleaned_output ... ok
test workflow::tests::test_apply_lenient_mode_preserves_internal_whitespace ... ok
test utils::tests::test_redact_secrets_api_key_value ... ok
test workflow::tests::test_apply_lenient_mode_empty_response_fails ... ok
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
test strategy::parallel_fanout::tests::any_fail_mid_list_fails ... ok
test strategy::verify::run_command::tests::verify_echo_passes ... ok
test strategy::verify::run_command::tests::verify_false_fails_with_code ... ok
test workflow::tests::test_condition_unparseable_returns_true ... ok
test backend::ollama::tests::capabilities_match_current_wiring ... FAILED
test workflow::tests::test_condition_steps_success ... ok
test workflow::tests::test_duplicate_step_names_error ... ok
test workflow::tests::test_evaluate_condition_error_recovery ... ok
test workflow::tests::test_condition_legacy_syntax ... ok
test workflow::tests::test_extract_json_field_bool ... ok
test workflow::tests::test_extract_json_field_multiline ... ok
test workflow::tests::test_extract_json_field_not_found ... ok
test workflow::tests::test_extract_json_field_number ... ok
test workflow::tests::test_extract_json_field_string ... ok
test workflow::tests::test_condition_equals ... ok
test workflow::tests::test_extract_json_from_markdown_block ... ok
test workflow::tests::test_condition_contains ... ok
test workflow::tests::test_extract_json_from_plain_block ... ok
test backend::tensorzero::tests::name_is_tensorzero ... FAILED
test backend::tests::tensorzero_create_backend_supported_when_capability_supported ... FAILED
test workflow::tests::test_extract_json_raw ... ok
test workflow::tests::test_extract_json_with_text_before ... ok
test workflow::tests::test_extract_json_with_literal_newlines ... ok
test workflow::tests::test_find_closing_fence ... ok
test workflow::tests::test_continue_on_error_toml_parsing ... ok
test workflow::tests::test_heuristic_contains_double_quotes ... ok
test workflow::tests::test_heuristic_contains_empty_string_always_passes ... ok
test workflow::tests::test_condition_not ... ok
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
test workflow::tests::test_interpolate_loop_vars_index ... ok
test workflow::tests::test_interpolate_loop_vars_item_string ... ok
test workflow::tests::test_for_each_parsed_output_not_array ... ok
test workflow::tests::test_for_each_with_parsed_output ... ok
test workflow::tests::test_interpolate_validation_prompt_basic ... ok
test workflow::tests::test_interpolate_loop_vars_missing_field ... ok
test workflow::tests::test_interpolate_validation_prompt_injection_safety ... ok
test workflow::tests::test_interpolate_loop_vars_multiple_fields_one_missing ... ok
test workflow::tests::test_interpolate_loop_vars_combined ... ok
test workflow::tests::test_interpolate_validation_prompt_no_stderr ... ok
test workflow::tests::test_interpolate_validation_prompt_no_truncation_when_under_limit ... ok
test workflow::tests::test_interpolate_loop_vars_item_object ... ok
test workflow::tests::test_interpolate_validation_prompt_truncation ... ok
test workflow::tests::test_interpolate_loop_vars_item_whole_object ... ok
test workflow::tests::test_interpolate_validation_prompt_with_stderr ... ok
test workflow::tests::test_interpolate_parsed_output_none_fallback ... ok
test workflow::tests::test_load_error_tracker_backoff_progression ... ok
test workflow::tests::test_load_error_tracker_bail_at_threshold ... ok
test workflow::tests::test_interpolate_with_fields_json ... ok
test workflow::tests::test_jinja_chained_filters ... ok
test workflow::tests::test_load_error_tracker_reset_on_success ... ok
test workflow::tests::test_jinja_default_filter ... ok
test workflow::tests::test_load_error_tracker_success_with_no_prior_errors ... ok
test workflow::tests::test_jinja_if_block ... ok
test workflow::tests::test_map_retry_failure_apply_error_with_paths ... ok
test workflow::tests::test_map_retry_failure_apply_error_without_paths ... ok
test workflow::tests::test_jinja_trim_filter ... ok
test workflow::tests::test_jinja_inline_for_loop ... ok
test workflow::tests::test_map_retry_failure_attempt_count_from_retries ... ok
test workflow::tests::test_jinja_missing_step_default_fallback ... ok
test workflow::tests::test_jinja_join_filter ... ok
test workflow::tests::test_map_retry_failure_empty_attempts ... ok
test workflow::tests::test_jinja_shell_escape_filter ... ok
test workflow::tests::test_map_retry_failure_parse_error ... ok
test workflow::tests::test_map_retry_failure_verify_exit_code ... ok
test workflow::tests::test_map_retry_failure_verify_has_priority_over_apply ... ok
test workflow::tests::test_map_retry_failure_stderr_truncated_to_1kb ... ok
test workflow::tests::test_map_retry_failure_verify_timeout ... ok
test workflow::tests::test_map_template_error_reports_offending_variable_in_multi_expression ... ok
test workflow::tests::test_parse_for_each_inline_array ... ok
test workflow::tests::test_parse_for_each_inline_array_objects ... ok
test workflow::tests::test_output_format_toml_parsing ... ok
test workflow::tests::test_min_deps_success_without_depends_on_error ... ok
test workflow::tests::test_parse_step_output_json ... ok
test workflow::tests::test_parse_step_output_lines ... ok
test workflow::tests::test_parse_step_output_none ... ok
test workflow::tests::test_parse_step_output_text ... ok
test workflow::tests::test_apply_once_with_format_runs_after_apply ... ok
test workflow::tests::test_parse_for_each_invalid_format ... ok
test workflow::tests::test_parse_for_each_step_not_found ... ok
test workflow::tests::test_parse_for_each_not_array ... ok
test workflow::tests::test_parse_validation_response_empty_string_is_error ... ok
test workflow::tests::test_parse_for_each_step_reference ... ok
test workflow::tests::test_min_deps_success_validation_empty_deps ... ok
test workflow::tests::test_parse_validation_response_invalid_status ... ok
test workflow::tests::test_parse_for_each_step_reference_with_code_block ... ok
test workflow::tests::test_min_deps_success_validation_exceeds_deps ... ok
test workflow::tests::test_parse_validation_response_json_fail ... ok
test workflow::tests::test_parse_validation_response_json_in_fences ... ok
test workflow::tests::test_parse_validation_response_json_pass ... ok
test workflow::tests::test_min_deps_success_validation_valid ... ok
test workflow::tests::test_parse_validation_response_json_pass_no_output ... ok
test workflow::tests::test_parse_validation_response_review_failed ... ok
test workflow::tests::test_parse_validation_response_unrecognized_is_error ... ok
test workflow::tests::test_sanitize_json_strings ... ok
test workflow::tests::test_step_failure_kind_copy_eq ... ok
test workflow::tests::test_step_failure_kind_display ... ok
test workflow::tests::test_step_for_each_inline_array_toml ... ok
test workflow::tests::test_step_result_error_backend_error ... ok
test workflow::tests::test_step_for_each_toml_parsing ... ok
test workflow::tests::test_step_if_alias ... ok
test workflow::tests::test_step_result_error_edit_failed ... ok
test workflow::tests::test_step_result_error_has_no_validation ... ok
test workflow::tests::test_parse_validate_config_absent ... ok
test workflow::tests::test_step_result_error_output_matches_failure_message ... ok
test workflow::tests::test_step_result_error_produces_failure ... ok
test workflow::tests::test_parse_validate_config_from_toml ... ok
test workflow::tests::test_step_result_error_skipped ... ok
test workflow::tests::test_step_result_error_verify_failed ... ok
test workflow::tests::test_strip_markdown_fences_json ... ok
test workflow::tests::test_strip_markdown_fences_none ... ok
test workflow::tests::test_strip_markdown_fences_plain ... ok
test workflow::tests::test_strip_markdown_fences_with_whitespace ... ok
test workflow::tests::test_success_step_has_no_failure ... ok
test workflow::tests::test_translate_contains_with_escaped_quotes ... ok
test workflow::tests::test_translate_contains_call ... ok
test workflow::tests::test_parse_validate_config_mixed_fields ... ok
test workflow::tests::test_translate_equals_call ... ok
test workflow::tests::test_translate_legacy_steps_output_contains ... ok
test workflow::tests::test_translate_fast_path_whitespace_variants ... ok
test workflow::tests::test_translate_legacy_double_quotes ... ok
test workflow::tests::test_translate_contains_with_steps_prefix ... ok
test workflow::tests::test_translate_contains_with_single_quoted_literal_containing_double_quote ... ok
test workflow::tests::test_timeout_at_minimum_allowed ... ok
test workflow::tests::test_translate_equals_with_steps_prefix ... ok
test workflow::tests::test_translate_mixed_legacy_new ... ok
test workflow::tests::test_translate_passthrough_already_valid ... ok
test workflow::tests::test_translate_passthrough_empty ... ok
test workflow::tests::test_parse_for_each_field_access ... ok
test workflow::tests::test_translate_nested_not ... ok
test workflow::tests::test_truncate_for_prompt_over_limit ... ok
test workflow::tests::test_translate_multiple_contains ... ok
test workflow::tests::test_truncate_for_prompt_under_limit ... ok
test workflow::tests::test_timeout_too_small_validation ... ok
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
test workflow::tests::test_timeout_zero_allowed ... ok
test workflow::tests::test_timeout_normal_value_allowed ... ok
test workflow::tests::test_validate_config_defaults ... ok
test workflow::tests::test_validate_config_new_fields_default_to_none ... ok
test workflow::tests::test_validate_config_new_fields_parsing ... ok
test workflow::tests::test_validate_config_parses_on_parse_error_field ... ok
test workflow::tests::test_validate_config_parses_mode_lenient_field ... ok
test workflows::tests::test_embedded_workflows_parse ... ok
test backend::retry::tests::test_retry_executor_honors_rate_limit_retry_after ... ok
test apply_verify::verification::tests::test_verify_elapsed_ms_nonzero ... ok
test strategy::verify::run_command::tests::verify_sleeps_timeout ... ok
test apply_verify::verification::tests::test_verify_timeout_kills_process_group ... ok
test apply_verify::verification::tests::test_verify_timeout_real_elapsed ... ok

failures:

---- backend::tensorzero::tests::maps_429_to_rate_limit_retryable stdout ----

thread 'backend::tensorzero::tests::maps_429_to_rate_limit_retryable' (46486633) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable' (46486632) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_401_to_auth_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_401_to_auth_not_retryable' (46486580) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime stdout ----

thread 'backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime' (46486569) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- backend::tensorzero::tests::maps_500_to_retryable_error stdout ----

thread 'backend::tensorzero::tests::maps_500_to_retryable_error' (46486641) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_generic_to_network_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_generic_to_network_retryable' (46486642) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable' (46486643) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable' (46486644) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_malformed_json_to_parse_error stdout ----

thread 'backend::tensorzero::tests::maps_malformed_json_to_parse_error' (46486645) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_request_timeout_to_timeout_error stdout ----

thread 'backend::tensorzero::tests::maps_request_timeout_to_timeout_error' (46486646) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::returns_text_on_200_success stdout ----

thread 'backend::tensorzero::tests::returns_text_on_200_success' (46486650) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model stdout ----

thread 'backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model' (46486652) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tests::tensorzero_create_backend_queries_wiremock_gateway stdout ----

thread 'backend::tests::tensorzero_create_backend_queries_wiremock_gateway' (46486660) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::ollama::tests::capabilities_match_current_wiring stdout ----

thread 'backend::ollama::tests::capabilities_match_current_wiring' (46486500) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
Attempted to create a NULL object.

---- backend::tensorzero::tests::name_is_tensorzero stdout ----

thread 'backend::tensorzero::tests::name_is_tensorzero' (46486647) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
Attempted to create a NULL object.

---- backend::tests::tensorzero_create_backend_supported_when_capability_supported stdout ----

thread 'backend::tests::tensorzero_create_backend_supported_when_capability_supported' (46486661) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
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

test result: FAILED. 635 passed; 16 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s

error: test failed, to rerun pass `--lib`

codex
`cargo test` does not clear in this environment, and the failure is before the branch-specific gate: several existing backend tests cannot start their local mock servers here (`wiremock` bind `Operation not permitted`, plus `system-configuration` null-object panics on macOS). I’m still checking the changed module’s own coverage and the branch’s API surface so the report is actionable rather than just “environment blocked.”
exec
/bin/zsh -lc 'git log --oneline --reverse main..HEAD' in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
d10d83b feat(CLO-273): add test runner hook core and parser tests
5c6651b feat(CLO-273): stabilize TestRunner parser and exit handling
72b5c11 feat(CLO-273): harden clippy-safe test assertions

exec
/bin/zsh -lc 'rg -n "TestRunner|SandboxOpts|CommandRun|CapturedOutput|parse_cargo_output|parse_pytest_output|to_verify_result" src tests' in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
src/strategy/verify/test_runner.rs:25:pub enum TestRunnerKind {
src/strategy/verify/test_runner.rs:33:/// `TestRunner` API doesn't expose `RunCommand` internals directly.
src/strategy/verify/test_runner.rs:35:pub struct SandboxOpts {
src/strategy/verify/test_runner.rs:47:impl Default for SandboxOpts {
src/strategy/verify/test_runner.rs:60:/// Executes a test suite via the configured [`TestRunnerKind`], parses the
src/strategy/verify/test_runner.rs:67:pub struct TestRunner {
src/strategy/verify/test_runner.rs:69:    pub runner: TestRunnerKind,
src/strategy/verify/test_runner.rs:76:    pub sandbox: SandboxOpts,
src/strategy/verify/test_runner.rs:79:impl TestRunner {
src/strategy/verify/test_runner.rs:81:    pub fn new(runner: TestRunnerKind, cwd: impl Into<PathBuf>) -> Self {
src/strategy/verify/test_runner.rs:86:            sandbox: SandboxOpts::default(),
src/strategy/verify/test_runner.rs:97:    pub fn with_sandbox(mut self, opts: SandboxOpts) -> Self {
src/strategy/verify/test_runner.rs:106:            TestRunnerKind::Cargo => {
src/strategy/verify/test_runner.rs:115:            TestRunnerKind::Pytest => {
src/strategy/verify/test_runner.rs:148:    pub fn parse_cargo_output(stdout: &str) -> TestResult {
src/strategy/verify/test_runner.rs:220:    pub fn parse_pytest_output(stdout: &str) -> TestResult {
src/strategy/verify/test_runner.rs:299:            TestRunnerKind::Cargo => Self::parse_cargo_output(stdout),
src/strategy/verify/test_runner.rs:300:            TestRunnerKind::Pytest => Self::parse_pytest_output(stdout),
src/strategy/verify/test_runner.rs:304:    pub fn to_verify_result(
src/strategy/verify/test_runner.rs:305:        run_command_run: super::run_command::CommandRun,
src/strategy/verify/test_runner.rs:421:impl VerifyHook for TestRunner {
src/strategy/verify/test_runner.rs:424:            TestRunnerKind::Cargo => "TestRunner(cargo)",
src/strategy/verify/test_runner.rs:425:            TestRunnerKind::Pytest => "TestRunner(pytest)",
src/strategy/verify/test_runner.rs:433:        Ok(Self::to_verify_result(run, parsed))
src/strategy/verify/test_runner.rs:485:        TestRunner::parse_cargo_output(fixture)
src/strategy/verify/test_runner.rs:489:        TestRunner::parse_pytest_output(fixture)
src/strategy/verify/test_runner.rs:656:    fn fake_captured_output(data: &str) -> super::super::run_command::CapturedOutput {
src/strategy/verify/test_runner.rs:657:        super::super::run_command::CapturedOutput {
src/strategy/verify/test_runner.rs:664:    fn fake_command_run(stdout_data: &str) -> super::super::run_command::CommandRun {
src/strategy/verify/test_runner.rs:665:        super::super::run_command::CommandRun {
src/strategy/verify/test_runner.rs:680:        let result = TestRunner::parse_cargo_output(stdout);
src/strategy/verify/test_runner.rs:682:        let vr = TestRunner::to_verify_result(run, result);
src/strategy/verify/test_runner.rs:691:        let result = TestRunner::parse_cargo_output(stdout);
src/strategy/verify/test_runner.rs:693:        let vr = TestRunner::to_verify_result(run, result);
src/strategy/verify/test_runner.rs:708:        let result = TestRunner::parse_cargo_output(stdout);
src/strategy/verify/test_runner.rs:710:        let vr = TestRunner::to_verify_result(run, result);
src/strategy/verify/verify.rs:273:///   (e.g. `"TestRunner"`, `"LLMVerifier"`).
src/strategy/verify/mod.rs:10://! - [`TestRunner`] — parses structured test output (CLO-273).
src/strategy/verify/mod.rs:25:pub use test_runner::{SandboxOpts, TestResult, TestRunner, TestRunnerKind};
src/strategy/verify/run_command.rs:144:    pub async fn run(&self) -> Result<CommandRun, VerifyError> {
src/strategy/verify/run_command.rs:239:        Ok(CommandRun {
src/strategy/verify/run_command.rs:251:pub struct CommandRun {
src/strategy/verify/run_command.rs:254:    pub stdout: CapturedOutput,
src/strategy/verify/run_command.rs:255:    pub stderr: CapturedOutput,
src/strategy/verify/run_command.rs:261:pub struct CapturedOutput {
src/strategy/verify/run_command.rs:267:impl CapturedOutput {
src/strategy/verify/run_command.rs:294:) -> CapturedOutput {
src/strategy/verify/run_command.rs:326:    CapturedOutput {
tests/verify_test_runner.rs:1://! Integration tests for the `TestRunner` verify hook.
tests/verify_test_runner.rs:5://! through `TestRunner::to_verify_result` to exercise the
tests/verify_test_runner.rs:10:use loker::strategy::verify::run_command::{CapturedOutput, CommandRun};
tests/verify_test_runner.rs:11:use loker::strategy::verify::{TestRunner, VerifyResult};
tests/verify_test_runner.rs:36:fn fake_captured_output(data: &str) -> CapturedOutput {
tests/verify_test_runner.rs:37:    CapturedOutput {
tests/verify_test_runner.rs:58:fn fake_command_run_with_code(stdout_data: &str, exit_code: i32) -> CommandRun {
tests/verify_test_runner.rs:59:    CommandRun {
tests/verify_test_runner.rs:69:fn fake_command_run(stdout_data: &str) -> CommandRun {
tests/verify_test_runner.rs:77:    let result = TestRunner::parse_cargo_output(&stdout);
tests/verify_test_runner.rs:79:    TestRunner::to_verify_result(run, result)
tests/verify_test_runner.rs:85:    let result = TestRunner::parse_pytest_output(&stdout);
tests/verify_test_runner.rs:87:    TestRunner::to_verify_result(run, result)
tests/verify_test_runner.rs:147:    let result = TestRunner::parse_cargo_output(&stdout);
tests/verify_test_runner.rs:201:    let result = TestRunner::parse_pytest_output(stdout);
tests/verify_test_runner.rs:210:    let result = TestRunner::parse_pytest_output(stdout);
tests/verify_test_runner.rs:215:    let run = CommandRun {
tests/verify_test_runner.rs:223:    let vr = TestRunner::to_verify_result(run, result);
tests/verify_test_runner.rs:241:    let result = TestRunner::parse_cargo_output(stdout);
tests/verify_test_runner.rs:243:    let vr = TestRunner::to_verify_result(run, result);

exec
/bin/zsh -lc 'cargo test --test verify_test_runner' in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 118ms:
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.10s
     Running tests/verify_test_runner.rs (target/debug/deps/verify_test_runner-289b0cb1641f081f)

running 9 tests
test pytest_non_json_exit ... ok
test verify_result_non_zero_exit_with_passing_tests_fails ... ok
test cargo_empty_no_tests ... ok
test cargo_2_pass_1_fail ... ok
test pytest_noisy_multiline_output_is_parsed ... ok
test cargo_3_pass_0_fail ... ok
test cargo_malformed_json_lines ... ok
test pytest_5_pass_0_fail ... ok
test pytest_4_pass_2_fail ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s


exec
/bin/zsh -lc "rg -n \"with_extra_args|build_run_command|with_sandbox|TestRunner::new|name\\(\\)\" src/strategy/verify/test_runner.rs tests/verify_test_runner.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
src/strategy/verify/test_runner.rs:91:    pub fn with_extra_args(mut self, args: impl IntoIterator<Item: AsRef<str>>) -> Self {
src/strategy/verify/test_runner.rs:97:    pub fn with_sandbox(mut self, opts: SandboxOpts) -> Self {
src/strategy/verify/test_runner.rs:104:    fn build_run_command(&self) -> RunCommand {
src/strategy/verify/test_runner.rs:335:                    .with_sandbox_violation(crate::strategy::verify::SandboxViolation::Timeout),
src/strategy/verify/test_runner.rs:345:                    .with_sandbox_violation(crate::strategy::verify::SandboxViolation::Signal {
src/strategy/verify/test_runner.rs:361:                            .with_sandbox_violation(
src/strategy/verify/test_runner.rs:373:                        .with_sandbox_violation(
src/strategy/verify/test_runner.rs:430:        let rc = self.build_run_command();

exec
/bin/zsh -lc "nl -ba src/strategy/verify/run_command.rs | sed -n '140,275p'" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
exec
/bin/zsh -lc "nl -ba src/strategy/verify/test_runner.rs | sed -n '300,395p'" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
   140	            .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
   141	            .collect()
   142	    }
   143	
   144	    pub async fn run(&self) -> Result<CommandRun, VerifyError> {
   145	        let command_path = self.resolve_command()?;
   146	
   147	        let mut command = Command::new(&command_path);
   148	        command
   149	            .args(&self.args)
   150	            .env_clear()
   151	            .stdin(Stdio::null())
   152	            .stdout(Stdio::piped())
   153	            .stderr(Stdio::piped());
   154	
   155	        if let Some(cwd) = &self.cwd {
   156	            command.current_dir(cwd);
   157	        }
   158	
   159	        let env_vars = self.build_environment();
   160	        let secret_values: Vec<String> = env_vars
   161	            .iter()
   162	            .filter(|(name, _)| is_secret_like_env_key(name))
   163	            .map(|(_, value)| value.clone())
   164	            .collect();
   165	        for (key, value) in env_vars {
   166	            command.env(key, value);
   167	        }
   168	
   169	        // Put each child in its own process group, so SIGKILL on timeout
   170	        // reaps descendants as well.
   171	        #[cfg(unix)]
   172	        command.process_group(0);
   173	        command.kill_on_drop(true);
   174	
   175	        #[cfg(unix)]
   176	        if let Some(cpu_timeout) = self.cpu_timeout {
   177	            let secs = cpu_timeout.as_secs().max(1);
   178	            // SAFETY: this closure runs in the child before exec and only
   179	            // mutates process-local resource limits.
   180	            unsafe {
   181	                command.pre_exec(move || {
   182	                    let limit = libc::rlimit {
   183	                        rlim_cur: secs as libc::rlim_t,
   184	                        rlim_max: secs as libc::rlim_t,
   185	                    };
   186	                    let rc = libc::setrlimit(libc::RLIMIT_CPU, &limit);
   187	                    if rc != 0 {
   188	                        return Err(std::io::Error::last_os_error());
   189	                    }
   190	                    Ok(())
   191	                });
   192	            }
   193	        }
   194	
   195	        let start = Instant::now();
   196	        let mut child = command.spawn().map_err(|err| {
   197	            VerifyError::new(format!("failed to spawn command '{}': {err}", self.cmd))
   198	        })?;
   199	
   200	        let stdout_stream = child
   201	            .stdout
   202	            .take()
   203	            .ok_or_else(|| VerifyError::new("failed to capture child stdout pipe".to_string()))?;
   204	        let stderr_stream = child
   205	            .stderr
   206	            .take()
   207	            .ok_or_else(|| VerifyError::new("failed to capture child stderr pipe".to_string()))?;
   208	
   209	        let stdout_handle = tokio::spawn(read_stream_bounded(stdout_stream, self.stdout_cap));
   210	        let stderr_handle = tokio::spawn(read_stream_bounded(stderr_stream, self.stderr_cap));
   211	
   212	        let (status, timed_out) = match timeout(self.wall_timeout, child.wait()).await {
   213	            Ok(result) => {
   214	                let status = result.map_err(|err| {
   215	                    VerifyError::new(format!("failed to wait for command '{}': {err}", self.cmd))
   216	                })?;
   217	                (status, false)
   218	            }
   219	            Err(_) => {
   220	                kill_process_group(child.id());
   221	                let _ = child.kill().await;
   222	                let status = child.wait().await.map_err(|err| {
   223	                    VerifyError::new(format!(
   224	                        "failed to reap timed-out command '{}': {err}",
   225	                        self.cmd
   226	                    ))
   227	                })?;
   228	                (status, true)
   229	            }
   230	        };
   231	
   232	        let stdout = stdout_handle
   233	            .await
   234	            .map_err(|_| VerifyError::new("stdout reader task failed"))?;
   235	        let stderr = stderr_handle
   236	            .await
   237	            .map_err(|_| VerifyError::new("stderr reader task failed"))?;
   238	
   239	        Ok(CommandRun {
   240	            status,
   241	            timed_out,
   242	            stdout,
   243	            stderr,
   244	            secret_values,
   245	            elapsed_ms: start.elapsed().as_millis() as u64,
   246	        })
   247	    }
   248	}
   249	
   250	#[derive(Debug)]
   251	pub struct CommandRun {
   252	    pub status: std::process::ExitStatus,
   253	    pub timed_out: bool,
   254	    pub stdout: CapturedOutput,
   255	    pub stderr: CapturedOutput,
   256	    pub secret_values: Vec<String>,
   257	    pub elapsed_ms: u64,
   258	}
   259	
   260	#[derive(Debug)]
   261	pub struct CapturedOutput {
   262	    pub data: Vec<u8>,
   263	    pub truncated: bool,
   264	    pub elided_bytes: usize,
   265	}
   266	
   267	impl CapturedOutput {
   268	    pub fn to_reason_text(&self) -> String {
   269	        let mut text = String::from_utf8_lossy(&self.data).into_owned();
   270	        if self.truncated {
   271	            text.push_str(&format!(
   272	                " …[truncated, {} bytes elided]",
   273	                self.elided_bytes
   274	            ));
   275	        }

 succeeded in 0ms:
   300	            TestRunnerKind::Pytest => Self::parse_pytest_output(stdout),
   301	        }
   302	    }
   303	
   304	    pub fn to_verify_result(
   305	        run_command_run: super::run_command::CommandRun,
   306	        result: TestResult,
   307	    ) -> VerifyResult {
   308	        let runner_stdout = run_command_run.stdout.to_reason_text();
   309	        let runner_stderr = run_command_run.stderr.to_reason_text();
   310	        let truncated = run_command_run.stdout.truncated || run_command_run.stderr.truncated;
   311	        let exit_code = run_command_run.status.code();
   312	        let signal = run_command_run
   313	            .status
   314	            .code()
   315	            .is_none()
   316	            .then(|| {
   317	                #[cfg(unix)]
   318	                {
   319	                    std::os::unix::process::ExitStatusExt::signal(&run_command_run.status)
   320	                }
   321	                #[cfg(not(unix))]
   322	                {
   323	                    None::<i32>
   324	                }
   325	            })
   326	            .flatten();
   327	
   328	        // Check for sandbox violations first
   329	        if run_command_run.timed_out {
   330	            return VerifyResult::Fail {
   331	                reason: FailureReason::new("test runner timed out")
   332	                    .with_stdout(runner_stdout)
   333	                    .with_stderr(runner_stderr)
   334	                    .with_truncated(truncated)
   335	                    .with_sandbox_violation(crate::strategy::verify::SandboxViolation::Timeout),
   336	            };
   337	        }
   338	
   339	        if let Some(sig) = signal {
   340	            return VerifyResult::Fail {
   341	                reason: FailureReason::new(format!("test runner killed by signal {sig}"))
   342	                    .with_stdout(runner_stdout)
   343	                    .with_stderr(runner_stderr)
   344	                    .with_truncated(truncated)
   345	                    .with_sandbox_violation(crate::strategy::verify::SandboxViolation::Signal {
   346	                        signal: sig,
   347	                    }),
   348	            };
   349	        }
   350	
   351	        // Parse test output
   352	        if let Some(code) = exit_code {
   353	            if code != 0 && result.failed == 0 {
   354	                if result.passed == 0 {
   355	                    return VerifyResult::Fail {
   356	                        reason: FailureReason::new("no tests ran")
   357	                            .with_stdout(runner_stdout)
   358	                            .with_stderr(runner_stderr)
   359	                            .with_truncated(truncated)
   360	                            .with_exit_code(code)
   361	                            .with_sandbox_violation(
   362	                                crate::strategy::verify::SandboxViolation::NonZeroExit { code },
   363	                            ),
   364	                    };
   365	                }
   366	
   367	                return VerifyResult::Fail {
   368	                    reason: FailureReason::new(format!("test runner exited with status {code}"))
   369	                        .with_stdout(runner_stdout)
   370	                        .with_stderr(runner_stderr)
   371	                        .with_truncated(truncated)
   372	                        .with_exit_code(code)
   373	                        .with_sandbox_violation(
   374	                            crate::strategy::verify::SandboxViolation::NonZeroExit { code },
   375	                        ),
   376	                };
   377	            }
   378	        }
   379	
   380	        if result.passed == 0 && result.failed == 0 {
   381	            return VerifyResult::Fail {
   382	                reason: FailureReason::new("no tests ran")
   383	                    .with_stdout(runner_stdout)
   384	                    .with_stderr(runner_stderr)
   385	                    .with_truncated(truncated)
   386	                    .with_exit_code(exit_code.unwrap_or(1)),
   387	            };
   388	        }
   389	
   390	        if result.failed > 0 {
   391	            let summary = match (&result.first_failure_name, &result.first_failure_excerpt) {
   392	                (Some(name), Some(excerpt)) => {
   393	                    format!(
   394	                        "{} test(s) failed (first: {name}: {excerpt})",
   395	                        result.failed

exec
/bin/zsh -lc 'rg -n "DEFAULT_OUTPUT_CAP|DEFAULT_WALL_TIMEOUT" src/strategy/verify/run_command.rs' in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
24:const DEFAULT_WALL_TIMEOUT: Duration = Duration::from_secs(30);
27:const DEFAULT_OUTPUT_CAP: usize = 4096;
58:            wall_timeout: DEFAULT_WALL_TIMEOUT,
60:            stdout_cap: DEFAULT_OUTPUT_CAP,
61:            stderr_cap: DEFAULT_OUTPUT_CAP,
488:        assert_eq!(rc.wall_timeout, DEFAULT_WALL_TIMEOUT);
490:        assert_eq!(rc.stdout_cap, DEFAULT_OUTPUT_CAP);
491:        assert_eq!(rc.stderr_cap, DEFAULT_OUTPUT_CAP);

exec
/bin/zsh -lc "nl -ba src/strategy/verify/test_runner.rs | sed -n '25,60p'" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
    25	pub enum TestRunnerKind {
    26	    Cargo,
    27	    Pytest,
    28	}
    29	
    30	/// Sandboxing configuration for test runner execution.
    31	///
    32	/// Maps to [`RunCommand`] fields but is kept as a separate struct so the
    33	/// `TestRunner` API doesn't expose `RunCommand` internals directly.
    34	#[derive(Debug, Clone)]
    35	pub struct SandboxOpts {
    36	    /// Environment variable names allowed in child process.
    37	    /// Default: empty (default‑deny).
    38	    pub env_allowlist: Vec<String>,
    39	    /// Wall‑clock timeout before process‑group SIGKILL.
    40	    pub wall_timeout: Duration,
    41	    /// Max bytes captured from stdout.
    42	    pub stdout_cap: usize,
    43	    /// Max bytes captured from stderr.
    44	    pub stderr_cap: usize,
    45	}
    46	
    47	impl Default for SandboxOpts {
    48	    fn default() -> Self {
    49	        Self {
    50	            env_allowlist: Vec::new(),
    51	            wall_timeout: Duration::from_secs(120),
    52	            stdout_cap: 8192,
    53	            stderr_cap: 8192,
    54	        }
    55	    }
    56	}
    57	
    58	/// Test runner verify hook.
    59	///
    60	/// Executes a test suite via the configured [`TestRunnerKind`], parses the

exec
/bin/zsh -lc "nl -ba docs/designs/clo-273-test-runner.md | sed -n '12,40p'" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
    12	
    13	`VerifyHook` is now available (`CLO-270`) and `RunCommand` already exists (`CLO-271`) as a reusable process-based verifier, but there is still no dedicated test-hook implementation. Without a test runner verifier, strategy retries are gated only by LLM-style checks and cannot reliably depend on deterministic pass/fail counts from project tests. This leaves the phase runner path (`T-029`) without a full production-quality test verification primitive.
    14	
    15	CLO-273 requires a hook that can execute project test commands and convert structured test output into a binary gate signal (`Pass`/`Fail`) with enough context for downstream feedback. The issue explicitly requires cargo and pytest support with fixture-based parser contracts, so the implementation should be mostly deterministic and parser-first.
    16	
    17	## 2. Goals & Non-goals
    18	
    19	### Goals
    20	
    21	1. **Implement `TestRunner` verify hook** in `src/strategy/verify/test_runner.rs` with `TestRunnerKind` enum (`Cargo`, `Pytest`) and builder-style configuration (`runner`, `cwd`, `extra_args`, `sandbox`).
    22	2. **Execute tests via `RunCommand` internals**, reusing existing `cwd`, env allowlist, output caps, timeouts, signal handling, and `FailureReason` mapping.
    23	3. **Parse structured output** into `TestResult` for:
    24	   - cargo JSON-lines (`type: "test"`, `event: "ok"/"failed"`, etc.),
    25	   - pytest JSON-report summary (`summary.passed`, `summary.failed`).
    26	4. **Map outcomes to verify results**:
    27	   - `failed > 0` ⇒ `Fail` with first-failure name/excerpt,
    28	   - `failed == 0 && passed > 0` ⇒ `Pass`,
    29	   - `passed == 0 && failed == 0` ⇒ `Fail { summary: "no tests ran" }`.
    30	5. **Keep parsing robust** by skipping malformed cargo lines and handling pytest missing/unparseable output without crashing the hook.
    31	6. **Add/retain fixture-driven parser tests** in `tests/verify_test_runner.rs` for the contract in ACs.
    32	
    33	### Non-goals
    34	
    35	- No new backend/protocol support beyond `cargo` and `pytest` in v0.
    36	- No historical trend tracking; gate only current run.
    37	- No attempt to implement external language-specific reporters outside pytest JSON output.
    38	- No phase-runner wiring changes in this task (that is handled in `T-029`).
    39	
    40	## 3. Architecture

exec
/bin/zsh -lc "nl -ba docs/plans/clo-273-test-runner.md | sed -n '24,55p'" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
    24	**Acceptance:** `cargo test --test verify_test_runner cargo_2_pass_1_fail cargo_empty_no_tests pytest_4_pass_2_fail -- --exact`
    25	**Estimate:** M
    26	
    27	Harden and finalize parsing semantics:
    28	- cargo JSON-lines: count `type == "test"`, handle `ignored == true` as not passed, capture first failure name/excerpt, skip malformed lines,
    29	- pytest JSON-report: extract `summary.passed/failed`, first failed `nodeid` + `longrepr`, parse/line-fallback on malformed stream,
    30	- keep all parser failures soft (return `0/0` with diagnostics).
    31	
    32	### ST3 Complete `to_verify_result()` mapping and sandbox violation paths
    33	**Files:** `src/strategy/verify/test_runner.rs`
    34	**Acceptance:** `cargo test --test verify_test_runner verify_result_no_tests_ran -- --exact`
    35	**Estimate:** M
    36	
    37	Map command execution outcomes + parsed counts into `VerifyResult`:
    38	- timeout → `Fail` with sandbox violation timeout,
    39	- signal/non-zero status → `Fail` with structured reason and signal/non-zero context,
    40	- `failed > 0` → first-failure summary + stdout/stderr passthrough,
    41	- `passed > 0 && failed == 0` → `Pass`,
    42	- `passed == 0 && failed == 0` → `Fail { reason: "no tests ran" }`.
    43	
    44	### ST4 Finish `VerifyHook` implementation and verify integration path
    45	**Files:** `src/strategy/verify/test_runner.rs`, `src/strategy/verify/mod.rs`
    46	**Acceptance:** `cargo test --test verify_test_runner pytest_non_json_exit -- --exact`
    47	**Estimate:** M
    48	
    49	Implement/retain `VerifyHook` execution path using `RunCommand::run()`:
    50	- execute via resolved cargo/pytest command,
    51	- pass parsed output through conversion path,
    52	- verify `mod.rs` exports include `TestRunner`, `TestRunnerKind`, `SandboxOpts`, `TestResult`.
    53	
    54	### ST5 Sanity gate for changed modules
    55	**Files:** `src/strategy/verify/test_runner.rs`, `src/strategy/verify/mod.rs`, `tests/verify_test_runner.rs`

exec
/bin/zsh -lc "nl -ba src/strategy/verify/run_command.rs | sed -n '20,30p'" in /Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner
 succeeded in 0ms:
    20	};
    21	use crate::utils::redact_secrets;
    22	
    23	/// Default wall-clock timeout for compile/test gates.
    24	const DEFAULT_WALL_TIMEOUT: Duration = Duration::from_secs(30);
    25	
    26	/// Default byte cap for stdout and stderr capture.
    27	const DEFAULT_OUTPUT_CAP: usize = 4096;
    28	
    29	/// Shell-command verify hook.
    30	#[derive(Debug, Clone)]

  ✓ Read the CLO-273 design, plan, and workflow context to extract acceptance criteria and planned tasks
  ✓ Inspect the branch diff for correctness, scope, public surface, error handling, and test coverage
  ✓ Run the required validation commands through make check and capture the first failing gate, if any
  → Produce the pre-PR validation report with findings and binding verdict
codex
# Codex pre-PR validation - CLO-273

## Context
- Branch: `feat/clo-273-test-runner`
- Plan / Spec: [docs/plans/clo-273-test-runner.md](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/docs/plans/clo-273-test-runner.md:1)
- Design: [docs/designs/clo-273-test-runner.md](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/docs/designs/clo-273-test-runner.md:1)

## Checklist
- [x] `cargo fmt --check`
- [x] `cargo clippy --all-targets --all-features -- -D warnings`
- [x] `cargo clippy --tests`
- [ ] `cargo test` (635 passed, 16 failed in this sandbox; `wiremock` bind is denied and `system-configuration` panics)
- [ ] `make check` green
- [x] All ACs covered
- [ ] No unintended public surface
- [ ] Error handling
- [ ] Tests
- [ ] Schema / docs

## Findings
### F1 [major] TestRunner no longer reuses RunCommand sandbox defaults
**Where:** [src/strategy/verify/test_runner.rs](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/src/strategy/verify/test_runner.rs:47) and [src/strategy/verify/run_command.rs](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/src/strategy/verify/run_command.rs:24)

**What:** The design explicitly says CLO-273 should reuse `RunCommand` timeout and output-cap behavior, but `SandboxOpts::default()` hardcodes `120s` and `8192` bytes instead of inheriting `RunCommand`’s existing `30s` / `4096` defaults. That silently changes retry-gate runtime behavior and makes `TestRunner` less consistent with the verifier it is supposed to wrap.

**Suggested fix:** Derive these defaults from shared `RunCommand` constants/helpers, or make the divergence explicit in the design and tests if it is intentional.

### F2 [major] Non-zero test exits lose `NonZeroExit` context on the common failure path
**Where:** [src/strategy/verify/test_runner.rs](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/src/strategy/verify/test_runner.rs:352)

**What:** `to_verify_result()` only attaches `SandboxViolation::NonZeroExit` when `code != 0 && result.failed == 0`. For normal failing test runs, where the runner exits non-zero and `result.failed > 0`, the returned `FailureReason` carries the exit code but drops the structured non-zero-exit context that ST3 called for. That makes downstream handling less consistent and leaves the most common cargo/pytest failure mode under-specified.

**Suggested fix:** Preserve `SandboxViolation::NonZeroExit { code }` for the `result.failed > 0` branch as well, and add a focused test for a parsed failure with non-zero status.

### F3 [major] RunCommand internals were widened into the public API just to support tests
**Where:** [src/strategy/verify/run_command.rs](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/src/strategy/verify/run_command.rs:144), [src/strategy/verify/run_command.rs](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/src/strategy/verify/run_command.rs:251), [src/strategy/verify/run_command.rs](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/src/strategy/verify/run_command.rs:261)

**What:** `RunCommand::run()`, `CommandRun`, and `CapturedOutput` were made fully `pub`, and the new integration test consumes them directly. That is broader than the CLO-273 design, leaks execution internals through `loker::strategy::verify::run_command::*`, and introduces undocumented public surface area that this task did not need to commit to.

**Suggested fix:** Reduce visibility to `pub(crate)`/`pub(super)` and keep the tests inside the module or behind crate-private helpers.

## Verdict
rework

I cannot sign off on this branch as PR-ready. The required build gate is not clean in this environment, so `cargo test` and therefore `make check` did not clear, and independent of that there are branch-specific issues in sandbox default reuse, non-zero-exit failure mapping, and unintended public API expansion that should be fixed before pushing.
tokens used
85,518
# Codex pre-PR validation - CLO-273

## Context
- Branch: `feat/clo-273-test-runner`
- Plan / Spec: [docs/plans/clo-273-test-runner.md](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/docs/plans/clo-273-test-runner.md:1)
- Design: [docs/designs/clo-273-test-runner.md](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/docs/designs/clo-273-test-runner.md:1)

## Checklist
- [x] `cargo fmt --check`
- [x] `cargo clippy --all-targets --all-features -- -D warnings`
- [x] `cargo clippy --tests`
- [ ] `cargo test` (635 passed, 16 failed in this sandbox; `wiremock` bind is denied and `system-configuration` panics)
- [ ] `make check` green
- [x] All ACs covered
- [ ] No unintended public surface
- [ ] Error handling
- [ ] Tests
- [ ] Schema / docs

## Findings
### F1 [major] TestRunner no longer reuses RunCommand sandbox defaults
**Where:** [src/strategy/verify/test_runner.rs](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/src/strategy/verify/test_runner.rs:47) and [src/strategy/verify/run_command.rs](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/src/strategy/verify/run_command.rs:24)

**What:** The design explicitly says CLO-273 should reuse `RunCommand` timeout and output-cap behavior, but `SandboxOpts::default()` hardcodes `120s` and `8192` bytes instead of inheriting `RunCommand`’s existing `30s` / `4096` defaults. That silently changes retry-gate runtime behavior and makes `TestRunner` less consistent with the verifier it is supposed to wrap.

**Suggested fix:** Derive these defaults from shared `RunCommand` constants/helpers, or make the divergence explicit in the design and tests if it is intentional.

### F2 [major] Non-zero test exits lose `NonZeroExit` context on the common failure path
**Where:** [src/strategy/verify/test_runner.rs](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/src/strategy/verify/test_runner.rs:352)

**What:** `to_verify_result()` only attaches `SandboxViolation::NonZeroExit` when `code != 0 && result.failed == 0`. For normal failing test runs, where the runner exits non-zero and `result.failed > 0`, the returned `FailureReason` carries the exit code but drops the structured non-zero-exit context that ST3 called for. That makes downstream handling less consistent and leaves the most common cargo/pytest failure mode under-specified.

**Suggested fix:** Preserve `SandboxViolation::NonZeroExit { code }` for the `result.failed > 0` branch as well, and add a focused test for a parsed failure with non-zero status.

### F3 [major] RunCommand internals were widened into the public API just to support tests
**Where:** [src/strategy/verify/run_command.rs](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/src/strategy/verify/run_command.rs:144), [src/strategy/verify/run_command.rs](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/src/strategy/verify/run_command.rs:251), [src/strategy/verify/run_command.rs](/Users/mk/Code/orchestrator/loker--feat-clo-273-test-runner/src/strategy/verify/run_command.rs:261)

**What:** `RunCommand::run()`, `CommandRun`, and `CapturedOutput` were made fully `pub`, and the new integration test consumes them directly. That is broader than the CLO-273 design, leaks execution internals through `loker::strategy::verify::run_command::*`, and introduces undocumented public surface area that this task did not need to commit to.

**Suggested fix:** Reduce visibility to `pub(crate)`/`pub(super)` and keep the tests inside the module or behind crate-private helpers.

## Verdict
rework

I cannot sign off on this branch as PR-ready. The required build gate is not clean in this environment, so `cargo test` and therefore `make check` did not clear, and independent of that there are branch-specific issues in sandbox default reuse, non-zero-exit failure mapping, and unintended public API expansion that should be fixed before pushing.
