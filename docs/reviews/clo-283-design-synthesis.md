# Design Review Synthesis — CLO-283

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini 3.1 Pro | OK | Manual fallback review (lok workflow TOML-parse error prevented automated pipeline) |
| Ollama | SKIPPED | Pipeline failure prevented invocation |
| Claude fallback | SKIPPED | External reviewers did not both fail — Gemini review succeeded |

## Source
Single reviewer: Gemini architect persona (manual fallback).

## Key Findings
| # | Finding | Severity |
|---|---------|----------|
| 1 | `chrono::serde::iso8601` does not exist; default serialization is already RFC 3339 | major |
| 2 | Parent-directory fsync via `File::open(parent)` is POSIX-only, not Windows portable | major |
| 3 | Orphan sweep mentions "phase/attempt marker values" but markers only reference sha256 | major |
| 4 | `ManifestError` lacks `#[non_exhaustive]` | minor |
| 5 | `NamedTempFile::persist` return type is version-dependent and ambiguous | minor |
| 6 | Missing capacity hint for the entries Vec | minor |
| 7 | Stray text at end of document | nit |

## Verdict
**Overall: APPROVE_WITH_SUGGESTIONS** — The single reviewer returned `approve_with_changes`. The findings are all mechanical (compile-time or doc-level); no architectural concerns were raised. No changes require returning to discovery.

## Priority Actions
1. **Before implementation**: Fix F1 (`chrono::serde::iso8601` → remove `with` attribute).
2. **Before implementation**: Fix F2 (document Windows parent-fsync limitation or gate with `#[cfg(unix)]`).
3. **Before implementation**: Fix F3 (remove imprecise "or" clause from orphan sweep description).
4. **Before PR**: Fix F4 (add `#[non_exhaustive]` to `ManifestError`).
5. **Before PR**: Fix F5 (clarify `persist` return type in design doc).
6. **Optional**: Fix F6–F7 (capacity hint, stray text).

## Decision Recommendation
**PROCEED_WITH_FIXES**: The design is fundamentally sound. Apply the three major fixes above (chrono serde attribute, parent-fsync portability, orphan sweep criteria), then advance to the plan phase. None of the findings alter the chosen approach or module layout.
