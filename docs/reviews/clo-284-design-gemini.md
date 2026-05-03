# Design Review: CLO-284

**Reviewer**: Gemini 2.5 Pro
**Reviewed**: 2026-05-02
**Pipeline**: lok design-review (manual invocation)
---

## 1. Completeness Check
All required sections (Problem, Goals, Architecture, Public API, Test plan, Migration/Rollout, Open questions) are present and well-detailed. The design is comprehensive, covering data structures, module layout, API surface, and a thorough test plan. It directly addresses the problem statement from the PRD (FR-21, FR-23c) and the roadmap task T-025.

## 2. Architecture Assessment
**Strengths**:
- **Atomic Writes**: The reuse and extraction of the existing `atomic_write` primitive from `src/manifest.rs` is an excellent choice. It avoids code duplication and ensures the same crash-safety semantics are applied to both manifests and status markers, which is critical for resumability.
- **Testability**: The design correctly identifies the need for a `Clock` trait to allow for deterministic testing of time-sensitive logic like heartbeats and staleness checks. This is a mature pattern that will prevent flaky tests.
- **Clear Separation of Concerns**: The proposed module layout (`run_state/{atomic,markers,heartbeat,order}`) cleanly separates the different responsibilities (atomic file operations, marker writing, heartbeat management, and state transition logic). This makes the code easier to understand and maintain.
- **State Machine Enforcement**: The `PhaseOrderGuard` is a strong addition. It enforces the correct sequence of operations (started -> artefact -> manifest -> completed) at the type-system and state-machine level, preventing logical errors in the phase runner implementation.

**Concerns**:
- The `next_attempt` helper's implementation reads all `*.started` marker files to find the max attempt number. For phases with many retries, this could become an I/O bottleneck. While acceptable for v0, it's worth noting that when attempt directories land (T-027), this logic should be updated to rely on directory listing rather than file parsing for better performance. The design correctly flags this in its open questions.

## 3. Alignment with Handoff & Roadmap
The design aligns perfectly with the project's goals and active work.
- It directly implements task T-025 from `docs/plans/001-implementation-roadmap.md`.
- It fulfills the crash-safety and resumability requirements laid out in `docs/prd/2026-04-25-loker.md` (FR-21, FR-23c).
- The approach of extracting a shared `atomic_write` helper follows the "don't mutate-in-place" intent from `docs/handoff.md` by refactoring common code into a new, shared module rather than exposing private internals.
- The work is additive and fits within the M1/M2 scope of building out the core strategy and run-state primitives. The `CLAUDE.md` file indicates M4 is active, which means this foundational work from the plan is being implemented now.

## 4. Security Review
The security posture is sound for this feature's scope.
- The use of `atomic_write` (tmp file in the same directory, fsync, rename) is the correct pattern to prevent data corruption from crashes, which is a form of integrity protection.
- Marker files contain metadata (hostname, PID) for forensics but no secrets.
- There is no direct user input that could lead to path traversal; paths are constructed internally from the phase name.
- The design does not introduce any new dependencies or network-facing surfaces.

## 5. Implementation Concerns
The implementation plan is solid and phased logically.
- The test plan is exceptionally thorough, covering unit tests, boundary conditions (`is_stale_boundary_exact_ttl`), crash simulation (`atomic_rename_crash_between_tmp_and_rename`), and state transitions. The 17 specified tests provide excellent coverage.
- The rollout plan for extracting `atomic_write` is safe, ensuring existing manifest tests continue to pass.
- The `PhaseOrderGuard`'s debug-panic/release-log behavior is a standard and effective Rust pattern for enforcing invariants during development without crashing production builds.

## 6. Concurrency & Async
- The `HeartbeatWriter` is correctly designed as a non-blocking `tokio` task using `tokio::time::interval`, which is the idiomatic way to handle periodic work.
- The use of `atomic_write` makes each file operation atomic from the filesystem's perspective, which is safe for concurrent writers to different files, as covered by test #15.
- The design acknowledges a potential issue with `PhaseOrderGuard` lifetime across `async` boundaries, placing responsibility on the caller. This is a reasonable trade-off for simplicity, but the phase runner implementation (T-028) will need to be careful about how it manages the guard's state.

## 7. Blind Spots
The design is very thorough and the "Open questions" section proactively addresses most potential blind spots.
- **Question 1 (Heartbeat cancellation)**: The decision to exit silently on a deleted markers directory is reasonable. The run is gone, so the heartbeat's job is done. Retrying on temporary I/O errors (like disk full) is also a good default. The design correctly identifies the trade-offs.
- **Question 2 (PhaseOrderGuard storage)**: The standalone struct approach is simpler. Storing it inside `MarkerWriter` would require `MarkerWriter` to become a `&mut self` state machine, which would complicate its use across the phase runner. The current design is preferable.
- **Question 5 (Writer PID type)**: Acknowledging the `u32` PID limit on Windows is good. For the target platforms (macOS/Linux), `u32` is sufficient. This is an acceptable v0 limitation.

The document shows a high degree of foresight.

## 8. Verdict
APPROVE

## 9. Actionable Feedback
The design is excellent and ready for implementation. No revisions are required. The following are minor suggestions for the implementation phase:
1.  **`next_attempt` performance**: When implementing, add a `// TODO(CLO-XXX)` comment in the `next_attempt` function pointing to the future task (T-027) for switching to attempt-directories to improve performance.
2.  **`HeartbeatWriter` error logging**: Ensure that when `atomic_write` fails within the heartbeat loop (e.g., disk full), the error is logged with `tracing::warn!` or `error!` so that it's visible to operators, as the current design implies ("log the error and continue").
3.  **Marker Filename Suffix**: The design uses marker filenames like `design.started`. While clear, this could lead to collisions if a phase is ever named `design.started`. A safer, albeit more verbose, convention might be `design.marker.started`. This is a minor point, and the current approach is acceptable, but it is worth a brief consideration before implementation hardens.
