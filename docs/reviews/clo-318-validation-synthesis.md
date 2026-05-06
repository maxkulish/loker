# Pre-PR validation: clo-318

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc syntax error (unmatched quote in wrapper script, line 30); no review produced |
| Gemini | REVIEW_FAILED | Same shell heredoc syntax error in wrapper script (line 38); both primary and fallback models never invoked |
| Claude fallback | OK | Produced 4 findings (1 low, 3 info) against design + plan; build, fmt, clippy, tests verified green |

Both external reviewers failed due to a tooling bug in the wrapper scripts (the `$(cat <<EOF ... EOF)` block is being passed through a shell that mangles the quoting), not due to the diff itself. Synthesis relies entirely on the Claude fallback.

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 [low] Negative pending fixture filename misrepresents what it tests** - `tests/fixtures/schemas/pending/negative/low_severity_null_timeout.json` was repurposed to violate `decision_options: minItems: 1` after the schema was relaxed to allow `timeout_at: null` for any severity, but the filename still implies the old violation. The harness at `tests/schema_validation.rs:90` documents the convention "one violation per file, name the file after it." Rename to something like `empty_decision_options.json` and update `prompt_summary` to describe the actual violation. Bounded, mechanical fix, no code changes.

## Out of Scope / Deferred
- **F2 [info]** `#[allow(clippy::large_enum_variant)]` on `VerifyHookName::HumanVerifier`. Defensible today; revisit (e.g., `Box<HumanVerifierConfig>`) if another variant grows.
- **F3 [info]** `HumanVerifier` hook is constructed twice per phase (once by `dispatch::resolve_verify_hook`, once by the HITL branch for `verify_with_report`). Minor wasted work; refactor when a second hook needs the same treatment.
- **F4 [info]** Malformed-response branch in `verify_with_report` returns `default_report()` and drops `timeout_at` context even if the deadline had already passed. Matches the design's "do not silently auto-approve" stance; future enhancement to thread the rule's deadline through this branch.

## False Positives / Tooling Artifacts
- **Codex review failure** - wrapper script bug, not a finding against the branch. The heredoc-inside-`$(cat <<EOF)` pattern combined with embedded backticks is being re-evaluated by an outer shell. Needs fix in `.pi/` review wrapper, not in this PR.
- **Gemini review failure** - identical wrapper bug; both models unreachable. Same disposition.

## Recommendation
PROCEED_WITH_FIXES — rename `tests/fixtures/schemas/pending/negative/low_severity_null_timeout.json` to a name that matches its actual schema violation (`empty_decision_options.json` or similar) and update its `prompt_summary` field, then re-run `make check` and open the PR. The implementation faithfully follows `docs/designs/clo-318-severity-ladder.md` (severity ladder with overrides, fake-clock seam, pending file as deadline source of truth, marker `HitlMarkerContext`, four `loker.hitl.*` trace fields, schema relaxations) and is otherwise ready. Separately, flag the `.pi/` Codex/Gemini wrapper heredoc bug to the orchestrator team so future synthesis runs aren't reduced to a single reviewer.

## Re-validation

Applied the single Must Fix item:

- Renamed `tests/fixtures/schemas/pending/negative/low_severity_null_timeout.json` to `tests/fixtures/schemas/pending/negative/empty_decision_options.json`.
- Updated the fixture `prompt_summary` to describe the actual `decision_options: minItems` violation.

Verification after fix: `make check` is green. The first post-fix `make check` hit a transient `resume::lock::tests::lock_acquires_and_releases` lock collision; immediate rerun passed with the same code.
