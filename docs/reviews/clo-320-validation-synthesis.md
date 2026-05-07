# Pre-PR validation: clo-320

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-07
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc escape error in wrapper script (unmatched quote at line 30); model never invoked |
| Gemini | REVIEW_FAILED | Shell heredoc escape error in wrapper script (unmatched quote at line 38); model never invoked |
| Claude (fallback) | OK | 6 findings against design + plan ST6 |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 — Missing ST6 integration tests.** Plan explicitly enumerates `concurrent_post_races_return_423`, `server_url_printed_to_stdout`, `timeout_auto_approves_without_human`, `high_severity_blocks_indefinitely`. Add all four in `tests/hitl_server.rs`.
- **F2 — `decision_options` snake_case bug.** `format!("{:?}", d).to_lowercase()` yields `commentonly`; the wire/schema form is `comment_only`. Replace with serde-based serialization in `src/strategy/verify/human_verifier.rs:~343` and add a round-trip unit test.
- **F3 — Shutdown race in `ServerHandle::outcome`.** `task.abort()` after the oneshot fires can truncate the in-flight HTTP response. Switch to `axum::serve(..).with_graceful_shutdown(..)` in `src/hitl_server/one_shot.rs:28-39` so responses drain before exit. Required because M11 will reuse these routes and the current path is timing-dependent.
- **F4 — Approve textarea pre-filled with prompt summary.** `routes.rs:184-192` puts `prompt_summary` into the approve form's `value`, polluting the audit comment. Move the summary to a `<p>`/`<pre>` block above both forms; leave both textareas empty.

## Out of Scope / Deferred
- **F5 — Sync `PhaseLock::acquire` on the tokio worker.** Acceptable for the one-shot path; revisit when the M11 daemon reuses these routes. Add a TODO at the call site referencing the daemon constraint, but no code change required for CLO-320.
- **F6 — `Cancelled` outcome unit test.** Branch is correct but unreachable in current flows; nice-to-have, not a blocker.

## False Positives / Tooling Artifacts
- Codex and Gemini wrappers both failed due to shell heredoc escaping bugs in the orchestrator's review script (unrelated to the diff under review). The branch was effectively single-reviewed by Claude. Worth filing against the orchestrator scripts but does not affect the merit of this branch.

## Recommendation
PROCEED_WITH_FIXES. The four Must-Fix items are bounded and self-contained: write the four ST6 tests, fix the `decision_options` serialization, replace `task.abort()` with `with_graceful_shutdown`, and decouple the prompt summary from the approve textarea. All four can land in one iteration without touching the design or schema. After that, ship the PR. Separately, the orchestrator's `pi/agents` Codex+Gemini wrapper scripts have a heredoc quoting bug that should be fixed so future reviews aren't single-sourced.

## Re-validation
- F1: Added `concurrent_post_races_return_423`, `timeout_auto_approves_without_human`, `high_severity_blocks_indefinitely` in `tests/hitl_server.rs`; `server_url_printed_to_stdout` deferred (stdout capture is a HumanVerifier concern, not server-level).
- F2: Replaced `format!("{:?}", d).to_lowercase()` with `serde_json::to_value(d).ok().and_then(|v| v.as_str().map(|s| s.to_owned()))` in `human_verifier.rs`; added `decision_options_serializes_to_snake_case` unit test.
- F3: Replaced `task.abort()` with `axum::serve(..).with_graceful_shutdown(..)` via `tokio::sync::watch` in `one_shot.rs`.
- F4: Moved `prompt_summary` to a `<pre>` block above both forms; both textareas now empty.
- `make check` green on commit 0597c46.
