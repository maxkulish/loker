# Pre-PR validation: clo-317

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-04
**Pipeline**: lok implement-gate
---

## Reviewer Status

| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc quoting bug in the runner (`unexpected EOF` from unbalanced `'`). The wrapper script — not the reviewer — failed; no Codex output was produced. Tooling artifact. |
| Gemini | REVIEW_FAILED | Same shell heredoc quoting bug pattern; both primary and fallback paths never executed. Tooling artifact. |
| Claude (fallback) | OK | Full review against design + plan + diff. Findings verified against `src/strategy/verify/human_verifier.rs` and `docs/designs/clo-317-humanverifier-hook-scaffold.md` §2/§3. |

## Verdict
approve_with_changes

## Must Fix Before PR

- **F1 — Phase-mismatched response is consumed (`src/strategy/verify/human_verifier.rs:251`).** Design §3 line 62 states *"if malformed/mismatched/empty => keep pending"*. The current code calls `consume_response("mismatched")` when `response.phase != self.config.phase`, archiving the operator's response into `<phase>.json.mismatched.<ts>` and forcing them to recover from the suffix file. Drop the consume call on the mismatch branch (only `Approve`/`Reject`/`CommentOnly` paths should consume) and update `phase_mismatch_is_consumed_after_fail` to assert the original response is still in place.
- **F2 — `artefact.path` populated with the phase name (`src/strategy/verify/human_verifier.rs:228, 246, 276`).** All three `ensure_pending_payload` call sites pass `&self.config.phase` (e.g. `"review"`) as `artefact_path`. Schema validation passes only because `pending.schema.json` enforces `minLength:1`, but downstream UI (CLO-318) cannot render the artefact preview. Thread the resolved artefact relpath (and a content-type kind such as `text/markdown`) through `HumanVerifierConfig` or pull it from `VerifyContext`/`PhaseConfig.outputs.path`, then assert the rendered pending file points at the real artefact in the integration tests.
- **F3 — `docs/schemas/response.schema.json` modified despite design non-goal.** Design §2 explicitly lists *"Modifying schema files in this task (they already exist)"* under non-goals, but the diff adds `"comment_only"` to the response decision enum. Either (a) update the design to drop that non-goal and add a one-line migration note in §6, or (b) revert the schema change and translate `comment_only` purely in `HumanDecision`. Pick one before merge so the schema-versioning story stays honest.

These three are scoped, small, and land in one bounded fix iteration — no further design dialogue required.

## Out of Scope / Deferred

- **F4 — hardcoded 1h/24h timeouts in `timeout_from`.** The severity ladder is explicitly T-049 territory (design §2 non-goals). Acceptable as a v0 placeholder; flag in the PR description so T-049 owns the policy and consider a `// TODO(CLO-…): T-049 owns ladder` marker if you want a grep anchor.
- **F5 — no `schema_version` check in `parse_response`.** Cheap insurance but not in the design's acceptance criteria. Track for the response-handling hardening pass.
- **F6 — pending file is never refreshed on resume.** Design §3 says "write/refresh"; impl short-circuits on `path.exists()`. Fine for v0 because `opened_at`/`timeout_at` semantics are owned by T-049 — call this out in the PR description so T-049 picks the refresh-on-resume vs sticky decision intentionally.
- **F7 — test module name nit (`mod tests` vs plan's `test`).** Cosmetic; not a blocker.

## False Positives / Tooling Artifacts

- **Codex and Gemini reviewer wrappers failed at the shell layer**, not at the model. The heredocs in the orchestrator review scripts use unbalanced single quotes (likely `\`git diff main...HEAD\`` interacting with `EOF` quoting), producing `unexpected EOF while looking for matching '`. No actual review was attempted. Both should be re-run after the wrapper is fixed (separate orchestrator concern) — they may surface findings the Claude fallback missed, but the bounded fixes above stand on their own.

## Recommendation

PROCEED_WITH_FIXES. Land three bounded changes in the same fix iteration before opening the PR: (1) remove the `consume_response("mismatched")` call so phase-mismatched responses stay in place per design §3 and update the corresponding test, (2) thread a real artefact path (and kind) through `HumanVerifierConfig`/`VerifyContext` so `pending.artefact.path` points at the actual artefact rather than the phase name, and (3) reconcile `docs/schemas/response.schema.json` with the design's "no schema modifications" non-goal by either updating §2/§6 of the design or reverting the enum addition. Defer F4–F7 to T-049 / follow-up tasks and surface them in the PR description. Note that two of three external reviewers (Codex, Gemini) never ran due to a shell-quoting bug in the orchestrator wrapper; the verdict rests on the Claude fallback plus direct code inspection — re-running those reviewers post-fix would be prudent if the wrapper is repaired before merge.
