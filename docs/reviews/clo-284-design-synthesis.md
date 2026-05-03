# Design Review Synthesis: CLO-284

**Synthesizer**: pi (orchestrator)
**Date**: 2026-05-02
**Design**: `docs/designs/clo-284-phase-status-markers.md`

## Reviewer Status

| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini 2.5 Pro | OK | Full structured review, verdict: APPROVE |
| Ollama (glm-5.1:cloud via opencode) | REVIEW_FAILED | Model `ollama/glm-5.1:cloud` not found by opencode provider |

## Key Findings (Single Review — Gemini only)

| # | Finding | Severity |
|---|---------|----------|
| 1 | Design correctly reuses/extracts `atomic_write` — prevents protocol divergence between manifest and markers | Low (positive) |
| 2 | `Clock` trait for heartbeat testability is a mature pattern — prevents flaky tests | Low (positive) |
| 3 | Module layout (`run_state/{atomic,markers,heartbeat,order}`) has clear separation of concerns | Low (positive) |
| 4 | `PhaseOrderGuard` state machine enforces started → artefact → manifest → completed order | Low (positive) |
| 5 | Test plan is exceptionally thorough (17 tests covering round-trip, crash, boundary, concurrency) | Low (positive) |
| 6 | Rollout plan for extracting `atomic_write` is safe — existing manifest tests continue to pass | Low (positive) |
| 7 | `next_attempt` could become I/O bottleneck with many retries — acceptable for v0, flag for T-027 | Medium (minor concern) |
| 8 | HeartbeatWriter cancellation semantics and error recovery are well-considered | Low (positive) |

## Verdict

**APPROVE** — Design is comprehensive, well-structured, and ready for implementation.

## Actionable Suggestions (from Gemini review)

1. **Add TODO comment in `next_attempt`** pointing to T-027 for attempt-directory-based performance improvement.
2. **Log heartbeat write errors** with `tracing::warn!` / `error!` so operators can see disk-full or permission issues.
3. **Consider marker filename naming**: `design.started` could hypothetically collide with a phase named `design.started` — a `design.marker.started` convention is safer. Minor point, acceptable as-is for v0.
4. **PhaseOrderGuard across async boundaries**: T-028 phase runner must be careful not to lose the guard across `.await` points.

## Decision Recommendation

**PROCEED** — No blocking issues. Move to implementation.
