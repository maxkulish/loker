# Design Review Synthesis: CLO-319

## Review Details

| Field | Value |
|-------|-------|
| Design doc | `docs/designs/clo-319-advisory-lock.md` |
| Task | CLO-319 |
| Date | 2026-05-06 |
| Reviewer | Gemini 2.5 Pro (architect persona) |
| Verdict | **APPROVE** |

## Feedback Classification

| # | Suggestion | Class | Action |
|---|-----------|-------|--------|
| 1 | **Clarify Phase Name Normalization**: Explicitly state that phase name sanitization should reuse the same logic as `AttemptDir` to ensure consistency across all filesystem artifacts within a run directory. | Refinement | Applied — added explicit reference to `AttemptDir` normalization in §4 (Symlink hardening) and §3 (Architecture). |
| 2 | **Consider Lock File Deletion on Graceful Release**: Truncate the lock file to 0 bytes upon graceful release so `PhaseLock::inspect` on an unlocked phase returns `Ok(None)` or a clear empty-file signal, simplifying downstream tools like `loker ls --blocked`. | Additive | Applied — updated §4 (Public API surface) and §7 (Open questions) to document the graceful-release behavior: truncate on `release()` / `drop()` to 0 bytes; `inspect` handles empty files as "not held." |

## Summary

- **0** suggestions contradicted the chosen approach (0 flagged)
- **2** suggestions applied (1 refinement, 1 additive)
- **0** suggestions deferred

The design is ready for implementation planning.
