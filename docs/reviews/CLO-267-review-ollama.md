# Design Review: CLO-267

**Reviewer**: Codex via Ollama (glm-5.1:cloud)
**Reviewed**: 2026-04-28
**Pipeline**: lok design-review

---

## 1. Completeness Check

| Section | Status | Assessment |
|---------|--------|------------|
| Problem | Present | Clear, well-scoped, references discovery doc |
| Goals | Present | Concrete and measurable |
| Non-Goals | Present | Explicitly fences scope; cites blocking tasks |
| Architecture | Present | Data flow diagram, type additions, execute loop, output shape all specified |
| Public API surface | Present | Minimal, well-bounded |
| Test plan | Present | 8 unit tests + schema validation + regression |
| Migration / Rollout | Present | No-op migration correctly identified |
| Open Questions | Present | Three resolved; well-reasoned |
| References | Present | Adequate |
| **Background** | **Missing** | Problem section partially substitutes; no narrative connecting this task to M3 or the aggregation vocabulary |
| **Implementation Plan** | **Missing** | Architecture describes *what* to build but not *phasing*: file-by-file commit order, stub-then-impl TDD cycle |
| **Acceptance Criteria** | **Missing** | Test plan partially substitutes; no separate AC table mapping to PRD FR-11 |
| **Rollback plan** | **Missing** | Only says "no migration"; no rollback if shipping reveals a bug |

## 2. Architecture Assessment

**Strengths**:
- Inline streaming evaluation (Approach A from discovery) is the right call. `FuturesUnordered` arrival-order semantics map directly onto AnyFail's "first failure wins" contract. Short-circuit saves real backend spend.
- `any_fail_evaluate` is small, pure, and testable in isolation. Good separation of the JSON parse from the strategy loop.
- `StrategyError::AnyFail` carrying the full `StrategyOutput` (plus `offender`) is the correct pattern - it lets a phase runner persist artefacts even on aggregation failure, matching FR-23b's intent.
- `AnyFailReason` discriminated into `VerdictRejected`, `VerdictContract`, `BackendError` gives callers actionable diagnostics.
- Disabling `min_responses` short-circuit under AnyFail is a critical correctness detail, correctly called out in §4.3.

**Concerns**:
- **Handoff conflict on module placement.** `docs/handoff.md` Intent states: "New primitives (Strategy, Aggregator, VerifyHook) land as new modules." AnyFail is aggregation logic placed *inside* `parallel_fanout.rs` (a strategy module). The discovery doc justifies this as temporary (T-017 forces refactor), but the handoff rule is a constraint, not a suggestion. Consider a `src/aggregator/` module now, even if thin, so the primitive has its own home.
- **`any_fail_evaluate` signature has a dead `Ok(false)` branch.** The comment says "preserves signature for future trait extraction" but currently it's unreachable, making match arms misleading. Either remove it and return `Result<(), AnyFailReason>`, or document the trait-extraction intent more clearly with a tracking issue reference.
- **`offender: Box<Attempt>` in `StrategyError::AnyFail` is problematic for `BackendError` case.** When a backend returns `Err`, there is no `QueryOutput` to build a meaningful `Attempt` from. The existing `Err` arm in `parallel_fanout.rs:149` synthesizes an `Attempt` with `FinishReason::Error` and zero usage, but this is a synthetic record, not an actual attempt. The design should clarify whether the offender in the `BackendError` case is a synthetic attempt or whether the field should be `Option<Attempt>`.
- **No handling of non-clean JSON from LLMs.** `serde_json::from_str(text.trim())` assumes the backend raw output is valid JSON. LLM outputs frequently wrap JSON in markdown fences (```json ... ```). The design should specify whether the verdict parser should strip common LLM formatting artifacts, or whether AnyFail is only intended for structured-verify backends (where JSON output is guaranteed).

## 3. Alignment with Handoff & Roadmap

- **PRD alignment**: Directly implements FR-11 ("AnyFail returns first failure from N parallel verify outputs"). Acceptance criteria in the test plan cover the FR-11 contract.
- **Roadmap alignment**: T-018 depends on nothing (the `--` After column is empty), which matches the roadmap table. Correctly blocks T-029 (phase runner).
- **M1 scope check**: CLAUDE.md says "Active milestone: M1" but the roadmap marks M1 and M2 as complete and Phase 3 (M3) as in-flight. The CLAUDE.md is stale regarding active milestone; this task is M3 work and is correctly sequenced.
- **Handoff HOW violation**: The handoff says "New primitives... land as new modules." AnyFail is aggregation logic that should live in `src/aggregator/`, not inline in the strategy module. This is the most substantive alignment gap.
- **TDD discipline**: The handoff mandates "TDD-first... write the failing test, then implement." The design lists tests but does not specify a TDD sequence (failing test first, then impl). The test plan is adequate for implementation but doesn't enforce TDD ordering.

## 4. Security Review

- **Posture: sound.** AnyFail fails closed: backend errors and schema mismatches are treated as failures, never silently demoted. No hardcoded secrets, no shell execution, no unvalidated input reaches dangerous code paths.
- **JSON parsing**: `serde_json::from_str` on untrusted backend output is safe (no eval, no code execution). The parsed `Value` is only inspected for a `"pass"` bool key.
- **No injection risk**: The `AnyFailReason` payloads (`payload`, `message` strings) are debug/diagnostic strings, not commands or URLs.
- **One minor concern**: `AnyFailReason::VerdictRejected { payload }` stores the raw backend text. If that text contains secrets from a prior prompt that leaked into the response, the error will carry them into logs/traces. The PRD's NFR (row 193) requires secret redaction in `trace.jsonl`; the design should note that `StrategyError::AnyFail` output needs the same redaction at the trace writer boundary (T-029's responsibility, but worth flagging).

## 5. Implementation Concerns

- **Test flakiness risk**. Tests `first_fails` and `mid_list_fails` assert specific backend names (`b0`, `b1`) as the offender. With `FuturesUnordered`, completion order depends on tokio scheduling, not target index. The `MockBackend` in the existing code has no delay control to force ordering. Either (a) add deterministic delays to mock backends, or (b) relax assertions to check *that* a specific backend appears as offender without requiring it to be a specific one in arrival-order tests.
- **Missing implementation phasing**. The design does not specify file-by-file commit order. Following handoff TDD discipline: (1) add `AnyFailReason` + `StrategyError::AnyFail` + `PhaseError::AggregatorContract` to `mod.rs`/`family.rs`, (2) write failing tests in `parallel_fanout.rs`, (3) implement `any_fail_evaluate` + execute loop, (4) run `make check`.
- **`make check` testability**: The test plan relies on mock backends (no I/O), which is correctly aligned with the pre-merge gate. Schema validation after each test case is smart but the design doesn't specify whether the existing `tests/schema_validation.rs` integration test covers this or whether a new harness is needed.
- **`valid_json_extra_keys` test**: Good forward-compat test, but the design doesn't define a JSON Schema for the verdict itself. Only `{ "pass": bool }` is specified in prose. A small verdict schema would formalize the forward-compat contract.

## 6. Concurrency & Async

- **`FuturesUnordered` + short-circuit is safe.** Dropping the `FuturesUnordered` after early return cancels in-flight futures cooperatively. This matches the existing Concat short-circuit pattern.
- **No blocking calls in async path.** `serde_json::from_str` on small verdict payloads is CPU-bound and fast (<1µs). Acceptable inside the async loop without `spawn_blocking`.
- **Cancellation safety**: Not explicitly discussed. When AnyFail short-circuits, remaining backend futures are dropped. The `Backend::query` implementations must handle cancellation gracefully (no partial writes, no leaked connections). The existing code already relies on this for Concat, so this is inherited behavior, not new risk, but worth a note.
- **No race conditions**: The evaluation loop is single-tasked (one `while let` over `FuturesUnordered`), so no concurrent mutation of `attempts` or `successes`. Correct.

## 7. Blind Spots

1. **Markdown-fence-wrapped JSON**: No provision for backends that return `` ```json
{"pass":true}
``` `` instead of bare `{"pass":true}`. This is the most common LLM formatting artifact. The fail-closed default (`VerdictContract`) is correct but will produce surprising errors on many real backends.
2. **Empty `query.text`**: Not covered by test plan. `serde_json::from_str("")` returns an error, mapped to `VerdictContract`. Correct but implicit; should have an explicit test case.
3. **Per-branch `VerifyOutcome`**: The design keeps `verify: VerifyOutcome::skipped()` for every branch. Under AnyFail, branches that reported `pass: true` have a meaningful outcome (they passed the verdict check). Should they get `VerifyOutcome::passed("Aggregator::AnyFail")`? The current design defers this to the finalize step, which only updates the top-level `verify`. This loses per-branch granularity.
4. **Trace events**: No mention of what `trace.jsonl` events (T-029) an AnyFail evaluation should emit. The design modifies runtime behavior but doesn't specify observability.
5. **Interaction with `FloorViolation`**: If `min_responses = 3` and `targets = [a, b, c]`, and backend `a` returns `{"pass":false}` while `b` fails with a network error, which error takes priority? The design says AnyFail short-circuits on *first* failure in arrival order. FloorViolation can only occur after the loop completes. But with AnyFail disabling the short-circuit exit, every branch is awaited. The priority is implicitly: AnyFail > FloorViolation. This should be stated explicitly.
6. **`offender` field semantic ambiguity**: For `BackendError`, there's no actual query result. The design doesn't clarify what goes in `offender`. A synthetic Attempt (with `FinishReason::Error`) works but is not documented.
7. **No verdict JSON schema**: The prose says "lock the JSON verdict schema at `{ "pass": bool }`" but no JSON Schema artifact is specified for it. Adding one under `docs/schemas/` would make the forward-compat contract machine-checkable.
8. **Secret leakage in error payloads**: `VerdictRejected { payload }` stores raw backend output. If a backend echoes prompt content (which LLMs sometimes do), the error carries potentially sensitive text into logs/traces. Note dependency on T-029's redaction layer.

## 8. Verdict

**APPROVE_WITH_SUGGESTIONS**

The design is technically sound, correctly scoped for T-018, and fail-closed by default. The inline approach is pragmatic. Primary concerns are: (1) handoff module-placement rule, (2) test flakiness from `FuturesUnordered` ordering assumptions, and (3) missing handling of real-world LLM JSON formatting. None are blocking, but all should be addressed before implementation hardens.

## 9. Actionable Feedback

| Priority | Item |
|----------|------|
| **High** | Create `src/aggregator/` module (even if thin: `mod.rs` with `any_fail_evaluate` + `AnyFailReason`). Aligns with handoff "new primitives land as new modules" rule. The function can be re-exported from `parallel_fanout.rs` until T-017 redesigns the trait. |
| **High** | Fix arrival-order test assertions. Add deterministic delays to `MockBackend` (e.g., `MockBackend::delayed_ok("b1", ..., 5ms)`) to enforce completion order, or relax assertions to check only that the offender is *one of* the failing backends. |
| **High** | Add JSON markdown-fence stripping to `any_fail_evaluate` before `serde_json::from_str`. Strip leading/trailing `` ```json `` / `` ``` `` fences. This is the #1 source of false `VerdictContract` errors in practice. |
| **Medium** | Add explicit test case for empty `query.text`. |
| **Medium** | Document what goes in `offender: Box<Attempt>` for the `BackendError` case (synthetic attempt with `FinishReason::Error`). |
| **Medium** | Simplify `any_fail_evaluate` return type to `Result<(), AnyFailReason>`. Remove the `Ok(bool)` dead branch; reference T-017 as the point where the signature generalizes. |
| **Medium** | Add a small `verdict.schema.json` under `docs/schemas/` to formalize the `{ "pass": bool, ... }` forward-compat contract. |
| **Low** | Note in §4.3 or Open Questions that `StrategyError::AnyFail` payloads need secret redaction at the T-029 trace boundary. |
| **Low** | Add an explicit priority rule: when both `AnyFail` and `FloorViolation` are possible, `AnyFail` takes precedence (early-return on first failure). |
| **Low** | Consider setting per-branch `verify` to `VerifyOutcome::passed("Aggregator::AnyFail")` on `pass: true` branches instead of leaving all `skipped()`. Improves downstream observability. |
