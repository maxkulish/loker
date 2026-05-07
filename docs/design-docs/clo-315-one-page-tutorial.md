# CLO-315: One-page tutorial: clone -> first run -> read trace

**Linear Task**: https://linear.app/cloud-ai/issue/CLO-315
**Status**: Design
**Author**: Team
**Created**: 2026-05-07

---

## Summary

Create `docs/tutorial.md`, a single-page getting-started guide that walks a new user from `git clone` through a successful first `loker run` to inspecting the resulting run directory (`manifest.json`) and understanding `trace.jsonl`. The tutorial uses the calculator example spec (T-036 / CLO-290) as its concrete input and cross-links with the restructured README (T-045 / CLO-314) and `docs/handoff.md`.

---

## Background

Loker is a pre-v0 Rust LLM orchestration engine. New users currently have no unified quickstart path. The README mentions "For a full end-to-end run example (requires Ollama), see the Quickstart guide (not yet written)." This task is that Quickstart guide.

The calculator example spec at `examples/specs/calculator.md` defines a minimal Rust calculator library (`add`, `subtract`, `multiply`, `divide`) and serves as the canonical small input for demonstrating loker's workflow pipeline.

### Prior Research

Discovery phase findings:
- `loker run` executes step-based workflows today; phase-based wiring (T-041) is in progress.
- The step-based runner creates `runs/<workflow>-<timestamp>-<id>/` containing `manifest.json` and `attempts/`.
- `trace.jsonl` is written by the phase-based runner (M5 ✅) but is not yet wired to `loker run` (T-041).
- `loker trace <run_id>` pretty-prints `trace.jsonl` and is fully functional.
- `loker explain <workflow>` works without any backends and is the best zero-friction first command.
- Ollama is the recommended local backend for a first LLM-backed run.

---

## Architecture

### Component Overview

This is a pure-documentation task. No code changes to loker itself.

```
docs/
├── tutorial.md          ← new file (this task)
├── handoff.md           ← add cross-link
└── prds/                ← reference roadmap

README.md                ← add cross-link
```

### Affected Components

| Component | Change Type | Description |
|-----------|-------------|-------------|
| `docs/tutorial.md` | New | Single-page getting-started guide |
| `README.md` | Modified | Add "Quickstart" or "Tutorial" link |
| `docs/handoff.md` | Modified | Add cross-link from onboarding section |

### Dependencies

- **Internal**: `examples/specs/calculator.md` (T-036 / CLO-290)
- **Internal**: `docs/handoff.md` (existing project intent doc)
- **Internal**: `README.md` (T-045 / CLO-314 restructure)
- **External**: None

---

## Detailed Design

### Tutorial Structure

The tutorial is a single Markdown file with the following sections:

1. **What you'll do** (30 s read)
   - One-paragraph summary: clone, build, run a workflow, inspect the run.

2. **Prerequisites** (1 min)
   - Rust toolchain (`cargo`)
   - Git
   - Optional: Ollama (for LLM-backed step)

3. **Install** (2 min)
   - `git clone https://github.com/maxkulish/loker.git`
   - `cd loker`
   - `cargo build --release` (or `cargo build` for debug)
   - Verify: `cargo run -- --help`

4. **Check your setup** (1 min)
   - `cargo run -- doctor`
   - Explain output: backends ready vs. API keys needed

5. **Explore a workflow without backends** (1 min)
   - `cargo run -- explain design-doc-tdd`
   - Explain what the output means (phases, strategies, backends)

6. **Run your first workflow** (2 min)
   - Use a simple step-based workflow that reads the calculator spec
   - Command: `cargo run -- run examples/workflows/calculator-tutorial.toml --spec examples/specs/calculator.md`
   - Show real output from an actual run

7. **Locate the run directory** (1 min)
   - `ls runs/`
   - Explain directory naming: `runs/<workflow_slug>-<YYYYMMDD-HHMMSS>-<short_uuid>/`
   - `cat runs/<id>/manifest.json`
   - Show real manifest.json output

8. **Read the trace** (1 min)
   - Explain: `trace.jsonl` contains OpenTelemetry GenAI spans
   - Command: `loker trace <run_id>`
   - Note: step-based runner does not yet write `trace.jsonl`; it is produced by phase-based workflows (T-041). Show `loker trace` output from a fixture or note the command syntax.
   - Show real `loker trace` pretty-printed output

9. **Next steps** (30 s)
   - Link to `docs/handoff.md` for project intent
   - Link to `loker --help` for full command reference
   - Link to T-042 (`loker explain`) for workflow exploration

### Output Examples Policy

Every command block in the tutorial must be followed by an output block that was captured from an **actual run** on the current CLI surface. No invented output. If a command requires a backend that is not universally available (e.g., Ollama), the tutorial must either:
- Mark it as optional with a clear prerequisite note, or
- Provide a backend-agnostic alternative (e.g., `loker explain`)

---

## Implementation Plan

### Phase 1: Draft the tutorial

- [ ] Create `docs/tutorial.md` with all 9 sections
- [ ] Run every command shown and paste real output
- [ ] Verify the tutorial can be completed in under 10 minutes by timing each section

### Phase 2: Verify and cross-link

- [ ] Add "Tutorial" link to `README.md`
- [ ] Add cross-link in `docs/handoff.md` onboarding section
- [ ] Spell-check and markdown-lint the file

### Phase 3: Review

- [ ] Self-review: follow the tutorial on a clean machine (or fresh clone)
- [ ] Fix any commands that fail or outputs that differ

---

## Constraints

**Must**:
- All commands shown must pass on the current `main` branch.
- Output examples must be from actual runs, not invented.
- File must be ≤200 lines to stay "one-page".
- Must reference `examples/specs/calculator.md` explicitly.

**Must-not**:
- Must not document features that are not yet wired (e.g., claiming `loker run design-doc-tdd --spec …` works end-to-end when phase-based runner is not yet connected).
- Must not require paid API keys for the core tutorial path.

**Prefer**:
- Prefer `cargo run --` over `make install` for the tutorial (lowest friction for first-time users).
- Prefer shell-only or `explain` commands for the mandatory path; Ollama-backed steps are optional.

**Escalate when**:
- If a core `loker` CLI command used in the tutorial changes behavior during implementation, stop and update the design doc.

---

## Acceptance Criteria

- [ ] `docs/tutorial.md` exists, is non-empty, and renders correctly as Markdown.
  - **Verification**: `test -s docs/tutorial.md && wc -l docs/tutorial.md` (expect >20 and ≤200 lines)
- [ ] Every shell command in the tutorial was executed on current `main` and produced the shown output.
  - **Verification**: Manual checklist during Phase 1; every ` ```bash ` block must have a matching ` ```text ` output block from an actual run.
- [ ] `examples/specs/calculator.md` is referenced in the tutorial.
  - **Verification**: `rg "examples/specs/calculator" docs/tutorial.md`
- [ ] `README.md` contains a link to `docs/tutorial.md`.
  - **Verification**: `rg "tutorial" README.md`
- [ ] `docs/handoff.md` contains a link to `docs/tutorial.md`.
  - **Verification**: `rg "tutorial" docs/handoff.md`

**Verification method**: `cargo test` is not applicable (no code changes). Run `markdownlint docs/tutorial.md` if available; otherwise manual read-through.

---

## Evaluation

| # | Test | Expected Result | Command / Steps |
|---|------|-----------------|-----------------|
| 1 | Tutorial file exists and is within length budget | `docs/tutorial.md` exists, ≤200 lines | `test -s docs/tutorial.md && test $(wc -l < docs/tutorial.md) -le 200` |
| 2 | Calculator spec referenced | Non-empty match | `rg "examples/specs/calculator" docs/tutorial.md` |
| 3 | README links to tutorial | Non-empty match | `rg -i "tutorial" README.md` |
| 4 | Handoff links to tutorial | Non-empty match | `rg -i "tutorial" docs/handoff.md` |
| 5 | All bash commands have real output | Every ` ```bash ` block is followed by ` ```text ` | Manual review |
| 6 | `loker explain` command works | Prints workflow DAG | `cargo run -- explain design-doc-tdd` |

**Edge cases to cover**:
- User has no Ollama: tutorial must still have a satisfying path (`loker explain` + simple shell workflow).
- User is on Windows: all commands should use `/` paths or note platform differences.
- `trace.jsonl` is absent from step-based runs: tutorial must not claim it exists there.

---

## Testing Strategy

- **Manual Testing**: A team member (or CI step) follows the tutorial on a fresh clone and times each section.
- **Linting**: Run `markdownlint` or `mdl` on `docs/tutorial.md` if available in the environment.
- **Link Checking**: Verify internal links (`docs/handoff.md`, `README.md`) resolve.

---

## Open Questions

- [ ] Should the tutorial include a `make install` path or stay `cargo run --` only? (Preference: `cargo run --` for lowest barrier.)
- [ ] Should the tutorial create a temporary `calculator-tutorial.toml` workflow file, or should we add one to `examples/workflows/` as part of this task? (Preference: add to `examples/workflows/` so it is maintained alongside other examples.)
- [ ] When T-041 lands and `loker run design-doc-tdd --spec examples/specs/calculator.md` works, should the tutorial be updated? (Yes — note this as a post-T-041 follow-up in Linear.)

---

## References

- [Linear Task](https://linear.app/cloud-ai/issue/CLO-315)
- [Calculator Spec](examples/specs/calculator.md) — T-036 / CLO-290
- [README Rewrite](https://linear.app/cloud-ai/issue/CLO-314) — T-045 / CLO-314
- [Handoff Doc](docs/handoff.md)
- [Roadmap: Phase 10](docs/plans/001-implementation-roadmap.md)
