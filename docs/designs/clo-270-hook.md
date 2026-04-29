# Design: CLO-270 — VerifyHook trait and VerifyResult enum

**Task:** T-020 (Roadmap Phase 4 - Verify hooks)  
**PRD:** FR-18 · Canonical design: §4.4  
**Status:** Draft  

---

## Problem

Every verify-shaped path in loker currently uses stand-in code. `EscalatingRetry`
(CLO-258) pattern‑matches on a plain `String` in `VerifyResult::Fail` and passes
`&QueryOutput` directly to the hook, coupling backend internals to verification
logic. `FailureReason` does not exist — CLO‑260's `pass_failure_context` can't
surface structured diagnostics. No `VerifyContext` exists — downstream hooks
(CLO‑271 RunCommand, CLO‑272 LLMVerifier, CLO‑273 TestRunner) have no shared
input type. T‑020 closes all three gaps: a single trait, a single forward‑compatible
result enum, and one context type that future phase‑runner callers (T‑029) reuse
without touching hook implementations.

---

## Goals / Non‑goals

### Goals
- Refactor `src/strategy/verify.rs` with `#[non_exhaustive]` on `VerifyResult`.
- Introduce `FailureReason` carrying stdout/stderr + structured reason + truncation flag.
- Introduce `VerifyContext` replacing `&QueryOutput` in the trait signature.
- Add `VerifyContext::from_query_output` factory for EscalatingRetry's call site.
- Add unit tests for a stub `VerifyHook` returning each concrete variant.
- Update `EscalatingRetry` to consume `FailureReason` and `VerifyContext`.
- All reserved variants (`Repair`, `Score`) compile and are matched with documented
  fallthrough in every consumer.

### Non‑goals
- **Do not** implement concrete hooks (RunCommand, LLMVerifier, TestRunner) —
  those are CLO‑271 / CLO‑272 / CLO‑273.
- **Do not** wire `pass_failure_context` to `FailureReason` end‑to‑end in this
  task — that's CLO‑260's retro‑fit.
- **Do not** change `src/apply_verify/verification.rs` — it has an unrelated
  `VerifyResult` for shell‑command runs.
- **Do not** change the `Aggregator` or `Strategy` trait boundaries.

---

## Architecture

### Module layout

```
src/strategy/verify.rs          ← refactored in-place
  ├── FailureReason             (new)
  ├── VerifyResult               (refactored)
  ├── VerifyError                (unchanged shape)
  ├── VerifyContext              (new)
  ├── VerifyHook trait           (signature change)
  └── #[cfg(test)] mod tests     (new — 0% coverage today)

src/strategy/escalating_retry.rs ← consumer updated
src/strategy/mod.rs              ← re-exports unchanged (same names)
```

### Data flow

```
Backend::query()
   │
   ▼
QueryOutput ─────────────────────────────────────┐
   │                                              │
   ▼                                              │
VerifyContext::from_query_output(&query) ◄────────┘
   │  .stdout, .stderr, .exit_code, .backend,
   │  .model, .structured, .duration
   ▼
VerifyHook::verify(&ctx)
   │
   ├── Ok(VerifyResult::Pass)           → ladder stops
   ├── Ok(VerifyResult::Fail { reason })→ reason is FailureReason
   ├── Ok(VerifyResult::Repair { .. })   → reserved, fallthrough
   ├── Ok(VerifyResult::Score(..))      → reserved, fallthrough
   └── Err(VerifyError)                 → hook fault, ladder continues
```

### Type taxonomy

| Type | Purpose | v0 concrete? |
|------|---------|-------------|
| `VerifyResult::Pass` | Hook says yes | ✅ |
| `VerifyResult::Fail { reason: FailureReason }` | Hook says no, with structured context | ✅ |
| `VerifyResult::Repair { suggestion }` | Reserved — retry same backend with suggestion | ❌ (compiles, fallthrough) |
| `VerifyResult::Score(f32)` | Reserved — threshold gate. Higher values = better quality. | ❌ (compiles, fallthrough) |
| `VerifyError` | Hook itself crashed (sandbox, network, spawn) | ✅ |
| `FailureReason` | Carries verifier stdout + stderr + structured reason + truncated flag | ✅ |
| `VerifyContext` | Input to hook: phase artefacts, not credentials | ✅ |

---

## Public API surface

### `src/strategy/verify.rs` (target state)

```rust
use crate::backend::QueryOutput;
use async_trait::async_trait;
use std::time::Duration;

// ── FailureReason ────────────────────────────────────────────

/// Structured reason a verification hook returned `Fail`.
///
/// Carries enough detail to feed `pass_failure_context` (CLO-260).
/// Fields are `pub` so callers can extract individual signals without
/// parsing the combined `display()` string.
///
/// ## Security: Redaction
///
/// `stdout` and `stderr` carry raw output that may contain secrets
/// (API keys in LLM responses, stack traces with env vars). Redaction
/// is **deferred to the consumer** — CLO-260's `pass_failure_context`
/// path runs `redact_secrets()` on the reason before flowing it into
/// the next prompt. Hook implementations that log or persist
/// `FailureReason` fields directly must apply their own redaction.
#[derive(Debug, Clone, PartialEq)]
pub struct FailureReason {
    /// Human-readable summary (e.g. "test `it_adds` failed").
    pub summary: String,
    /// Captured stdout from the verification run (may be truncated).
    /// **Unredacted** — consumers must apply redaction before prompt injection.
    pub stdout: String,
    /// Captured stderr from the verification run (may be truncated).
    /// **Unredacted** — consumers must apply redaction before prompt injection.
    pub stderr: String,
    /// `true` iff stdout or stderr was truncated at `MAX_OUTPUT_BYTES`.
    pub truncated: bool,
    /// Exit code if the verifier ran as a process. `None` for in‑process
    /// verifiers (e.g. `LLMVerifier`).
    pub exit_code: Option<i32>,
}

impl FailureReason {
    pub fn new(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            stdout: String::new(),
            stderr: String::new(),
            truncated: false,
            exit_code: None,
        }
    }

    pub fn with_stdout(mut self, stdout: impl Into<String>) -> Self {
        self.stdout = stdout.into();
        self
    }

    pub fn with_stderr(mut self, stderr: impl Into<String>) -> Self {
        self.stderr = stderr.into();
        self
    }

    pub fn with_truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }

    pub fn with_exit_code(mut self, exit_code: i32) -> Self {
        self.exit_code = Some(exit_code);
        self
    }
}

impl std::fmt::Display for FailureReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.summary)?;
        if self.truncated {
            write!(f, " (truncated)")?;
        }
        Ok(())
    }
}

// ── VerifyResult ─────────────────────────────────────────────

/// Verdict returned by a `VerifyHook::verify()` call.
///
/// **Variant lifecycle** (per design doc §10):
///
/// | Variant | v0 status | Notes |
/// |---------|-----------|-------|
/// | `Pass`  | **live** — emitted by v0 hooks | |
/// | `Fail { reason }` | **live** — `reason` is `FailureReason` | |
/// | `Repair { suggestion }` | **reserved** — compiles, no caller acts on it yet | M10 `HumanVerifier` will emit this |
/// | `Score(f32)` | **reserved** — compiles, no caller acts on it yet | Future cascadeflow‑style semantic gates |
///
/// Callers that pattern‑match MUST include arms for reserved variants;
/// the recommended pattern is a documented fallthrough (see
/// `escalating_retry.rs` for the reference consumer).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum VerifyResult {
    Pass,
    Fail { reason: FailureReason },
    Repair { suggestion: String },
    Score(f32),
}

impl VerifyResult {
    /// Convenience constructor for a `Pass` variant.
    pub fn pass() -> Self {
        Self::Pass
    }

    /// Convenience constructor for a `Fail` variant with a simple summary.
    /// Other `FailureReason` fields default to empty.
    pub fn fail(summary: impl Into<String>) -> Self {
        Self::Fail {
            reason: FailureReason::new(summary),
        }
    }

    /// Convenience constructor for a `Fail` variant with a fully populated reason.
    pub fn fail_with(reason: FailureReason) -> Self {
        Self::Fail { reason }
    }

    /// `true` iff this is a `Pass` variant.
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    /// `true` iff this is a `Fail` variant.
    pub fn is_fail(&self) -> bool {
        matches!(self, Self::Fail { .. })
    }
}

// ── VerifyError ──────────────────────────────────────────────

/// Error surfaced when a `VerifyHook` implementation itself fails.
///
/// Distinct from `VerifyResult::Fail`:
/// - `Fail` means the hook ran, decided "that output isn't good enough",
///   and produced a structured `FailureReason`.
/// - `VerifyError` means the hook could not run at all: sandbox crash,
///   backend unreachable, `make` missing from `$PATH`, etc.
///
/// ## Future: error source chain
/// For v0 the `message` string suffices. When CLO-271 (RunCommand)
/// introduces I/O errors and subprocess failures, `VerifyError` should
/// gain a `#[source]`-annotated field (e.g. `source: Option<Box<dyn std::error::Error + Send + Sync>>`)
/// to preserve the original error chain for debugging.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[error("verify hook failed: {message}")]
pub struct VerifyError {
    pub message: String,
}

impl VerifyError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

// ── VerifyContext ────────────────────────────────────────────

/// Input passed to every `VerifyHook::verify()` call.
///
/// Carries the output under verification plus metadata about the phase
/// and backend that produced it. Does **not** carry credentials
/// (API keys, tokens) — those live in `BackendConfig` and are never
/// exposed to verify hooks.
///
/// `#[non_exhaustive]` so the phase runner (T-029) can add fields
/// (manifest pointer, run‑dir paths) without breaking hook implementations.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct VerifyContext {
    /// Raw stdout from the backend call under verification.
    pub stdout: String,
    /// Raw stderr from the backend call, if any.
    pub stderr: Option<String>,
    /// Exit code if the backend ran as a subprocess.
    pub exit_code: Option<i32>,
    /// Name of the backend that produced this output (e.g. `"claude-3"`).
    pub backend_name: String,
    /// Model name reported by the backend, if known.
    pub model: Option<String>,
    /// Parsed JSON if the output was successfully deserialized.
    pub structured: Option<serde_json::Value>,
    /// Wall‑clock duration of the backend call, as measured by `Backend::query()`.
    pub duration: Duration,
}

impl VerifyContext {
    /// Build a `VerifyContext` from a `QueryOutput` plus a backend name.
    ///
    /// This is the EscalatingRetry call‑site constructor. When the phase
    /// runner (T-029) replaces EscalatingRetry as the direct caller, it
    /// can build `VerifyContext` from other sources (manifest, run dir)
    /// without touching hook implementations.
    pub fn from_query_output(query: &QueryOutput) -> Self {
        Self {
            stdout: query.stdout.clone(),
            stderr: query.stderr.clone(),
            exit_code: query.exit_code,
            backend_name: query.backend.clone(),
            model: query.model.clone(),
            structured: query.structured.clone(),
            duration: query.duration,
        }
    }
}

// ── VerifyHook trait ─────────────────────────────────────────

/// Verification hook that gates strategy progress.
///
/// Implementations are `Send + Sync` so they can be shared behind `Arc`
/// and driven across async tasks by the phase runner.
///
/// ## Method contract
///
/// - `name()` returns a stable, human‑readable label for trace output
///   (e.g. `"TestRunner"`, `"LLMVerifier"`).
/// - `verify(ctx)` inspects the backend output in `ctx` and returns a
///   verdict. `Err(VerifyError)` signals the hook itself failed;
///   `Ok(VerifyResult::Fail { .. })` signals the hook ran successfully
///   but judged the output insufficient.
///
/// ## Required context contract
///
/// As `VerifyContext` gains fields over time (via `#[non_exhaustive]`),
/// a hook may receive a context that lacks a field required for its
/// operation. In that case the hook MUST return `Err(VerifyError)` with
/// a descriptive message — it MUST NOT panic.
///
/// ## Cancellation safety
///
/// `verify()` implementors are responsible for cancellation safety.
/// If a `tokio::timeout` (or similar) drops the future mid-execution,
/// any spawned subprocesses or in-flight HTTP requests must be cleaned
/// up (e.g. via `tokio::spawn` with a cancellation token, or by
/// ensuring the future is abort-safe).
#[async_trait]
pub trait VerifyHook: Send + Sync {
    fn name(&self) -> &str;

    async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError>;
}
```

### Consumer update: `escalating_retry.rs`

The call site changes from:

```rust
// Before (current)
self.verify.verify(&query).await
```

To:

```rust
// After (target)
let ctx = VerifyContext::from_query_output(&query);
self.verify.verify(&ctx).await
```

The pattern‑match on `VerifyResult::Fail { reason }` changes from `reason` being
`String` to `reason: FailureReason`. The existing `reason.clone()` call must be
replaced with `reason.summary.clone()` (or the full `FailureReason` depending on
what `pass_failure_context` needs — the full struct is the intent of CLO‑260).

---

## Test plan

### Unit tests (`src/strategy/verify.rs`)

| Test | What it proves |
|------|----------------|
| `stub_verify_hook_returns_pass` | A stub `VerifyHook` returning `Pass` → `is_pass()` is `true`, `is_fail()` is `false`. |
| `stub_verify_hook_returns_fail` | A stub returning `Fail { reason }` → `is_fail()` is `true`, `FailureReason` fields intact. |
| `stub_verify_hook_returns_fail_with_full_reason` | `FailureReason` with all fields populated round‑trips through the hook. |
| `stub_verify_hook_returns_error` | A stub returning `Err(VerifyError)` → caller sees `VerifyError` with correct message. |
| `reserved_repair_compiles_but_not_pass` | A stub returning `Repair { .. }` → `is_pass()` is `false` (fallthrough behavior verified). |
| `reserved_score_compiles_but_not_pass` | A stub returning `Score(0.9)` → `is_pass()` is `false`. |
| `failure_reason_builder_api` | `FailureReason::new("msg").with_stdout("out").with_stderr("err").with_truncated(true).with_exit_code(1)` produces correct values. |
| `verify_context_from_query_output` | All `QueryOutput` fields transfer correctly to `VerifyContext`. |

### Consumer tests (`escalating_retry.rs`)

Existing EscalatingRetry tests already use mock backends and verify hooks (via
`VerifyOutcome`). The `FailureContext::from_verify_fail` function accepts
`impl Into<String>` for the reason — after the change, it should accept
`&FailureReason` instead. Update the test helper and verify all existing
tests pass.

### Manual verification

1. `cargo test` — all strategy tests pass, no compilation errors on reserved variants.
2. `cargo build` — `VerifyResult` is used in `EscalatingRetry` struct definition (`Arc<dyn VerifyHook>`), confirms trait object safety.
3. `cargo doc --no-deps` — verify `#[non_exhaustive]` and reserved variant docs appear.

---

## Migration / rollout (TDD-first)

1. **Write failing unit tests** in `verify.rs` — stub hooks returning each concrete variant. Red-first per handoff TDD guidance.
2. **Refactor `verify.rs`** — add `FailureReason`, `VerifyContext`, `#[non_exhaustive]` on `VerifyResult`. Confirm tests pass.
3. **Update `escalating_retry.rs`** — call site uses `VerifyContext::from_query_output`, match arm extracts `reason.summary.clone()` for `FailureContext` (reserved `Repair`/`Score` variants fall through to `"verify did not pass"`).
4. **`cargo test`** — all existing tests must pass.
5. **`cargo build`** — verify compilation.

No feature flags, no config changes, no data migration. This is a pure
type‑level change that doesn't alter runtime behavior (the same fields flow
through, just with stronger types).

CLO‑260 retro‑fit (consuming `FailureReason` in `pass_failure_context`) is a
separate task that follows this one. The `FailureContext::from_verify_fail`
constructor in `escalating_retry.rs` will accept `&FailureReason` instead of
`impl Into<String>` — that's the integration seam.

### Truncation cap: `MAX_OUTPUT_BYTES`

Both `FailureReason.stdout` and `FailureReason.stderr` are truncated at
`MAX_OUTPUT_BYTES` before being stored. This cap is defined in
`src/strategy/escalating_retry.rs` as `MAX_RESPONSE_EXCERPT_BYTES = 4096`
and is **inherited** by this task — the same constant is used for
`FailureReason` fields. CLO-271 (RunCommand) will introduce its own
byte-cap constants; that change is scoped to CLO-271.

---

## Acceptance Criteria

1. `src/strategy/verify.rs` compiles with `#[non_exhaustive]` on `VerifyResult`, `FailureReason`, `VerifyContext` types added.
2. `VerifyHook` trait signature takes `&VerifyContext` instead of `&QueryOutput`.
3. `EscalatingRetry` builds `VerifyContext::from_query_output(&query)` and extracts `reason.summary.clone()` from `Fail` variants.
4. `Repair` and `Score` variants compile and fall through to `"verify did not pass"` in `EscalatingRetry`'s match.
5. At least 8 unit tests pass in `src/strategy/verify.rs` (all stub-hook variants + builder API + context mapping).
6. All existing strategy tests remain green after the refactor.
7. `cargo doc --no-deps` shows reserved-variant lifecycle docs and `#[non_exhaustive]` annotations.

### Rollback

This is a purely type-level change with no config or data migration. Rollback = revert the commit. No schema or file format changes.

---

## Open questions

1. **`VerifyResult::Pass` drops `notes` field?**  
   The current impl has `Pass { notes: Option<String> }`. The CLO‑270 issue
   and design doc §4.4 both show `Pass` without notes. The notes field was
   never populated in the existing codebase. **Resolution:** Drop `notes` —
   if needed later, `#[non_exhaustive]` lets us add a `PassContext` struct
   without breaking callers.

2. **Should `VerifyContext` carry `duration`?**  
   The CLO‑260 `pass_failure_context` doesn't use duration today, but
   future trace‑level enrichment might want it. The field costs one `Copy`
   and is already available. **Resolution:** Keep `duration` in `VerifyContext`.

3. **Should `FailureReason` be `#[non_exhaustive]`?**  
   Adding a field to `FailureReason` is a backward‑compatible change
   (all fields have defaults / builders). `#[non_exhaustive]` on a struct
   prevents pattern‑matching on all fields. **Resolution:** Don't add
   `#[non_exhaustive]` on `FailureReason` — the builder API already supports
   additive field changes.

5. **Should `VerifyResult` and `FailureReason` derive `Serialize`/`Deserialize`?**  
   The T-029 phase runner will need to serialize verify results for trace
   output. However, the schema mapping (`phase_result_escalating.schema.json`)
   depends on T-029's design. **Resolution:** Defer serialization derives to
   T-029. If needed earlier, both types are additive-safe and derives can be
   added without breaking changes.

4. **Does `EscalatingRetry` store the new types or just pass them through?**  
   `EscalatingRetry` stores `Arc<dyn VerifyHook>` — the trait object type
   changes only if the trait signature changes, which it does. But the
   storage field type (`Arc<dyn VerifyHook>`) is the same. **Resolution:**
   No change to `EscalatingRetry`'s struct fields other than the internal
   call site.
