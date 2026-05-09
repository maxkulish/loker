# CLAUDE.md - loker

Status: **v0 shipped** (tag `v20260509.0.0`, 2026-05-09). M1-M11 complete; Slices A+B+C closed. No active milestone - awaiting v1 scope.

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
