# Design: CLO-314 — README rewrite: thesis, primitives, install, one-page example

**Linear Task**: https://linear.app/cloud-ai/issue/CLO-314
**Status**: Design
**Author**: Mk Km
**Created**: 2026-05-06

---

## Summary

Replace the current README.md with a v0 surface that leads with loker's thesis, explains the three orchestration primitives in digestible form, shows install via `cargo install loker` (or `make release`), and demonstrates a single end-to-end example the reader can copy-paste and run — all within one page for the install + run section. The old README content is archived to `docs/old-readme.md`.

---

## Background

The current README (~220 lines) was inherited from the lok fork and grew by accretion as milestones shipped. It mixes thesis, architecture diagram, deployment instructions, full roadmap table, design doc links, and lineage into a single wall of text. A newcomer cannot get "what does this do and how do I start" in under one screen.

Key problems:
- **Code is ahead of docs:** The three primitives (Strategy, Aggregator, VerifyHook) shipped in M2-M4, but the README still says "None ship yet (M2-M4)."
- **No quick-start experience:** The reader must scroll past architecture, roadmap, and lineage before seeing a single command.
- **Buried install:** Only shows `cargo install --path .` (source build), not `cargo install loker` or `make release`.
- **No end-to-end example:** No runnable command the reader can copy-paste.
- **Roadmap duplication:** The full roadmap table duplicates what `docs/plans/001-implementation-roadmap.md` tracks more carefully.

### Prior Research

Discovery report: `docs/discovery/clo-314.md`. Baseline score 3/10. Three approaches evaluated:
- **A: Incremental edits** (S effort, low risk) — cannot meet "under one screen" AC.
- **B: Full rewrite** (M effort, low risk) — chosen approach. Clean break, own voice, meets all ACs.
- **C: Modular sub-sections** (M effort, medium risk) — over-engineered; sub-files risk docs rot.

---

## Architecture

This is a documentation-only change. The README is a single Markdown file at the repository root. No code, no schema, no config changes.

### Content structure (new README.md)

```
┌─────────────────────────────────────┐
│ loker — LLM orchestration engine     │  ← one-paragraph thesis
│                                       │
├─────────────────────────────────────┤
│  What is loker?                      │  ← thesis expanded: why, vs lok
│                                       │
├─────────────────────────────────────┤
│  Three primitives                    │  ← Backend / Strategy / Aggregator + VerifyHook
│  • Backend — talk to models          │     with short examples
│  • Strategy — how to call backends   │
│  • Aggregator — how to merge results │
│  • VerifyHook — how to gate retries  │
│                                       │
├─────────────────────────────────────┤
│  Install                             │  ← cargo install loker + make release
│                                       |
├─────────────────────────────────────┤
│  One-page example                    │  ← loker run design-doc-tdd --spec ...
│  (copy-paste)                        │     expected output, artefacts
│                                       │
├─────────────────────────────────────┤
│  Roadmap & design docs (links)       │  ← compact pointers, not full table
│                                       │
├─────────────────────────────────────┤
│  License & lineage                   │  ← MIT, fork of ducks/lok
└─────────────────────────────────────┘
```

### Affected files

| File | Change Type | Description |
|------|-------------|-------------|
| `README.md` | Modified | Full rewrite with new structure |
| `docs/old-readme.md` | New | Archive of current README content |

No other files are touched.

---

## Detailed Design

### Section 1: Thesis (one paragraph)

The opening paragraph must answer three questions in ~4-5 sentences:
1. What is loker? (LLM orchestration engine)
2. What gap does it fill? (cross-family aggregation, escalating retry)
3. How is it different from lok? (hard fork with new primitives)

Keeps the "Why loker exists" content from the current README but reduces it to one paragraph + a short "vs lok" callout.

### Section 2: Three primitives

Each primitive gets a subsection with:
- One-line definition
- Enum variants (short, no full Rust code blocks — use inline `code` spans)
- One sentence on when you'd use each variant

The current README has this already but says "None ship yet (M2-M4)." The new version says "Shipped in M2-M4" and links to the appropriate design docs for depth.

### Section 3: Install

Two paths:
- **From source:** `make release` (auto-versions, builds, installs to `/usr/local/bin`)
- **From crates.io:** `cargo install loker` (once published; pre-v0 note if not)

The install section must fit in ~10 lines and be above the fold.

### Section 4: One-page example

A single copy-paste block:

```bash
# Create an example spec
cat > calculator.md << 'EOF'
...
EOF

# Run the design-doc-tdd workflow
loker run design-doc-tdd --spec calculator.md

# See the trace
loker trace <run_id>
```

With 1-2 sentences of expected output and where artefacts land (`runs/` directory, `trace.jsonl`, `manifest.json`).

This is the critical section — it must be verified end-to-end before publishing. The discovery debt item covers this.

### Section 5: Roadmap & references

A compact section with bullet links to:
- `docs/handoff.md` — deep project context
- `docs/plans/001-implementation-roadmap.md` — task list
- The design doc (external repo)
- `docs/prds/` — PRD files per milestone

No full roadmap table — the reader who needs depth clicks through.

### Section 6: License & lineage

Preserved from current README but shortened: "MIT — see LICENSE. Fork of ducks/lok."

---

## Implementation Plan

### Phase 1: Write new README.md

- [ ] Draft the thesis paragraph
- [ ] Draft the three primitives section (update ship status from aspirational to shipped)
- [ ] Draft the install section
- [ ] Draft the one-page example
- [ ] Draft the roadmap & references section
- [ ] Draft the license & lineage section

### Phase 2: Archive old content

- [ ] Read current README.md
- [ ] Write `docs/old-readme.md` with the full original content
- [ ] Add a note at the top of `docs/old-readme.md` explaining it was replaced by the new README for CLO-314

### Phase 3: Verification

- [ ] Verify `make release` / `cargo build` still works (nothing changed in code)
- [ ] Verify the example command works end-to-end (requires TensorZero Tier 2 or all backends available)
- [ ] Verify README renders cleanly on GitHub (check markdown syntax)
- [ ] Verify install + run section fits under one screen (approx 50 lines)
- [ ] `make check` — nothing should break

---

## Acceptance Criteria

Each criterion is specific and verifiable.

- [ ] README renders cleanly on GitHub — no broken markdown, all links resolve. **Verification:** visual inspection of the rendered file.
- [ ] Install + run section (from "Install" heading through the example output) fits in one terminal screen (~50 lines). **Verification:** `wc -l` on that section or visual check.
- [ ] Every command in the README is verified to work end-to-end. **Verification:** run each command in a fresh clone or checkout.
- [ ] Old README content is preserved under `docs/old-readme.md`. **Verification:** `diff` shows old README content exists in the archive file.
- [ ] `make check` passes with no changes to any Rust source. **Verification:** `make check` exit 0.

---

## Constraints

**Must:**
- Preserve ducks's MIT copyright in `LICENSE`. Both copyright lines stay.
- The old README content must be fully preserved in `docs/old-readme.md` — no data loss.
- The "under one screen" AC is non-negotiable — the install + run section must be short enough to read without scrolling.

**Must-not:**
- Must not modify any Rust source files, config files, or docs outside the scope (README.md + docs/old-readme.md).
- Must not remove or break existing anchor targets if the old README is linked from other docs (check `rg "README.md" docs/` before changing).
- Must not add new dependencies.

**Prefer:**
- Prefer inline `code` spans over full Rust code blocks for primitive enum variants to keep line count low.
- Prefer bullet links over a full roadmap table for the references section.

**Escalate when:**
- If `cargo install loker` does not work from crates.io (pre-v0), ask user whether to use `cargo install --git https://github.com/maxkulish/loker` as the install path or skip the crates.io line.
- If the example command (`loker run design-doc-tdd --spec examples/specs/calculator.md`) fails due to missing backends or TensorZero, ask user whether to document the preconditions or use a simpler example.

---

## Testing Strategy

This is a documentation change, so testing is manual verification:

- **Markdown rendering:** Render the README locally or push to a branch and view on GitHub.
- **Command verification:** Run every shell command in the README in sequence from a clean state.
- **Line count check:** `wc -l` on the install + run section — must be ≤ ~50 lines.
- **Archive integrity:** Diff the old README against `docs/old-readme.md` to confirm no content loss.
- **CI:** `make check` must pass (fmt + clippy + test — no Rust changes, so this is a no-op gate).

---

## Open Questions

- [ ] Is `cargo install loker` available on crates.io yet, or should we use `cargo install --git` for pre-v0? (Escalate to user.)
- [ ] Does the example command (`loker run design-doc-tdd --spec examples/specs/calculator.md`) work without TensorZero Tier 2 running? If not, what preconditions should the README document? (Escalate to user.)
- [ ] Should the "under one screen" AC count the section heading lines or only the content? (Prefer: content + headings together.)

---

## References

- [Linear Task CLO-314](https://linear.app/cloud-ai/issue/CLO-314)
- [Discovery Report](docs/discovery/clo-314.md)
- [PRD](docs/prds/clo-314-readme-rewrite.md)
- [Implementation Roadmap — Phase 10](docs/plans/001-implementation-roadmap.md)
