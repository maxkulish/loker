# Design Review: m1-tensorzero

**Reviewer**: Codex via Ollama (glm-5.1:cloud)
**Reviewed**: 2026-04-26
**Pipeline**: lok design-review

---

Let me read all the relevant documents in parallel to gather context.
Now let me read the security and specs documents, and explore the source code:
Now let me read the key source files and the spike document:
Now let me also check the `create_backend` function for whether `tensorzero` is wired in, and look at the Cargo.toml for dependencies:
Now I have comprehensive context. Let me look at one more thing - whether `tensorzero` is registered in the `create_backend` dispatcher:
I now have all the context needed for a thorough review. Let me compose it.

---

## 1. Completeness Check

| Section | Present? | Assessment |
|---------|----------|------------|
| Summary / Scope | Present | Clear scope: add `TensorZeroBackend` without breaking existing backends |
| Background | Present (via external design doc reference) | Adequate - points to canonical design |
| Architecture | Missing | No architecture diagram or trait interaction narrative. The file lists implementation steps but does not explain how `TensorZeroBackend` composes with `Backend`, `RetryExecutor`, config, or the broader system. |
| Detailed Design | Partial | Five implementation steps given, but no Rust type signatures, error mapping table, config schema shape, or trait method contracts. Compared to the spike doc (which is thorough), the plan is thin. |
| Implementation Plan | Present | Ordered implementation steps. Lacks TDD test-first structure that handoff.md mandates. |
| Test Contract | Present | Well-specified: six wiremock cases + one integration gate. |
| Acceptance Criteria | Missing | No explicit AC. The spike doc has an implicit AC ("unit tests pass, integration passes with env gate"), but the plan itself doesn't state it. |
| Constraints | Present | Three constraints documented. |

**Verdict**: The document functions as a task card, not a design document. It is acceptable for a narrow-scoped milestone like M1 but has gaps compared to what a full design review would expect.

## 2. Architecture Assessment

**Strengths**:
- Clean separation: `genai` crate handles all HTTP transport; the backend is a thin adapter around `Client + ServiceTargetResolver`.
- Error mapping is exhaustive and well-structured in `map_genai_error` / `map_webc_error` / `map_status`, covering all `genai::Error` variants.
- The `Backend` trait contract is clear: `query()`, `name()`, `is_available()`. The `TensorZeroBackend` correctly implements all three.
- `is_available() -> true` is the right call for a server-side gateway (no synchronous probe needed).
- `wiremock` tests already cover all six contract scenarios (200, 429, 500, malformed JSON, timeout, auth). This matches and exceeds the plan's test contract.
- The D1 spike is thorough and produced actionable findings; the defect in `map_status` (502 for upstream auth mapped to retryable `Network`) is documented and assigned to CLO-247.

**Concerns**:
1. **`create_backend` dispatcher missing `tensorzero` branch** (`src/backend/mod.rs:269-286`). The match arm covers `codex`, `gemini`, `claude`, `ollama`, `bedrock` but not `tensorzero`. A backend that compiles but cannot be instantiated from config is a half-shipped feature. This is the most pressing gap.
2. **`TensorZeroConfig` is standalone, not wired into `BackendConfig`**. The existing config uses `BackendConfig` (with fields like `command`, `args`, `api_key_env`, `timeout`). `TensorZeroConfig` has `endpoint`, `model`, `api_key`, `timeout` - there's no bridge between TOML and `TensorZeroConfig`. Plan step 3 ("Wire into `src/config.rs`") is not yet done.
3. **No `BackendCapabilities`**. PRD FR-4 requires a struct describing tool-use, streaming, file-edit support per backend. The plan mentions nothing about this, and it's listed in the roadmap (T-010) but the design doc doesn't acknowledge it.
4. **Endpoint path mismatch**. Spike §6.1 documents that the real gateway expects `/openai/v1/chat/completions` but the scaffolded tests use `/v1/`. This will cause runtime 404s against a real TensorZero gateway. Plan step 1 should explicitly address path reconciliation.
5. **502 body-inspection error mapping**. Spike §5 identifies that TensorZero wraps upstream auth failures in 502, but the current `map_status` maps all 5xx to `Network` (retryable). The plan's step 4 just says "Error mapping: genai errors -> BackendError variants" without mentioning the 502 body-inspection requirement. This is a correctness bug that will cause infinite retries on permanent auth failures.

## 3. Alignment with Handoff & Roadmap

**Aligned**:
- TDD-first for orchestration primitives: test contract is specified before implementation.
- `wiremock` before real gateway: all six unit tests are `wiremock`-based; integration test is opt-in via `LOKER_TZ_INTEGRATION=1`.
- `reqwest` coexistence: `genai` added alongside `reqwest`; not replacing it.
- `make check` gate: no live network dependency.
- Constraints are explicitly restated ("do not relitigate").

**Misaligned**:
- **TDD contract should precede implementation.** Handoff.md says: "Read the design doc §8 M_N test contract, write the failing test, then implement." The plan puts "Write the failing tests first" in the "Where to start" section, which is correct posture, but the implementation steps 1-5 are written as though the backend code comes first and tests second. The actual codebase already has both tests and implementation, which suggests they were written simultaneously rather than test-first. This is a minor process concern, not a blocking one.
- **Roadmap T-005 through T-010** are more granular than the plan's five steps. The plan should reference these task IDs for traceability.

## 4. Security Review

**Adequate for M1**:
- API keys are handled via `Option<String>` in config and forwarded as Bearer tokens via `ServiceTargetResolver`. No hardcoded secrets.
- The spike confirmed that TensorZero gateway accepts any Bearer token by default (auth is upstream); loker needs to ensure it doesn't log the API key. Current code does not log it, which is correct.
- The `map_genai_error` function does not include the API key in error messages.

**Concerns**:
- **`TensorZeroConfig.api_key` is `Option<String>` rather than `SecretString`**. The `claude.rs` backend uses `secrecy::SecretString` for API keys. M1 should match this pattern to prevent accidental key exposure in debug output or `Display` impls.
- **Error messages may leak response bodies**. `map_status` formats the full HTTP body into `BackendError::Network { message }` and `BackendError::Auth { message }`. If TensorZero returns provider error details in the body (which it does - see spike §5), those messages could end up in `trace.jsonl` or logs. This conflicts with PRD §5 ("No secrets in logs or `trace.jsonl`").
- **502 body inspection for auth** (spike §5 concern) means parsing error bodies for strings like `"401"`, `"Unauthorized"`. This is fragile string matching. Consider a more robust approach: parse the JSON error envelope `{"error": {"message": "..."}}` and check for structured fields if TensorZero adds them.

## 5. Implementation Concerns

1. **Config wiring is the highest-priority gap.** A `TensorZeroBackend` that cannot be created from `lok.toml` is unusable. T-008 in the roadmap covers this, but the design doc should acknowledge it as a prerequisite for any end-to-end flow.
2. **`BackendConfig` is the wrong shape for HTTP-gateway backends.** Current `BackendConfig` has `command: Option<String>` (a CLI binary), but `TensorZeroBackend` needs `endpoint: String`, and there's no TOML path from `BackendConfig` to `TensorZeroConfig`. The plan should specify whether to extend `BackendConfig` with optional fields (`endpoint`, `api_key` direct value) or create a separate `BackendConfig::Http` variant.
3. **`wiremock` is already in dev-dependencies** (Cargo.toml line 63), matching the plan.
4. **The `genai` version pin** is `0.6.0-beta.17`. The plan says `genai = "0.6.0-beta.17"` and the spike confirms it works. This is correct. However, beta crates have no stability guarantee; the PRD §8 risk row ("`genai` crate API breaks before v0") identifies this and the mitigation (pin minor version) is in place.
5. **No streaming in v0** is explicitly constrained. Good. But the `Backend` trait's `query()` returns `QueryOutput` synchronously - this is fine for non-streaming, but when streaming is added later, the trait will need a `query_stream()` or similar. This is a forward-compatibility concern that should be documented, even if deferred.

## 6. Concurrency & Async

- **`TensorZeroBackend::query()` is properly async**. It calls `self.client.exec_chat()` which is `genai`'s async entry point. No blocking calls in the async path.
- **No `spawn_blocking` needed** for gateway HTTP calls; `genai` uses `reqwest` under the hood which is tokio-native.
- **`tokio::time::timeout` wrapping** is handled at the `WebConfig` level (`with_timeout`), not in the backend itself. This is correct - the timeout applies to the entire HTTP round-trip managed by genai. However, the existing `run_query_with_config` in `mod.rs` also wraps each backend call with `tokio::time::timeout`. For `TensorZeroBackend`, this means a double timeout: one from `WebConfig` and one from `run_query_with_config`. This is not necessarily wrong (the inner timeout fires first), but it may cause confusion. The design doc should note this double-timeout and specify which one wins.
- **Cancellation safety**: If the outer `tokio::time::timeout` in `run_query_with_config` fires, the `genai` client's in-flight request is dropped. `reqwest` handles this correctly (the connection is returned to the pool). No cancellation safety concern.
- **Thread safety**: `TensorZeroBackend` holds a `genai::Client`, which is `Clone + Send + Sync` per genai docs. Since the struct is constructed once and wrapped in `Arc<dyn Backend>`, this is safe.

## 7. Blind Spots

1. **No `BackendCapabilities` in scope.** PRD FR-4 requires it for M1 (T-010), but the design doc doesn't mention it. `TensorZeroBackend` should report its capabilities (no tool-use, streaming = off, no file-edit) so that strategy validation later can reject invalid configurations.
2. **Family resolution is decided but not implemented.** Spike §3 resolves the open question (family derives from the function-name suffix in `backend_id`), but `src/backend/tensorzero.rs` has no `family_of()` function. This is T-015's scope, but the M1 plan should acknowledge that `TensorZeroBackend` will need to expose its family before M3.
3. **Token usage mapping drops `total_tokens`, `cached_tokens`, and `tensorzero_cost`.** The spike §4 acknowledges this and says "out of scope for M1." This is fine, but `QueryOutput.usage` is `Option<TokenUsage>` and `TokenUsage` only has `prompt_tokens` + `completion_tokens`. When cost reporting lands in M5 (FR-23a), `TokenUsage` will need extension. This is a schema migration that should be noted.
4. **Response model field mangling.** TensorZero returns `model: "tensorzero::function_name::<fn>::variant_name::<variant>"`. The current code (`with_model(Some(effective_model))`) stores the *requested* model, not the *responded* model. For observability (trace.jsonl, FR-20), the response model is more useful. The `genai` `ChatResponse` exposes the model; the code should capture `response.metadata.model` (or whatever genai provides) as the effective model.
5. **No structured output extraction.** The code does `response.into_first_text().unwrap_or_default()`. If the gateway returns an empty content block, this silently produces an empty string with no error. The plan's test contract mentions "malformed JSON response" but not "valid JSON with empty content." An empty-string response from a model is a real edge case that should produce `BackendError::Parse` or at minimum a log warning.
6. **`into_first_text()` panic risk.** `unwrap_or_default()` silently returns `""` for `None`. If `genai` returns no message choices at all (which would be a protocol violation, but defensive coding applies), this could mask an error. Consider checking `response.choices` length or the returned `Usage` for zero-completion tokens.
7. **Integration test scaffold not present.** The plan specifies `LOKER_TZ_INTEGRATION=1` gated tests, but no `tests/tensorzero_integration.rs` file exists yet. This is expected (M1 is in flight), but the design doc should note where it will live.

## 8. Verdict

**APPROVE_WITH_SUGGESTIONS**

The implementation is solid for M1 scope. The core `Backend` trait implementation, error mapping, wiremock test coverage, and `genai` integration are all well-done. The spike produced excellent findings. The primary risk items (502 auth misclassification, config wiring gap, `SecretString` for API keys) are all addressable within the M1 task window (CLO-247 through CLO-252).

## 9. Actionable Feedback

**Must-fix before M1 close (blocks `make check` or correctness)**:

1. **Add `tensorzero` branch to `create_backend`** (`src/backend/mod.rs:269`). Without this, the backend cannot be instantiated from config. This is the #1 integration gap.
2. **Fix 502 body-inspection error mapping** (`src/backend/tensorzero.rs:174-188`). When status is 5xx, inspect the body for auth/rate-limit signatures before defaulting to `Network`. Add wiremock cases for TensorZero's actual error envelopes (spike fixtures are available).
3. **Fix endpoint path** - update the default and test helper from `/v1/` to `/openai/v1/` per spike §6.1, or make the path configurable so both local and remote deployments work.
4. **Wire `TensorZeroConfig` into `BackendConfig` / `lok.toml`** (T-008). Define whether to extend `BackendConfig` with `endpoint: Option<String>` and `api_key: Option<String>` fields, or create a separate enum variant. Document the decision in the plan.
5. **Use `SecretString` for `api_key`** to match the `claude.rs` pattern. Never store raw API key strings in memory.

**Should-fix before M1 close (quality, observability, forward-compat)**:

6. **Capture the response model, not just the requested model**. After `exec_chat`, store the model the gateway reports (which includes variant info) in `QueryOutput.model`. This is critical for `trace.jsonl` (FR-20) and cross-family enforcement.
7. **Guard against empty responses.** If `into_first_text()` returns `None`, produce at minimum a structured warning or error instead of silently defaulting to `""`.
8. **Document double-timeout behavior.** `WebConfig::with_timeout` and `run_query_with_config`'s outer `tokio::time::timeout` both apply. Specify which timeout wins and what the recommended config relationship is.
9. **Add `BackendCapabilities` stub for TensorZero** (T-010 dependency). Even a hardcoded `fn capabilities() -> BackendCapabilities { BackendCapabilities { tool_use: false, streaming: false, file_edit: false } }` prevents M2 from having to retrofit it.

**Nice-to-have (post-M1)**:

10. **Add integration test file placeholder** (`tests/tensorzero_integration.rs`) with the `LOKER_TZ_INTEGRATION` gate so the scaffold exists for T-009.
11. **Plan `TokenUsage` schema evolution** for M5 by noting in a code comment or doc that `total_tokens`, `cached_tokens`, and `tensorzero_cost` will need surface area later.
