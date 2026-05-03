# CLO-290 Author calculator example spec at examples/specs/calculator.md

**Status:** draft
**Type:** specification
**Linear:** https://linear.app/cloud-ai/issue/CLO-290/author-calculator-example-spec-at-examplesspecscalculatormd

## 1. Problem and goal

Author the tiny example spec that the M6 end-to-end integration test (CLO-291) runs against. The spec must be small enough that a full `design-doc-tdd` run (design → review → implement → verify) completes within the M6 pre-merge gate target of ≤60 s (PRD §M6 line 213), yet real enough that each phase has something to do. The spec defines a calculator library with `add`, `subtract`, `multiply`, `divide` operations as pure functions over integers and floats, with typed error handling for division by zero. Language: Rust (matches existing `cargo test --message-format=json` tooling from CLO-273).

## 2. Acceptance criteria

- [ ] **AC1**: `examples/specs/calculator.md` exists and is non-empty. (**verification command:** `test -s examples/specs/calculator.md`)
- [ ] **AC2**: Spec contains all four expected H2 sections: `Requirements`, `Constraints`, `Out of Scope`, `Acceptance`. (**verification command:** `rg "^## " examples/specs/calculator.md | sort`)
- [ ] **AC3**: Spec defines `add`, `subtract`, `multiply`, `divide` as separate operations. (**verification command:** `rg "add|subtract|multiply|divide" -n examples/specs/calculator.md | wc -l`)
- [ ] **AC4**: Spec requires integer and float support for all four operations. (**verification command:** `rg "integer|float|int|float" -i examples/specs/calculator.md`)
- [ ] **AC5**: Spec requires division by zero to return a typed error (not a panic or a sentinel value like `None`). (**verification command:** `rg "division by zero|typed error|Err" -i examples/specs/calculator.md`)
- [ ] **AC6**: Spec requires all operations to be pure (no side effects, no mutation of inputs). (**verification command:** `rg "pure" -i examples/specs/calculator.md`)
- [ ] **AC7**: Spec requires results to be deterministic (same input → same output). (**verification command:** `rg "deterministic" -i examples/specs/calculator.md`)
- [ ] **AC8**: Spec lists no external dependencies for the library. (**verification command:** `rg "dependency|external|crate" -i examples/specs/calculator.md` (should return nothing or clearly limit to std))
- [ ] **AC9**: Spec describes the project as a library (not a binary). (**verification command:** `rg "library|binary" -i examples/specs/calculator.md`)
- [ ] **AC10**: Spec lists tests as living alongside code (not in a separate integration test directory). (**verification command:** `rg "tests live" -i examples/specs/calculator.md`)
- [ ] **AC11**: Spec's Acceptance section contains at least 3 concrete examples (e.g., `add(2, 3) == 5`, `divide(1, 0) -> Err`). (**verification command:** `rg "add\(" -A1 examples/specs/calculator.md | rg "^\s*[0-9]" | wc -l`)
- [ ] **AC12**: All code-fenced example blocks in the Acceptance section are valid Rust syntax. (**verification command:** `rg "```rust" -A5 examples/specs/calculator.md | rg -v "^```$" > /tmp/check_rust.rs && rustfmt --check /tmp/check_rust.rs 2>&1 | head -5`)

## 3. Sub-tasks

### ST1 Author the spec file
**Files:** `examples/specs/calculator.md`
**Tests:** none (this is the spec itself)
**Estimate:** S

Write the spec following the shape described in the issue: problem statement, functional requirements, constraints, out-of-scope, acceptance examples. Target 40–80 lines. No architecture decisions needed — purely a content authoring task.

### ST2 Verify spec meets structural requirements
**Files:** `examples/specs/calculator.md` (already written in ST1)
**Tests:** manual verification commands (AC1–AC12 above)
**Estimate:** XS

Run each verification command from AC1–AC12 and confirm all pass. No additional files needed.

## 4. Evaluation table

| # | Scenario | Input | Expected | Verification |
|---|---|---|---|---|
| 1 | File exists | `test -s examples/specs/calculator.md` | exit 0 | `test -s examples/specs/calculator.md && echo OK` |
| 2 | Four sections present | `rg "^## " examples/specs/calculator.md` | 4 lines (Requirements, Constraints, Out of Scope, Acceptance) | `rg "^## " examples/specs/calculator.md | wc -l` |
| 3 | Four operations named | `rg "add\|subtract\|multiply\|divide" examples/specs/calculator.md` | at least 4 occurrences | `rg "add\|subtract\|multiply\|divide" examples/specs/calculator.md | wc -l` |
| 4 | Division-by-zero error handling | `rg "division by zero\|typed error" -i examples/specs/calculator.md` | non-empty match | `rg "division by zero\|typed error" -i examples/specs/calculator.md` |
| 5 | 3+ acceptance examples | count concrete `add(`, `divide(` examples | ≥3 | `rg "add\([0-9]" examples/specs/calculator.md | wc -l` |
| 6 | Rust valid syntax | code blocks in Acceptance | valid Rust | `rg "```rust" -A5 examples/specs/calculator.md \| rg -v "^```$" > /tmp/check_rust.rs && rustfmt --check /tmp/check_rust.rs` |

## 5. Edge cases

- **Wording variants**: Accept any phrasing for `typed error` — `return an error`, `Err` variant, `Result<_, _>`, `Error` type — as long as panic/not-a-number is excluded.
- **Code block language**: Accept `rust`, `text`, or no language tag — the spec is for human readers; validation in CLO-291's test runner handles syntax checking.
- **Word count**: Hard cap is ≤80 lines to stay within M6 timing budget; target 40–60 lines.
- **No external deps**: If the spec mentions "standard library only" or "no crates needed", that satisfies the constraint.