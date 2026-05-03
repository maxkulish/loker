# PRD: CLO-288 — `design-doc-tdd` workflow file

## Overview

Author the canonical `design-doc-tdd` workflow file at `.lok/workflows/design-doc-tdd.toml` — the four-phase pipeline that is loker's reference flow per PRD UC-1.

## Background

loker is a Rust orchestration engine that runs multi-phase LLM workflows where each phase deliberately picks its model(s), provider(s), and aggregation strategy. The reference workflow is `design-doc-tdd`, defined in PRD §UC-1 (lines 63–72).

## Scope

### File: `.lok/workflows/design-doc-tdd.toml`

**Four phases** (per PRD §UC-1 and the project thesis):

1. **`design`** — `strategy = "single"`, single strong backend (e.g., `tensorzero/design`), produces `design.md`. Inputs: `spec`.
2. **`review`** — `strategy = "parallel"` with `min_responses = 2`, fans out across cross-family reviewers (e.g., `claude/`, `gemini/`, `ollama/glm-5.1`), aggregator `concat`. Produces `review.md`. Inputs: `phase:design`.
3. **`implement`** — `strategy = "escalating_retry"` with `pass_failure_context = true` (per FR-8), backends ordered cheap-to-strong (e.g., `ollama/`, `claude/`, `codex/`). Verify hook: `test_runner`. Produces `changes/`. Inputs: `phase:design`, `phase:review`.
4. **`verify`** — `strategy = "parallel"` with `min_responses = 1`, aggregator `any_fail`, runs `run_command` (build + lint) and `llm_verifier`. Produces `verify.json`. Inputs: `phase:implement`.

### Backend selection guidance (top-of-file comment)

- The file references backend identifiers; the actual model bindings live in `lok.toml` / TensorZero config.
- For local dev, `tensorzero/*` schemes route through the local gateway; `ollama/glm-5.1` is the local-reviewer slot called out in PRD line 271.
- If a reviewer slot fails to instantiate (e.g., Ollama unavailable), the run degrades to fewer reviewers per `min_responses = 2`.

### Cost budget

Include a top-level `cost_budget_usd` placeholder (commented) so users see where it goes — the M6 PRD line 35 wires this through `summary.json`.

## TDD test contract

Tests in `tests/workflows_design_doc_tdd.rs`:

1. **Round-trip**: `Workflow::from_str(include_str!("../.lok/workflows/design-doc-tdd.toml"))` parses cleanly with zero errors.
2. **Phase shape**: assert phase names, strategies, and backend lists match PRD §UC-1 byte-for-byte.
3. **Family diversity**: review-phase backends span at least two families per `family_of` (FR-13). Test uses the family resolver from CLO-265.
4. **Verify hook wiring**: implement-phase has a verify hook of kind `test_runner`; verify-phase has both `run_command` and `llm_verifier`.
5. **Input topological order**: every phase's inputs reference only earlier phases or `spec` / `var:`.
6. **Forward-compat key tolerance**: a copy of the file with `[phase.contract]` blocks added still parses (verifies FR-31 reservation works on the canonical file).

## Acceptance criteria

- [ ] `.lok/workflows/design-doc-tdd.toml` lands at the documented path.
- [ ] File is the byte-for-byte fixture used by CLO-287's design-doc-tdd round-trip test (replace its hand-rolled placeholder).
- [ ] Comments at the top of the file explain backend swap points and the local-reviewer degradation.
- [ ] `make check` is green.

## Non-goals

- Prompt templates for each phase (T-035).
- The example spec file the workflow consumes (T-036, `examples/specs/calculator.md`).
- End-to-end integration test (T-037).
- HITL review phase — UC-1's reference flow is fully automated; HITL is a separate workflow in M10.

## Dependencies

- **Blocked by CLO-287**: workflow grammar parser must exist to validate the file.

## References

- PRD §UC-1 (lines 63-72), §FR-8 (line 114), §line 271 (local-reviewer slot)
- `docs/plans/001-implementation-roadmap.md` Phase 7 row T-034