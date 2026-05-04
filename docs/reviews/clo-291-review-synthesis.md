# Design Review: CLO-291 M6 end-to-end integration test on calculator spec

**Reviewer**: Manual synthesis (Gemini pipeline succeeded but write step failed)
**Reviewed**: 2026-05-04
**Pipeline**: lok design-review (partial — synthesis template resolution issue)

---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini 3.1 Pro | OK | Review produced successfully |
| Ollama (glm-5.1:cloud) | REVIEW_FAILED | Empty output |
| Claude Fallback | SKIPPED | Pipelines skipped because Gemini succeeded |
| Synthesis | NOTE | Template resolution failed in write step; manual synthesis below |

## Key Findings

| # | Finding | Severity |
|---|---------|----------|
| 1 | Approach A (PhaseRunner mocks) is well justified; trade-offs are explicit | None |
| 2 | API signatures are concrete and match existing patterns | None |
| 3 | Template rendering open question is resolved in the design | None |
| 4 | Live mode uses `#[ignore]` + env gate — good pattern | None |
| 5 | No mention of how `RunDir::create` is called twice for resume test | Low |
| 6 | No explicit mention of `Makefile` or `make check` integration | Low |
| 7 | Test helper function signature doesn't show `min_deps_success` handling | Low |

## Verdict

**APPROVE** — design is complete, test plan is concrete, API surface matches
existing patterns. Minor clarifications (findings 5-7) are implementation
details, not blocking issues.

## Priority Actions

1. Ensure the resume test documents how `RunDir` is reused (findings 5).
2. Ensure `make check` runs the new test (it will auto-discover via Cargo).
3. Verify the test helper handles the case where some phase backends fail
   (error paths for the aggregator/ strategy).
