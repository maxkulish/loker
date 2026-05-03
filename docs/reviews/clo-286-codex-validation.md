**Findings**

1. **High:** [src/run_state/attempt_dir.rs](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/src/run_state/attempt_dir.rs:59) falls back to recursive copy/remove for *any* `rename` error, not just cross-device. The design only permits fallback for cross-device rename, and this can silently merge an attempt into an existing non-empty canonical directory, leaving stale files while removing the attempt archive. That breaks the D3 atomic-promotion guarantee.

2. **High:** [src/run_state/latest.rs](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/src/run_state/latest.rs:59) does not match the latest-pointer contract. The design requires `latest` / `latest.json` to point at `attempts/<phase>/<n>/`, including the test-plan acceptance at [docs/designs/clo-286-attempt-directories.md](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/docs/designs/clo-286-attempt-directories.md:358). After promotion this implementation writes `"path": "design/"` instead. It also leaves any existing `latest` symlink in place when writing `latest.json`, and `resolve()` checks the symlink first, so a stale symlink can shadow the newer JSON pointer.

3. **Medium:** The design lists producer wiring and manifest-attempt population as in scope at [docs/designs/clo-286-attempt-directories.md](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/docs/designs/clo-286-attempt-directories.md:38), but the branch only proves `ManifestEntry::from_payload(..., Some(attempt), ...)` in a test. There is no `src/run_state/producer.rs` stub and no production code path that passes an actual attempt number into manifest entries. If this is intentionally deferred to T-028, the design/acceptance criteria should be tightened; as written, this branch is incomplete.

**Tests**

I attempted `CARGO_TARGET_DIR=/tmp/loker-target-clo286 cargo test --test run_state_attempts --test run_state_markers`, but this environment rejected target-dir creation with `Operation not permitted`. I did run `git diff main...HEAD` and reviewed the requested files.

## Verdict
rework
