# Pre-PR validation: clo-358

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-11
**Pipeline**: lok implement-gate
---

Verified the Claude findings against the actual files. The schema only added `"plan.md"` to the closed enum (no `oneOf` pattern), no `OtherMd` fixture exists, and the integration test commits a manifest containing `kind: "synthesis.md"` that would fail schema validation. `kind_from_filename` takes `&str` and lowercases the whole string instead of taking `&Path` with `basename()` as the design specified.

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Heredoc/shell-quoting bug in the runner script — unterminated single quote inside the embedded `$(cat <<EOF ... EOF)`; bash never executed the review. Tooling artifact, not a model failure. |
| Gemini | REVIEW_FAILED | Same heredoc/shell-quoting bug as Codex in its runner script. No review produced. |
| Claude fallback | OK | Substantive review with 2 Major + 1 Minor + 1 Info findings. Verified against files. |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 — Schema not relaxed per plan ST4.** `docs/schemas/manifest.schema.json:40-51` is still a closed enum (only `plan.md` was added). Plan ST4 required `oneOf: [ {enum:[...]}, {type:"string", pattern:"^.*\\.md$"} ]` plus `analysis.md` / `OtherMd` positive fixtures. The new integration test itself commits a manifest with `kind: "synthesis.md"`, which the current schema rejects — external consumers validating manifests will break the moment a workflow uses any non-enum `.md` name. Fix: replace the enum with `oneOf` per ST4 and add `tests/fixtures/schemas/manifest/positive/other_md.json` (e.g. `kind: "analysis.md"`).
- **F2 — `kind_from_filename` deviates from design.** `src/workflow/phase_bridge.rs:37-47` takes `&str` and lowercases the whole string. The design (Architecture §3 pseudocode and Public API §4) specifies `fn kind_from_filename(path: &std::path::Path) -> Kind` with `let name = basename(path).to_ascii_lowercase()`. Latent today (all `phase.output` values are flat names), but the design called this out explicitly. Trivial fix: take `&Path`, extract `file_name()` before matching, add a nested-path unit test.

## Out of Scope / Deferred
- **F3 — `OtherMd` lowercase normalization causes `entry.name` vs `entry.kind` case divergence.** Design Open Question #2 resolved silently. Document the lowercase policy in `Kind`'s doc-comment or preserve case in `OtherMd`. Minor; can land in a follow-up.
- **F4 — `tests/schema_validation.rs` does not validate runtime-produced manifests.** Structural reason F1 slipped past `make check`. Worth adding after F1 lands, but not strictly required for this PR.

## False Positives / Tooling Artifacts
- Codex and Gemini reviewer harnesses both failed before invoking their models due to identical bash heredoc/single-quote escaping bugs in the runner scripts (`unexpected EOF while looking for matching '`). These are infrastructure failures in `.pi/agents/*-pre-pr` runners, not signals about this PR. Worth fixing the runners separately before the next pre-PR gate, otherwise the orchestrator silently falls back to single-reviewer synthesis every time.

## Recommendation
PROCEED_WITH_FIXES. Two bounded fixes before opening the PR: (1) implement the schema relaxation specified in plan ST4 (`oneOf` enum+pattern) and add an `analysis.md`/`OtherMd` positive fixture; (2) change `kind_from_filename` to take `&Path` and operate on the basename per the design's pseudocode, with one unit test covering a nested-path `output`. Both are localized, no design rework needed. Separately, the Codex/Gemini reviewer harnesses are broken — flag this to the user so the pre-PR gate produces real multi-reviewer signal on future tasks rather than silently degrading to Claude-only.
