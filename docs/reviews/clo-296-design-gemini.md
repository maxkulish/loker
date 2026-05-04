# Design Review: CLO-296

**Reviewer**: Gemini 2.5 Pro
**Reviewed**: 2026-05-03
**Pipeline**: lok design-review (Gemini only; Ollama/Codex unavailable)
---

## 1. Completeness Check
All required sections are present and well-detailed: Summary, Background (as "Problem"), Architecture, Detailed Design (as "Concrete Rust types"), Implementation Plan (as "Test plan" and "Migration"), and Acceptance Criteria (within "Goals"). The inclusion of "Open questions" is particularly thorough and demonstrates proactive design thinking.

## 2. Architecture Assessment
**Strengths**:
- The proposed architecture is clean and follows established patterns within the `loker` codebase, specifically mirroring the `TraceSink`/`TraceWriter` pattern for `SummarySink`/`SummaryWriter`. This promotes consistency and reduces cognitive load.
- The separation of concerns is excellent, with distinct modules for reading traces (`TraceReader`), handling pricing (`PriceTable`), and writing the summary. This makes the design modular and easy to test.
- The data flow is logical and moves from raw data (trace files, markers) to an aggregated, structured format, which is a robust ETL-like pattern.
- The plan for idempotency on resume (`re-finalize overwrites the existing summary.json`) is critical for a resumable workflow engine and is correctly identified as a requirement.

**Concerns**:
- The design proposes reading and aggregating the entire `trace.jsonl` file in memory at once. For very large runs with extensive tracing, this could become a performance bottleneck. This is acceptable for v0 but should be noted as a potential area for future optimization (e.g., streaming aggregation).

## 3. Alignment with Handoff & Roadmap
The design is perfectly aligned with project documents:
- **`docs/handoff.md`**: It respects the TDD-first convention by defining a clear test plan upfront. It is an additive feature, creating new modules rather than mutating existing ones, which aligns with the handoff's intent.
- **`docs/plans/001-implementation-roadmap.md`**: This work corresponds to task T-032, which is correctly placed in Phase 6, depending on the completion of the phase runner and trace infrastructure.
- **`docs/prd/2026-04-25-loker.md`**: The design directly addresses functional requirements FR-23 (`summary.json`) and FR-23a (`cost_budget_usd`).

## 4. Security Review
The security posture is sound for this feature. As `summary.json` is a derivative artefact, the primary security responsibility lies with the upstream trace writer to redact secrets. The design does not introduce new security risks. The proposed loading of `prices.toml` from a file path is safe, assuming standard path validation is implemented to prevent traversal attacks, which is a reasonable expectation for production-quality Rust code.

## 5. Implementation Concerns
The implementation plan is solid. The test plan is comprehensive, covering unit, schema, and fixture-based testing. The "Open questions" section is a major strength, as it identifies and proposes sensible solutions for key implementation details and potential ambiguities:
- The recommendations for `prices.toml` location (Q1) and `cost_budget_usd` sourcing (Q2) are excellent, favoring configurability and decoupling.
- The proposed logic for computing run status (Q3), handling partial pricing (Q4), and managing manifest entries on resume (Q5) are robust and address important edge cases.

One minor point for clarification is the error handling for a malformed `prices.toml` file. While the design correctly states that a *missing* price entry should not fail the summary, it should also specify that a TOML *syntax error* should behave similarly (log a warning, result in all costs being `None`) rather than causing `SummaryWriter::finalize` to fail with a `SummaryError::Price`.

## 6. Concurrency & Async
The design operates at the end of a run, so it avoids complex concurrency issues with active phases. The proposed use of `Mutex` for the `InMemorySummarySink` is appropriate for testing. The overall approach is simple and does not introduce concurrency-related risks. It correctly builds on the project's atomic write protocols defined in `docs/run-state.md`.

## 7. Blind Spots
The design is exceptionally thorough, leaving few blind spots. As mentioned in the architecture assessment, the main unaddressed point is the performance consideration for extremely large trace files. This is a reasonable omission for an initial design but should be kept in mind for future iterations if `loker` is used for workflows that generate millions of trace events.

## 8. Verdict
APPROVE

## 9. Actionable Feedback
1.  **Endorse proposals from "Open Questions"**: The proposed solutions in the "Open questions" section are well-reasoned. I recommend formally adopting them as part of the implementation plan. Specifically:
    -   **Q1 (Pricing)**: Implement the "default-with-override" model for `prices.toml`.
    -   **Q2 (Budget)**: Pass `cost_budget_usd` as an `Option<f64>` parameter to `finalize()`.
    -   **Q5 (Manifest)**: Implement a "replace existing" or "update" logic in the manifest for the summary entry on re-finalize.
2.  **Clarify `prices.toml` Parse Error Handling**: Explicitly state in the design that a syntax error while parsing `prices.toml` should be a non-fatal error for summary generation. It should result in all `cost_usd` fields being `None`, accompanied by a warning in the logs, rather than causing `SummaryWriter::finalize` to return an error.
3.  **Acknowledge Performance as a Non-Goal**: Add a note acknowledging that the initial implementation of `TraceReader` will not be optimized for multi-gigabyte trace files, and that a streaming aggregation approach is a potential post-v0 enhancement if performance becomes an issue.
