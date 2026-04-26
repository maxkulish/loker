# Review Synthesis: m1-tensorzero

**Synthesized**: 2026-04-26
**Pipeline**: lok design-review
**Reviewers**: Gemini 3.1 Pro, Codex/Ollama (glm-5.1:cloud), Claude (fallback if needed)

---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini | OK | Verdict: NEEDS_REVISION |
| Ollama | OK | Verdict: APPROVE_WITH_SUGGESTIONS |
| Claude Fallback | SKIPPED | External reviewers succeeded |

## Agreement (High Confidence)
| # | Finding | Severity |
|---|---------|----------|
| 1 | Endpoint path must be `/openai/v1/` not `/v1/`; tests and config defaults still use the wrong prefix and will 404 against a real gateway | High |
| 2 | `map_status` 5xx branch must inspect 502 bodies for upstream auth/rate-limit signatures; current mapping treats permanent 401s as retryable `Network`, causing infinite retry loops | High |
| 3 | Document is structured as a task card and lacks standard sections (Summary, Background, Architecture, Detailed Design, Acceptance Criteria) | Medium |
| 4 | Family resolution (FR-13) is unaddressed in the design; backend must expose family derived from function-name suffix or M3 cross-family judging breaks | Medium |
| 5 | D1 spike findings (T-005) are not reconciled into the implementation steps or test contract; tests assert clean 401/429 which TensorZero never emits | High |
| 6 | Streaming should be explicitly stated as out of scope for M1 | Low |
| 7 | Error/response bodies risk leaking provider internals into `BackendError` messages and `trace.jsonl`; redaction or structured parsing required (PRD §5) | Medium |

## Disagreement (Needs Human Decision)
| # | Topic | Position A (Gemini) | Position B (Ollama) |
|---|-------|---------------------|---------------------|
| 1 | Overall verdict | NEEDS_REVISION — doc is too thin and ignores D1 entirely | APPROVE_WITH_SUGGESTIONS — implementation is solid, gaps fixable within M1 task window |
| 2 | TDD posture | Plan correctly puts test contract before code | Implementation appears written alongside tests, not strictly test-first per handoff.md |

## Novel Insights (Single Reviewer)
| # | Finding | Source | Severity |
|---|---------|--------|----------|
| 1 | `create_backend` dispatcher in `src/backend/mod.rs:269` has no `tensorzero` arm — backend cannot be instantiated from config | Ollama | High |
| 2 | `TensorZeroConfig.api_key` should use `secrecy::SecretString` to match `claude.rs` pattern and prevent debug-output leaks | Ollama | Medium |
| 3 | `TensorZeroConfig` is standalone and not bridged to `BackendConfig`/TOML — extension vs. enum-variant decision is undocumented | Ollama | High |
| 4 | `BackendCapabilities` (FR-4) is not declared for TensorZero; M2 will need retrofit | Ollama | Medium |
| 5 | `into_first_text().unwrap_or_default()` masks empty/missing content as `""` instead of erroring | Ollama | Medium |
| 6 | Double timeout: `WebConfig::with_timeout` plus outer `tokio::time::timeout` in `run_query_with_config` — semantics undocumented | Ollama | Low |
| 7 | `QueryOutput.model` stores requested model, not the variant-qualified model TensorZero returns; weakens trace.jsonl observability (FR-20) | Ollama | Medium |
| 8 | `ServiceTargetResolver` must transform `backend_id` to `tensorzero::function_name::<fn>` form | Gemini | High |
| 9 | Defer `cached_tokens` and `tensorzero_cost` extraction explicitly to M5/M6 in the doc | Gemini | Low |
| 10 | `genai` credential model: loker sends Bearer to gateway; upstream provider keys live in TensorZero — should be documented | Gemini | Low |

## Consolidated Verdict
**NEEDS_REVISION** (Gemini blocks; Ollama suggests).

## Priority Actions
1. **Reconcile D1 spike findings (T-005)** into the design: model namespace transform, `/openai/v1/` endpoint, 502 body inspection for auth/rate-limit. *(Agreement)*
2. **Fix `map_status`** to map 502+auth-body to non-retryable `Auth` and 502+rate-limit-body to retryable `RateLimit`. *(Agreement)*
3. **Wire `tensorzero` into `create_backend`** dispatcher and define the `BackendConfig` ↔ `TensorZeroConfig` bridge in TOML. *(Novel, blocking)*
4. **Update endpoint default + wiremock fixtures** to `/openai/v1/`. *(Agreement)*
5. **Restructure document** with Summary / Background / Architecture / Detailed Design / Acceptance Criteria sections. *(Agreement)*
6. **Define family resolution** (parse function-name suffix) on the backend now to satisfy FR-13. *(Agreement)*
7. **Adopt `SecretString`** for `api_key` and redact response bodies in `BackendError` messages. *(Novel + Agreement on logging)*
8. **Capture response model** (variant-qualified) in `QueryOutput.model`; harden empty-response handling. *(Novel)*
9. **Add `BackendCapabilities` stub** for TensorZero (tool_use=false, streaming=false, file_edit=false). *(Novel)*
10. **Explicitly defer** streaming, `cached_tokens`, `tensorzero_cost` to later milestones in the doc. *(Agreement)*

## Decision Recommendation
**REVISE** — block on items 1–4 before resuming implementation:
- D1 reconciliation (T-005) including endpoint path and 502 error mapping
- `create_backend` registration + `BackendConfig` wiring decision
- Restored doc structure with explicit acceptance criteria

Items 5–10 can proceed as PROCEED_WITH_FIXES once the blocking revisions land.
