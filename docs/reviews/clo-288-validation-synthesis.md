# Pre-PR validation: clo-288

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-03
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc parse error in wrapper script (`unexpected EOF while looking for matching '`) — tooling artifact, not a model-substance failure |
| Gemini | REVIEW_FAILED | Same shell heredoc parse error in wrapper script — tooling artifact |
| Claude (fallback) | OK | Full review delivered: `make check` green, 6 findings (1 major, 3 minor, 2 nits) |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 — ST3 fixture replacement / PRD acceptance criterion unmet** (`tests/fixtures/workflows/design-doc-tdd.toml`, `tests/workflow_grammar.rs:28`, `docs/status/clo-288-workflow.yaml:80`). PRD explicitly required the byte-for-byte fixture used by CLO-287 to be replaced with the canonical file. Current state: placeholder still has old phase names, status YAML records "deferred" without a tracked CLO follow-up. Pick one and ship in this PR: (a) update `byte_for_byte_design_doc_tdd` assertions to match the new canonical content and replace the placeholder with a byte-identical copy, or (b) rename the placeholder to `legacy-placeholder.toml`, file a CLO follow-up to retire it, and reference that ticket from the status YAML and the test's comment. Without either, the criterion silently rots.
- **F2 — `[phases.contract]` block on wrong phase** (`.lok/workflows/design-doc-tdd.toml:35-39, 48-50, 59-61`). Design §6 (Option B) places hook configuration on `implement` and `verify`; current file places the live empty block on `review` and leaves the other two commented out. This causes the lint warning to fire on the wrong phase and signals copy-paste rather than design intent. Move the live `[phases.contract]` from review to implement (and uncomment verify, or delete it), drop the empty review block.

## Out of Scope / Deferred
- **F4 — File CLO follow-ups for OQ1 (Phase.hooks) and OQ2 (Strategy::ParallelFanOut.aggregator)** and reference them from `#[ignore]` reason on `implement_phase_has_test_runner_hook` (`tests/workflows_design_doc_tdd.rs:100`) and from canonical TOML hook-block comments. Tracking only — no code change required for this PR's scope.
- **F5 — `output = "changes/"` semantics** (`.lok/workflows/design-doc-tdd.toml:47`). Trailing-slash semantics belong to M6 (CLO-271/CLO-273) consumers; flag in those tickets rather than block this PR.

## False Positives / Tooling Artifacts
- Codex and Gemini wrapper scripts in `.pi/agents/` (or wherever the heredoc-bearing review scripts live) both failed on shell heredoc parsing — `unexpected EOF while looking for matching '`. This is a wrapper bug (likely a quote inside the heredoc body that breaks `bash -c` invocation), not a model unavailability. The reviews didn't run; their absence is not signal. Worth a separate fix to the review harness so future synthesis isn't single-sourced from Claude.
- **F3 (header comment drift) and F6 (loose warning assertion)** are real but nit-level polish; Claude flagged them as nits and either folding into this PR or M6 wiring is acceptable. Not classifying as false positives, but not gating either.

## Recommendation
PROCEED_WITH_FIXES — bounded fix iteration: (1) resolve F1 by updating `byte_for_byte_design_doc_tdd` assertions and replacing the placeholder with a byte-identical copy of the canonical TOML (preferred over the rename-and-defer path since it actually closes the PRD criterion); (2) fix F2 by relocating the live `[phases.contract]` block from `review` to `implement` and uncommenting the `verify` block (or deleting the review marker). F3/F6 polish optional in same PR. F4 (file two CLO follow-ups) and F5 (M6 ticket note) can be done in parallel and don't gate. Separately, fix the Codex/Gemini wrapper heredoc bug so future synthesis has triangulation instead of single-source Claude review.
