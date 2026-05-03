# Pre-PR validation: clo-290

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-03
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

`make check` passes. Diff is doc-only (no Rust code changed). Reviewed against the workflow yaml's 12 ACs.

## Findings

### F1 [low] AC12 verification passes vacuously
**Where:** examples/specs/calculator.md:35 and specs/2026-05-03-clo-290-calculator-example-spec.md:31
**What:** The Acceptance code block uses an untagged fence (```), but AC12 greps for ```` ```rust ```` then runs `rustfmt --check`. With no `rust`-tagged blocks the rustfmt input is empty, so AC12 passes regardless of the contents. The block contents (`add(2, 3) == 5`, `divide(1, 0) -> Err(DivisionByZero)`) are pseudocode and would not parse as Rust if actually checked.
**Suggested fix:** Either drop AC12 (the spec is human-prose, code blocks are illustrative), or tag the fence as ```` ```text ```` and rewrite AC12 to assert the language tag rather than rustfmt validity. Don't tag it `rust` — the contents aren't valid Rust.

### F2 [low] divide(10, 2) == 5.0 mixes integer inputs with float result
**Where:** examples/specs/calculator.md:39
**What:** Spec states "All operations support both integer and floating-point inputs" but the int example `divide(10, 2)` produces `5.0`. In Rust, integer `/` returns an integer; producing `5.0` from int inputs requires either a generic-with-coercion API or a separate `divide_f64`. CLO-291's TestRunner integration may stumble on this ambiguity when the design phase has to pick a signature.
**Suggested fix:** Make integer division yield an integer (`divide(10, 2) == 5`) and add a separate float case (`divide(7.0, 2.0) == 3.5`), or explicitly state "divide always returns f64" in Requirements.

### F3 [trivial] Missing trailing newlines
**Where:** examples/specs/calculator.md:45, specs/2026-05-03-clo-290-calculator-example-spec.md:58
**What:** Both files end without a final newline (`\ No newline at end of file` in diff). Consistent POSIX-style trailing newline is the convention elsewhere in the repo.
**Suggested fix:** Add a trailing newline to both files.

### F4 [info] Plan file absent, but workflow yaml records "Skipping plan phase"
**Where:** docs/plans/ has no clo-290 entry
**What:** The validator instructions ask for the plan doc, but the workflow yaml shows the plan phase was intentionally skipped for specification-type tasks (auto-approval reason: mechanically testable, content authoring only). Not a defect — just confirming the absence is by design, not a missing artifact.
**Suggested fix:** None. Mention only because the reviewer prompt called for the plan.

## Verdict
approve_with_changes

The change is doc-only, scoped tightly to M6 prep (CLO-291's integration target), introduces no Rust code, and `make check` passes cleanly with no regressions. The spec content is correct in intent and meets 11 of 12 ACs substantively. F1 and F2 are worth fixing before this spec is consumed by the M6 phase-runner: the `divide` int/float ambiguity will surface as a real design decision when CLO-291 runs, and the vacuous AC12 weakens the discovery's "mechanically testable" justification. F3 is a trivial cleanup. None of the findings block the merge — they're improvements to land before M6 picks this up.
