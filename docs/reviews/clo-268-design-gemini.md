# Design Review: CLO-268 — Aggregator::LLMJudge with cross-family enforcement

**Reviewer:** gemini-architect (simulated)
**Input:** `docs/designs/clo-268-llm-judge.md`
**Date:** 2026-04-28

## Verdict

**approve_with_changes**

The design follows the established M3 aggregator pattern and correctly leverages CLO-265's `family_of` infrastructure. The decision to place judge logic in `src/aggregator/llm_judge.rs` is consistent with `concat.rs` and keeps `parallel_fanout.rs` from growing unmanageably. The test plan is concrete and covers the three required AC dimensions: family-overlap refusal, opt-out path, and snapshot validation.

### Key Strengths
- **Pattern consistency:** `llm_judge.rs` mirrors `concat.rs` in module placement, config-through-enum, and `Result<AggregatedArtifact, AggregatorError>` return shape.
- **Clean error taxonomy:** `LLMJudgeError` handles family, backend, transport, and contract errors internally, then maps to `PhaseError` at the strategy boundary. This prevents `PhaseError` from becoming a kitchen-sink enum.
- **Index clamping:** Saturating `min(candidates.len() - 1)` on judge response is a safe v0 guard against hallucinated indices.
- **Reusable markdown-fence stripping:** Calls the existing `strip_markdown_fences` helper rather than duplicating parsing logic.

### Actionable Findings & Suggestions

#### 1. Prompt template default
The design mentions an "example default template" but does not specify whether loker ships a built-in default or whether the template is always user-supplied.
- **Recommendation:** Clarify that `prompt_template` is mandatory (no built-in default). A future iteration may provide a default, but v0 requires explicit configuration to avoid hidden prompt assumptions.

#### 2. Candidate output truncation
Candidate outputs could be arbitrarily large (e.g. 100K tokens from a code-generation phase). Injecting raw outputs into a ballot prompt without truncation risks exceeding the judge backend's context window.
- **Suggestion:** Add a note that v0 does NOT truncate. Document this as an open question / follow-up. Workflow authors control candidate backends and can limit output length via backend config.

#### 3. `AggregateInput` expansion
Adding `backends` and `ctx` to `Aggregator::aggregate()` is a minor breaking change for internal callers.
- **Recommendation:** Ensure `Concat` and `AnyFail` call sites are updated in the same PR. The design already notes this; add a checklist item to the plan phase.

#### 4. Warning scope for opt-out
The family-check helper logs a warning when `require_different = false` and overlap is detected.
- **Suggestion:** Use `log::warn!` (not `println!`) and include both the judge backend name and the overlapping candidate name so operators can grep logs for `"opted out"`.

#### 5. `BackendError` to `PhaseError::JudgeUnavailable` mapping
The design maps ALL `BackendError` variants to `JudgeUnavailable`.
- **Suggestion:** This is correct for v0. A future refinement may distinguish `JudgeUnavailable` (transient) from `JudgeAuth` (configuration), allowing retry or HITL escalation. No change needed now.

### Implementation Risks
- **Low.** The scope is additive, `PhaseError` is `#[non_exhaustive]`, and the family check is pure logic already tested in CLO-265.
