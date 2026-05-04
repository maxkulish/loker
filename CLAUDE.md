# CLAUDE.md - loker

Active milestone: **M6 - Reference workflow**. M1/M2/M3/M4/M5 shipped.

In progress:
- CLO-291 (Phase 7, T-037): M6 e2e on calculator spec.
- CLO-301 (Phase 6 follow-up): wire `ResumeRunner::execute()` end-to-end.

## Read first

- `docs/handoff.md` - project WHY/Intent/HOW, constraints, conventions
- `docs/plans/001-implementation-roadmap.md` - canonical task list, dependencies, status per phase
- `/Users/mk/Work/investigations/sakana-fugu/loker-design.md` - canonical design

## Guides

- `docs/guides/linear-mcp.md` - Linear MCP usage (project `CLO`, tool reference)

## Pre-merge gate

```bash
make check    # fmt + clippy + test
```

## Confirm before

- `make release` (auto-versions, tags, pushes, installs to `/usr/local/bin`)
- Anything destructive on shared state
