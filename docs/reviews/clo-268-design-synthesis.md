# Design Review Synthesis: CLO-268 — Aggregator::LLMJudge with cross-family enforcement

## Verdict

**approve_with_changes**

Gemini approved the architecture with clarifications. The design remains Approach B from discovery: extract `llm_judge.rs` under `src/aggregator/`.

## Applied suggestions

1. **Prompt template default** — clarified that `prompt_template` is mandatory in v0; no built-in default shipped.
2. **Warning scope** — `log::warn!` message includes both judge backend and overlapping candidate name for grep-friendly ops logs.
3. **Candidate truncation** — added an open-question note that v0 does not truncate candidate outputs; workflow authors control candidate length.

## Flagged suggestions

1. **BackendError granularity for judge** — flagged for future consideration. Mapping all `BackendError` variants to `PhaseError::JudgeUnavailable` is correct for v0. A future refinement may introduce `JudgeAuth`, `JudgeRateLimit`, etc. Deferred to post-T-029.

## Final assessment

The design is ready for plan. Public API signatures expand `aggregate()` with two additive parameters; `PhaseError` adds one variant; the test plan lists 9 unit tests and 5 integration tests with concrete inputs and expected outputs. All open questions are resolved or explicitly deferred.
