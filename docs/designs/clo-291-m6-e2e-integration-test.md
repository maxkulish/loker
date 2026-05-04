# CLO-291 Design: M6 End-to-End Integration Test on Calculator Spec

## 1. Problem

Loker's three primitives (HTTP-gateway backend via TensorZero, named execution
strategies, verify hooks) are tested individually but never composed into a
full end-to-end pipeline. The M6 exit gate requires a single integration test
that runs the four-phase `design-doc-tdd` workflow against the calculator spec
(`examples/specs/calculator.md`) end-to-end and asserts the expected artefacts,
trace shape, and exit code. Without this test, there is no proof the
primitives compose correctly for v0 readiness.

## 2. Goals / Non-goals

### Goals

- A **mocked-mode** integration test (`tests/m6_design_doc_tdd_e2e.rs`) that
  runs inside `make check` with a wall-time budget under 5 seconds.
- Asserts: run-dir layout, manifest sha256 integrity, trace.jsonl schema
  validity, per-phase artefact presence/shape, summary.json, and resume
  idempotency.
- A **live-mode** path gated by `LOKER_M6_INTEGRATION=1` that routes through
  a real TensorZero gateway, with structural (not byte-equality) assertions.
- All assertions reference canonical paths from `RunDir` accessors (no
  hardcoded `runs/<id>/` strings outside fixture constants).

### Non-goals

- HITL phases — UC-1 is fully automated.
- CLI surface (`loker run` flags beyond `--spec`) — T-040/Phase 9.
- Resume from arbitrary crash points — T-031.
- Cost-budget enforcement — T-032 wires `summary.json`; this test only
  checks the file exists.
- Vote aggregator testing — T-019 was demoted to post-v0 per roadmap.

## 3. Architecture

### Test structure (`tests/m6_design_doc_tdd_e2e.rs`)

```
tests/
├── m6_design_doc_tdd_e2e.rs    # main test file
└── fixtures/
    └── m6/
        ├── design.md            # expected design output (fixture)
        ├── review.md            # expected review output (fixture)
        ├── implement/
        │   └── calculator.rs   # expected implementation output (fixture)
        ├── verify.json          # expected verify output (fixture)
        ├── design-doc-tdd.yaml  # workflow config for mock backends
        └── template_context.json # template context for {{ spec }}, etc.
```

### Mock backend architecture

The test uses in-memory `MockBackend` instances (following the pattern from
`tests/phase_runner_integration.rs`) that implement the `Backend` trait:

```rust
struct M6MockBackend {
    name: &'static str,
    output: &'static str,         // deterministic response
    fail: bool,                   // if true, returns BackendError
}
```

Each phase gets its own mock backend with pre-determined output that matches
the expected fixture shape:

| Phase | Backend name | Output content | Expected artefact |
|-------|-------------|----------------|-------------------|
| design | `m6-design` | Calculator design doc text | `design.md` |
| review | `m6-review-a`, `m6-review-b` | Reviewer markdown with headings | `review.md` (concat) |
| implement | `m6-implement` | Rust code for calculator lib | `changes/` |
| verify | `m6-verify` | `{"verdict": "pass"}` | `verify.json` |

### Conductor-level orchestration

Rather than calling `PhaseRunner` for each phase individually (which would
duplicate the conductor logic), the test calls the workflow conductor at a
level that:
1. Creates a `RunDir`
2. Loads and parses the `design-doc-tdd` workflow TOML
3. Renders each phase's template with the calculator spec
4. Executes each phase sequentially through `PhaseRunner` with mock backends
5. Asserts the combined output after each phase

The key abstraction is a `test_conductor` helper (or direct code in the test)
that:

```rust
fn run_mocked_workflow(
    spec: &Path,
    workflow_toml: &str,
    mock_backends: &[(&str, &str)],  // (name, output) pairs
    verify_result: VerifyResult,
) -> Result<RunDir, ...>
```

### Template rendering

Template substitution (CLO-289) is integrated: the spec file content and
upstream phase outputs are injected via `Template::render()` before calling
`PhaseRunner`. The test asserts that `{{ spec }}`, `{{ phase:design.output }}`,
etc. all resolve correctly.

### Trace recording

Each phase run is recorded into `trace.jsonl` via the `TraceWriter` (CLO-293).
The test asserts:
- One `gen_ai.invoke_agent` span per backend call
- Start/end timestamps are present and monotonic
- Semantic attributes (`gen_ai.system`, `gen_ai.request.model`, etc.) match
  the mock backend configuration

### Summary emission

After all four phases complete, `SummaryWriter::finalize` (CLO-296) aggregates
trace data and writes `summary.json`. The test asserts the file exists with
valid JSON containing per-backend token counts.

### Live mode

When `LOKER_M6_INTEGRATION=1` is set, the test:
1. Creates a TensorZero gateway config pointing at the local gateway
2. Uses a workflow TOML with `tensorzero/` backends (not mock backends)
3. Runs the same four-phase pipeline
4. Asserts structural properties (files exist, JSON parses, manifest entries
   present) but not byte-equality on artefact bodies

This is additive code in a `#[cfg(feature = ...)]` or
`#[cfg(not(any()))]` + env gate, not a separate test module.

## 4. Public API surface

### New test function signatures

```rust
// tests/m6_design_doc_tdd_e2e.rs

/// ST1: wiring smoke test — full pipeline exits 0 with mocked backends.
#[tokio::test]
async fn m6_smoke_test_mocked_pipeline_exits_zero() -> Result<()>;

/// ST2: Layout assertions — run-dir, manifest, trace.jsonl exist.
#[tokio::test]
async fn m6_layout_assertions() -> Result<()>;

/// ST3: Manifest sha256 verification for all phase artefacts.
#[tokio::test]
async fn m6_manifest_sha256s_verify() -> Result<()>;

/// ST4: Trace.jsonl is schema-valid against T-029 schema.
#[tokio::test]
async fn m6_trace_schema_valid() -> Result<()>;

/// ST5: Per-phase artefacts have correct shape.
#[tokio::test]
async fn m6_per_phase_artefact_assertions() -> Result<()>;

/// ST6: Summary.json exists with per-backend tokens.
#[tokio::test]
async fn m6_summary_json_exists() -> Result<()>;

/// ST7: Resume idempotency — re-run produces no new manifest entries.
#[tokio::test]
async fn m6_resume_idempotent() -> Result<()>;

/// Live mode (gated by LOKER_M6_INTEGRATION=1).
#[tokio::test]
#[ignore = "requires LOKER_M6_INTEGRATION=1"]
async fn m6_live_mode_structural_assertions() -> Result<()>;
```

### Internal helper

```rust
/// Run the 4-phase design-doc-tdd pipeline with mocked backends.
/// Returns the RunDir for assertion inspection.
fn run_mocked_design_doc_tdd(
    spec_path: &Path,
    workflow_toml: &str,
    backends: Vec<(Arc<dyn Backend>, &str)>,
    verify_result: VerifyResult,
) -> Result<RunDir, Box<dyn std::error::Error>>;
```

### Fixture data

```rust
/// Expected outputs for each phase (loaded from tests/fixtures/m6/).
const EXPECTED_DESIGN_MD: &str = include_str!("fixtures/m6/design.md");
const EXPECTED_REVIEW_MD: &str = include_str!("fixtures/m6/review.md");
```

## 5. Test Plan

### Unit tests

| # | Test | What it asserts |
|---|------|-----------------|
| ST1 | Smoke test | Full 4-phase pipeline exits 0 against mock backends. No assertion on artefact bodies. |
| ST2 | Layout | `runs/<workflow>-<ts>-<uuid>/` exists with expected layout per T-030. |
| ST3 | Manifest | `manifest.json` has entries for all 4 phase artefacts; sha256s verify. |
| ST4 | Trace | `trace.jsonl` passes jsonschema validation against T-029 schema. |
| ST5 | Artefacts | `design.md` non-empty, `review.md` has ≥2 reviewer headings, `changes/` exists, `verify.json` has `verdict: "pass"`. |
| ST6 | Summary | `summary.json` exists with valid JSON, per-backend token counts. |
| ST7 | Resume | Re-run with same `RunDir` makes zero backend calls (all markers say "done"). |

### Integration test (live mode)

| # | Test | What it asserts |
|---|------|-----------------|
| ST8 | Live | Same structural assertions as ST2-ST6, tolerates artefact body variation. |

### Performance budget

- ST1-ST7 combined: under 5s (mocked mode, no HTTP).
- ST8 (live mode): no time budget (depends on TensorZero gateway latency).

## 6. Migration / Rollout

This is **additive code only**: a new test file and fixture directory.
No existing files are modified. The new test is automatically picked up
by `cargo test` and `make check`.

**Risks:**
- If `make check` exceeds the 60s pre-merge gate (PRD §M6 line 213), the
  test must be optimized or the gate adjusted.
- If the workflow grammar changes in a later phase, the fixture workflow
  TOML must be updated to match.

**Rollback:** Revert the test file and fixture directory. No production
impact — this is a test-only change.

## 7. Open Questions

1. **Conductor API exposure**: The workflow conductor (`src/workflows/mod.rs`
   or equivalent) may not expose a public `run_pipeline()` API. If the
   conductor is too tightly coupled to CLI arg parsing, the test may need
   to call `PhaseRunner::run()` for each phase directly. This is acceptable
   but duplicates the conductor's orchestration logic.

2. **Template resolution in tests**: `Template::render()` takes a
   `TemplateContext` with spec content and phase outputs. The test needs
   to pre-compute the context for phase 2+ (which depends on phase 1's
   output). Should the test call `render()` manually or wire through the
   conductor?

   **Resolution:** The test calls `Template::render()` directly with
   the expected upstream outputs. This keeps the test deterministic
   (no dependency on prior phase execution) while still exercising the
   template engine.

3. **Markers and resume**: The `MarkerWriter` API checks for existing
   `*.completed` markers. For the resume test, we need to simulate a
   completed run. The same `RunDir` can be reused — the test runs once,
   then calls `run_mocked_design_doc_tdd` again with the same dir and
   asserts no new entries.

4. **Live mode TensorZero gateway**: The live mode requires a running
   TensorZero gateway (CLO-252). If the gateway is not running, the
   test is silently skipped (`#[ignore]` + env gate). No hard
   dependency.
