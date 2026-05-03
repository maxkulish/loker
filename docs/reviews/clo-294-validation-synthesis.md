# Pre-PR validation: clo-294

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-03
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc quoting error in invocation script (`unexpected EOF while looking for matching '`); model was never called |
| Gemini | REVIEW_FAILED | Same shell heredoc quoting error in invocation script; primary and fallback models never called |
| Claude (fallback) | OK | Produced 7 findings (F1–F7) with verdict `approve_with_changes` |

Only the Claude fallback produced substantive findings. Synthesis is based on that single source — no cross-reviewer corroboration was possible. The orchestrator should be aware that the wrapper scripts at `.pi/agents/codex-pre-pr.md` and `.pi/agents/gemini-architect.md` (or the heredoc construction in the invoking shell) appear to be broken and need repair so future runs get real multi-model coverage.

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 — CLI swallows `RunDir::create` failure** (`src/main.rs:1183-1192`). Propagate the error via `anyhow::Context` rather than logging-and-continuing. The whole point of `RunDir` is "single source of truth for run paths"; silent fallthrough hides storage/permission failures and sets a bad precedent before downstream wiring lands.
- **F2 — `/runs/` missing from `.gitignore`**. `git status` already shows `?? runs/`. Without this entry every developer's tree dirties the moment they exercise `loker run`. One-line fix matching the existing `/target/` pattern.
- **F3 — Retry branch maps every `io::Error` to `Collision`** (`src/run_state/run_dir.rs:61`). Match on `ErrorKind::AlreadyExists`; forward any other error through the existing `From<io::Error>` so `PermissionDenied` / `NotFound` aren't mis-reported.
- **F4 — Retry path has no real test coverage** (design AC #6 unmet). `tests/run_dir_layout.rs:108-145` admits it doesn't actually simulate a collision. Either factor out a `try_create_with(slug, now, run_id)` helper and unit-test the retry branch directly, or inject a name generator so a test can pre-create the first attempted path. Without this the only safety net against UUID collisions is unverified.

All four are bounded, fit one fix iteration, and don't require design changes.

## Out of Scope / Deferred
- **F5 — Redundant `Json` vs `Manifest` variants in `RunDirError`**. Pure cleanup; no incorrect behavior today.
- **F6 — `RunDir: Clone` weakens single-owner invariant**. No `Drop` today, so harmless. Revisit if/when cleanup-on-drop is added.
- **F7 — Informational note on `ManifestError` import path**. No action.

## False Positives / Tooling Artifacts
- Codex and Gemini "reviews" themselves — both failed due to shell-script bugs in the wrapper, not due to legitimate model output. Treat their `success=false` as infrastructure failure, not as review signal. Recommend filing a follow-up to fix `.pi/agents/*` invocation before relying on multi-reviewer consensus again.

## Recommendation
PROCEED_WITH_FIXES. Apply the four Must-Fix items in one bounded iteration:
1. Propagate `RunDir::create` errors with `anyhow::Context` in `src/main.rs` Run handler.
2. Add `/runs/` to `.gitignore`.
3. Narrow the retry-branch error mapping in `src/run_state/run_dir.rs:61` to `ErrorKind::AlreadyExists` only.
4. Refactor `create` to expose a testable seam (injected name generator or `try_create_with` helper) and add a unit test that exercises the collision-retry path so design AC #6 is actually verified.

Re-run `make check` after fixes, then proceed to PR. Separately, recommend repairing the Codex/Gemini wrapper scripts so the next synthesis has real multi-model input.
