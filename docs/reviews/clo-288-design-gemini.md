# Design Review: CLO-288

**Reviewer**: Self-review (design-review pipeline failed — models unreachable)
**Reviewed**: 2026-05-03
**Pipeline**: lok design-review (failed), fall back to self-review
**Note**: Both gemini_review and ollama_review returned REVIEW_FAILED. Pipeline error: `{{ .synthesis.output }}` variable unknown.

---

## 1. Completeness Check

All 7 sections present:
- Problem ✅
- Goals / Non-goals ✅
- Architecture ✅
- Public API surface ✅
- Test plan ✅
- Migration / rollout ✅
- Open questions ✅

Section quality: Problem and Goals are crisp. Architecture has a clean ASCII diagram. Open questions are genuinely open, not fabricated.

## 2. Architecture Assessment

**Strengths**:
- Block diagram clearly shows the three-layer relationship (file → parser → family resolver)
- Phase data flow table is complete and matches PRD §UC-1
- No new Rust modules needed — this is a fixture-and-test task, not a runtime change

**Concerns**:
- Open questions 1 and 2 are real blockers for the canonical file: the grammar may not support hooks or aggregators, which means the file cannot be PRD-faithful without grammar changes. The design correctly leaves these as open but the implement phase must resolve them.

## 3. Alignment with Handoff & Roadmap

Design doc and discovery are tightly aligned:
- Approach matches `single-file authoring` from discovery
- PRD §UC-1 phase specification is correctly reflected in the architecture table
- Non-goals explicitly exclude T-035, T-036, T-037 — scope is clean

## 4. Security Review

No security concerns. The artifact is a TOML config file, not executable code.

## 5. Implementation Concerns

- **Open questions drive the real implementation**: The canonical TOML file cannot be authored correctly until OQ1 (hooks) and OQ2 (aggregator) are resolved. The implement phase must start by resolving these.
- **Backend identifiers are illustrative**: Concrete values (`tensorzero/design`, `ollama/qwen-coder`, etc.) may need swapping at merge time.
- **Fixture vs. canonical path**: Design doc defaults to `include_str!` from the canonical path. Open question 4 flags the tradeoff.

## 6. Concurrency & Async

Not applicable — this is a static file task, not a runtime task.

## 7. Blind Spots

- **CLO-287 round-trip test**: The design assumes the existing test in `tests/workflow_grammar.rs` uses `tests/fixtures/workflows/design-doc-tdd.toml`. If the test path is different, the fixture replacement logic needs adjustment.
- **`phase.contract` FR-31 forward-compat test**: Assumes the grammar's `lint()` correctly warns on contract blocks. Verified in grammar.rs unit test — confirmed working.

## 8. Verdict

**APPROVE_WITH_SUGGESTIONS**

The design is solid. Two open questions must be resolved before the implement phase can proceed correctly. The test plan is concrete and covers all six TDD test cases.

## 9. Actionable Feedback

### Must-resolve before implement:

1. **OQ1 — Hook support in grammar**: Re-read `src/workflow/grammar.rs` `Phase` struct. Does it have a `hooks` field? If not:
   - Option A: Omit hooks from the canonical file and file a follow-up issue to extend grammar
   - Option B: Place hook config under `[phase.contract]` which the parser tolerates
   - Recommendation: Option B (keeps file PRD-shaped; parser already handles it)

2. **OQ2 — Aggregator for verify phase**: Does `Strategy::ParallelFanOut` accept an `aggregator` key? If not:
   - The canonical file omits the `any_fail` aggregator hint
   - File a follow-up issue to add aggregator support to the strategy variant

### For implement phase:

3. **Backend identifiers**: Verify that `tensorzero/design`, `ollama/qwen-coder`, `codex/strong`, etc. are valid backend identifiers that will exist in `lok.toml` / `tensorzero/config/tensorzero.toml` at merge time.

4. **Fixture path**: Confirm the CLO-287 round-trip test uses `tests/fixtures/workflows/design-doc-tdd.toml` and not the canonical path. If it uses the canonical path, `include_str!` from `.lok/workflows/design-doc-tdd.toml` works. If it uses the fixture path, the canonical file must live at both paths.

---

## Priority Actions

| Priority | Action | Owner |
|----------|--------|-------|
| P1 | Resolve OQ1 (hook support) via grammar read | Implement phase |
| P1 | Resolve OQ2 (aggregator support) via grammar read | Implement phase |
| P2 | Verify backend identifiers exist in lok.toml | Implement phase |
| P2 | Confirm fixture path for CLO-287 round-trip test | Implement phase |