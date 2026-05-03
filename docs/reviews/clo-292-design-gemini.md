# Design Review: CLO-292

**Reviewer**: Gemini 2.5 Pro
**Reviewed**: 2026-05-03
**Pipeline**: manual Gemini invocation after lok design-review failed to write outputs

---

## 1. Completeness Check

The design document is comprehensive and contains all necessary sections, though with slightly different naming.

- **Present**: Problem, Goals / Non-goals, Architecture, Public API surface, Test plan, Migration / rollout, and Open questions. All sections are well-detailed and meaningful.

## 2. Architecture Assessment

**Strengths**:

- **Clear Separation of Concerns**: The design introduces a `PhaseRunner` as a pure coordinator, correctly separating the orchestration logic from the underlying strategy, aggregation, and verification primitives. This is a clean, modular approach that aligns well with the project's intent.
- **Decoupled Configuration**: Using simple enums (`StrategyName`, etc.) in `PhaseConfig` and resolving them to `Arc<dyn ...>` implementations at the last moment is excellent. This decouples the workflow definition from the runtime implementation, as intended by the roadmap (T-030).
- **Filesystem as Source of Truth**: The architecture correctly relies on filesystem markers and artefacts for state, adhering to the project's principle of resumability without requiring a live daemon. The proposed private `persist` module to handle the write protocol is a good encapsulation of this logic.
- **Testability**: The composition model makes the `PhaseRunner` highly testable with mocks, which is a core project requirement.

**Concerns**:

- The introduction of a new `AggregatorAdapter` trait feels slightly redundant when the existing `aggregator::Aggregator` enum could likely be extended. Adding `First` and `AllPass` variants to the existing enum seems cleaner than introducing a new, parallel trait for the same purpose.

## 3. Alignment with Handoff & Roadmap

The design is perfectly aligned with project goals.

- It directly addresses task T-028 from the implementation roadmap, unblocking M5 and M6.
- It respects the TDD-first and mock-the-HTTP-layer conventions from `docs/handoff.md` by providing a detailed, mock-based test plan.
- It follows the don't-mutate-in-place principle by adding a new `phase_runner` module rather than altering the existing `consensus.rs` flow.

## 4. Security Review

The design's security posture is adequate for this component, but relies on downstream implementation details.

- The primary security-sensitive component invoked is the `VerifyHook`, particularly `RunCommand`. The design correctly identifies this but defers sandboxing details to the existing hook implementation. The `PhaseRunner` itself does not introduce new security risks; it orchestrates existing components.

## 5. Implementation Concerns

The implementation plan is solid.

- **API Surface**: The proposed public API in `src/phase_runner.rs` is minimal and well-defined. The re-exports from `lib.rs` are appropriate. The error enum `PhaseError` is comprehensive and provides good diagnostic potential.
- **Test Plan**: The test plan is excellent. It covers unit tests for helpers, integration tests mapped directly to PRD acceptance criteria, and a clear manual verification step. It correctly emphasizes the use of wiremock and stubs to ensure tests run without a live network.

## 6. Concurrency & Async

The proposed API correctly uses `async` and the data flow is logical for a `tokio`-based runtime. The use of `Arc<dyn ...>` for strategies and hooks is idiomatic for sharing thread-safe implementations across async tasks. The design does not raise any obvious concurrency red flags.

## 7. Blind Spots

The design document does an excellent job of identifying its own blind spots in the Open questions section. These are not failures of the design, but rather a mature acknowledgment of necessary follow-up decisions.

- Branch debris and canonical bytes are important details for run-state consistency and diagnostics.
- `AggregatorAdapter` is the most significant architectural point of discussion. The trade-off is well-articulated, but leaning towards extending the existing enum seems more consistent.
- Error class mapping is crucial for downstream consumers. This mapping should eventually be formalized in schema documentation.

## 8. Verdict

APPROVE_WITH_SUGGESTIONS

## 9. Actionable Feedback

1. **Resolve AggregatorAdapter**: Before implementation, make a final decision on whether to introduce `AggregatorAdapter` or extend the existing `aggregator::Aggregator`. Recommendation: extend the existing enum to avoid a parallel abstraction.
2. **Formalize Error Class Mapping**: Update or document the relevant schema contract to include allowed `error_class` strings.
3. **Clarify Artefact I/O**: Document the chosen approach for handling branch debris and reading canonical bytes. The runner should probably receive bytes directly from the `StrategyOutput` to minimize I/O, or explicitly read from the surfaced output path if that is the existing contract.
4. **Define `all_pass` Behavior**: The implementation should explicitly define whether it short-circuits or gathers all failures. Recommendation: wait for all verifications to complete to provide full diagnostics, and add a test for this case.
