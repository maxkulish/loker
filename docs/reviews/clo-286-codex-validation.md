**Findings**

1. **MSRV break: `ErrorKind::CrossesDevices` requires Rust 1.85, but crate declares 1.80.**  
   [Cargo.toml](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/Cargo.toml:5) sets `rust-version = "1.80"`, while [attempt_dir.rs](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/src/run_state/attempt_dir.rs:51) uses `io::ErrorKind::CrossesDevices`, stabilized in Rust 1.85. The `#[allow(clippy::incompatible_msrv)]` only suppresses a lint on newer compilers; it will not compile on the declared MSRV.

2. **`promote_to_canonical` does not fsync the destination parent after directory rename.**  
   [attempt_dir.rs](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/src/run_state/attempt_dir.rs:48) returns after `std::fs::rename` without syncing the canonical parent dir. D3’s durability model requires parent fsync after rename; otherwise a power loss can leave `markers/<phase>.completed` durable while the canonical phase directory entry is not.

3. **Latest pointer behavior diverges from both design and plan.**  
   The design/plan specify `run_dir/<phase>/latest -> ../attempts/<phase>/<n>/` and JSON path `attempts/<phase>/<n>/` ([design](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/docs/designs/clo-286-attempt-directories.md:49), [plan](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/docs/plans/clo-286-attempt-directories.md:32)). The implementation switches to `.` / `{phase}/` when the attempt dir no longer exists ([latest.rs](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/src/run_state/latest.rs:39), [latest.rs](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/src/run_state/latest.rs:63)). That changes the public contract and creates a self-referential `design/latest -> .` symlink after promotion.

4. **Design-scoped producer and manifest wiring is not implemented.**  
   The design explicitly includes producer wiring and passing `Some(attempt_number)` into manifest entries ([design](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/docs/designs/clo-286-attempt-directories.md:38), [design](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/docs/designs/clo-286-attempt-directories.md:44)). In `src/`, there are no production callers of `AttemptDir`, `LatestPointer`, or `ManifestEntry::from_payload`; only tests construct attempt-aware entries. So the branch adds primitives, but does not deliver the design’s “manifest attempt field populated by producers” behavior.

5. **Tests encode weaker or contradictory contracts.**  
   [tests/run_state_attempts.rs](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/tests/run_state_attempts.rs:60) says attempt-0 debris does not exist in the failure test, then creates it after attempt 1, which does not validate the planned “failed attempt leaves attempt-0 untouched before retry” flow. The manifest test only round-trips a manually constructed entry ([tests/run_state_attempts.rs](/Users/mk/Code/orchestrator/loker--feat-clo-286-attempt/tests/run_state_attempts.rs:84)), not producer population.

I reviewed `git diff main...HEAD` and the requested files. I did not run `make check`; this environment is read-only, and the MSRV issue is already visible statically.

## Verdict
rework
