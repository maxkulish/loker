# Pre-PR validation: clo-290

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-03
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc quoting bug in wrapper (unmatched `'` at line 30) - tooling failure, no review produced |
| Gemini | REVIEW_FAILED | Same shell heredoc quoting bug in wrapper (unmatched `'` at line 38) - tooling failure, no review produced |
| Claude (fallback) | OK | `make check` passes; doc-only diff reviewed against 12 ACs in workflow yaml |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 - AC12 passes vacuously.** `examples/specs/calculator.md:35` and `specs/2026-05-03-clo-290-calculator-example-spec.md:31` use untagged ``` ``` ``` fences, so AC12's `rustfmt --check` over `rust`-tagged blocks runs against empty input and trivially succeeds. The block contents (`add(2, 3) == 5`, `divide(1, 0) -> Err(DivisionByZero)`) are pseudocode that would not parse as Rust. Fix by either tagging the fence as ``` ```text ``` and rewriting AC12 to assert the language tag, or dropping AC12 entirely. Do not tag as `rust` - contents are not valid Rust.
- **F2 - `divide(10, 2) == 5.0` mixes int inputs with float result.** `examples/specs/calculator.md:39`. The spec claims integer and float support, but integer `/` in Rust returns an integer. CLO-291's TestRunner phase will hit this ambiguity. Fix by either: (a) `divide(10, 2) == 5` plus a separate `divide(7.0, 2.0) == 3.5` case, or (b) explicit Requirements line "`divide` always returns f64".

## Out of Scope / Deferred
- **F3 - Missing trailing newlines** on both spec files. Trivial cleanup; bundle with the F1/F2 fix iteration if convenient, otherwise defer.

## False Positives / Tooling Artifacts
- **Codex and Gemini wrapper failures** are pure tooling artifacts - both `.pi/` review scripts have a heredoc quoting bug (the embedded `git diff main...HEAD` backtick block escapes are interacting badly with the outer single-quoted heredoc). Not a defect of the branch under review. Worth fixing in `.pi/` separately but out of scope here.
- **F4 - Missing plan doc.** Not a finding; workflow yaml documents the plan phase as intentionally skipped for specification-type tasks. Mentioned only because the prompt asked for the plan.

## Recommendation
PROCEED_WITH_FIXES. Bounded fixes for one iteration: (1) fix AC12 in the workflow yaml so it asserts the fence language tag rather than running rustfmt on absent rust blocks, and retag the Acceptance fences as `text`; (2) resolve the int/float `divide` ambiguity in Requirements + Acceptance so CLO-291 has an unambiguous signature to test; (3) add trailing newlines. All three are localized to the two doc files plus the workflow yaml AC12 entry. The change is doc-only with no Rust impact and `make check` clean - safe to merge once the spec ambiguities are tightened, since this spec will be consumed by CLO-291 in M6.

---

## Re-validation (fix iteration 1)

All Must Fix items have been addressed:

- **F1 (AC12)**: Fixed. Acceptance fence changed from `` ``` `` to `` ```text ``. AC12 rewritten to check for `text` tag and absence of `rust` tag. Verification: `rg "^```text$" examples/specs/calculator.md` returns 1 match; `rg "^```rust$" examples/specs/calculator.md` returns no match.
- **F2 (divide int/float)**: Fixed. Integer case now reads `divide(10, 2) == 5` (int result). Separate float case `divide(7.0, 2.0) == 3.5` added. Verification: `rg "^divide" examples/specs/calculator.md` shows both `divide(10, 2) == 5` and `divide(7.0, 2.0) == 3.5`.
- **F3 (trailing newlines)**: Fixed. Both files now end with a trailing newline (confirmed via `tail -c 5` = `...0a`).

`make check` is green on the fixed HEAD.

Verdict unchanged: **approve_with_changes** (fix iteration complete).
