# Pre-PR validation: clo-320

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-07
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [major] Missing integration tests required by plan ST6
**Where:** `tests/hitl_server.rs`
**What:** The implementation plan (`docs/plans/clo-320-per-gate-fallback-axum-server.md` ST6) explicitly enumerates required tests: `concurrent_post_races_return_423`, `server_url_printed_to_stdout`, `timeout_auto_approves_without_human`, `high_severity_blocks_indefinitely`. None of these are present. Only six tests cover the happy paths and one 409 race; the 423 lock-contention path, stdout URL contract, and timeout/severity policy paths are entirely untested.
**Suggested fix:** Add the four missing tests. For 423, fire two concurrent POSTs after grabbing the `PhaseLock` from a separate thread to force contention. For stdout, capture stdout in `HumanVerifier` invocation. For timeout, set `HumanTimeoutPolicy` with a short duration and assert `auto_approve_after_timeout`. For high-severity, assert no auto-decision past timeout.

### F2 [major] `decision_options` format mismatch with schema
**Where:** `src/strategy/verify/human_verifier.rs:~343` (the `format!("{:?}", d).to_lowercase()` mapping when building `GateConfig`)
**What:** `HumanDecision` derives `serde(rename_all = "snake_case")` so the canonical wire form is `comment_only`, but `Debug + to_lowercase()` produces `commentonly`. The pending JSON schema and the rest of the codebase use snake_case; the mismatch is currently masked because `decision_options` is only used internally by the renderer, but it will diverge from `pending/<phase>.json` and any future consumer of `GateConfig.decision_options`.
**Suggested fix:** Use serde to serialize: `serde_json::to_value(&d).ok().and_then(|v| v.as_str().map(str::to_owned))` or a small explicit match. Add a unit test asserting `comment_only` round-trip.

### F3 [major] Server shutdown race in `ServerHandle::outcome`
**Where:** `src/hitl_server/one_shot.rs:28-39`
**What:** The POST handler calls `tx.send(ServerOutcome::Decided)` *before* returning `StatusCode::OK`. The parent task awaits `decision_rx`, then immediately `self.task.abort()`s the axum server. Under load (or under any scheduling delay between handler return and TCP write flush), the abort can preempt response delivery and the client sees a connection reset rather than 200. Current tests pass because reqwest reads the buffered response in microseconds, but this is timing-dependent and will flake on slower CI or when the daemon (M11) reuses these routes.
**Suggested fix:** Either (a) send the outcome from a `tokio::spawn` after a short `axum::serve(..).with_graceful_shutdown(...)` so the server drains in-flight responses, or (b) have the handler return first and the route fire the oneshot via a middleware/`on_response` hook. Option (a) is closer to the existing structure: replace `task.abort()` with a `graceful_shutdown` signal.

### F4 [minor] HTML textarea asymmetry — prompt summary leaks into approve form
**Where:** `src/hitl_server/routes.rs:184-192` (`render_html`)
**What:** The approve `<textarea>` has its content set to `{comment}` (which is `prompt_summary`), while the reject `<textarea>` is empty. A reviewer who clicks Approve will inadvertently submit the prompt summary text as their `comment` unless they manually clear it, polluting the audit trail.
**Suggested fix:** Move `prompt_summary` to a separate `<p>` or `<pre>` block above both forms (or into the placeholder, not the value). Leave both textareas empty.

### F5 [minor] Sync `PhaseLock::acquire` blocks the tokio worker
**Where:** `src/hitl_server/routes.rs:122-128`
**What:** `PhaseLock::acquire` is sync (fcntl-based via fs2). For the one-shot fallback this is fine — single client, brief lock — but the comment "the oneshot server has exactly one target, so brief blocking is fine" will silently break the M11 daemon goal of reusing these routes. A long-held cross-process lock can stall the tokio runtime.
**Suggested fix:** Wrap the acquire in `tokio::task::spawn_blocking`, or add an async `try_acquire` variant before reusing in the daemon. Document the constraint at the route level.

### F6 [info] Missing `Cancelled` outcome wiring
**Where:** `src/hitl_server/one_shot.rs:34-38`
**What:** `decision_rx.await` returning `Err` (sender dropped) maps to `ServerOutcome::Cancelled`. This is reachable only if the axum task panics or is aborted before any handler fires; in normal cancel-via-`ServerHandle::cancel` the receiver itself is dropped, never observed. The branch is correct but currently dead in tests.
**Suggested fix:** Add a unit test that drops the sender (e.g., panics in a custom handler) and asserts `Cancelled`. Optional.

## Verdict

**approve_with_changes**

The implementation is structurally sound: module split (`mod.rs` / `routes.rs` / `one_shot.rs`) matches the design's reuse goal, no schema changes were introduced, `fallback_server` defaults to `false`, `atomic_write` + `PhaseLock` ordering is correct, recursion via `Box::pin` correctly re-enters `verify_with_report`, and existing tests were updated rather than weakened. The blockers to merge are the test-coverage gap against plan ST6 (F1) and the `decision_options` format bug (F2). F3 and F4 should also be fixed before this lands so the M11 daemon work doesn't inherit a flaky abort path or a confusing UX. None of the findings indicate scope creep or design regression.
