# Pre-PR validation: clo-314

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [high] Example command may not actually work end-to-end
**Where:** `README.md`:104-107 (the copy-paste block)
**What:** The CLI help itself states `loker run` is "Step-based today; phase-based wiring lands in T-041" — and CLAUDE.md confirms CLO-309 (T-040) is the *next* pick, unshipped. There is no `runs/.../trace.jsonl` from any design-doc-tdd run anywhere on disk; existing `runs/` entries are all step-based test fixtures. This directly violates the design doc's AC #3 ("Every command in the README is verified to work end-to-end") and the plan's ST6 ("Run `loker run design-doc-tdd --spec examples/specs/calculator.md` — confirm it produces a run directory with expected artefacts"). Open questions in the design doc were never resolved.
**Suggested fix:** Either (a) actually run the workflow and capture real output before merging, or (b) wait until CLO-309 lands and merge this README as part of that PR, or (c) demote the example to a `loker explain design-doc-tdd` walkthrough (which the CLI already supports today) and add the `loker run` example in the CLO-309 PR.

### F2 [high] Per-phase artefacts list contradicts the workflow file
**Where:** `README.md`:114, 119-120
**What:** README says artefacts include "design.md, review.md, **implement.rs**, verify.json", but `.lok/workflows/design-doc-tdd.toml`:50 sets `output = "changes"` for the implement phase — there is no `implement.rs`. README also says verify "run[s] `cargo test` on the generated code", but the verify phase uses `strategy = parallel` with `codex/, gemini/` backends and the `run_command = "cargo build && cargo test"` line is commented out (FR-31 forward-compat). The implement phase has no verify hook either, so "escalating retry: cheap → strong until code compiles" is also aspirational.
**Suggested fix:** Replace `implement.rs` with `changes/` (or whatever the actual implement output directory contains). Rewrite L119-120 to describe what the workflow does today: "implement: escalating retry across `ollama → claude → codex` with failure context passed forward" and "verify: parallel review by `codex/` and `gemini/` producing `verify.json`". Note the `cargo test` hook is FR-31 forward-compat, not shipped.

### F3 [medium] Run-directory layout omits the `attempts/` subdirectory
**Where:** `README.md`:111-114
**What:** Real layout (per CLO-286) is `runs/<id>/manifest.json` plus `runs/<id>/attempts/<n>/<phase>/...` — phase outputs live under `attempts/`, not at the run-dir root. README presents a flat layout that won't match what users see. `trace.jsonl` location should also be confirmed (search of existing runs found zero `trace.jsonl` files).
**Suggested fix:** Reflect the actual layout: `manifest.json` at the root, per-phase artefacts under `attempts/<n>/<phase>/`, and the canonical path of `trace.jsonl` (verify by reading `src/run_state/attempt_dir.rs` or `src/manifest.rs`).

### F4 [medium] Broken empty-href badge link
**Where:** `README.md`:7
**What:** `[!["loker"](https://img.shields.io/badge/status-pre--v0-yellow)]()` — the trailing `()` creates a link with an empty href; on GitHub it renders as a no-op link. Either drop the link wrapping or point it somewhere meaningful.
**Suggested fix:** Use `![status](https://img.shields.io/badge/status-pre--v0-yellow)` (no link), or wrap with `[![…](…)](docs/plans/001-implementation-roadmap.md)`.

### F5 [low] Deployment recipe disappeared, but M7 is marked shipped ✅
**Where:** `README.md`:165 (milestone table) and absence elsewhere
**What:** Old README had a "Deployment (TensorZero Tier 2)" section pointing to `deploy/tensorzero/`. The rewrite removes it entirely yet still claims M7 ✅. A reader who needs the Docker Compose recipe has no entry point from the README.
**Suggested fix:** Add one bullet under "Design docs & roadmap" linking `deploy/tensorzero/README.md`, or a single sentence in the install section: "For the TensorZero gateway + ClickHouse stack, see `deploy/tensorzero/`."

### F6 [low] Pre-merge gate not run
**Where:** plan ST7
**What:** Branch has no commits (`git log main..HEAD` empty); all work is uncommitted. Plan ST7 requires `make check`. While there are no Rust changes, the plan's explicit gate hasn't been ticked.
**Suggested fix:** Run `make check` and confirm green before committing/PR'ing.

## Verdict
**rework**

The rewrite cleanly delivers structure, archive integrity, line-count budget, and license preservation — but it ships factual claims that don't match the codebase: a copy-paste example for a command path that the CLI itself flags as not yet wired (CLO-309 is the *next* task), an artefact list that contradicts the workflow TOML (`implement.rs` does not exist), a verify-phase description that describes commented-out FR-31 forward-compat hooks, and a broken markdown link. The design doc's own AC #3 ("every command verified end-to-end") was bypassed, and the plan's ST6 open questions were never resolved. Fix the example to use a today-working command (or hold the README until CLO-309 lands), correct the artefacts/verify description against `.lok/workflows/design-doc-tdd.toml`, repair the empty badge href, and run `make check` before merging.
