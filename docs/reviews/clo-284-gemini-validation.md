# Gemini design / implementation review - CLO-284

## Context
- **Branch**: `feat/clo-284-phase-status-markers`
- **Design**: `docs/designs/clo-284-phase-status-markers.md`
- **Plan / Spec**: `docs/plans/clo-284-plan.md`

## Findings
### F1 [minor] Heartbeat tests use real time instead of the provided FakeClock
**Where:** `src/run_state/heartbeat.rs` and `tests/run_state_markers.rs`
**What:** The design correctly specified a `Clock` trait with a `FakeClock` implementation for deterministic testing of time-sensitive logic. While `FakeClock` was implemented, the tests for `HeartbeatWriter` in `tests/run_state_markers.rs` use `tokio::time::sleep` and real wall-clock time, making them potentially flaky and slow.
**Why it matters:** Tests that rely on real time can fail intermittently in CI due to system load. Using the `FakeClock` would make the tests instant and fully deterministic.
**Suggested fix:** Refactor `HeartbeatWriter::spawn` to accept a generic `impl Clock`. Inject `RealClock` in production code and `FakeClock` in the tests. The tests can then call `fake_clock.advance()` to simulate time passing without actually sleeping.

### F2 [nit] Release-mode behavior of PhaseOrderGuard is untested
**Where:** `src/run_state/order.rs`, `tests/run_state_markers.rs`
**What:** The `PhaseOrderGuard` correctly uses `#[cfg(debug_assertions)]` to panic on invalid state transitions during development, and logs an error in release builds. The `phase_order_guard_invalid_skip` test correctly verifies the panic in debug mode, but there is no corresponding test to verify that the error is logged (and does not panic) in a release build.
**Why it matters:** This is a minor gap in test coverage. While the implementation is simple, an explicit test would guarantee the release-mode behavior is preserved during future refactoring.
**Suggested fix:** This is a low-priority nit. The ideal fix would be a test that runs with `--release`, but this can complicate the test runner. Acknowledging this as an accepted low risk is also fine.

### F3 [nit] Atomic write crash-safety test name is misleading
**Where:** `tests/run_state_markers.rs`
**What:** The test `atomic_rename_crash_between_tmp_and_rename` does not actually simulate a crash. Instead, it verifies the success case: that after a successful write, the final marker file exists and no temporary files are left behind.
**Why it matters:** The test name implies a higher level of fault-injection testing than is actually present. True crash simulation is very difficult in a unit test.
**Suggested fix:** Rename the test to something more accurate, like `atomic_write_leaves_no_temporary_files_on_success`, to better reflect its behavior. No logic change is needed, as relying on `tempfile::NamedTempFile::persist`'s atomicity is sufficient for this component.

## Strengths
- **Design Fidelity:** The implementation adheres exceptionally well to the design document. The module structure, data types, public API, and logic all directly map to the spec. The inclusion of suggestions from the design review (like the `TODO(T-027)` comment) is excellent.
- **Code Quality:** The code is clean, idiomatic, and well-structured. The separation of concerns into `atomic`, `markers`, `heartbeat`, and `order` modules makes the new `run_state` crate easy to understand and maintain.
- **Test Coverage:** Despite the minor findings above, the overall test coverage is very strong. The suite covers round-trips, error conditions, concurrency, and boundary cases for the `is_stale` logic, providing high confidence in the implementation's correctness.

## Verdict
approve_with_changes

The implementation is robust, well-tested, and almost perfectly aligned with the comprehensive design document. The core logic is sound. The recommended changes are minor and focused on improving test quality and determinism rather than fixing logic bugs. This change is safe to merge after addressing the minor test-related findings.
