# loker - v1 backlog (draft)

Holding pen for follow-ups deferred during v0. Not a roadmap. v1 scope
will be triaged from this list once a v1 milestone is opened.

## Source: deferred review findings

Items left as "post-v0" or "out of scope" during v0 design / validation
reviews. Each entry cites the workflow YAML or review doc that filed it.

| Origin | Kind | Item | Notes |
|--------|------|------|-------|
| CLO-301 (resume runner) | design nit | F3 - binary artefact handling: tighten `Vec<u8>` ergonomics in resume path | Carried forward as implementation-time refinement; not blocking. See `docs/status/clo-301-workflow.yaml` design.flagged_suggestions. |
| CLO-310 (resume CLI) | descoped test | CLO-325 - runner-level resume round-trip integration test (SIGTERM mid-phase → `loker resume` → assert no duplicate work) | Filed per validation gate F1 when CLO-310 shipped. Structural blockers cleared (CLO-301 wired the phase runner). Value is mid-tier: covers OS-level signal/atomic-write race interactions that unit tests miss, but the failure mode it guards against is "user re-runs a phase", not data loss. Schedule only if a real resume bug surfaces in usage, or before any multi-user/CI scenario opens. |
| CLO-324 (threat-model suite) | out-of-scope | F6 - external penetration test | Explicit non-goal in PRD M11; revisit if multi-tenant deployment ever becomes a target. |
| CLO-324 (threat-model suite) | out-of-scope | F7 - hardening for multi-tenant / shared-host deployment | Same posture as F6. v0 ships single-user localhost-only. |

## Source: PRD §6 (out-of-scope / future phases)

PRD `docs/prd/2026-04-25-loker.md` §6 enumerates items intentionally
deferred from v0. Not duplicated here - read the PRD as the source of
truth and pull items into a v1 plan when prioritised.

## How to use

1. When opening a v1 milestone, walk this list + PRD §6 and decide what
   makes the cut.
2. Move accepted items into `docs/plans/003-v1-roadmap.md` (or whatever
   the v1 plan file is named) with proper task IDs.
3. Drop entries from this draft as they get promoted or rejected.
