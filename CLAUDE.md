# CLAUDE.md - loker

Active milestone: **M7/M8 - Slice B (deployment + CLI surface)**. M1-M6 shipped.

Next pick: CLO-309 (Phase 9, T-040) - `loker run <workflow> [--spec] [--var] [--rerun phase=]`.
Parallel-OK: CLO-307 (T-038, docker-compose), CLO-308 (T-039, doctor TZ check), CLO-312 (T-043, `loker trace`), CLO-317 (T-048, HumanVerifier scaffold).

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
