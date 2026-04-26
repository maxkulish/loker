# Design Review: m1-tensorzero

**Reviewer**: Gemini 3.1 Pro
**Reviewed**: 2026-04-26
**Pipeline**: lok design-review

---

## 1. Completeness Check
- **Summary**: Missing. No high-level overview of why TensorZero is being integrated and what its role is.
- **Background**: Missing. Does not explain the relationship with the `genai` crate or upstream `lok` context.
- **Architecture / Detailed Design**: Missing. Lacks detail on how the HTTP routing works, how model namespaces are mapped, and how provider families are derived.
- **Implementation Plan**: Present ("Implementation steps"), but incomplete as it completely ignores the D1 spike findings (Task T-005).
- **Acceptance Criteria**: Partially covered by the "Test contract", but missing specific scenarios discovered during the D1 spike.

## 2. Architecture Assessment
**Strengths**:
- Reuses the `genai` crate's `AdapterKind::OpenAI` to avoid writing a custom HTTP client from scratch.
- Cleanly scopes the integration behind the `Backend` trait, maintaining coexistence with legacy `reqwest` backends (e.g., `claude`, `ollama`).
- Opt-in integration tests (`LOKER_TZ_INTEGRATION=1`) ensure CI stays fast and decoupled from live gateway dependencies.

**Concerns**:
- Fails to address how `backend_id` maps to the TensorZero function name.
- Fails to specify how loker will extract the model family (e.g., Anthropic vs. OpenAI) for cross-family aggregation constraints.
- Ignores critical architectural failure modes: TensorZero obscures upstream errors (401, 429) inside 502 gateways errors. The design must address this to avoid infinite retry loops on permanent auth failures.

## 3. Alignment with Handoff & Roadmap
The document aligns with the high-level M1 goal in the PRD, but **violates the active implementation roadmap**. 
Roadmap task T-005 explicitly requires reconciling the in-flight `tensorzero.rs` with the D1 spike findings (`docs/spikes/2026-04-25-tensorzero-roundtrip.md`). The design document completely ignores D1, leaving the implementation steps and test contract dangerously outdated.

## 4. Security Review
- **Authentication**: The design does not clarify how `genai` passes credentials to TensorZero. It needs to note that the API key sent by loker is for the gateway, while upstream keys (`OPENAI_API_KEY`, etc.) are managed by TensorZero.
- **Error Logs**: By inspecting 502 bodies to map upstream errors (as required by D1), there is a risk of logging or leaking sensitive upstream provider URLs or internal gateway state. Ensure error mappings truncate or redact raw HTML/JSON bodies when bubbling up to `BackendError`.
- **Command Sandboxing**: Not applicable for this specific HTTP backend, but properly adheres to the overall security model by keeping the HTTP boundary strict.

## 5. Implementation Concerns
- **Model Name Translation**: `ServiceTargetResolver` must be updated to strip the `tensorzero/` prefix and format the model exactly as `tensorzero::function_name::<function-name>`.
- **Endpoint Pathing**: The OpenAI adapter in `genai` will hit `/chat/completions`. The design must enforce that the configured endpoint URL contains the `/openai/v1/` prefix (not just `/v1/`), otherwise requests will 404.
- **Error Mapping**: The `map_status` function in the scaffolded `tensorzero.rs` incorrectly maps all 5xx errors to retryable `Network` errors. It must be updated to inspect the body for strings like `"401"`, `"Unauthorized"`, or `"rate_limit"` to map them to `Auth` (non-retryable) and `RateLimit` (retryable with backoff) respectively.
- **Testability**: `make check` will pass with the current scaffolding, but the tests in `tests/tensorzero_backend.rs` (or inline in `src/backend/tensorzero.rs`) are testing the wrong assumptions. They test clean 401s and 429s which TensorZero does not emit for upstream failures.

## 6. Concurrency & Async
- The tokio and `async_trait` usage is idiomatic.
- Passing the `WebConfig::default().with_timeout(...)` directly to the `genai` client is good for enforcing strict bounds without wrapping futures in `tokio::time::timeout` unnecessarily.
- No blocking filesystem or synchronous network calls are present in the scaffolded async path.

## 7. Blind Spots
- **Family Identity**: The design document completely ignores FR-13 (`family_of` lookup). D1 resolved that this must be parsed from the function name suffix (e.g., `loker_d1_openai` -> `openai`). If this isn't built into the backend resolution, the cross-family judge validation will fail later.
- **Token Usage**: D1 noted that `prompt_tokens_details.cached_tokens` and `tensorzero_cost` are returned. The design should explicitly defer these to M5/M6 so developers don't waste time trying to extract them into `TokenUsage` now.
- **Streaming**: D1 deferred streaming, but the design doc should explicitly state that streaming is `false` for M1.

## 8. Verdict
NEEDS_REVISION

## 9. Actionable Feedback
1. **Restructure Document**: Add missing standard sections (Summary, Background, Architecture, Detailed Design, Acceptance Criteria).
2. **Incorporate D1 Spike Findings (T-005)**:
   - **Model Namespace**: Detail the `tensorzero::function_name::<fn>` transformation inside the `ServiceTargetResolver`.
   - **Endpoint Path**: Update the endpoint requirements and wiremock tests from `/v1/` to `/openai/v1/`.
   - **Error Inspection**: Specify the logic for inspecting 502 response bodies to rescue upstream `Auth` and `RateLimit` errors.
3. **Address Family Resolution**: Define how the backend will expose its model family (via suffix parsing) to satisfy FR-13.
4. **Update Test Contract**: Require wiremock tests that specifically mock the 502 upstream wrapping behavior observed in the D1 fixtures (`anthropic_auth_failure_response.json`).
5. **Explicitly Defer Features**: Add notes that streaming, `cached_tokens`, and `tensorzero_cost` extraction are out of scope for M1.
