# Design Review: clo-295

**Reviewer**: Synthesis engine
**Reviewed**: 2026-05-03
**Pipeline**: lok design-review

---

## Reviewer Status

| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini 3.1 Pro | REVIEW_FAILED | CLI initialization errors; validator rejected |
| Codex via Ollama | REVIEW_FAILED | Prompt injection / non-review output; validator rejected |
| Claude (fallback) | OK | Sole valid review source |

## Single Review Format

### Source
Claude fallback review (docs/reviews/clo-295-review-fallback.md)

### Key Findings

| # | Finding | Severity |
|---|---------|----------|
| 1 | `ResumeError` duplicates `ArtefactCorrupt`/`ArtefactMissing` already in `LoadError` | P2 — minor inconsistency |
| 2 | Heartbeat TTL is not persisted; resume must recover it | P1 — blocks correct stale detection |
| 3 | `PhaseConfig` derivation from `Workflow` is undefined | P1 — may block CLI integration |
| 4 | "Archive current attempt" operation is not concretely specified | P2 — implementation gap |
| 5 | `PhaseRunner::run()` may not accept upstream manifest entries | P2 — signature verification needed |
| 6 | Advisory lock is OS-cooperative only; error naming implies stronger semantics | P3 — documentation fix |
| 7 | `trace.jsonl` behaviour on resume is unspecified | P3 — documentation gap |
| 8 | Disk-full during sweep is unhandled | P3 — edge case |

### Verdict
**APPROVE_WITH_SUGGESTIONS**

### Priority Actions (Ordered)

1. **P1 — Heartbeat TTL recovery:** Add `ttl_seconds` to `heartbeat.json` schema, or require `--ttl` on `resume` CLI (default 300s).
2. **P1 — PhaseConfig from Workflow:** Add a `Workflow::to_phase_configs()` adapter, or document that `resume` v0 only accepts programmatic phase lists.
3. **P2 — Archive operation:** Define the concrete rename/move semantics for archiving a current attempt into `attempts/<phase>/<n>/`.
4. **P2 — ResumeError cleanup:** Remove redundant `ArtefactCorrupt`/`ArtefactMissing` variants from `ResumeError`; surface them through `LoadError`.
5. **P3 — Documentation fixes:** Advisory lock semantics, trace.jsonl append behaviour, disk-full sweep error handling.

### Decision Recommendation

**PROCEED_WITH_FIXES:** The architecture is sound, the TDD contract is solid, and all findings are actionable and minor. Proceed to implementation once P1 items (heartbeat TTL, PhaseConfig adapter) are resolved in the design doc or deferred to the plan phase with explicit sub-tasks.
