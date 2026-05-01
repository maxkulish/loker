# CLAUDE.md - loker

Active milestone: **M4 - Verify hooks**. M1/M2/M3 shipped.

Open in Phase 4: CLO-271 (RunCommand), CLO-273 (TestRunner, blocked by 271).

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
