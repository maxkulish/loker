# Design Review: CLO-270 — gemini-architect persona

**Reviewed:** `docs/designs/clo-270-hook.md`  
**Date:** 2026-04-29  
**Persona:** gemini-architect (simulated — no MCP/tooling available)

---

## Overall assessment

The design is sound, well-scoped, and grounded in the discovery report's
findings. The type taxonomy is clear, the data flow diagram is useful, and
the test plan is concrete. The builder API on `FailureReason` is a good
pattern that avoids constructor explosion.

Three areas need attention before finalizing.

---

## Suggestions

### S1 — `is_fail()` should check `Reason` not just the variant discriminant

**Location:** `VerifyResult::is_fail()`  
**Severity:** Medium  

```rust
pub fn is_fail(&self) -> bool {
    matches!(self, Self::Fail { .. })
}
```

This returns `true` even if a reserved consumer accidentally matches `Fail`
without inspecting `reason`. At minimum, add a doc comment that callers
should inspect `reason` fields. Better: rename to `is_fail_variant()` to
signal it's a shape check, not a semantic check.

**Verdict:** Additive — apply a doc note.

### S2 — `VerifyContext` is missing phase identity fields

**Location:** `VerifyContext` struct definition  
**Severity:** High  

The struct has `backend_name`, `model`, etc., but no `phase_name` or
`run_id`. The CLO-270 issue says "carries phase artefacts + manifest
pointer". While the manifest pointer is deferred to T-029, `phase_name`
is already available in `PhaseContext` and costs nothing to pass through.
Without it, hooks can't differentiate which phase they're verifying (e.g.,
a `RunCommand` hook might use different commands for `implement` vs `design`
phases).

**Verdict:** Additive — add `phase_name: String` to `VerifyContext` and
the `from_query_output` factory (accept it as a parameter).

### S3 — `from_query_output` signature mismatch with `escalating_retry.rs`

**Location:** `VerifyContext::from_query_output()`  
**Severity:** Medium  

The factory takes only `&QueryOutput`, but the EscalatingRetry call site
has access to `ctx: &PhaseContext` which carries `phase_name` and `run_id`.
If we add `phase_name` per S2, the factory needs that parameter. But
`from_query_output` should stay focused on translating backend output.
The cleanest pattern is:

```rust
pub fn from_query_output(query: &QueryOutput) -> Self { .. }
pub fn with_phase_name(mut self, name: impl Into<String>) -> Self { .. }
```

That way hooks implemented before T-029 lands don't change, and EscalatingRetry
chains `.from_query_output(&query).with_phase_name(ctx.phase_name)`.

**Verdict:** Refinement — accept. Update factory to use builder chaining.

### S4 — `VerifyResult::Pass` silently drops `notes` field

**Location:** Open question #1  
**Severity:** Low  

The current `Pass { notes: Option<String> }` is being replaced with `Pass`.
Open question #1 says "notes was never populated". But what if a consumer
(anywhere in the codebase or in-memory test helpers) builds `Pass { notes:
Some("...") }`? A git grep confirms no call site writes notes, so this is
safe. **Still:** worth a second grep to be certain before implement.

**Verdict:** Additive — add a pre-implement check step to grep for `notes`.

### S5 — `FailureReason` should implement `Display`

**Location:** `FailureReason` struct  
**Severity:** Low  

CLO-260's `pass_failure_context` will need to render `FailureReason` into
a prompt envelope string. Implementing `Display` (delegating to `summary`
with a fallback) ensures callers don't each invent their own format. Add:

```rust
impl std::fmt::Display for FailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.summary)
    }
}
```

**Verdict:** Additive — apply.

### S6 — EscalatingRetry reserved‑variant match arms aren't shown

**Location:** Design doc § Public API surface → Consumer update  
**Severity:** Medium  

The design says "Match arms for reserved variants exist (with a documented
fallthrough) in every consumer that pattern‑matches `VerifyResult`" (per AC).
But the design doc only shows the call-site change, not the match arms. The
EscalatingRetry currently matches only `Pass` and `Fail` — adding
`#[non_exhaustive]` to the enum without updating the match arms will produce
compiler warnings (non-exhaustive match). The implement step must add:

```rust
VerifyResult::Repair { suggestion } => {
    // Reserved — no caller acts on this in v0. Treat as fail to
    // keep the ladder walking (per design doc §10 non‑goals).
    tracing::warn!(suggestion, "ignoring reserved Repair variant");
    // fall through to fail behavior
}
VerifyResult::Score(s) => {
    tracing::warn!(score = s, "ignoring reserved Score variant");
    // fall through to fail behavior
}
```

**Verdict:** Additive — expand the consumer update section with concrete
match arms.

### S7 — No `Debug` / `PartialEq` derive on `VerifyError`

**Location:** `VerifyError` struct  
**Severity:** Low  

The current impl derives `Debug, PartialEq, Eq`. The design doc shows those
derives in the target state. **No action** — just confirming they're preserved.

---

## Summary

| ID | Class | What |
|----|-------|------|
| S1 | Additive | Doc note on `is_fail()` |
| S2 | Additive | Add `phase_name` to `VerifyContext` |
| S3 | Refinement | Builder chaining for `from_query_output` + `with_phase_name` |
| S4 | Additive | Pre-implement grep for `notes` usage |
| S5 | Additive | `impl Display for FailureReason` |
| S6 | Additive | Show reserved‑variant match arms in design |
| S7 | — | Confirmed `Debug`/`PartialEq` on `VerifyError` preserved |

**Verdict:** approve_with_changes — 5 additive, 1 refinement, 0 contradictions.
