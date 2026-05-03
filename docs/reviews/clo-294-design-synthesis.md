# Design Review Synthesis: CLO-294 — Run Directory Layout

**Reviewed**: 2026-05-03
**Pipeline**: Manual (lok design-review automation failed)

---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini 3.1 Pro | REVIEW_FAILED | Trust/approval mode issue — empty output both primary and fallback models |
| Codex via Ollama (glm-5.1:cloud) | REVIEW_FAILED | Error message and stack trace, not review output |
| Claude (fallback) | SKIPPED | Both external reviewers failed before fallback trigger |

Both automated reviewers failed. Manual review performed and documented in `docs/reviews/clo-294-design-gemini.md`.

## Key Findings
| # | Finding | Severity |
|---|---------|----------|
| F1 | `RunDir::create` has implicit `cwd` dependency, untestable with temp dirs | minor |
| F2 | `trace.jsonl` "reserved" vs "created on demand" mismatch between goals and protocol | minor |
| F3 | No cleanup on partial creation failure (orphaned empty dirs) | minor |
| F4 | `slug` crate may fail under MSRV 1.80; consider inlining | nit |
| F5 | `AttemptDir::create()` responsibility not documented on accessor | nit |
| F6 | Missing runtime assertion for `run_id` consistency between `RunDir` and manifest | nit |

## Verdict
approve_with_changes

The design is structurally sound and follows established codebase patterns. Three minor issues (F1, F2, F3) should be addressed before implementation. The remaining nits (F4, F5, F6) can be addressed during implementation.

## Priority Actions
1. **[F1] Add `base_dir` parameter to `RunDir::create`** — Without this, test code cannot use the API. Add `create_in(base_dir, name)` or accept `base_dir: &Path`.
2. **[F2] Resolve `trace.jsonl` inconsistency** — Either remove "reserves trace.jsonl" from goals (let trace writer create it) or add `File::create` to the creation protocol.
3. **[F3] Add cleanup on partial failure** — Remove the leaf directory if manifest write or `attempts/` creation fails after `mkdir` succeeds.
4. **[F4] Verify `slug` crate compiles with MSRV 1.80** — Or inline the slug function as a `src/run_state/slug.rs` utility.
5. **[F5/F6] Documentation and assertions** — Document `AttemptDir::create()` expectation on the accessor; add `debug_assert_eq!` for run_id consistency.

## Decision Recommendation
PROCEED_WITH_FIXES: Approve the design once F1, F2, and F3 are addressed in the design document, then move to implementation.
