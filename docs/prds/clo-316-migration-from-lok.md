# PRD: Migration from lok to loker — command and concept mapping

**Linear:** CLO-316  
**Milestone:** M9  
**Roadmap:** T-047 (`docs/migration-from-lok.md`)  
**Dependencies:** T-045 (`CLO-314`, README rewrite)

## Goal

Create a migration guide that lets existing lok users map their known workflow concepts and commands to the equivalent loker primitives and commands without reading source code.

## Scope

- Produce `docs/migration-from-lok.md` with:
  - Side-by-side concept mapping (workflow/phases/backends, run artefacts, configuration model).
  - Command translation table: legacy lok usage vs loker equivalent.
  - Breaking changes and compatibility notes.
  - Notes on features intentionally not ported from lok and why.
  - Deprecation-window statement for legacy entrypoints and configuration paths.
- Verify all command examples on both sides before publication.
- Add a short link from the README to the new migration doc once complete.

## Acceptance criteria

1. A lok user can map `ask/hunt/audit/diff` and config/workflow concepts to loker equivalents in one page.
2. All command examples listed in the migration doc are real and verified.
3. The doc calls out every current `lok`-only behavior and why it is not ported.
4. Deprecation window note exists for compatibility assumptions (e.g., `lok.toml`, `.lok/workflows`).

## Non-goals

- Auto-migration tooling or conversion scripts.
- Guaranteeing complete semantic parity with all historical lok behaviors.

## References

- `docs/old-readme.md` (legacy migration context + compatibility notes)
- `README.md` (current user-facing command surface)
- `docs/plans/001-implementation-roadmap.md` (M9 scope and task context)
- `docs/prd/2026-04-25-loker.md` (§9 Rollout)
- Linear issue `CLO-314` for foundational docs dependency
