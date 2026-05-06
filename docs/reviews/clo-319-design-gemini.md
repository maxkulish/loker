YOLO mode is enabled. All tool calls will be automatically approved.
YOLO mode is enabled. All tool calls will be automatically approved.
Ripgrep is not available. Falling back to GrepTool.
## 1. Completeness Check
All sections are present and well-detailed. The document provides a clear problem statement, concrete goals and non-goals, a logical architecture, a well-defined public API, and a comprehensive test and rollout plan. The inclusion of open questions demonstrates a thorough design process.

## 2. Architecture Assessment
**Strengths**:
- **Clear Separation of Concerns**: The design introduces `PhaseLock` as a distinct primitive from the existing `RunLock`, correctly identifying that run-level and phase-level locking have different requirements. This is an additive and non-disruptive change.
- **Robust Locking Strategy**: Relying on OS-level advisory locks (`fs2::FileExt::try_lock_exclusive`) as the ultimate source of truth is a sound and robust approach. The stale-body inspection is a sensible optimization to provide better error messages without compromising the correctness of the lock itself.
- **Observability**: Persisting a human-readable JSON body (`PhaseLockBody`) in the lock file is an excellent feature. It allows for easy debugging and enables other tools like `loker ls --blocked` to inspect state without needing to acquire the lock.
- **Atomic Operations**: Reusing the existing `atomic_write` helper for the lock body ensures crash safety for state persistence, consistent with the patterns established in `docs/run-state.md`.

**Concerns**:
- The proposed public API for `PhaseLock` uses standard synchronous file I/O from `std::fs` and `fs2`. However, it is integrated into `ResumeRunner::run_phase`, which is an `async` function. Performing blocking I/O on an async task without offloading it to a blocking-aware thread pool (e.g., via `tokio::task::spawn_blocking`) can stall the entire Tokio runtime, leading to performance degradation and potential deadlocks.

## 3. Alignment with Handoff & Roadmap
The design aligns perfectly with the project's established practices and roadmap.
- **Handoff Document**: It respects the "don't mutate-in-place" principle by adding a new, self-contained module. The emphasis on a thorough test plan, including unit tests against mocks (or filesystem states) and separate integration tests, is consistent with `docs/handoff.md`.
- **Roadmap**: The design correctly identifies itself as T-050 in the implementation plan (`docs/plans/001-implementation-roadmap.md`). It correctly states its dependencies (T-004, T-048) and the tasks it blocks (T-051, T-053), fitting neatly into the Slice C (HITL & UI) work.

## 4. Security Review
The design demonstrates a good security posture for its scope.
- **Symlink Hardening**: The explicit mention of validating that `run_dir` is a real directory and sanitizing the phase name against path separators addresses potential symlink and traversal attacks. This is consistent with the mitigations outlined in the UI threat model (`docs/security/2026-04-25-ui-threat-model.md`).
- **Stale Lock Takeover**: The design correctly identifies that the OS lock is the authoritative guard, mitigating the risk of a malicious process creating a fake stale lock file to bypass the mechanism. This aligns with the "Stale-lock takeover" (T5) mitigation strategy in the threat model.
- **PID Reuse**: The design acknowledges the risk of PID reuse on Unix systems but correctly assesses it as low-impact given the short TTL and single-machine scope.

## 5. Implementation Concerns
The implementation plan is solid. The test plan is particularly strong, covering happy paths, concurrency, error conditions, and platform-specific behavior. The phased rollout is logical.
- The proposed API in `src/run_state/phase_lock.rs` is clean and ergonomic.
- The error mapping from `PhaseLockError` to `ResumeError` is well-considered, providing specific, user-friendly errors (`PhaseLocked`) for common cases while forwarding others.
- The open question regarding phase-name normalization should be resolved. It would be safer for `PhaseLock` to perform its own normalization or validation rather than assuming upstream sanitization, preventing potential issues with backend- or user-defined phase names.

## 6. Concurrency & Async
This is the area with the most significant concern. The `PhaseLock::acquire` function, as designed, will perform blocking file system operations (open, stat, read, `try_lock_exclusive`). When called from an `async` function like `ResumeRunner::run_phase`, these blocking calls will pause the thread, preventing other async tasks from making progress. This should be addressed by wrapping the blocking logic in `tokio::task::spawn_blocking`.

## 7. Blind Spots
The design is very thorough, and the "Open Questions" section proactively addresses most potential blind spots.
- **Filesystem Permissions**: The design doesn't explicitly discuss how `locks/` directory and file permissions are handled. While likely inherited, it's worth ensuring that the lock files are created with appropriate permissions to be readable by other `loker` processes run by the same user but not world-writable.
- **Lock File Cleanup**: The decision to leave lock files on disk after release is reasonable, but it could lead to an accumulation of files in the `locks/` directory over many resumes. A periodic cleanup mechanism for very old, stale lock files (e.g., older than a few days) might be worth considering in a future iteration, though it is not a v0 requirement.

## 8. Verdict
APPROVE_WITH_SUGGESTIONS

The design is well-researched, architecturally sound, and aligns with project goals. The core ideas are excellent. The only required change is to address the blocking I/O issue in the async context, which is critical for correctness in a `tokio`-based application.

## 9. Actionable Feedback
1.  **[Blocker] Offload Blocking I/O**: The `PhaseLock::acquire` and `PhaseLock::inspect` methods must be converted to `async fn`. All internal file system operations (opening files, reading metadata, acquiring the lock, writing the body) must be wrapped in `tokio::task::spawn_blocking` to avoid stalling the async runtime.
    
    *Example:*
    ```rust
    // in src/run_state/phase_lock.rs
    
    pub async fn acquire(
        run_dir: PathBuf, // Use owned types for spawn_blocking
        phase: String,
        //...
    ) -> Result<Self, PhaseLockError> {
        tokio::task::spawn_blocking(move || {
            // All original synchronous code for acquiring the lock goes here
            // ...
        }).await.map_err(|e| /* join error */)?
    }
    ```
    
2.  **[Recommended] Add Phase Name Validation**: Add validation logic inside `PhaseLock::acquire` to reject phase names containing filesystem-unsafe characters (`/`, `..`, `\`, etc.), returning `PhaseLockError::InvalidPhaseName`. This makes the module more robust and self-contained.

3.  **[Consider] Clarify Lock File Permissions**: Briefly document or ensure that files and directories created under `locks/` have permissions (`0o600` for files, `0o700` for the directory) that restrict access to the owner user, as a matter of good practice.
