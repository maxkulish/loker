# PRD: README rewrite — thesis, primitives, install, one-page example

**Linear:** CLO-314  
**Milestone:** M9 (Documentation)  
**Roadmap:** T-045 (Phase 10)  
**PRD reference:** `docs/prd/2026-04-25-loker.md` §M9

## Goal

Replace the current README with a v0 surface that explains loker's thesis, the three primitives, install steps, and a single end-to-end example a reader can copy-paste.

## Scope

- New top-level `README.md`:
  - One-paragraph thesis (Why loker, vs. lok).
  - Three primitives section (Backend / Strategy / Aggregator + verify hooks).
  - Install: `cargo install loker` + `make release` for source builds.
  - One-page example: `loker run design-doc-tdd --spec examples/specs/calculator.md`, expected output, where artefacts land.
  - Pointer to `docs/handoff.md`, the design doc, the PRD.
- Old README content is preserved under `docs/old-readme.md` if anything is salvageable.

## Acceptance criteria

1. README renders cleanly on GitHub.
2. Every command in the README is verified to work end-to-end.
3. Length is under one screen for the install + run section.

## Non-goals

- Multi-page docs site — belongs to a later milestone if it ever lands.
- Animated demos / asciinema recordings — text only for v0.

## Dependencies

- Phase 9 except T-044 — the CLI surface must be stable before the example can be promised.
  - ✅ CLO-309 (T-040, `loker run`) — shipped
  - ✅ CLO-310 (T-041, `loker resume`) — shipped
  - ✅ CLO-311 (T-042, `loker explain`) — shipped
  - ✅ CLO-312 (T-043, `loker trace`) — shipped

## References

- `docs/plans/001-implementation-roadmap.md` Phase 10 row T-045
- `docs/discovery/clo-314.md`
- Current README.md (to be replaced)
