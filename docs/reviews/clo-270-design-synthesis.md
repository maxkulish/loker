# Design Review Synthesis: CLO-270

**Reviewed:** `docs/designs/clo-270-hook.md` + gemini review  
**Date:** 2026-04-29

---

## Verdict: **approve_with_changes**

All suggestions are additive or refinements. None contradict the chosen
approach (C — Enrich in-place + factory constructor). All six actionable
suggestions should be applied before the design is finalized.

---

## Applied suggestions

| ID | Suggestion | Applied to |
|----|------------|-----------|
| S1 | Add doc note on `is_fail()` — rename to `is_fail_variant()` or document semantic vs shape check | `VerifyResult::is_fail()` |
| S2 | Add `phase_name: String` to `VerifyContext` + builder `with_phase_name()` | `VerifyContext` |
| S3 | Use builder chaining: `from_query_output(&query)` returns partial, `.with_phase_name(...)` completes it | `VerifyContext` factory |
| S4 | Pre-implement grep for `notes` usage to confirm safe to drop | Pre‑implement checklist |
| S5 | `impl Display for FailureReason` delegating to `summary` | `FailureReason` |
| S6 | Show concrete reserved‑variant match arms in `escalating_retry.rs` update section | Design doc consumer update |

## Flagged suggestions

None. All suggestions align with the chosen approach.

---

## Updated design doc sections (summary of changes)

### `FailureReason` gains `Display`

```rust
impl std::fmt::Display for FailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.summary)
    }
}
```

### `VerifyContext` gains `phase_name` + builder

```rust
#[non_exhaustive]
pub struct VerifyContext {
    pub stdout: String,
    pub stderr: Option<String>,
    pub exit_code: Option<i32>,
    pub backend_name: String,
    pub model: Option<String>,
    pub structured: Option<serde_json::Value>,
    pub duration: Duration,
    pub phase_name: String,              // ← NEW
}
```

Factory becomes:

```rust
impl VerifyContext {
    pub fn from_query_output(query: &QueryOutput) -> Self {
        Self {
            stdout: query.stdout.clone(),
            stderr: query.stderr.clone(),
            exit_code: query.exit_code,
            backend_name: query.backend.clone(),
            model: query.model.clone(),
            structured: query.structured.clone(),
            duration: query.duration,
            phase_name: String::new(),  // filled by with_phase_name()
        }
    }

    pub fn with_phase_name(mut self, name: impl Into<String>) -> Self {
        self.phase_name = name.into();
        self
    }
}
```

### `is_fail()` doc note

```rust
/// `true` iff this is a `Fail` variant (shape check).
///
/// Callers should inspect `reason` fields for semantic meaning;
/// this method only confirms the discriminant, not the payload.
pub fn is_fail(&self) -> bool {
    matches!(self, Self::Fail { .. })
}
```

### EscalatingRetry consumer update (full match arms)

```rust
match self.verify.verify(&ctx).await {
    Ok(VerifyResult::Pass) => {
        // ladder stops — pass
    }
    Ok(VerifyResult::Fail { reason }) => {
        let fail_summary = reason.summary.clone();
        // store FailureReason for pass_failure_context (CLO-260)
        // advance to next rung
    }
    Ok(VerifyResult::Repair { suggestion }) => {
        // Reserved in v0 — treat as soft-fail, advance ladder.
        tracing::warn!(%suggestion, "ignoring reserved Repair variant");
        // advance to next rung
    }
    Ok(VerifyResult::Score(s)) => {
        // Reserved in v0 — treat as soft-fail, advance ladder.
        tracing::warn!(score = s, "ignoring reserved Score variant");
        // advance to next rung
    }
    Err(verify_err) => {
        // hook fault — advance to next rung
    }
}
```

---

## Open questions resolved

| Question | Resolution after review |
|----------|------------------------|
| Q1 — Drop `notes` from `Pass` | ✅ Drop it. Pre‑implement grep confirms no call site writes notes. |
| Q2 — `duration` in `VerifyContext` | ✅ Keep it. No cost, future trace enrichment. |
| Q3 — `#[non_exhaustive]` on `FailureReason` | ✅ Don't add it. Builder API handles additive changes. |
| Q4 — EscalatingRetry storage change | ✅ No struct‑field changes, only internal call site. |
