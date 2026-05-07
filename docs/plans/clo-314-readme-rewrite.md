# Plan: CLO-314 — README rewrite: thesis, primitives, install, one-page example

## Context
- **Design:** `docs/designs/clo-314-readme-rewrite.md`
- **Discovery:** `docs/discovery/clo-314.md`
- **PRD:** `docs/prds/clo-314-readme-rewrite.md`
- **Linear:** https://linear.app/cloud-ai/issue/CLO-314/t-045-readme-rewrite-thesis-primitives-install-one-page

## Summary

This is a **documentation-only task** — no Rust source code changes. The deliverable is a rewritten `README.md` with the old content archived to `docs/old-readme.md`. The plan below captures the writing steps, archive step, and verification steps.

## Sub-tasks

### ST1 — Write new README.md (thesis + primitives)
**Files:** `README.md`
**Acceptance:** Thesis paragraph reads cleanly; three primitives section uses shipped status (not aspirational); under 50 lines for the top portion.
**Estimate:** S

Write the top half of the new README:
1. **Thesis paragraph** — one paragraph answering "What is loker?", "What gap does it fill?", "How is it different from lok?".
2. **Three primitives** — Backend / Strategy / Aggregator + VerifyHook subsections. Update "None ship yet" to "Shipped in M2-M4". Use inline `code` spans, not full Rust code blocks.
3. **What works today** — compact list of CLI commands (`loker run`, `loker doctor`, `loker trace`, `loker explain`, `loker resume`).

### ST2 — Write new README.md (install + one-page example)
**Files:** `README.md`
**Acceptance:** Install section is ~10 lines; the example command fits in one copy-paste block; the combined install + run section fits under one screen (~50 lines total).
**Estimate:** S

Write the bottom half:
1. **Install** — `cargo install loker` (or `--git` if not on crates.io) + `make release` for source builds. Note pre-v0 if on crates.io.
2. **One-page example** — `loker run design-doc-tdd --spec examples/specs/calculator.md` with expected output. Show artefacts: `runs/<id>/trace.jsonl`, `runs/<id>/manifest.json`.
3. **Roadmap & references** — compact bullet links to `docs/handoff.md`, `docs/plans/001-implementation-roadmap.md`, the design doc, `docs/prds/`.

### ST3 — Write new README.md (license + lineage)
**Files:** `README.md`
**Acceptance:** License and lineage section preserved but shortened. Both copyright lines present.
**Estimate:** XS

1. **License** — "MIT — see LICENSE. Fork of ducks/lok." Both copyright holders named.
2. Remove the full roadmap table (it duplicates `docs/plans/001-implementation-roadmap.md`).

### ST4 — Archive old README to docs/old-readme.md
**Files:** `docs/old-readme.md` (new)
**Acceptance:** The full current README content is preserved verbatim. No data loss.
**Estimate:** XS

1. Read current `README.md`.
2. Write `docs/old-readme.md` with the full original content.
3. Add a header note: `# Old README (pre-CLO-314) — replaced May 2026`.

### ST5 — Verify markdown rendering and line count
**Files:** —
**Acceptance:** README renders cleanly on GitHub preview; install + run section ≤ 50 lines; all links resolve.
**Estimate:** XS

1. Render locally or push a preview branch.
2. Count lines from "## Install" (or equivalent heading) through the end of the example output section.
3. Click every link to confirm it resolves.

### ST6 — Verify every command end-to-end
**Files:** —
**Acceptance:** Every shell command in the README runs successfully in a clean checkout.
**Estimate:** M

1. Run `loker run design-doc-tdd --spec examples/specs/calculator.md` — confirm it produces a run directory with expected artefacts.
2. Run `loker trace <run_id>` — confirm output is readable.
3. Run `make release` (or `cargo build`) — confirm build is clean.
4. If `cargo install loker` is listed, verify the install path is correct.
5. Address the three open questions from the design doc:
   - Is `cargo install loker` on crates.io?
   - Does the example need TensorZero Tier 2 running?
   - What is the exact line cap for "under one screen"?

### ST7 — Pre-merge gate
**Files:** —
**Acceptance:** `make check` clean. All verification steps pass.
**Estimate:** XS

1. `make check` (fmt + clippy + test — should be a no-op since no Rust changes).
2. Final proofread of the full README.
3. `git diff` to confirm no unintended changes.

## Pre-merge gate
- `make check` (fmt + clippy + test)

## Risks

| # | Risk | Mitigation |
|---|---|---|
| 1 | `cargo install loker` not on crates.io yet | Use `cargo install --git` as fallback; escalate to user in ST6 |
| 2 | Example command requires TensorZero Tier 2 | Document preconditions in the example; escalate to user |
| 3 | Old content accidentally lost during archive | Verify `diff` between old README and `docs/old-readme.md` in ST5 |
| 4 | Links to old README anchors break | `rg "README.md" docs/` already checked — no external anchor references found |
