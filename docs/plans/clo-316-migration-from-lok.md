# Plan: CLO-316 migration-from-lok.md

## Context
- **Design:** `docs/designs/clo-316-migration-from-lok.md`
- **Discovery:** `docs/discovery/clo-316.md`
- **PRD:** `docs/prds/clo-316-migration-from-lok.md`
- **Linear:** https://linear.app/cloud-ai/issue/CLO-316/t-047-docsmigration-from-lokmd

## Summary

This is a documentation-only migration-task that creates a canonical `docs/migration-from-lok.md` guide for legacy `lok` users and adds one discoverability link from `README.md`. No source code or runtime behavior changes are required.

## Sub-tasks

### ST1 Write migration guide: concept + command mapping
**Files:** `docs/migration-from-lok.md`
**Acceptance:** `cargo run --bin loker -- --help >/tmp/loker-help.txt` and `loker --help` remain unchanged for documented command mappings; the new doc includes a valid one-page side-by-side mapping with explicit compatibility notes.
**Estimate:** S

Create `docs/migration-from-lok.md` with:
1. **Concept mapping** table covering workflows, phases, backends, run artefacts, and config locations.
2. **Command translation** table for `lok` (`ask`, `hunt`, `audit`, `diff`, `lok.toml`, `.lok/workflows/`) versus `loker` equivalents.
3. **Breaking changes / non-ported** section with concrete rationale.
4. **Deprecation window** statement aligned to existing config-rename policy wording.

### ST2 Capture source-of-truth command signatures and validation evidence
**Files:** `docs/migration-from-lok.md`, command transcripts
**Acceptance:** For each loker command shown in ST1, `loker <command> --help` output matches documented shape.
**Estimate:** S

1. Run `loker --help` and collect the top-level command list.
2. Run `loker <command> --help` (for each documented command) and validate syntax/order vs the migration table.
3. Add a short `Verification appendix` to the document showing command examples and what was validated.

### ST3 Add migration discoverability link to README
**Files:** `README.md`
**Acceptance:** `grep -n "migration-from-lok" README.md` finds exactly one migration callout line, and link resolves to existing file.
**Estimate:** XS

1. Add one concise link near docs-oriented sections: `Migrating from lok? See [docs/migration-from-lok.md](docs/migration-from-lok.md)`.
2. Preserve existing headings/structure; avoid reformatting unrelated prose.

### ST4 Manual compatibility check for legacy entrypoints
**Files:** `docs/migration-from-lok.md`
**Acceptance:** Compatibility section explicitly notes current support for `lok.toml` and `.lok/workflows/` and cites reason for deferred config-rename.
**Estimate:** XS

1. Validate from source of truth that these paths are still expected by current code/docs (`src/main.rs`, `CLAUDE.md`, existing docs references).
2. Document any residual uncertainty in a clearly marked follow-up note rather than guessing behavior.

### ST5 Gate and verification sweep
**Files:** `README.md`, `docs/migration-from-lok.md`
**Acceptance:** `make check` passes; all links in the new migration page and README entry resolve locally.
**Estimate:** S

1. Run `make check` to keep baseline PR gate current.
2. Manually verify markdown links in `README.md` and `docs/migration-from-lok.md`.
3. Confirm review checklist in design ACs is satisfied end-to-end.

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Risks

- Command examples may drift as CLI evolves: lock migration table to current `--help` output and avoid inferred syntax.
- The PRD leaves deprecation window duration unspecified: if upstream config-rename milestone scope changes, update only the compatibility section.
- Legacy behavior claims may become stale if `lok.toml` or `.lok/workflows` semantics change before merge.
