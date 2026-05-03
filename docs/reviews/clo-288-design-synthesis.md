# Design Review Synthesis: CLO-288

**Task**: Author the canonical design-doc-tdd workflow file at .lok/workflows/design-doc-tdd.toml
**Reviewers**: Self-review (Gemini + Ollama both failed; pipeline produced REVIEW_FAILED)
**Synthesized**: 2026-05-03

---

## Reviewer Status

| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini 3.1 Pro | REVIEW_FAILED | CLI error output, not a review |
| Ollama (glm-5.1:cloud) | REVIEW_FAILED | Empty or trivially short output |
| Self-review | OK | Full 9-section review produced |

---

## Source

Self-review (fallback after both external reviewers failed)

## Key Findings

| # | Finding | Severity |
|---|---------|----------|
| OQ1 | Grammar Phase struct lacks `hooks` field — canonical file cannot express verify hooks directly | High |
| OQ2 | Grammar Strategy::ParallelFanOut lacks `aggregator` field — verify-phase any_fail hint not expressible | High |
| F1 | Backend identifiers in design doc sketch are illustrative — must verify against lok.toml | Medium |
| F2 | `phase.contract` FR-31 forward-compat already confirmed working in grammar.rs lint() | Low |
| F3 | CLO-287 round-trip test fixture path needs confirmation | Medium |

## Verdict

**APPROVE_WITH_SUGGESTIONS**

## Priority Actions

1. **Implement phase must resolve OQ1 and OQ2 before authoring the canonical file** — the TOML cannot be PRD-faithful without resolving whether hooks and aggregators are in the grammar.
2. **Option B for both OQs**: Place hook config under `[phase.contract]` (parser already tolerates this) and omit the `any_fail` aggregator hint (or express it as a phase-level field the parser ignores). This keeps the file PRD-shaped while remaining parseable by today's grammar.
3. **Verify backend identifiers** against `lok.toml` / `tensorzero/config/tensorzero.toml` at implement time.
4. **Confirm CLO-287 round-trip test fixture path** before finalizing the canonical file path strategy.

## Decision Recommendation

**PROCEED_WITH_FIXES** — approve the design document; implement phase must resolve OQ1 and OQ2 as the first order of business.