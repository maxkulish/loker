# Pre-PR validation: clo-292

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-03
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc quoting broke the wrapper script (`unexpected EOF while looking for matching '`). Codex CLI never ran. |
| Gemini | REVIEW_FAILED | Same shell heredoc quoting bug as the Codex wrapper; Gemini CLI never ran. |
| Claude (fallback) | OK | `make check` passed locally (660+ unit + integration tests, no clippy warnings); produced 10 findings. |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 (high) — PhaseRunner skips its own verify hook for `EscalatingRetry`** (`src/phase_runner.rs:233-235`). The `matches!(strategy_output.strategy, StrategyKind::Escalating)` short-circuit means a phase configured with `RunCommand`/`LlmVerifier` never actually runs that hook against canonical bytes when the strategy is escalating retry. PRD requirement and design §6 are violated. Drop the unconditional skip; only short-circuit when the strategy already produced a passing `VerifyOutcome` of the same hook kind, and document the contract.
- **F2 (high) — Per-attempt `<phase>.started.<n>` markers never emitted for retries** (`src/phase_runner.rs:196`). `start_attempt` is called once at attempt 0 only. Design §3.1 + PRD requirement 5 (resumability from disk) require a marker per rung/branch. Either thread an attempt-callback into strategies so they write `markers.write_started(phase, n)` per rung, or explicitly amend the design + resumability story to admit phase-level coarseness. Current state hides mid-rung crashes from on-disk recovery.
- **F3 (medium) — `parallel + all_pass` integration test passes for the wrong reason** (`tests/phase_runner_integration.rs:163-192`). Missing backend resolves to `BackendNotFound` → `strategy_failed` before the aggregator runs, so the `all_pass collects failures` assertion never exercises the aggregator path. Register both backends, have the second return a `BackendError` from `query()`, and assert `error_class() == "aggregator_failed"` with branch indices in the failed-marker reason.
- **F4 (medium) — Missing PRD acceptance test for `parallel + concat + any_fail`** (`tests/phase_runner_integration.rs`). PRD acceptance list explicitly calls out "three replicas, RunCommand verifier, `review.completed`." Add the positive case plus a negative variant where one branch fails and surfaces `aggregator_failed`. PRD acceptance is in-scope for CLO-292.
- **F6 (medium) — `winning_attempt` returns a misleading index for parallel strategies** (`src/phase_runner.rs:298-304`). `rposition` over branches yields whatever happened to land last; the manifest's `attempt` field can disagree with the branch whose bytes are actually on disk (especially Vote/AnyFail which use `first_success_bytes` forward-iteration vs. `Aggregator::First` reverse-iteration). Have `dispatch::canonical_bytes` return `(Vec<u8>, usize)` and pass that authoritative index to `commit_success`.

## Out of Scope / Deferred
- **F5 (medium) — `archive_failed_attempt` writes only `failure-summary.json`, no debris.** Design intent was richer post-mortem; deferring is acceptable if a follow-up Linear issue is filed against M5 to either copy per-attempt outputs into `attempts/<phase>/<n>/` or document where the strategy already wrote them.
- **F7 (low) — Marker-write failures during terminal-failure paths silently swallowed via `let _ =`.** Add at minimum a `tracing::error!` log; can land as a small follow-up.
- **F8 (low) — Synthesized `VerifyContext` (`stderr=None`, `exit_code=None`, `duration=ZERO`) is a contract drift.** Either re-issue the hook against the artefact path or document the runner-level invariant on `VerifyContext`. Defer with a doc-comment for now.
- **F9 (low) — Always calling `.with_pass_failure_context(false)`.** Cosmetic; safe to land as-is or fix in a one-line follow-up.
- **F10 (low) — Wasted aggregator re-runs in `dispatch::canonical_bytes` for Vote/LLMJudge/AnyFail.** Performance cleanup; not blocking.

## False Positives / Tooling Artifacts
- Both Codex and Gemini reviewer failures are tooling artifacts (broken heredoc quoting in the wrapper scripts under `.pi/` — the `cat <<EOF ... EOF` block sits inside a `$(...)` capture that the parent shell can't parse). They are not findings about the diff itself; they should be tracked as a separate fix to the validation tooling so that future runs aren't single-reviewer (Claude-only).

## Recommendation
PROCEED_WITH_FIXES. Bounded fix iteration before opening PR: (1) F1 — remove the `EscalatingRetry` verify-skip and gate only on a matching prior `VerifyOutcome`; (2) F2 — either emit per-attempt `started.<n>` markers via a strategy callback or amend the design doc + resumability story to make phase-level markers explicit; (3) F3 — rewrite the `all_pass` test so the aggregator actually folds branch failures; (4) F4 — add the missing `parallel + concat + any_fail` PRD acceptance test (positive + negative); (5) F6 — return the selected attempt index from `dispatch::canonical_bytes` and thread it into `commit_success`. Defer F5/F7/F8/F9/F10 as M5 follow-ups (file Linear sub-issues). Separately, fix the Codex/Gemini wrapper heredoc quoting so the next validation pass isn't single-reviewer.

## Re-validation

Single bounded fix iteration applied in commit `5c5e994`:

- F1 fixed: PhaseRunner no longer skips runner-level verification for `EscalatingRetry`; the canonical bytes are verified when `cfg.verify != None`.
- F2 fixed for the current PhaseRunner contract: the runner now writes `started.<n>` markers for every attempt represented in `StrategyOutput`, so completed retry runs surface each recorded attempt in markers.
- F3 fixed: the `all_pass` integration test now registers both targets and uses a backend query error so the aggregator folds a real branch failure and returns `aggregator_failed`.
- F4 fixed: added `parallel + any_fail` three-replica positive and negative coverage; the negative path maps inline `any_fail` rejection to `aggregator_failed`.
- F6 fixed: `dispatch::canonical_bytes` now returns `(bytes, selected_attempt)` and `commit_success` uses that selected attempt for manifest attribution.

Validation after fix:

- `cargo test -q --test phase_runner_integration` passed.
- `make check` passed after the fix iteration.

No additional Must Fix items remain for this bounded validation cycle. Deferred items F5/F7/F8/F9/F10 remain non-blocking follow-ups per the synthesis.
