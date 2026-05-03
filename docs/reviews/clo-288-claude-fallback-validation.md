# Pre-PR validation: clo-288

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-03
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

`make check` is green. All tests pass (8 in `workflows_design_doc_tdd`, 1 ignored as planned; existing CLO-287 grammar tests unchanged). Now writing the review.

## Findings

### F1 [major] ST3 fixture replacement deferred without a tracked follow-up — misses PRD acceptance criterion
**Where:** `tests/fixtures/workflows/design-doc-tdd.toml` (untouched); `docs/status/clo-288-workflow.yaml:80`
**What:** The plan's ST3 (`design-doc:` plan §ST3) and the design doc §6 both call for the placeholder fixture to be rewritten as a byte-identical copy of the canonical file *or* the CLO-287 test repointed via `include_str!`. Neither happened. The placeholder fixture still has the wrong phase names (`research`/`design`/`implement`/`review` with the *old* meaning) and `byte_for_byte_design_doc_tdd` (`tests/workflow_grammar.rs:28`) still asserts those names. The PRD line "byte-for-byte fixture used by CLO-287's round-trip test (replace its hand-rolled placeholder)" is not satisfied. Status YAML records this as "deferred — CLO-287 test has hard-coded expectations for old fixture content," but no follow-up issue is filed and no comment in either file points at the gap.
**Suggested fix:** Either (a) update `tests/workflow_grammar.rs:byte_for_byte_design_doc_tdd` so its assertions match the new canonical file and rewrite the placeholder as a byte-identical copy, or (b) leave the byte-for-byte test alone but rename the placeholder fixture to something like `tests/fixtures/workflows/legacy-placeholder.toml` and file a CLO-### follow-up to delete it once the byte-for-byte test moves to the canonical path. Today's state silently misses a stated acceptance criterion.

### F2 [minor] `[phases.contract]` block landed on the wrong phase
**Where:** `.lok/workflows/design-doc-tdd.toml:35-39`
**What:** The design intended hook configuration (under `[phase.contract]` per Option B) to live on the `implement` and `verify` phases (`docs/designs/clo-288-...:84-88`). The actual file places an empty `[phases.contract]` table on the `review` phase only (lines 37-39, body is comments). The `implement` (lines 48-50) and `verify` (lines 59-61) phases have entire `[phases.contract]` headers commented out — so `Phase.contract` is `None` for those phases. Net result: lint emits a `phase.contract reserved for post-v0` warning for `review` (which the design did not call out as a hook host) and silence for the two phases that the design did call out.
**Suggested fix:** Move the live (uncommented) `[phases.contract]` header from review to implement, and either uncomment the verify-phase block or delete it. Optionally drop the empty contract on review entirely; an empty marker table adds noise without conveying intent.

### F3 [minor] Top-of-file comment block contradicts the actual backends list
**Where:** `.lok/workflows/design-doc-tdd.toml:6-13`
**What:** The header says the local-reviewer slot is `ollama/` (no model). The review phase backends list (line 31) uses `ollama/qwen3-coder-next`. The header also says verify uses "any backend that can run a command," but the verify phase (line 55) uses `codex/` and `gemini/` — both pure LLM backends, not run-command capable. A reviewer reading the comment block first will form an incorrect mental model of the workflow.
**Suggested fix:** Either (a) update the comment block to name `ollama/qwen3-coder-next` explicitly and describe verify as "two-LLM consensus verifier (M6 will swap to run_command + llm_verifier hooks)" or (b) keep the comments aspirational and add a single `# Note:` line acknowledging that current backends are placeholders pending hook-grammar support.

### F4 [minor] Discovery-debt items have no tracked follow-ups
**Where:** `docs/status/clo-288-workflow.yaml:34-37` (hooks gap); design `Open Questions` 1 & 2
**What:** OQ1 (Phase struct lacks `hooks` field) and OQ2 (Strategy::ParallelFanOut lacks `aggregator` field) were resolved by deferring rather than fixing. The `#[ignore]` on `implement_phase_has_test_runner_hook` (`tests/workflows_design_doc_tdd.rs:100`) carries a comment that says "follow-up to add hooks support" but no Linear/CLO ticket is referenced and none is filed.
**Suggested fix:** File two CLO follow-ups (grammar-extends-hooks; strategy-extends-aggregator) and reference them in the `#[ignore]` reason and in the canonical TOML's hook-block comments. Otherwise these gaps fall out of view between this PR and M6.

### F5 [nit] `output = "changes/"` — directory-shaped output is grammatically valid but semantically untested
**Where:** `.lok/workflows/design-doc-tdd.toml:47`
**What:** `Phase.output` is a free-form `String` in the parser, so `"changes/"` parses. But no other phase in the codebase or fixtures uses a trailing-slash form, and downstream M6 wiring may treat this as a literal filename including the slash. The trailing slash is not asserted by any test in this PR.
**Suggested fix:** Either drop the trailing slash (leave it as `changes`) or add a comment in the TOML noting that M6 must interpret `output` ending in `/` as a directory; flag in the M6 task (CLO-271 / CLO-273) so it isn't a surprise.

### F6 [nit] `roundtrip_parses_clean` tolerates *any* lint warning containing the substring
**Where:** `tests/workflows_design_doc_tdd.rs:42-47`
**What:** The loop asserts every warning contains `"phase.contract reserved for post-v0"`, which is correct for today's lint. But it doesn't bound *how many* such warnings are expected. With F2 fixed, the count would change from 1 to 1; with future lint additions the test would silently swallow new warning categories that happen to share that substring.
**Suggested fix:** Replace with `assert_eq!(warnings.len(), <expected>)` plus the substring check. Cheap and tightens the contract.

## Verdict
approve_with_changes

The core deliverables land cleanly: canonical TOML at the documented path, eight-test contract green, `make check` green, and the CLO-287 grammar tests untouched. F1 is the only finding I'd block on — the PRD's "byte-for-byte fixture replaces the placeholder" criterion is unmet and not tracked anywhere durable, which means the placeholder will quietly drift indefinitely. F2 (wrong-phase contract) is small but signals copy-paste rather than design intent and is worth fixing in the same PR. F3-F6 are polish; ship them now or fold into the M6 wiring task.
