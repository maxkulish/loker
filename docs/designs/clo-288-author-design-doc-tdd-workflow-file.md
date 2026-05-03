# Design: CLO-288 - Author the canonical design-doc-tdd workflow file at .lok/workflows/design-doc-tdd.toml

## 1. Problem

Per the discovery report, `.lok/workflows/design-doc-tdd.toml` does not exist. CLO-287 shipped the `[[phases]]` grammar parser (PR #35), but the canonical workflow file - the byte-for-byte fixture that PRD §UC-1 (lines 63-72) prescribes as loker's reference flow - is still represented by a hand-rolled placeholder at `tests/fixtures/workflows/design-doc-tdd.toml` whose phase names (`research`, `design`, `implement`, `review`) and structure do not match the PRD. Workflow authors who try to run the reference flow have nothing to point at, and the M6 end-to-end integration test (T-037) is blocked until this file lands at the documented path.

## 2. Goals / Non-goals

### Goals

- Land `.lok/workflows/design-doc-tdd.toml` with the four PRD-mandated phases (`design`, `review`, `implement`, `verify`), in that topological order.
- Use the strategy variants the CLO-287 grammar accepts: `{ single = {} }`, `{ parallel = { min_responses = N } }`, `{ escalating = { pass_failure_context = true } }`.
- Replace the placeholder fixture at `tests/fixtures/workflows/design-doc-tdd.toml` with a copy of the canonical file (or, preferably, point the round-trip test at the canonical path via `include_str!`).
- Top-of-file comment block explaining backend swap points, the local-reviewer slot (`ollama/glm-5.1`), and degradation behavior when a reviewer fails to instantiate.
- Commented `cost_budget_usd` placeholder so M6 wiring (per PRD line 35) has an obvious home.
- Add a focused TDD test file `tests/workflows_design_doc_tdd.rs` covering round-trip parse, phase shape, family diversity, hook wiring, topological inputs, and forward-compat tolerance for `[phase.contract]`.
- `make check` green at the end.

### Non-goals

- Per-phase prompt templates (T-035).
- Authoring the example spec the workflow consumes (T-036, `examples/specs/calculator.md`).
- The end-to-end integration test that drives the workflow (T-037, M6).
- HITL review phase - the reference flow stays automated; HITL lands in M10.
- Adding hook fields (`test_runner`, `run_command`, `llm_verifier`) to the grammar if they don't already exist - that becomes a follow-up, see Open Questions.
- Changing the grammar parser shipped in CLO-287.

## 3. Architecture

This task is fixture-and-test, not a runtime change. The artifact is a TOML file consumed by the existing parser; there are no new Rust modules.

```
+-----------------------------------------+
| .lok/workflows/design-doc-tdd.toml      |  <-- canonical file (this task)
+--------------------+--------------------+
                     |
                     | include_str!()
                     v
+-----------------------------------------+
| tests/workflows_design_doc_tdd.rs       |  <-- TDD test contract (this task)
+--------------------+--------------------+
                     |
                     | Workflow::from_str()
                     v
+-----------------------------------------+
| src/workflow/grammar.rs (CLO-287)       |  <-- existing parser
|   Strategy { Single | ParallelFanOut    |
|              | EscalatingRetry }        |
|   Phase { name, strategy, backends,     |
|           prompt_template, inputs,      |
|           output, contract }            |
|   Workflow { name, description, phases, |
|              defaults }                 |
+--------------------+--------------------+
                     |
                     | family_of(backend_id) (CLO-265)
                     v
+-----------------------------------------+
| family_of: split_at last '/' suffix     |
| -> family from suffix mapping           |
+-----------------------------------------+
```

### Phase data flow

```
spec ──► design ──► review ──► implement ──► verify
                       │           ▲
                       └───────────┘ (review feeds implement alongside design)
```

Inputs per phase:

| Phase     | Strategy                                      | Inputs                       | Output       |
|-----------|-----------------------------------------------|------------------------------|--------------|
| design    | `single`                                      | `spec`                       | `design.md`  |
| review    | `parallel` (min_responses = 2)                | `phase:design`               | `review.md`  |
| implement | `escalating` (pass_failure_context = true)    | `phase:design`, `phase:review` | `changes/` |
| verify    | `parallel` (min_responses = 1)                | `phase:implement`            | `verify.json`|

The aggregator hint for `verify` is `any_fail`. Whether the parser already accepts an aggregator key on the phase or whether it lives at the workflow defaults level is captured in Open Questions.

### Hook wiring (subject to grammar support - see Open Questions)

- `implement` carries a verify hook of kind `test_runner`.
- `verify` carries hooks of kind `run_command` (build + lint) and `llm_verifier`.

If the current `Phase` struct in `src/workflow/grammar.rs` lacks a `hooks` field, the canonical file still uses the documented hook syntax behind a `[phase.contract]`-style block that the parser tolerates as forward-compat (FR-31). The design document does not invent grammar fields; it documents the intended shape and lets Open Questions resolve the gap.

## 4. Public API surface

This task does not introduce new public Rust types. It exercises the surface CLO-287 already shipped:

```rust
// src/workflow/grammar.rs (existing, CLO-287 - reproduced for reference, not changed)
pub struct Workflow {
    pub name: String,
    pub description: Option<String>,
    pub phases: Vec<Phase>,
    pub defaults: Option<Defaults>,
}

pub struct Phase {
    pub name: String,
    pub strategy: Strategy,
    pub backends: Vec<String>,
    pub prompt_template: Option<String>,
    pub inputs: Vec<InputRef>,
    pub output: Option<String>,
    pub contract: Option<toml::Value>, // FR-31 reservation
}

pub enum Strategy {
    Single,
    ParallelFanOut { min_responses: usize },
    EscalatingRetry { pass_failure_context: bool },
}

impl Workflow {
    pub fn from_str(s: &str) -> Result<Self, GrammarError>;
    pub fn lint(&self) -> Vec<LintWarning>;
}
```

### File contents - `.lok/workflows/design-doc-tdd.toml`

The canonical file is the public artifact. Sketch:

```toml
# design-doc-tdd: loker's canonical reference workflow (PRD §UC-1).
#
# Backend swap points:
#   - design.backends:    a single strong design model.
#   - review.backends:    must span >= 2 families (FR-13). Local-reviewer
#                         slot is `ollama/glm-5.1` per PRD line 271. If
#                         Ollama is unavailable the run degrades to the
#                         remaining reviewers, gated by min_responses = 2.
#   - implement.backends: ordered cheap -> strong (escalating retry).
#
# cost_budget_usd: <PLACEHOLDER>  # wired through summary.json in M6.

name = "design-doc-tdd"
description = "Four-phase design -> review -> implement -> verify pipeline."

[[phases]]
name = "design"
strategy = { single = {} }
backends = ["tensorzero/design"]
inputs = ["spec"]
output = "design.md"

[[phases]]
name = "review"
strategy = { parallel = { min_responses = 2 } }
backends = ["claude/sonnet", "gemini/pro", "ollama/glm-5.1"]
inputs = ["phase:design"]
output = "review.md"

[[phases]]
name = "implement"
strategy = { escalating = { pass_failure_context = true } }
backends = ["ollama/qwen-coder", "claude/sonnet", "codex/strong"]
inputs = ["phase:design", "phase:review"]
output = "changes/"

[[phases]]
name = "verify"
strategy = { parallel = { min_responses = 1 } }
backends = ["run_command/build-and-lint", "llm_verifier/judge"]
inputs = ["phase:implement"]
output = "verify.json"
```

Concrete backend identifiers are illustrative - they reference entries in `lok.toml` / TensorZero config and may be tuned during review without changing this design.

## 5. Test plan

New file: `tests/workflows_design_doc_tdd.rs`. Each test parses the canonical file via `include_str!("../.lok/workflows/design-doc-tdd.toml")`.

### Unit tests

1. `roundtrip_parses_clean` - `Workflow::from_str(...)` returns `Ok` and `lint()` returns no errors (warnings about reserved `phase.contract` are tolerated).
2. `phase_names_and_order_match_prd` - phase names are exactly `["design", "review", "implement", "verify"]` in that order.
3. `phase_strategies_match_prd` - asserts each phase's `Strategy` variant: `Single`, `ParallelFanOut { min_responses: 2 }`, `EscalatingRetry { pass_failure_context: true }`, `ParallelFanOut { min_responses: 1 }`.
4. `review_phase_spans_two_families` - applies `family_of` (CLO-265) over the review phase's backends; `HashSet` size >= 2 (FR-13).
5. `implement_phase_backends_ordered_cheap_to_strong` - asserts the backend list length >= 2 and that the order is preserved.
6. `inputs_are_topologically_valid` - for every phase, every `phase:X` input names a phase that appears earlier in the array; `spec` and `var:` prefixes are accepted; nothing else is.
7. `verify_hook_kinds_present` (gated on grammar support - see Open Questions) - implement-phase has a `test_runner` hook; verify-phase has both `run_command` and `llm_verifier` hooks. If hooks are not yet in the grammar, this test is `#[ignore]` with a comment pointing at the follow-up.
8. `forward_compat_phase_contract_block_parses` - take the canonical file string, append a `[phases.contract]` table to one phase, parse - must succeed; `lint()` returns the FR-31 reservation warning.

### Integration tests

- The replaced fixture at `tests/fixtures/workflows/design-doc-tdd.toml` (or its replacement: `include_str!` from the canonical path) is exercised by CLO-287's existing round-trip test - it must continue to pass.

### Manual verification

- `make check` is green.
- `cargo test --test workflows_design_doc_tdd` is green.
- `rg -n "design-doc-tdd" tests/` shows the new test file references the canonical path, not the old fixture path.

## 6. Migration / rollout

- **Fixture replacement**: the placeholder at `tests/fixtures/workflows/design-doc-tdd.toml` is removed in the same change, and CLO-287's round-trip test is repointed to `include_str!("../../.lok/workflows/design-doc-tdd.toml")`. If repointing is risky in this PR, the fixture is rewritten to be a byte-identical copy of the canonical file and a follow-up issue tracks the redirection.
- **No backward-compatibility surface**: no other code currently consumes the old phase names (`research`, `design`, `implement`, `review`) - the placeholder existed only to exercise the parser.
- **No feature flags**: the file is inert until a workflow runner consumes it; M6 picks it up.
- **Rollout order**: (1) land the canonical TOML file, (2) land the new test file, (3) repoint or rewrite the fixture, (4) verify `make check`. Single PR.

## 7. Open questions

1. **Does the CLO-287 grammar's `Phase` struct already carry a `hooks` field?** Discovery flagged this as unverified. If hooks are absent, the canonical file either (a) omits the hook lines and a follow-up extends the grammar, or (b) places hook configuration under a `[phase.contract]` block that today's parser tolerates and tomorrow's parser interprets. Tradeoff: option (a) keeps the file faithful to the parser at the cost of a delayed PRD §UC-1 fidelity; option (b) keeps the file PRD-shaped at the cost of behavior that won't run end-to-end until a later milestone wires hooks. **Resolution must come from reading `src/workflow/grammar.rs` before authoring.**
2. **Where does the verify-phase aggregator hint (`any_fail`) live?** The grammar's `Strategy::ParallelFanOut` carries only `min_responses`. The aggregator is either (a) a separate phase-level field (`aggregator = "any_fail"`), (b) a workflow-level default that phases inherit, or (c) implicit in the strategy variant for the verify case. The canonical file should pick one form, and the choice depends on what CLO-287 actually accepts. Tradeoff: (a) is most explicit but requires a grammar field; (b) reads cleanly for the common case but hides per-phase intent; (c) is invisible and risks confusion. Reviewers should pin this before merge.
3. **Concrete backend identifiers** - the file lists illustrative names (`tensorzero/design`, `claude/sonnet`, `gemini/pro`, `ollama/glm-5.1`, `ollama/qwen-coder`, `codex/strong`). The actual identifiers depend on the entries that ship in `lok.toml` / `tensorzero/config/tensorzero.toml`. The PRD pins only the *families* and the local-reviewer slot. Reviewers should swap these to whatever is canonical at merge time without changing the design intent.
4. **Should the canonical file double as the test fixture, or should the fixture be a byte-identical copy?** The PRD's acceptance criterion says "byte-for-byte fixture used by CLO-287's round-trip test (replace its hand-rolled placeholder)." `include_str!` from the canonical path keeps a single source of truth; a duplicate copy under `tests/fixtures/` keeps the test self-contained but invites drift. Default: `include_str!` from the canonical path. Flag any reason this won't work.
