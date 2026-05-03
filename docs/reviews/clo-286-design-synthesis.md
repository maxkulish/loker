# Design Review Synthesis: clo-286

**Reviewer**: Synthesis engine
**Date**: 2026-05-03
**Pipeline**: Manual fallback (external reviewers failed)

---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini 3.1 Pro | FAILED | Trust directory restriction (--sandbox rejected) |
| Codex via Ollama | FAILED | Model ollama/glm-5.1:cloud not found |
| Claude (fallback) | SKIPPED | External reviewers failed; manual review performed by pi |
| Synthesis | OK | Single review source (manual fallback) used |

## Source
Manual review by primary agent (pi)

## Key Findings
| # | Finding | Severity |
|---|---------|----------|
| 1 | `next_attempt_from_dirs` off-by-one bug: stores `n+1` instead of raw `n` | **CRITICAL** |
| 2 | `promote_to_canonical` code shows directory rename, but design note claims per-file rename | **MEDIUM** |
| 3 | `run_state/mod.rs` export changes not documented | **MEDIUM** |
| 4 | `Config` needs new `RunStateConfig` sub-struct for `AttemptRetention` | **MEDIUM** |
| 5 | Missing Migration / Rollout section | LOW |
| 6 | Missing Open Questions section | LOW |
| 7 | Orphan sweep interaction with attempt dirs not addressed | LOW |

## Verdict
**APPROVE_WITH_SUGGESTIONS**

## Priority Actions
1. Fix off-by-one in `next_attempt_from_dirs`.
2. Clarify `promote_to_canonical` mechanism (directory rename).
3. Document `run_state/mod.rs` module additions.
4. Document `Config`/`RunStateConfig` wiring.
5. Add Migration / Rollout and Open Questions sections.

## Decision Recommendation
**PROCEED_WITH_FIXES**: Apply the 5 priority actions above. The design is fundamentally sound and aligned with D3. No blocking architectural issues.
