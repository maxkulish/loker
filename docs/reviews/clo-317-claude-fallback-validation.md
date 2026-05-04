# Pre-PR validation: clo-317

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-04
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

Now I have a complete picture. Writing up findings.

## Findings

### F1 [medium] Phase-mismatched response is consumed, contradicting design
**Where:** `src/strategy/verify/human_verifier.rs:251`
**What:** The design (line 66) explicitly states *"if malformed/mismatched/empty => keep pending"*, i.e. mismatched responses should not be consumed. The implementation renames the response to `…review.json.mismatched.<ts>` on phase mismatch. An operator who wrote a response for the wrong phase loses it and has to know to look in the archived suffix file. This deviates from the design contract and could surprise reviewers in production.
**Suggested fix:** Drop the `self.consume_response("mismatched")?;` call so the file stays put for the operator to fix in place — only `Approve`/`Reject`/`CommentOnly` paths should consume. Update the test `phase_mismatch_is_consumed_after_fail` accordingly (rename to `phase_mismatch_keeps_response_in_place`) and assert the original response file still exists.

### F2 [medium] `artefact.path` is wrong — uses phase name instead of artefact path
**Where:** `src/strategy/verify/human_verifier.rs:228, 246, 276`
**What:** All three call sites pass `&self.config.phase` (e.g. `"review"`) as the `artefact_path` argument to `ensure_pending_payload`. This populates `pending.artefact.path` with the phase identifier, not a real file path under `runs/<id>/`. The pending schema's `artefact.path` is what the UI uses to render the artefact preview; with this value, downstream UI cannot locate the artefact. Schema validation passes only because the field is `minLength: 1` rather than a path constraint.
**Suggested fix:** Either thread the artefact path/kind from `PhaseConfig` (e.g. existing `outputs.path`) via `HumanVerifierConfig`, or look up the artefact path from `VerifyContext` (it has access to the strategy output). Update `ensure_pending_payload` to take the resolved relative path and a content-type kind (e.g. `text/markdown`), and adjust integration tests to assert the rendered pending file points at `review.md`.

### F3 [low] `response.schema.json` modified despite "do not modify schemas" non-goal
**Where:** `docs/schemas/response.schema.json:23`
**What:** The design explicitly lists *"Modifying schema files in this task (they already exist)"* as a non-goal (design §2). The diff adds `"comment_only"` to the response decision enum. This is a reasonable resolution of design open question #1, but it contradicts the stated scope. Reviewers downstream (and any consumer pinned to the prior schema) need to know the schema is now v1+1-equivalent.
**Suggested fix:** Update §2 of the design to remove this from non-goals (or add a §6 migration note), and confirm there is no external consumer (CLO-318 UI / response writer) already validating against the prior enum. If you want to be strict, leave the schema alone and have `HumanVerifier` translate `comment_only` from the pending decision_options in code only.

### F4 [low] Hardcoded severity timeouts pulled in from T-049
**Where:** `src/strategy/verify/human_verifier.rs:292-299`
**What:** Design §2 says *"Severity ladder (low/medium/high timing semantics) … handled in T-049"* is a non-goal, but `timeout_from` hard-codes 1h/24h timeouts. Some value is needed to satisfy the schema's `if/then/else` (high=null, else=date-time), but the specific durations are an unowned policy decision that T-049 should make.
**Suggested fix:** Either land in T-049 with the rest of the ladder, or leave the durations in code with a one-line comment that they're a placeholder until T-049 — and add a `// TODO(CLO-…): T-049 owns ladder` link so the policy choice is locatable later. Don't expand here.

### F5 [low] No `schema_version` check on response parse
**Where:** `src/strategy/verify/human_verifier.rs:115-125`
**What:** `parse_response` deserializes any `HumanResponse`-shaped JSON regardless of `schema_version`. If the schema bumps, an old or future-versioned response will be silently accepted as long as the field shape matches. Cheap insurance to add now, while the type lives in one place.
**Suggested fix:** After `serde_json::from_str`, reject `response.schema_version != SCHEMA_VERSION` with a "keep pending" error path (same branch as malformed). Add a unit test `rejects_response_with_unknown_schema_version`.

### F6 [info] Pending file is never refreshed once written
**Where:** `src/strategy/verify/human_verifier.rs:85-90`
**What:** `ensure_pending_file` short-circuits on `path.exists()`, so `opened_at` and `timeout_at` capture the very first verify attempt and never update on resume. The design says *"write/refresh"* (§3) but the impl only writes. Fine for v0 because severity/timeouts are out of scope, but worth noting so T-049 doesn't get surprised when the timeout window starts at first attempt rather than last.
**Suggested fix:** Leave as-is for this task; flag in the CLO-317 PR description so T-049's design picks up "refresh-on-resume vs sticky" as an explicit decision.

### F7 [info] Test path naming nit
**Where:** `src/phase_runner/dispatch.rs:206` (`mod tests`)
**What:** Plan ST4 acceptance is `phase_runner::dispatch::test::resolve_verify_hook_returns_human_verifier` (singular `test`) but the module is `tests`. Cosmetic — tests still run.
**Suggested fix:** Update plan or the existing module name to align; not a blocker.

## Verdict
approve_with_changes

The scaffold is correct in shape — phase runner dispatch, pending/response state machine, response consumption, and integration tests all line up with the stated acceptance criteria, and `make check` is the right gate (I did not run it; please confirm before merge). Two issues should land in this PR: the mismatched-response consume contradicts the design's "keep pending" rule (F1) and `artefact.path` is the phase name instead of the artefact's actual path, which will break UI rendering once CLO-318 lands (F2). The schema change (F3), placeholder timeouts (F4), and missing `schema_version` guard (F5) are smaller but worth resolving while the surface is fresh; the rest are notes for T-049's design review.
