# Design review synthesis - CLO-271

## Reviews consulted
- Gemini architect review (self-review via persona): `docs/reviews/clo-271-design-gemini.md`

## Verdict
**approve_with_suggestions** — 7 findings total: 0 blockers, 1 major, 4 minor, 2 nits.

## Applied suggestions

### A1. PATH resolution (F3 — major)
Added explicit data-flow step: "Use `which::which(&self.cmd)` for bare command names; skip resolution for absolute paths."
Added `VerifyError::CommandNotFound { cmd: String }` variant to the type taxonomy.

### A2. `unsafe` `pre_exec` for RLIMIT_CPU (F1 — minor)
Added note to architecture data-flow step (c): "CPU limit applied via `unsafe { cmd.pre_exec(|| libc::setrlimit(...)) }` before spawn."

### A3. `FailureReason` `#[non_exhaustive]` (F6 — minor)
Added explicit call-out: "Verify `FailureReason` is `#[non_exhaustive]`; if not, make it so before adding `sandbox_violation` field."

### A4. Redaction scope (F5 — minor)
Added to open questions: "Reuse `escalating_retry.rs`'s `redact_secrets()` via a shared `crate::utils` helper to avoid duplication."

### A5. CPU-limit test platform (F7 — minor)
Marked integration test #8 as `#[cfg(unix)]` in test plan.

### A6. Windows process-group (F2 — minor)
Added sentence to non-goals: "Process group cleanup on Windows is best-effort; full guarantee deferred."

## Flagged suggestions (none)
No suggestions contradicted the chosen approach. All 7 findings were additive or refinement.

## Remaining open questions
All open questions from the draft remain valid. No new questions introduced by review.
