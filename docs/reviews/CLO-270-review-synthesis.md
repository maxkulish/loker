# Review Synthesis: CLO-270

**Synthesized**: 2026-04-29
**Pipeline**: lok design-review
**Reviewers**: Gemini 3.1 Pro, Codex/Ollama (glm-5.1:cloud), Claude (fallback if needed)

---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini | OK | Full review, verdict APPROVE |
| Ollama | OK | Full review, verdict APPROVE_WITH_SUGGESTIONS |
| Claude Fallback | SKIPPED | External reviewers succeeded |

## Agreement (High Confidence)
| # | Finding | Severity |
|---|---------|----------|
| 1 | `FailureContext::from_verify_fail` integration must extract `reason.summary.clone()` from `VerifyResult::Fail` rather than treating the full `FailureReason` as a string (Gemini explicit, Ollama implicit via call-site analysis) | Medium |
| 2 | `#[non_exhaustive]` on `VerifyResult` and `VerifyContext` is correct forward-compat hygiene for reserved `Repair`/`Score` variants and future context fields | Info (positive) |
| 3 | `VerifyContext` rightly omits credentials/API keys; aligns with handoff security posture | Info (positive) |
| 4 | Bounding `FailureReason.stdout/stderr` with a `truncated` flag is good defensive memory hygiene | Info (positive) |
| 5 | `VerifyError` (hook crashed) vs `VerifyResult::Fail` (hook ran, rejected) separation is correct | Info (positive) |
| 6 | `VerifyHook` remains `async`, `Send + Sync` — appropriate for `Arc` sharing across tasks | Info (positive) |
| 7 | Reserved variants (`Repair`, `Score`) will fall through to `"verify did not pass"` in `EscalatingRetry`'s existing match — both reviewers consider this acceptable for v0 | Low |
| 8 | `serde_json::Value` in `VerifyContext.structured` requires `serde_json` import in `src/strategy/verify.rs` | Low |

## Disagreement (Needs Human Decision)
| # | Topic | Position A (Reviewer) | Position B (Reviewer) |
|---|-------|----------------------|----------------------|
| 1 | Overall verdict severity | APPROVE — design is "solid, complete, exactly what is needed" with no architectural concerns (Gemini) | APPROVE_WITH_SUGGESTIONS — 9 actionable items including 3 P1s (Ollama) |
| 2 | Migration step ordering | Implicitly accepts the doc's 5-step plan as written (Gemini) | Step 3 (add unit tests) must run before step 2 (update EscalatingRetry) to honor handoff TDD-first guidance (Ollama) |
| 3 | Redaction of `FailureReason.stdout/stderr` | Not flagged (Gemini) | P1 gap: must specify whether redaction happens at construction or is deferred to CLO-260 consumer; risk of secret leakage when `pass_failure_context` flows it into prompts (Ollama) |

## Novel Insights (Single Reviewer)
| # | Finding | Source | Severity |
|---|---------|--------|----------|
| 1 | `VerifyContext` drops `usage: Option<TokenUsage>` from `QueryOutput` — cost-aware hooks have no access to token counts without going through `structured` | Ollama | Medium |
| 2 | `VerifyHook` trait doc should contractually require: when a required context field is `None`, return `Err(VerifyError)` rather than panic (matters as T-029 adds fields) | Ollama | Medium (P1) |
| 3 | `VerifyResult` and `FailureReason` lack `Serialize`/`Deserialize` — T-029 trace writer will need it for `phase_result_escalating.schema.json` mapping | Ollama | Medium |
| 4 | No `impl Display for FailureReason` despite the design referencing `display()` in comments — logging falls back to verbose `Debug` | Ollama | Low |
| 5 | `verify()` implementors must be cancellation-safe (e.g., kill child processes on drop) — should be documented as trait contract | Ollama | Medium |
| 6 | `Score(f32)` semantics undocumented — is higher better? Matters for future threshold gates | Ollama | Low |
| 7 | `VerifyError { message: String }` lacks a `#[source]` chain — loses I/O error origin for RunCommand debugging | Ollama | Low |
| 8 | `FailureReason.truncated` is a bool; carrying `original_len: Option<usize>` would let HITL UI judge whether to re-fetch | Ollama | Low |
| 9 | `VerifyContext::from_query_output` clones every field including potentially large `stdout`; T-029 phase runner may want `Arc<QueryOutput>` to avoid full copies | Ollama | Low |
| 10 | `max_output_bytes` for `FailureReason` truncation is referenced but not defined — needs to coexist with `EscalatingRetry::MAX_RESPONSE_EXCERPT_BYTES = 4096` and CLO-271's caps | Ollama | Medium |
| 11 | Sequencing debt: T-020 was supposed to land before T-013 (CLO-258 / EscalatingRetry) per roadmap, but T-013 already shipped with the old `&QueryOutput` signature — design correctly back-fills the migration | Ollama | Info |
| 12 | No explicit Acceptance Criteria section; ACs are buried as test names, and no rollback plan beyond "purely type-level change" | Ollama | Low |
| 13 | `VerifyResult::Pass { notes: Option<String> }` becoming bare `Pass` is a name-only break (notes was never populated), but `#[non_exhaustive]` will force callers to add `_` arms | Ollama | Info |

## Consolidated Verdict
**Overall: APPROVE_WITH_SUGGESTIONS**

Gemini approves outright; Ollama approves conditional on suggestions. No reviewer says NEEDS_REVISION, so the design is not blocked — but the P1 items raised by Ollama materially affect downstream tasks (CLO-260, CLO-271) and should be resolved in the design before implementation, not after.

## Priority Actions

**P1 — Resolve in design before implementation**
1. Document redaction policy for `FailureReason.stdout/stderr`. Either redact at construction time or explicitly hand off the responsibility to CLO-260's `pass_failure_context` consumer. *(Agreement-adjacent: security gap flagged by Ollama, not contradicted by Gemini.)*
2. Add `VerifyHook` trait-doc contract: when a required `VerifyContext` field is `None`, implementors return `Err(VerifyError)` and must not panic.
3. Reorder migration plan: write failing unit tests (current step 3) before updating `EscalatingRetry` (current step 2), per handoff TDD guidance.
4. Confirm the `EscalatingRetry` integration uses `reason.summary.clone()` for the `Fail` arm and lets `Repair`/`Score` fall through to `"verify did not pass"` (both reviewers agree).
5. Define or pin down `max_output_bytes` for `FailureReason`. Reconcile with existing `MAX_RESPONSE_EXCERPT_BYTES = 4096` and the cap CLO-271 will introduce.

**P2 — Address in design or note as deferred**
6. Decide: add `usage: Option<TokenUsage>` to `VerifyContext`, or document the rationale for excluding it.
7. Derive `Serialize`/`Deserialize` on `VerifyResult` and `FailureReason`, or document how T-029 will serialize them into `phase_result_escalating.schema.json`.
8. Add `impl Display for FailureReason` (single-line summary + truncation indicator).
9. Add a trait-doc note: `verify()` implementors are responsible for cancellation safety (child process cleanup on drop).

**P3 — Nice-to-have**
10. Document `Score(f32)` ordering convention (higher = better, presumably).
11. Add `#[source]` field to `VerifyError` to preserve error chains.
12. Carry `original_len: Option<usize>` alongside `FailureReason.truncated`.
13. Add an explicit Acceptance Criteria section to the design doc.
14. Note `serde_json` dependency on `src/strategy/verify.rs`.

## Decision Recommendation
**PROCEED_WITH_FIXES** — address the five P1 items in the design doc, then begin implementation:

- Redaction policy for `FailureReason` stdout/stderr
- `VerifyHook` "missing context = `VerifyError`" contract
- TDD-first migration ordering (tests before EscalatingRetry update)
- Confirm `reason.summary.clone()` integration for `EscalatingRetry`
- Pin `max_output_bytes` value and reconcile with existing/future caps

P2/P3 items can be tracked as follow-ups (e.g., logged against CLO-260 for `pass_failure_context` retrofit and against T-029 for serialization), and need not block implementation kickoff.
