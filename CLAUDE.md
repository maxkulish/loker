# CLAUDE.md - loker

Active milestone: **M1 - TensorZero backend**.

## Read first

- `docs/handoff.md` - project WHY/Intent/HOW, constraints, conventions
- `docs/plans/2026-04-25-m1-tensorzero-backend.md` - current task contract
- `/Users/mk/Work/investigations/sakana-fugu/loker-design.md` - canonical design

## Pre-merge gate

```bash
make check    # fmt + clippy + test
```

## Confirm before

- `make release` (auto-versions, tags, pushes, installs to `/usr/local/bin`)
- Anything destructive on shared state
