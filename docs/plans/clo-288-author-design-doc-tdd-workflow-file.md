# Plan: CLO-288 — Author design-doc-tdd workflow file at .lok/workflows/design-doc-tdd.toml

## Context

- **Design**: `docs/designs/clo-288-author-design-doc-tdd-workflow-file.md`
- **Discovery**: `docs/discovery/clo-288.md`
- **Linear**: <https://linear.app/cloud-ai/issue/CLO-288/author-design-doc-tdd-workflow-file-at-lokworkflowsdesign-doc-tddtoml>
- **Grammar**: `src/workflow/grammar.rs` (CLO-287) — confirmed: no `hooks` field, no `aggregator` field. Option B applied: forward-compat `[phase.contract]` block and no `any_fail` aggregator hint.
- **Available backends** (from `lok.toml`): `claude/`, `gemini/`, `codex/`, `ollama/qwen3-coder-next` (model: `qwen3-coder-next:latest`). Note: `tensorzero/<fn>` is NOT in `lok.toml` — backend identifiers must use the available schemes.
- **Existing fixture**: `tests/fixtures/workflows/design-doc-tdd.toml` (hand-rolled placeholder with wrong phase names)
- **CLO-287 blocks**: `CLO-288` (per Linear)

## Sub-tasks

### ST1 Author the canonical `.lok/workflows/design-doc-tdd.toml`

**Files:** `.lok/workflows/design-doc-tdd.toml` (new)

**Acceptance:**
- `Workflow::from_str(include_str!("../.lok/workflows/design-doc-tdd.toml"))` returns `Ok`
- `lint()` returns only the expected `[phase.contract]` reservation warnings (no errors)
- Phase names in order: `["design", "review", "implement", "verify"]`
- Strategy variants match PRD §UC-1
- `review` phase backends span ≥ 2 families (FR-13)
- `implement` phase has ≥ 2 backends for escalating retry

**Estimate:** S

**Notes:**
- Backend identifiers use actual schemes from `lok.toml`: `claude/`, `gemini/`, `codex/`, `ollama/qwen3-coder-next` (no `tensorzero/` prefix — that scheme is not registered)
- Top-of-file comment block with backend swap points and degradation behavior
- `cost_budget_usd` commented placeholder per PRD line 35
- Hook syntax placed under `[phases.contract]` per Option B (FR-31 forward-compat). No top-level `hooks` key — grammar lacks this field.
- No `aggregator = "any_fail"` on verify phase — `Strategy::ParallelFanOut` has no aggregator field in current grammar.

### ST2 Create `tests/workflows_design_doc_tdd.rs` — round-trip TDD test contract

**Files:** `tests/workflows_design_doc_tdd.rs` (new)

**Acceptance:** `cargo test --test workflows_design_doc_tdd` passes

**Notes:**
Test cases (matching design §Test plan):

1. `roundtrip_parses_clean` — `Workflow::from_str(...)` returns `Ok`, `lint()` has no errors
2. `phase_names_and_order_match_prd` — exactly `["design", "review", "implement", "verify"]`
3. `phase_strategies_match_prd` — `Single`, `ParallelFanOut { min_responses: 2 }`, `EscalatingRetry { pass_failure_context: true }`, `ParallelFanOut { min_responses: 1 }`
4. `review_phase_spans_two_families` — `family_of` (CLO-265) over review backends → `HashSet` size ≥ 2
5. `implement_phase_backends_ordered_cheap_to_strong` — list length ≥ 2, order preserved
6. `inputs_are_topologically_valid` — all `phase:X` refs name earlier phases; `spec` and `var:` accepted
7. `verify_hook_kinds_present` — `#[ignore]` with comment: grammar lacks `hooks` field (follow-up filed as discovery debt)
8. `forward_compat_phase_contract_block_parses` — append `[phases.contract]`, parse → succeeds, `lint()` returns FR-31 warning

### ST3 Replace existing fixture at `tests/fixtures/workflows/design-doc-tdd.toml`

**Files:**
- `tests/fixtures/workflows/design-doc-tdd.toml` (rewritten)
- `tests/workflow_grammar.rs` (updated `include_str!` path if needed)

**Acceptance:** `cargo test --test workflow_grammar` passes

**Notes:**
- Option A: rewrite the fixture as a byte-identical copy of the canonical file (no drift risk, self-contained)
- Option B (preferred if CLO-287 test uses a relative path): repoint `include_str!` in `tests/workflow_grammar.rs` to `include_str!("../../.lok/workflows/design-doc-tdd.toml")` and remove the fixture copy
- If CLO-287 round-trip test does NOT use this fixture at all, the fixture rewrite is optional — still worth doing for developer ergonomics
- Verify with `rg -n "design-doc-tdd" tests/`

## Pre-merge gate

- `make check` (fmt + clippy + test)
- `cargo test --test workflows_design_doc_tdd` passes
- `cargo test --test workflow_grammar` passes

## Risks

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| `family_of` not exported from public API | Low | Medium | Use `BackendRef` variants to detect cross-family backends (ollama vs claude vs gemini vs codex) |
| CLO-287 round-trip test uses fixture with hard-coded expectations | Medium | Medium | Check `tests/workflow_grammar.rs` before rewriting fixture |
| Hook forward-compat test (`verify_hook_kinds_present`) is `#[ignore]` and is removed later | Low | Low | Test documents the gap; follow-up is in discovery debt |
| `make check` fails on unrelated lint | Low | Low | Fix in place; not a risk for this task |

## Open questions (resolved in this plan)

| OQ | Resolution |
|----|------------|
| OQ1 — hooks in grammar | Option B: hook config goes in `[phases.contract]`. Test ST2-7 is `#[ignore]`. Follow-up issue: extend grammar Phase struct with `hooks` field. |
| OQ2 — aggregator in grammar | Omitted. `any_fail` is implicit behavior of `verify` phase per PRD. Follow-up issue: add `aggregator` field to `Strategy::ParallelFanOut`. |
| OQ3 — backend identifiers | Use actual schemes from `lok.toml`: `ollama/qwen3-coder-next`, `claude/`, `gemini/`, `codex/`. No `tensorzero/` prefix. |
| OQ4 — canonical file vs fixture | Option B: `include_str!("../../.lok/workflows/design-doc-tdd.toml")` in test. Fixture at `tests/fixtures/workflows/design-doc-tdd.toml` is rewritten as copy. |