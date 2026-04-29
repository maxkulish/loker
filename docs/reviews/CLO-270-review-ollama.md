# Design Review: CLO-270

**Reviewer**: Codex via Ollama (glm-5.1:cloud)
**Reviewed**: 2026-04-29
**Pipeline**: lok design-review

---

Now let me read the security doc, specs doc, and the relevant Rust source files the design references.
Now let me check the `QueryOutput` and `Backend` types to validate the `VerifyContext::from_query_output` field mapping:
I now have all the context needed for a thorough review. Here is my assessment:

---

## 1. Completeness Check

| Section | Present? | Assessment |
|---------|----------|------------|
| Problem | Yes | Precise, identifies three concrete gaps and names the dependent tasks |
| Goals / Non-goals | Yes | Crisp boundary; correctly fences CLO-260, CLO-271/272/273 |
| Architecture | Yes | Module layout, data flow, type taxonomy |
| Public API surface | Yes | Full Rust code for all three new types + trait change |
| Consumer update | Yes | Before/after diff and migration note |
| Test plan | Yes | 8 unit tests + consumer test update + manual verification |
| Migration / rollout | Yes | Ordered steps, no feature flags |
| Open questions | Yes | 4 questions with resolutions |
| Implementation plan | Partial | Steps are listed but no time estimates or rollback plan |
| Acceptance criteria | Implicit | Buried in test plan; no explicit AC checklist |

**Missing**: No explicit acceptance criteria section (ACs exist only as test names). No rollback plan. No structured error handling discussion beyond `VerifyError`.

## 2. Architecture Assessment

**Strengths**:
- `#[non_exhaustive]` on `VerifyResult` and `VerifyContext` is correct forward-compatibility hygiene. Reserved variants (`Repair`, `Score`) compiling but being fallthrough-only is the right approach for FR-18.
- `FailureReason` as an owned struct with a builder API (not `impl Into<String>`) gives CLO-260 a structured seam. The decision not to make it `#[non_exhaustive]` (open question 3) is sound: additive fields with defaults are already safe without it, and `#[non_exhaustive]` on a struct prevents destructuring, which would hurt callers.
- `VerifyContext` explicitly documents that it does not carry credentials. This is a direct alignment with the security posture in `docs/handoff.md` and the PRD NFR rows.
- The `VerifyHook` trait remains `Send + Sync` for `Arc` sharing across tasks, matching the existing convention in `Strategy`.
- Separation of `VerifyError` (hook crashed) vs `VerifyResult::Fail` (hook ran, judged insufficient) is correct and matches the existing codebase semantics.

**Concerns**:

1. **`VerifyContext::from_query_output` clones every field.** `stdout` can be large (LLM outputs). The design should acknowledge the allocation cost or consider `Arc<QueryOutput>` internally with `Deref`-based field access. For v0 with one consumer (EscalatingRetry) this is acceptable, but T-029 (phase runner) will construct `VerifyContext` from multiple sources; the clone-on-construction pattern forces a full copy even when only metadata changes.

2. **`VerifyContext` drops `usage`.** `QueryOutput` has `usage: Option<TokenUsage>`, but `VerifyContext` omits it. A verify hook that wants to gate on token count (plausible for cost-aware hooks) has no access. The design lists only `structured` as the "parsed JSON" field; token usage would need to go through `structured`, which is inconsistent.

3. **`VerifyHook::verify` takes `&VerifyContext`.** This is fine for read-only inspection, but `RunCommand` (CLO-271) needs the run directory path and the phase's declared command string. Those are not on `VerifyContext`. The open question acknowledges T-029 will add fields, but `#[non_exhaustive]` only helps at compile time; at runtime, a `RunCommand` hook receiving a `VerifyContext` without a `cwd` or `command` field has no way to fail constructively. The design should define what a hook does when required context is absent: panic (no), return `VerifyError` (yes - should be documented as a contract).

4. **`FailureReason` carries `stdout`/`stderr` + truncation flag but no byte cap value.** The design says "may be truncated at `max_output_bytes`" but does not specify what `max_output_bytes` is or where it is configured. EscalatingRetry already has `MAX_RESPONSE_EXCERPT_BYTES = 4096`. RunCommand (CLO-271) will have its own cap (PRD NFR row 194: "stdout/stderr byte caps"). Should `FailureReason` carry the cap? Or should truncation be applied before constructing `FailureReason`? The design is silent.

5. **`VerifyResult` derives `PartialEq` but not `Eq`.** `Score(f32)` makes `Eq` impossible (NaN). The design is correct not to derive `Eq`, but should document that `PartialEq` on `Score` is IEEE 754 semantics (NaN != NaN). This matters for test assertions.

## 3. Alignment with Handoff & Roadmap

- **Handoff intent**: "New primitives land as new modules. The existing code stays working until the new modules subsume it. Don't mutate-in-place." The design refactors `verify.rs` in-place, which technically violates this. However, the handoff also says "don't mutate-in-place" for the `consensus.rs` / `apply_verify/` code, and the design explicitly scopes itself away from `src/apply_verify/verification.rs`. The in-place refactor of `verify.rs` is justified because the module is the canonical location for the `VerifyHook` trait (it predates the handoff guidance, and moving it would fragment the crate for no behavioral gain). Flagged but acceptable.

- **Roadmap alignment**: T-020 is Phase 4, which gates T-013 (EscalatingRetry), T-021/022/023 (concrete hooks), and T-050 (advisory lock). The design correctly identifies that T-013 already depends on the trait signature change and provides the call-site migration. The dependency on T-013 (listed as "After: -" then "Blocks: T-013") is odd: T-013 (CLO-258) is already done and shipped, but the roadmap says "After: T-007, T-020". This means T-020 was supposed to land *before* T-013 but didn't; EscalatingRetry shipped with the old `&QueryOutput` signature. The design acknowledges this by documenting the before/after call site change. This is a sequencing debt, not a design flaw.

- **M1 scope**: CLAUDE.md says active milestone is M1. T-020 is M4 work. The design does not claim to be M1-scoped; it correctly references the roadmap phase. No contradiction.

- **FR-18 alignment**: The design matches FR-18 exactly: `Pass`/`Fail` concrete, `Repair`/`Score` reserved, `#[non_exhaustive]` for forward compat.

## 4. Security Review

- **No credential leakage**: `VerifyContext` explicitly omits API keys/tokens. Good.
- **`FailureReason.stdout` / `stderr`**: These carry raw output, which may contain secrets from the LLM response. The design does not mention redaction of `FailureReason` fields. EscalatingRetry already has `redact_secrets()` applied to `response_excerpt`, but the design does not say whether `FailureReason.stdout` should be redacted before or after construction. If CLO-260's `pass_failure_context` flows `FailureReason` into the next prompt, unredacted secrets would leak. This is a gap.
- **`VerifyContext.structured`**: Contains parsed JSON, which could include API response headers or secret-bearing fields. No redaction mentioned. Lower risk since it is not flowed into prompts, but `trace.jsonl` will serialize it.
- **Input validation**: No validation on `VerifyContext` fields (empty `backend_name`, absurd `duration`, etc.). Not a security threat, but a robustness concern.

## 5. Implementation Concerns

- **`make check` testability**: The test plan lists 8 unit tests plus consumer test updates. All are pure in-memory, no network, no env-var gates. Will pass `make check` cleanly. Good.

- **Phasing**: The 5-step migration is correctly ordered. Step 2 (update EscalatingRetry) depends on step 1 (refactor verify.rs). Step 4 (`cargo test`) is the gate. However, step 3 (add unit tests) should happen *before* step 2 and run red-first (TDD), per handoff guidance. The current ordering implies tests are added after the refactor and consumer update, which is not TDD.

- **Backward compatibility**: `VerifyResult::Pass { notes: Option<String> }` is being changed to `VerifyResult::Pass`. Open question 1 resolves this (drop `notes`), but any downstream code pattern-matching `Pass { notes }` will break. The design says `notes` was never populated, so the break is in name only. Still, the `#[non_exhaustive]` attribute will force all match arms to have a `_` wildcard, which is the point.

- **`VerifyError` shape**: The design keeps `VerifyError { message: String }` unchanged. This is a missed opportunity to add a `source` chain (e.g., `io::Error`) for debugging. The `thiserror` attribute is already on the struct but no `#[source]` field. For hooks that fail due to I/O (RunCommand), a bare string message loses the original error chain.

- **`is_fail()` is missing from current code**: The design adds it. Good.

## 6. Concurrency & Async

- **`VerifyHook::verify` is `async fn`.** Correct: hooks like RunCommand spawn subprocesses; LLMVerifier makes HTTP calls. All are naturally async.
- **No blocking call risk identified in the design.** The `VerifyContext` construction is pure, no I/O. Good.
- **Cancellation safety**: Not discussed. If a `tokio::timeout` wraps a `verify()` call and the future is dropped, what happens to a spawned subprocess? RunCommand (CLO-271) needs cancellation-safe design; the CLO-270 design does not preclude it, but should at least note that `VerifyHook` implementors are responsible for cancellation safety (e.g., killing child processes on drop). This is a gap that should be noted as a contract requirement on implementors.
- **`Arc<dyn VerifyHook>`** is `Send + Sync` per the trait bounds. Safe to share across tasks. No mutable state in the trait. Good.

## 7. Blind Spots

1. **Redaction of `FailureReason.stdout/stderr`**: Not addressed. If these flow into prompts (CLO-260), secret leakage is a real risk. The design should specify whether `FailureReason` fields are redacted at construction time or deferred to the consumer.

2. **Missing `usage` field on `VerifyContext`**: Token counts are on `QueryOutput` but not on `VerifyContext`. Cost-aware hooks have no access.

3. **Hook contract for missing context**: `VerifyContext` will gain fields over time. What should a hook do when a required field is `None`? This error path needs to be documented as "return `VerifyError`" rather than panic.

4. **`FailureReason.truncated` semantics**: Truncation is a boolean but does not record the original byte length. A consumer that wants to re-fetch the full output (e.g., HITL UI) cannot determine whether the full output is meaningfully larger.

5. **Serialization of `VerifyResult`**: The design does not derive `Serialize`/`Deserialize` on `VerifyResult` or `FailureReason`. The trace writer (T-029) will need to serialize verify results. The `phase_result_escalating.schema.json` already has a `verify.status` field. How does `VerifyResult` map to the schema? The design is silent.

6. **`VerifyContext.backend_name` vs `QueryOutput.backend`**: The design renames the field from `backend` to `backend_name`. This is a clarity improvement but could confuse consumers who expect the `QueryOutput` field name. Minor.

7. **No `Display` impl for `FailureReason`**: The design mentions `display()` in the `FailureReason` comment but does not provide an impl. Without `Display`, logging and error messages will use `Debug` formatting, which is verbose and not user-facing.

8. **`VerifyResult::Score(f32)` ordering**: `Score(0.9)` vs `Score(0.8)` - is higher better? The design does not specify. This matters for future threshold gates.

## 8. Verdict

**APPROVE_WITH_SUGGESTIONS**

The design is solid, well-scoped, and correctly aligned with the PRD, roadmap, and handoff. The core type abstractions (`VerifyResult`, `FailureReason`, `VerifyContext`) are correct for v0. The suggestions below are improvements, not blockers.

## 9. Actionable Feedback

| Priority | Item | Why |
|----------|------|-----|
| **P1** | Specify redaction policy for `FailureReason.stdout/stderr`. Either document that consumers must redact before flowing into prompts (CLO-260's job), or apply `redact_secrets()` at `FailureReason` construction time. | Prevents secret leakage in `pass_failure_context` path |
| **P1** | Add a contract note in the `VerifyHook` trait doc: if a required context field is `None`, the hook must return `Err(VerifyError)`, not panic. | Prevents runtime crashes when T-029 adds new fields |
| **P1** | Reorder migration step 3 (add unit tests) before step 2 (update EscalatingRetry). Write the failing test first per handoff TDD guidance. | Align with handoff intent |
| **P2** | Consider adding `usage: Option<TokenUsage>` or a minimal `TokenUsageReport` to `VerifyContext`. | Enables cost-aware hooks without going through `structured` |
| **P2** | Add `#[derive(Serialize, Deserialize)]` to `VerifyResult` and `FailureReason`, or document explicitly why they are not serializable (and how T-029 will handle them). | T-029 trace writer will need this |
| **P2** | Add `impl Display for FailureReason` producing a human-readable single-line summary (the `summary` field + truncation indicator). | Logging and error messages |
| **P2** | Note in the trait doc that `verify()` implementors must ensure cancellation safety (e.g., child process cleanup on drop). | Prevents zombie processes under `tokio::timeout` |
| **P3** | Document `Score(f32)` semantics: is higher better? | Reduces ambiguity for future threshold gate implementors |
| **P3** | `VerifyError` should gain a `source: Option<Box<dyn std::error::Error + Send + Sync>>` field or at minimum a `#[source]`-annotated field for I/O errors. | Preserves error chain for RunCommand debugging |
| **P3** | `FailureReason.truncated` should carry `original_len: Option<usize>` so consumers can assess whether re-fetching full output is worthwhile. | HITL UI usefulness |
