# Design Review: CLO-317

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| gemini-3.1-pro-preview | OK | Provided structured review with 3 actionable findings |
| ollama/glm-5.1:cloud | REVIEW_FAILED | Tool unavailable in workflow; review not produced |
| claude_fallback | SKIPPED | Skipped because one external review produced usable output |

## Source
Gemini review (`docs/reviews/clo-317-design-gemini.md`)

## Key Findings
| # | Finding | Severity |
|---|---------|----------|
| 1 | Consume accepted `response.json` after use (rename/delete) to prevent stale replay across retry/resume | High |
| 2 | Treat malformed/invalid response as still pending rather than terminal verify failure | High |
| 3 | Define `comment_only` deterministically; recommend keeping phase pending until explicit approval/reject | Medium |

## Verdict
APPROVE_WITH_SUGGESTIONS

## Priority Actions
1. Implement response consumption path with retention-safe strategy (e.g. rename to `.handled`).
2. Keep malformed/mismatched response files in pending flow and avoid returning hard verify-failed states.
3. Encode `comment_only` as blocked/pending behavior in both implementation and tests.

## Decision Recommendation
PROCEED_WITH_FIXES: implement the three priority actions above before implementation begins; all other design structure is acceptable for scope.