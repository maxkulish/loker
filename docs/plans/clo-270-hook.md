# Plan: CLO-270 — Define VerifyHook trait and VerifyResult enum

## Context
- **Design:** docs/designs/clo-270-hook.md
- **Discovery:** docs/discovery/clo-270.md
- **Linear:** https://linear.app/cloud-ai/issue/CLO-270/define-verifyhook-trait-and-verifyresult-enum
- **Approach:** C — Enrich in-place VerifyHook trait + add VerifyContext factory constructor
- **Pre-merge gate:** `make check` (fmt + clippy + test)

## Sub-tasks

### ST1 — Add types and refactor `src/strategy/verify.rs`
**Files:** `src/strategy/verify.rs`, `src/strategy/mod.rs`

**Changes:**

1. Add `FailureReason` struct with builder API (`new`, `with_stdout`, `with_stderr`, `with_truncated`, `with_exit_code`) and `Display` impl.
2. Add `VerifyContext` struct with `from_query_output(query: &QueryOutput)` factory constructor.
3. Add `#[non_exhaustive]` to `VerifyResult` enum.
4. Change `VerifyResult::Pass { notes: Option<String> }` → `VerifyResult::Pass` (drop `notes`).
5. Change `VerifyResult::Fail { reason: String }` → `VerifyResult::Fail { reason: FailureReason }`.
6. Add `VerifyResult::fail_with(reason: FailureReason)` convenience constructor.
7. Add `VerifyResult::is_fail()` helper.
8. Update `VerifyHook` trait signature: `async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError>` (replaces `&QueryOutput`).
9. Add doc comments: redaction policy on `FailureReason`, missing-context contract + cancellation safety on `VerifyHook`, future `#[source]` note on `VerifyError`.
10. Update `src/strategy/mod.rs` re-exports — no new types needed (same names), but `QueryOutput` import is no longer re-exported through `verify.rs`.

**Acceptance:** `cargo build` compiles with no errors. `cargo doc --no-deps` shows `#[non_exhaustive]` on `VerifyResult` and `VerifyContext`.
**Estimate:** S

### ST2 — Write unit tests for `verify.rs` (TDD — before consumer update)
**Files:** `src/strategy/verify.rs` (add `#[cfg(test)] mod tests`)

**Test cases:**

| Test | What it proves |
|------|----------------|
| `stub_verify_hook_returns_pass` | Stub returning `Pass` → `is_pass()` true, `is_fail()` false |
| `stub_verify_hook_returns_fail` | Stub returning `Fail { reason }` → `is_fail()` true, `FailureReason` fields intact |
| `stub_verify_hook_returns_fail_with_full_reason` | All `FailureReason` fields round-trip through the hook |
| `stub_verify_hook_returns_error` | Stub returning `Err(VerifyError)` → caller sees correct error |
| `reserved_repair_compiles_but_not_pass` | Stub returning `Repair { .. }` → `is_pass()` false |
| `reserved_score_compiles_but_not_pass` | Stub returning `Score(0.9)` → `is_pass()` false |
| `failure_reason_builder_api` | Builder chain produces correct values |
| `verify_context_from_query_output` | All `QueryOutput` fields transfer correctly |

**Acceptance:** `cargo test strategy::verify::tests` passes.
**Estimate:** S

### ST3 — Update `src/strategy/escalating_retry.rs` consumer
**Files:** `src/strategy/escalating_retry.rs`

**Changes:**

1. Change call site `self.verify.verify(&query).await` → `self.verify.verify(&ctx).await` with `VerifyContext::from_query_output(&query)`.
2. Update match arm on `VerifyResult::Fail { reason }` — extract `reason.summary.clone()` instead of `reason.clone()` (since `reason` is now `FailureReason`, not `String`).
3. Reserved variants (`Repair`, `Score`) fall through to `"verify did not pass"` via the existing `_` arm — no new match branches needed.
4. No change to `FailureContext::from_verify_fail` signature — that retrofit is deferred to CLO-260. The existing `impl Into<String>` parameter receives `reason.summary.clone()`.

**Acceptance:** `cargo test strategy::escalating_retry::tests` passes.
**Estimate:** S

### ST4 — Update consumer tests in `escalating_retry.rs`
**Files:** `src/strategy/escalating_retry.rs` (test module)

**Changes:**

1. Verify `FailureContext::from_verify_fail` calls in tests still pass — they use string literals, which implement `impl Into<String>`, unchanged.
2. Verify mock/stub hooks in tests compile with new `&VerifyContext` signature — update `MockVerifyHook::verify` signature to accept `&VerifyContext`.
3. Verify all `from_query_output` field mappings are correct if any test constructs a `VerifyContext` directly.

**Acceptance:** `cargo test` — all strategy tests green.
**Estimate:** S

### ST5 — Pre-merge gate: `make check`
**Files:** All changed files

**Acceptance:** `make check` passes (rustfmt + clippy + test).
**Estimate:** S

## Dependency graph

```
ST1 (types + refactor) ──► ST2 (unit tests) ──► ST3 (consumer update) ──► ST4 (test update) ──► ST5 (gate)
```

ST2 must run before ST3 (TDD-first per handoff guidance). Each sub-task builds on its predecessor.

## Pre-merge gate
- `make check` (rustfmt + clippy + test)
- No feature flags, no config changes, no data migration — pure type-level change

## Risks
- **Low:** `VerifyContext::from_query_output` clones `stdout` (potentially large). For v0 with a single consumer (`EscalatingRetry`) this is acceptable. T-029 phase runner can optimize with `Arc<QueryOutput>` if needed.
- **Low:** `VerifyResult::Pass { notes: Option<String> }` → bare `Pass` is a name-only break (notes was never populated). The `#[non_exhaustive]` attribute forces callers to add `_` wildcard arms, which is the intended forward-compat mechanism.
- **None:** Existing `from_verify_fail` signature is unchanged — CLO-260 retro-fit is deferred.
