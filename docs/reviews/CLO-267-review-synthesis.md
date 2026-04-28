# Review Synthesis: CLO-267

**Synthesized**: 2026-04-28
**Pipeline**: lok design-review
**Reviewers**: Gemini 3.1 Pro, Codex/Ollama (glm-5.1:cloud), Claude (fallback if needed)

---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini | REVIEW_FAILED | Gemini CLI unreachable (pre-flight check failed) |
| Ollama | OK | Full review produced |
| Claude Fallback | SKIPPED | External reviewers succeeded |

## Source
Ollama (sole successful reviewer)

## Key Findings
| # | Finding | Severity |
|---|---------|----------|
| 1 | Handoff rule violation: AnyFail aggregation logic placed inline in `parallel_fanout.rs` instead of a new `src/aggregator/` module, contradicting handoff's "new primitives land as new modules" rule | High |
| 2 | Test flakiness risk: `first_fails`/`mid_list_fails` tests assert specific backend names as offender, but `FuturesUnordered` completion order is non-deterministic without delay control on `MockBackend` | High |
| 3 | No handling of markdown-fenced JSON (` ```json ... ``` `) - the most common LLM formatting artifact, will produce surprising `VerdictContract` errors on real backends | High |
| 4 | `offender: Box<Attempt>` semantics ambiguous for `BackendError` case - no actual `QueryOutput` exists; design doesn't document the synthetic attempt pattern | Medium |
| 5 | `any_fail_evaluate` signature has dead `Ok(false)` branch with unclear "future trait extraction" rationale; should simplify to `Result<(), AnyFailReason>` | Medium |
| 6 | No explicit test case for empty `query.text` | Medium |
| 7 | No JSON Schema artifact for verdict shape (`{ "pass": bool }`) under `docs/schemas/` to formalize forward-compat | Medium |
| 8 | Missing sections vs. handoff template: Background, Implementation Plan (file-by-file TDD phasing), Acceptance Criteria table mapping to FR-11, Rollback plan | Medium |
| 9 | `VerdictRejected { payload }` stores raw backend output - secret-leakage risk into logs; dependency on T-029 redaction should be flagged | Low |
| 10 | Priority rule between `AnyFail` and `FloorViolation` not stated explicitly (AnyFail wins via early-return) | Low |
| 11 | Per-branch `VerifyOutcome` left as `skipped()` even for branches that returned `pass: true` - loses observability granularity | Low |
| 12 | No spec for `trace.jsonl` events emitted by AnyFail evaluation (T-029 boundary) | Low |
| 13 | CLAUDE.md says "Active milestone: M1" but T-018 is M3 work - stale milestone marker | Low |

## Verdict
APPROVE_WITH_SUGGESTIONS - design is technically sound, correctly scoped, and fail-closed by default. Inline streaming approach is pragmatic. None of the concerns are blocking, but the high-severity items should be addressed before implementation hardens.

## Priority Actions

**High**
1. Move `any_fail_evaluate` + `AnyFailReason` into a new `src/aggregator/` module (re-export from `parallel_fanout.rs` until T-017) to satisfy handoff's new-primitives rule.
2. Fix arrival-order test determinism: add configurable delays to `MockBackend`, or relax assertions to "offender is one of the failing backends."
3. Add markdown-fence stripping (` ```json ` / ` ``` `) to `any_fail_evaluate` before `serde_json::from_str`, or explicitly scope AnyFail to structured-verify backends only.

**Medium**
4. Document `offender: Box<Attempt>` semantics for `BackendError` (synthetic `Attempt` with `FinishReason::Error`).
5. Simplify `any_fail_evaluate` return type to `Result<(), AnyFailReason>`; remove dead `Ok(false)` branch and reference T-017.
6. Add explicit test case for empty `query.text`.
7. Add `docs/schemas/verdict.schema.json` to formalize the verdict contract.
8. Fill in missing template sections: Background, Implementation Plan with TDD phasing, Acceptance Criteria table mapping FR-11, Rollback plan.

**Low**
9. Note T-029 redaction dependency for `VerdictRejected.payload`.
10. State `AnyFail` > `FloorViolation` priority explicitly.
11. Consider setting per-branch `VerifyOutcome::passed("Aggregator::AnyFail")` on passing branches.
12. Specify `trace.jsonl` events for AnyFail evaluation outcomes.
13. Update CLAUDE.md active milestone from M1 to M3.

## Decision Recommendation
**PROCEED_WITH_FIXES** - Approve once the three High-severity items are addressed:
- Module placement under `src/aggregator/`
- Test determinism for arrival-order assertions
- Markdown-fence handling (or explicit scope statement)

Medium and Low items can be folded into implementation PRs or follow-up tickets without re-review.
