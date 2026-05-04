# Plan: CLO-291 M6 end-to-end integration test on calculator spec

## Context

- Design: `docs/designs/clo-291-m6-e2e-integration-test.md`
- Discovery: `docs/discovery/clo-291.md`
- Linear: https://linear.app/cloud-ai/issue/CLO-291/m6-end-to-end-integration-test-on-calculator-spec

## Sub-tasks

### ST1 — Create fixture directory and mock response data

Create `tests/fixtures/m6/` with:
- `design.md` — expected design phase output (markdown doc for calculator lib)
- `review.md` — expected review phase output (markdown with ≥2 reviewer heading blocks)
- `implement/calculator.rs` — expected implementation output
- `verify.json` — `{"verdict": "pass"}` fixture
- `template_context.json` — template context with spec content for testing template rendering

**Files:** `tests/fixtures/m6/*`
**Acceptance:** Files exist and have expected content shapes under `tests/fixtures/m6/`
**Estimate:** S

### ST2 — Build test helper infrastructure

Create the shared test helper that:
- Creates mock backends (`M6MockBackend`) implementing the `Backend` trait
- Renders phase templates with the calculator spec
- Runs all 4 phases through `PhaseRunner` with mock backends and a verify hook
- Creates a `RunDir` and wires trace recording
- Returns the `RunDir` for assertion inspection

This is the orchestration layer that all ST3-ST9 tests call.

**Files:** `tests/m6_design_doc_tdd_e2e.rs` (helper functions)
**Acceptance:** Helper compiles and can be called from a no-op test
**Estimate:** M

### ST3 — Smoke test (wiring)

Test that the full 4-phase pipeline against mocked backends exits cleanly
and produces a run-dir. No assertions on artefact bodies yet — just
verifies the pipeline doesn't panic, hang, or error.

**Files:** `tests/m6_design_doc_tdd_e2e.rs` (test: `m6_smoke_test_mocked_pipeline_exits_zero`)
**Acceptance:** `cargo test m6_smoke_test_mocked_pipeline_exits_zero -- --nocapture` passes
**Estimate:** S

### ST4 — Run-dir layout assertions

Assert the run directory follows the T-030 layout:
- Dir name matches `design-doc-tdd-<timestamp>-<uuid_prefix>` regex
- `manifest.json` exists and parses
- `trace.jsonl` exists and parses
- `attempts/` subdirectory exists
- `markers/` subdirectory (or equivalent) exists for phase tracking

**Files:** `tests/m6_design_doc_tdd_e2e.rs` (test: `m6_layout_assertions`)
**Acceptance:** `cargo test m6_layout_assertions` passes
**Estimate:** S

### ST5 — Manifest sha256 verification

Assert `manifest.json` has entries for all 4 phase artefacts:
- `design.md` with correct `kind`, `producer`, `phase`
- `review.md` with correct `kind`, `producer`, `phase`
- `changes/` (directory digest) with correct entry
- `verify.json` with correct entry

All sha256 digests must verify against the actual files on disk.

**Files:** `tests/m6_design_doc_tdd_e2e.rs` (test: `m6_manifest_sha256s_verify`)
**Acceptance:** `cargo test m6_manifest_sha256s_verify` passes
**Estimate:** S

### ST6 — Trace schema validation

Assert `trace.jsonl` validates against the T-029 JSON schema:
- One `gen_ai.invoke_agent` span per phase/backend call
- Start/end timestamps present and monotonic
- Semantic attributes (`gen_ai.system`, `gen_ai.request.model`) match mock config
- No duplicate span IDs

**Files:** `tests/m6_design_doc_tdd_e2e.rs` (test: `m6_trace_schema_valid`)
**Acceptance:** `cargo test m6_trace_schema_valid` passes
**Estimate:** M

### ST7 — Per-phase artefact assertions

Assert each phase produced the expected artefact with correct shape:
- `design.md`: non-empty, contains expected headings
- `review.md`: concat aggregator output with ≥2 reviewer comment blocks
- `changes/`: directory exists, non-empty
- `verify.json`: valid JSON with `verdict: "pass"`

**Files:** `tests/m6_design_doc_tdd_e2e.rs` (test: `m6_per_phase_artefact_assertions`)
**Acceptance:** `cargo test m6_per_phase_artefact_assertions` passes
**Estimate:** S

### ST8 — Summary.json and resume idempotency

Two assertions in one test (they share the same run setup):
1. `summary.json` exists with valid JSON containing per-backend token counts
2. Re-running the same `RunDir` produces no new manifest entries, no new markers
   (all markers say "done")

**Files:** `tests/m6_design_doc_tdd_e2e.rs` (test: `m6_summary_and_resume`)
**Acceptance:** `cargo test m6_summary_and_resume` passes
**Estimate:** M

### ST9 — Live mode (gated)

Structural-only assertions against a real TensorZero gateway.
Gated by `LOKER_M6_INTEGRATION=1`. Same assertions as ST4-ST8 but tolerates
artefact body variation (no byte-equality).

**Files:** `tests/m6_design_doc_tdd_e2e.rs` (test: `m6_live_mode_structural`)
**Acceptance:** `LOKER_M6_INTEGRATION=1 cargo test m6_live_mode_structural` passes
**Estimate:** S (depends on ST2-ST8 being done)

## Pre-merge gate

```bash
make check
# which runs: cargo fmt --check && cargo clippy -- -D warnings && cargo test
```

The new tests are auto-discovered by Cargo (no `Makefile` changes needed).

## Risks

1. **Conductor API exposure**: If the workflow conductor is tightly coupled
   to CLI arg parsing, the test may need to call `PhaseRunner::run()` for
   each phase directly. Design resolves this by pre-rendering templates
   and calling `PhaseRunner` directly — works regardless of conductor
   coupling.
2. **Trace writer mutability**: `TraceWriter` may require `&mut self`
   access that conflicts across concurrent tests. Mitigation: each test
   creates its own `RunDir` and `TraceWriter`, so there's no shared state.
3. **Template rendering paths**: Template `{{ phase:design.output }}`
   references require the rendering context to know the output path of
   prior phases. The test must populate these manually from the mock
   outputs.
4. **Make check time budget**: With 8 sub-tests, the total could approach
   5s if each creates RunDirs with sha256 computation. Mitigation: use
   fast mock backends (in-memory, no I/O except writing artefact files).
