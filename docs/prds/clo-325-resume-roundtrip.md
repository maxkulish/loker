# CLO-325 PRD: Runner-Level Resume Round-Trip Integration Test

## Overview

Add a proper pause/resume round-trip integration test that creates real phase markers (via the phase runner or a fixture), simulates interruption, resumes the run, and validates that completed phases are not re-executed.

## Requirements

### Functional

1. **Marker creation**: The test must create real phase markers (`.started`, `.completed`, `.failed`) in a temp run directory using the production `PhaseRunner` code path.
2. **Mid-phase interruption**: The test must simulate a mid-phase interruption, leaving a `.started.X` marker but no terminal marker for the interrupted phase.
3. **Resume planning**: `ResumePlanner` must correctly classify the completed phase as `Skip` and the interrupted phase as `Resume { next_attempt: N }`.
4. **Resume execution**: `ResumeRunner` must execute the interrupted phase to completion without re-executing the completed phase.
5. **Sentinel integrity**: After resume, the artefact from the completed phase must have the same mtime as before the resume (no duplicate work).

### Scenarios

| Scenario | Phase 1 | Phase 2 | Expected outcome |
|---|---|---|---|
| Kill mid-phase-2 | Completed.0 | Started.0 | Phase 1 skipped, phase 2 resumed at attempt 1 |
| Phase 2 failed then resume | Completed.0 | Failed (attempt 0) | Phase 1 skipped, phase 2 resumed at attempt 1 |
| All complete | Completed.0 | Completed.0 | All phases skipped |

### Non-functional

- Test must complete in < 1 second (no sleeps, no subprocesses)
- No external dependencies (no mock servers, no real backends)
- Test must be deterministic (no timing-dependent signal delivery)

## Success Criteria

1. All three scenarios pass consistently across CI runs
2. `make test` includes the round-trip test
3. No flaky failures after 100 repeated runs

## References

- CLO-310 (parent — resume surface + guard clause tests)
- CLO-301 (ResumeRunner wiring into phase runner)
- CLO-295 (resumability via status markers)
