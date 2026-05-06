# Pre-PR validation: clo-314

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
---

Verifying the findings before synthesizing — both external reviewers crashed on shell escaping, so the synthesis rests on the Claude fallback plus my spot-checks.

Spot-check confirms:
- F1: zero `design-doc-tdd` runs in `runs/`, and the design doc's own Open Questions explicitly flag the example command as needing user escalation that never happened.
- F2: `.lok/workflows/design-doc-tdd.toml`:50 sets `output = "changes"` (not `implement.rs`); verify phase is `parallel` over `codex/, gemini/` with `cargo build && cargo test` commented out as FR-31 forward-compat.
- F3: `runs/<id>/` contains `manifest.json` + `attempts/` — README's flat layout is wrong.
- F4: `README.md`:7 has empty `()` href.
- F6: branch has no commits; `make check` not run.

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Heredoc-inside-backtick shell escaping bug in `.pi/agents` runner; never reached the model |
| Gemini | REVIEW_FAILED | Same shell escaping bug in the runner; primary and fallback both unreachable |
| Claude fallback | OK | Six findings produced; spot-verified against README, workflow TOML, and `runs/` layout |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 — Example command unverified against ACs (high).** Design AC #3 requires every README command verified end-to-end; design Open Question explicitly required escalation on whether `loker run design-doc-tdd --spec examples/specs/calculator.md` works today, and that escalation never happened. No `design-doc-tdd` run exists in `runs/`. Resolution: either (a) actually run the workflow against the calculator spec and capture real artefacts to verify the README claims, or (b) replace the centerpiece example with `loker explain design-doc-tdd` (already in the "What works today" list and trivially runnable) and defer the full `loker run` walkthrough to land with CLO-309.
- **F2 — Artefacts list contradicts the workflow TOML (high).** `README.md`:114 says `implement.rs`; the workflow's implement phase outputs `changes` (a directory). `README.md`:120 says verify "runs `cargo test`"; the verify phase is parallel `codex/`+`gemini/` review producing `verify.json`, with the cargo command commented out as FR-31 forward-compat. Fix to match what the TOML actually does today.
- **F3 — Run-directory layout omits `attempts/` (medium).** `README.md`:111-114 shows a flat layout; real layout is `manifest.json` at root with per-phase artefacts under `attempts/<n>/<phase>/`. This is the same paragraph as F2; fix together.
- **F4 — Broken empty-href badge link (medium).** `README.md`:7 — `[!["loker"](...)]()` renders as a no-op link. Drop the `[…]()` wrapping or point the link somewhere meaningful. Violates AC #1 ("all links resolve").
- **F6 — Plan ST7 gate not executed (low).** Plan requires `make check`; branch has no commits yet. Run it before committing — trivial but explicit in the plan.

## Out of Scope / Deferred
- **F5 — Missing deploy/tensorzero pointer.** Real but optional. The milestone table claims M7 ✅ without a link to the recipe. Adding one bullet under "Design docs & roadmap" is a one-line fix and could be folded into the same iteration, but it's not blocking — landing it as a follow-up is fine.

## False Positives / Tooling Artifacts
- Codex/Gemini "REVIEW_FAILED" are tooling artifacts, not findings about the change. The runner script in `.pi/agents/codex-pre-pr.md`/equivalent has a heredoc-inside-`$(…)` quoting bug that prevents either model from being invoked. Worth filing a separate ticket against the harness; does not affect this PR's substance.

## Recommendation
PROCEED_WITH_FIXES. All Must-Fix items are bounded edits to a single file (`README.md`) and a one-shot `make check`. Concretely: (1) decide between running the workflow for real vs. swapping the example to `loker explain design-doc-tdd`; (2) rewrite the per-phase artefacts paragraph against `.lok/workflows/design-doc-tdd.toml` and the `manifest.json` + `attempts/` layout; (3) fix the empty-href badge; (4) run `make check` and commit. After that, ready for PR. Separately, file a bug against `.pi/agents/*` for the shell escaping that broke both Codex and Gemini reviewers — synthesis on a single fallback is thinner than the gate intends.
