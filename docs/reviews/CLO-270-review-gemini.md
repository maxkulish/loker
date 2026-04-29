# Design Review: CLO-270

**Reviewer**: Gemini 3.1 Pro
**Reviewed**: 2026-04-29
**Pipeline**: lok design-review

---

I now have enough context to review the design document for CLO-270.

Let's organize the review according to the requested criteria:

1.  **Completeness Check**:
    *   Summary/Problem: Present. Explains the current state (stand-in code) and the gaps (`FailureReason`, `VerifyContext`).
    *   Goals/Non-goals: Present and clear. Specifies what is included (trait, enum, context, tests, updating `EscalatingRetry`) and what is excluded (implementing concrete hooks, `pass_failure_context` end-to-end wiring).
    *   Architecture: Present. Explains module layout, data flow, and type taxonomy.
    *   Public API surface: Present. Shows the exact target state for `VerifyResult`, `FailureReason`, `VerifyContext`, and `VerifyHook`.
    *   Test plan: Present. Good coverage of unit tests and consumer tests.
    *   Migration/rollout: Present. Straightforward purely type-level change.
    *   Open questions: Present with resolutions.

2.  **Architecture Assessment**:
    *   **Strengths**:
        *   Introduces strong typing with `FailureReason` to replace a plain string.
        *   Uses `VerifyContext` to decouple hooks from `QueryOutput` directly, allowing future enrichment (e.g. manifest pointer, run-dir paths) without breaking trait signatures.
        *   Uses `#[non_exhaustive]` appropriately on `VerifyResult` and `VerifyContext` to allow forward compatibility.
        *   Clearly separates `VerifyError` (hook fault) from `VerifyResult::Fail` (hook ran and rejected output).
        *   Keeps `Aggregator` and `Strategy` trait boundaries intact.
    *   **Concerns**:
        *   `FailureContext` in `src/strategy/escalating_retry.rs` has `verify_reason: Option<String>`. In the `from_verify_fail` factory, the design doc says "The FailureContext::from_verify_fail function accepts impl Into<String> for the reason — after the change, it should accept &FailureReason instead." If it takes `&FailureReason`, how is it serialized into the envelope? The doc says "The existing reason.clone() call must be replaced with reason.summary.clone() (or the full FailureReason depending on what pass_failure_context needs)". Currently `pass_failure_context` only expects a `String` (and runs `redact_secrets` on it). The design doc leaves this slightly ambiguous: "that's the integration seam".
        *   `VerifyResult` `pass()` dropping `notes: Option<String>` is fine, as noted in the open questions.

3.  **Alignment with Handoff & Roadmap**:
    *   Matches docs/handoff.md intent (first-class verify hooks).
    *   Fits M4 (Phase 4) scope. It gates the rest of the Verify Hooks (T-021, T-022, T-023).
    *   No contradictions.

4.  **Security Review**:
    *   Strong typing of `FailureReason` prevents arbitrary things from being passed.
    *   The doc correctly identifies that `VerifyContext` should *not* carry credentials (API keys, tokens).
    *   Does not introduce shell execution (that's T-021 / CLO-271).
    *   The `FailureReason` struct carries `.stdout`, `.stderr`, and `.truncated` flags, which is good for bounding memory.

5.  **Implementation Concerns**:
    *   The transition for `escalating_retry.rs` is straightforward but needs care:
        ```rust
        previous_failure = self.pass_failure_context.then(|| {
            let reason = match &result {
                VerifyResult::Fail { reason } => reason.summary.clone(), // Must extract summary!
                _ => "verify did not pass".to_string(),
            };
            FailureContext::from_verify_fail(
                rung.tier,
                backend.name(),
                reason,
                Some(&query.stdout),
            )
        });
        ```
        The design doc correctly points this out: "The existing `reason.clone()` call must be replaced with `reason.summary.clone()`".
    *   The design specifies `VerifyContext::from_query_output` takes `&QueryOutput`.
    *   Testability with `make check` is very high since it's a pure Rust trait/struct refactoring.

6.  **Concurrency & Async**:
    *   `VerifyHook` remains `#[async_trait]` and `Send + Sync`.
    *   No blocking calls introduced.

7.  **Blind Spots**:
    *   The design doc does not explicitly state that `VerifyOutcome::failed(self.verify.name())` will remain unchanged when `VerifyResult::Fail` is matched. It's implied, but worth keeping in mind.
    *   Currently, `src/strategy/escalating_retry.rs` does:
        ```rust
        let passed = result.is_pass();
        // ...
        match &result {
            VerifyResult::Fail { reason } => reason.clone(),
            _ => "verify did not pass".to_string(),
        }
        ```
        The `VerifyResult` enum will now have `Repair` and `Score`. `EscalatingRetry` needs to treat them as *not passing* (which it currently will, since `passed = result.is_pass()`), but what should the `FailureContext` reason be? The match block `_ => "verify did not pass".to_string()` will catch them, which is probably correct for now.
    *   The `VerifyContext` struct has `pub structured: Option<serde_json::Value>`. This means `VerifyContext` will require `serde_json` to be available in `src/strategy/verify.rs`. This is fine.

8.  **Verdict**: APPROVE. The design is solid, complete, and exactly what is needed to unblock the rest of the Verify Hooks milestone.

Let's write down the feedback.
I have completed the review of the design document for CLO-270 (`docs/designs/clo-270-hook.md`) based on the provided context, PRDs, and implementation roadmap. 

The design is well thought out, strongly typed, and aligns perfectly with the roadmap and milestones. I have captured the synthesis of the review and stored it in `docs/reviews/CLO-270-review-synthesis.md`.

Here is the finalized review report:

## 1. Completeness Check
- **Problem & Goals**: Present and clear. Accurately scopes the work to type definitions, trait signature changes, and updating the single consumer (`EscalatingRetry`).
- **Architecture**: Present. Module layout, data flow, and type taxonomy are well-defined.
- **Public API surface**: Present. Thoroughly specifies the shapes of `FailureReason`, `VerifyResult`, `VerifyError`, `VerifyContext`, and `VerifyHook`.
- **Consumer update**: Present. Accurately identifies the changes required in `escalating_retry.rs`.
- **Test plan**: Present. Good coverage of unit tests and existing consumer tests.
- **Migration**: Present.
- **Open questions**: Present with clear resolutions.

## 2. Architecture Assessment
**Strengths**:
- **Strong Typing**: Moving away from stringly-typed reasons to `FailureReason` enables structured diagnostics and prepares for `pass_failure_context` improvements (CLO-260).
- **Forward Compatibility**: `VerifyContext` and `VerifyResult` are correctly marked `#[non_exhaustive]`, allowing the phase runner to inject more context later without breaking hook implementations.
- **Clear Separation of Concerns**: Explicit distinction between `VerifyResult::Fail` (output rejected by hook) and `VerifyError` (hook itself failed to run).
- **Security-minded**: Explicitly notes that credentials live in `BackendConfig` and are kept out of `VerifyContext`.

**Concerns**:
- None. The architectural choices are solid and fit the current boundaries well.

## 3. Alignment with Handoff & Roadmap
- The design perfectly aligns with the handoff intent: TDD-first, introducing `VerifyHook` primitives to replace stand-in code.
- It squarely addresses task T-020 (CLO-270) in the roadmap, which gates all subsequent Verify Hook tasks (T-021, T-022, T-023).
- Maintains backward compatibility with the existing v0 binary pass/fail semantics while reserving variants (`Repair`, `Score`) for future milestones.

## 4. Security Review
- The design correctly isolates credentials from the `VerifyContext`.
- Bounding stdout/stderr with truncation flags in `FailureReason` is a good defensive measure against memory exhaustion from runaway verifiers.

## 5. Implementation Concerns
- The transition for `src/strategy/escalating_retry.rs` is straightforward but requires care. When extracting the reason for `FailureContext`, using `reason.summary.clone()` (as noted in the design doc) is correct for this task, as the full `pass_failure_context` retrofit is deferred to CLO-260.
- For the reserved variants (`Repair`, `Score`), the existing logic in `EscalatingRetry` (`result.is_pass()`) will correctly treat them as failures. The fallback reason `"verify did not pass"` is appropriate for these variants until they are officially supported.

## 6. Concurrency & Async
- The `VerifyHook` trait remains `#[async_trait]` and `Send + Sync`.
- The factory method `VerifyContext::from_query_output` is synchronous and inexpensive. No blocking calls are introduced.

## 7. Blind Spots
- `VerifyContext` relies on `serde_json::Value` for the `structured` field. Ensure `serde_json` is imported in `src/strategy/verify.rs`.

## 8. Verdict
**APPROVE**

## 9. Actionable Feedback
1. **Proceed with Implementation**: The design is approved. You can begin implementation following the TDD approach outlined in the roadmap.
2. **EscalatingRetry Integration**: When updating `escalating_retry.rs`, ensure that `FailureContext::from_verify_fail` uses `reason.summary.clone()` for the `VerifyResult::Fail` arm, and relies on the fallback string for the reserved variants (`Repair`, `Score`).
