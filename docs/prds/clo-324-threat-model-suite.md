# PRD: CLO-324 — Threat-Model Test Suite (M11 Close Gate)

| Field | Value |
|-------|-------|
| Task ID | CLO-324 |
| Linear | [CLO-324](https://linear.app/cloud-ai/issue/CLO-324) |
| Type | Development |
| Milestone | M11 |
| Author | Max Kulish |

## Goal

Lock in the security posture of the UI daemon and HITL surface with an automated threat-model test suite gating M11 close.

## Scope

1. **Threat-model document** (`docs/threat-model.md`): trust boundaries, in-scope assets, out-of-scope (e.g., shared multi-user hosts).
2. **Test cases** covering at minimum:
   - Loopback-only binding rejected from non-loopback interfaces.
   - Per-gate URL is unguessable (sufficient entropy in path/token).
   - Replay of an approval after the gate resolves is rejected.
   - Concurrent approval attempts honor advisory lock (T-050).
   - SSE connections reject cross-origin requests (CSRF defense).
   - Path traversal attempts on `/runs/:id` are rejected.
3. **Integration**: tests run as part of `make check`.

## Acceptance Criteria

- [ ] All threat-model tests pass on CI.
- [ ] Threat model doc reviewed and merged.
- [ ] Any failure listed above blocks M11 close.

## Non-Goals

- Penetration test by external party.
- Hardening for multi-tenant deployment (out of scope per design).

## Dependencies

- T-004 (run directory layout, done).
- T-054 (SSE tail-f, done).

## References

- Roadmap: `docs/plans/001-implementation-roadmap.md` Phase 12.
- Design threat model: `docs/security/2026-04-25-ui-threat-model.md`.
- PRD M11 security NFRs (§5 rows 197–198).
