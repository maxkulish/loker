# Design Review: CLO-321

**Reviewer**: Synthesis engine
**Reviewed**: 2026-05-07
**Pipeline**: lok design-review

---

## Reviewer Status

| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini 3.1 Pro | FAILED | CLI initialization errors with empty model output |
| Ollama (glm-5.1:cloud) | FAILED | Provider model not found |
| Claude (fallback) | OK | Manual review following same criteria |

## Source

Single valid review from Claude fallback (both external reviewers failed). The review is at `docs/reviews/clo-321-review-gemini.md`.

## Key Findings

| # | Finding | Severity |
|---|---------|----------|
| 1 | Design is architecturally sound — separate `src/ui/` module, route extraction for FR-27 | — |
| 2 | `Commands::Ui { serve: bool }` has no behaviour defined when `serve` is absent | Medium |
| 3 | All 7 required sections present and substantive | — |
| 4 | Error handling for run discovery (skip+log) is correct daemon posture | — |
| 5 | Graceful shutdown on SIGINT/SIGTERM is properly designed | — |
| 6 | No new dependencies needed — all crates already in Cargo.toml | — |
| 7 | Port 0 binding for integration tests documented but helper function not specified | Low |
| 8 | Phase-name sanitisation in RunSummary implicitly depends on PhaseLock validation | Low |

## Verdict

**APPROVE_WITH_SUGGESTIONS**

All identified concerns are minor refinements (CLI UX, documentation, helper signatures). No architectural flaws were found. The design is ready to proceed to the plan phase with the actionable items addressed.

## Priority Actions

1. **[Medium]** Define `loker ui` no-flag behaviour — emit usage error or make `--serve` the default.
2. **[Low]** Add stderr convention documentation to §4.4.
3. **[Low]** Specify `find_project_root()` failure exit message and code.
4. **[Low]** Add security note about phase-name sanitisation dependency on PhaseLock.
5. **[Low]** Specify `spawn_daemon()` helper signature for integration test reuse.
6. **[Minor]** Reserve `DaemonState` extension point comment in `routes.rs`.

## Decision Recommendation

**PROCEED_WITH_FIXES**: Approve the design with the 6 actionable feedback items addressed. Items 1 (CLI UX) is the only behaviour-affecting change; items 2-6 are documentation/clarity improvements. None are blocking — the design can proceed to the plan phase immediately and fixes applied during implementation.
