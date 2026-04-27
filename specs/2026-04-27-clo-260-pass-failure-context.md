# Spec: CLO-260 Wire `pass_failure_context` on `Strategy::EscalatingRetry`

**Created**: 2026-04-27
**Estimated scope**: S (3 production files + 1 test file, ~4 sub-tasks)
**Linear**: [CLO-260](https://linear.app/cloud-ai/issue/CLO-260/add-pass-failure-context-flag-on-strategyescalatingretry)
**PRD**: FR-8 (`docs/prd/2026-04-25-loker.md`) - priority **Should**
**Roadmap**: T-014 (`docs/plans/001-implementation-roadmap.md`)
**Design doc**: `/Users/mk/Work/investigations/sakana-fugu/loker-design.md` §6 (escalating retry), §11 #1 (resolved: pass context, gated on flag)

## 1. Problem Statement

CLO-258 landed the `EscalatingRetry` walker in `src/strategy/escalating_retry.rs`
and added a scaffold field `pass_failure_context: bool` (default `false`) plus a
builder method `with_pass_failure_context()` (`src/strategy/escalating_retry.rs:53,70`).
The flag is not wired: today the walker renders the prompt **once** before the
loop (`escalating_retry.rs:88-90`) and re-uses that single rendered string for
every rung. The flag value is dead - rungs 2..N never receive any signal that
rung N-1 failed, what verifier said, or what the previous backend produced.

PRD FR-8 says: when `pass_failure_context = true`, "the prior backend's
failed output and verify error are appended to the next backend's prompt
context, gated by `pass_failure_context = true` on the strategy". Design doc
§11 #1 confirms: "cascadeflow does this and reports it materially helps
escalations succeed. Close as decided." So the behaviour is not in question -
only the wiring.

What's missing concretely:

1. A `FailureContext` payload type that captures the three signals the PRD
   names: verifier reason (`VerifyResult::Fail { reason }` or `VerifyError`),
   backend error class (`BackendError` variant name), and a truncated excerpt
   of the previous backend's response body.
2. A way to thread that payload into the next rung's render so the template
   can consume it. Two options - inject via `TemplateContext` variable, or
   prepend a deterministic header to the rendered string. We pick the second
   (simpler, no template-engine surface change, and the location is locked
   regardless of how the user's template is shaped).
3. A redaction step. The PRD/task description says "redaction hook reusing
   same secret-stripping filter as normal prompts" - **but no such filter
   exists today**. Searches for `redact|scrub|sanitize|secret_filter` in
   `src/` return only unrelated hits (`template/filters.rs:21` strips NUL
   bytes, not secrets). So this spec lands a minimal `redact_secrets`
   function inline in the strategy module, documents its boundary
   ("applies to FailureContext payload bytes before they enter the next
   prompt envelope"), and the task description's "reuse the same filter" is
   reframed: *this* IS the filter, and future tasks (whenever a centralised
   secret scrubber is built) MUST consume this same helper, not invent a
   second one.
4. A documented byte budget so the failure context cannot blow up the next
   backend's input window. Pick **4 KiB for the truncated response body**
   and **8 KiB total for the rendered FailureContext block** as soft caps,
   enforced by the implementation, asserted by tests.
5. Wire the existing `EscalatingRetry::with_pass_failure_context(true)` so
   it actually changes behaviour, with off-state preserving the current
   identity (no observable diff vs. CLO-258).

What is **out of scope** (deferred):

- Shipping a `design-doc-tdd` reference workflow file with the flag on.
  No such file exists in the repo today (`grep design-doc-tdd src/workflows/`
  returns nothing; only `audit.toml`, `diff.toml`, `explain.toml`, `hunt.toml`
  exist). That workflow file is a separate authoring task (likely landing
  with M6 / T-035 reference workflows). This spec instead documents the
  flag-on default in rustdoc on `EscalatingRetry::with_pass_failure_context`
  and adds a `# When to enable` comment block - so the next person to author
  the design-doc-tdd workflow has the guidance pinned in code, not just in
  the PRD.
- A central redaction filter shared with normal prompts. The task description
  asks us to "reuse the same secret-stripping filter as normal prompts" - that
  filter does not exist. We do NOT build a project-wide redaction system here
  (out of scope for an `S` task). We DO build a tiny, well-documented
  `redact_secrets` helper in `src/strategy/escalating_retry.rs` so the boundary
  is explicit and so a future centralised filter can absorb it.
- Wiring `EscalatingRetry` into the workflow loader / phase runner. That is
  CLO-261 / T-029. The flag plumbs through `EscalatingRetry::execute()` only.
- TOML schema parsing for the flag. The strategy struct is constructed by
  Rust code in tests; production loader work is CLO-261's concern. We do
  add a serde round-trip test on a serialisable strategy-config struct in
  `src/strategy/escalating_retry.rs` to lock the wire shape, but we do not
  edit `src/config.rs` or any workflow `.toml` file.

## 2. Acceptance Criteria

- [ ] **AC1**: `FailureContext` value type exists in `src/strategy/escalating_retry.rs`
      with these fields:
      ```rust
      pub struct FailureContext {
          pub previous_tier: Tier,
          pub previous_backend: String,
          pub verify_reason: Option<String>,         // VerifyResult::Fail.reason or VerifyError.message
          pub backend_error_class: Option<String>,   // e.g. "Timeout", "RateLimited", or None on verify-fail-only
          pub response_excerpt: Option<String>,      // truncated, redacted body; None if backend errored
      }
      ```
      Constructors: `FailureContext::from_verify_fail(rung, backend, reason, response, max_excerpt)`
      and `FailureContext::from_backend_error(rung, backend, error)`. Both
      apply redaction + truncation internally; callers cannot bypass.
- [ ] **AC2**: `redact_secrets(input: &str) -> String` helper in
      `src/strategy/escalating_retry.rs`, scoped to the module, replaces the
      following common secret shapes with the literal token `[REDACTED]`:
      1. AWS access keys (regex: `AKIA[0-9A-Z]{16}`)
      2. Generic API key = value (regex: `(api[_-]?key|secret|token|password)\s*[=:]\s*[^\s'"]+`, case-insensitive, replaces only the *value* side)
      3. Bearer tokens (regex: `[Bb]earer\s+[A-Za-z0-9._\-]+`)
      4. Long base64-ish blobs that contain `key`/`secret`/`token` adjacent text
         (heuristic: same sentence, ≥32 alphanumeric chars)
      The function is deterministic, allocation-bounded (does not panic on
      empty / huge inputs), and returns a fresh `String`. Documented
      boundary: "applied to every byte of FailureContext text before it
      reaches the next rung's rendered prompt."
- [ ] **AC3**: `truncate_excerpt(s: &str, max_bytes: usize) -> String` helper.
      If `s.len() <= max_bytes`, returns `s.to_string()`. Otherwise truncates
      to `max_bytes`, **at a UTF-8 char boundary** (no panic on multibyte
      cut), and appends ` …[truncated, N bytes elided]`. The byte budget
      constant `MAX_RESPONSE_EXCERPT_BYTES: usize = 4096` lives in the same
      module.
- [ ] **AC4**: `EscalatingRetry::execute()` is modified so that when a rung
      attempt does not pass (verify fail OR backend error), the walker
      builds a `FailureContext` from that attempt and threads it into the
      next rung's render only if `self.pass_failure_context == true`. When
      the flag is `false`, the walker behaves exactly as today (no diff in
      observable output). The off-path is asserted by AC8's snapshot test.
- [ ] **AC5**: Injection location is **locked**: when injecting, the walker
      prepends a deterministic header block to the freshly rendered prompt:
      ```
      <previous-attempt>
        tier: cheap
        backend: ollama-local
        verify_reason: "expected JSON object, got prose"
        backend_error: null
        response_excerpt: |
          [redacted, truncated body bytes 0..N]
      </previous-attempt>

      <original-prompt>
      [the rendered template body]
      </original-prompt>
      ```
      Format is YAML-ish inside angle-bracketed sections, chosen because (a)
      it is unambiguous to a model, (b) does not collide with common Markdown,
      (c) is greppable in audit logs. The exact format is asserted byte-for-
      byte by AC8's snapshot test - changes to it are a breaking change for
      anyone whose downstream tools parse `loker_artifacts/`.
- [ ] **AC6**: The walker re-renders the prompt template per rung when
      `pass_failure_context == true`, because the rendered body itself does
      not depend on the failure context (the failure context is prepended
      around it). Concretely: when the flag is on, the walker still renders
      the template once outside the loop (no per-rung TemplateContext
      mutation), then inside the loop wraps it with the
      `<previous-attempt>...<original-prompt>` envelope only for rungs ≥ 2.
      Rung 1 always sees the bare rendered prompt (no header) regardless of
      flag.
- [ ] **AC7**: Total injected envelope length is bounded at
      `MAX_FAILURE_CONTEXT_BYTES: usize = 8192` bytes. If the constructed
      header would exceed this budget, fields are truncated in this order
      until it fits: `response_excerpt` (most aggressive), `verify_reason`
      (cap at 1024 bytes), `backend_error_class` (already short, last to
      cut). The 8 KiB cap is asserted by a test passing in a 100 KiB
      response body and checking `header.len() <= 8192`.
- [ ] **AC8**: `tests/strategy_escalating_retry.rs` gains tests:
      - **flag-off identity**: rung 1 fails verify, rung 2 succeeds; assert
        rung 2 was called with the *exact* rendered prompt (no header).
        Snapshot via `insta` - the existing test pattern in this file.
      - **flag-on injection**: rung 1 fails verify, rung 2 succeeds; assert
        rung 2 was called with a prompt that **starts with**
        `<previous-attempt>\n  tier: cheap\n  backend: ` and that the body
        appears verbatim inside `<original-prompt>...</original-prompt>`.
        Snapshot the full envelope via `insta`.
      - **flag-on after backend error**: rung 1 errors (Timeout), rung 2
        succeeds; assert envelope's `verify_reason: null` and
        `backend_error: "Timeout"` and `response_excerpt: null`.
      - **flag-on truncation**: rung 1 returns a response of length 100 KiB,
        rung 2 succeeds; assert envelope length ≤ `MAX_FAILURE_CONTEXT_BYTES`,
        and that the excerpt section ends with `…[truncated, N bytes elided]`.
      - **flag-on redaction**: rung 1 returns a response containing
        `api_key=AKIA0123456789ABCDEF` and `Bearer eyJhbGciOiJIUzI1NiIsInR5cCI`,
        rung 2 succeeds; assert envelope contains `[REDACTED]` and does NOT
        contain the literal secret strings.
      - **flag-on three-rung chain**: rung 1 fails, rung 2 fails, rung 3
        succeeds; assert rung 3 was called with envelope referencing rung
        2's failure (most-recent-only; we do **not** chain the entire
        history). Document this choice in rustdoc.
- [ ] **AC9**: serde round-trip test for the strategy config shape. Add a
      `#[derive(Serialize, Deserialize)] struct EscalatingRetryConfig {
      rungs: Vec<RungConfig>, prompt_template: String, verify: VerifyHookRef,
      pass_failure_context: bool, }` (or update the existing one if it
      already exists in `src/strategy/escalating_retry.rs`). Test asserts:
      `toml::to_string(&cfg)` then `toml::from_str(&...)` round-trips with
      `pass_failure_context` preserved both as `true` and as `false`.
      Default of `pass_failure_context` is `false` via `#[serde(default)]`.
- [ ] **AC10**: rustdoc on `EscalatingRetry::with_pass_failure_context`
      documents the on-default for the design-doc-tdd workflow:
      ```rust
      /// When to enable: turn this on for workflows where escalation
      /// quality measurably improves with prior context (typical of
      /// strict-output TDD-style flows). The reference `design-doc-tdd`
      /// workflow ships with `pass_failure_context = true`. Off by
      /// default in v0 because (a) it widens the input prompt and so
      /// has a token cost, (b) it can leak failure-mode noise into the
      /// next rung if the verifier is too chatty.
      ```
- [ ] **AC11**: `make check` exits 0 (fmt + clippy + lib + integration tests).
- [ ] **AC12**: The existing test `pass_failure_context_defaults_false`
      (currently in `tests/strategy_escalating_retry.rs:261`) still passes
      unchanged. Default behaviour does not regress.

**Verification method**:
- AC1, AC2, AC3, AC9, AC10: `cargo build` + diff inspection.
- AC4-AC8, AC12: `cargo test --test strategy_escalating_retry`.
- AC11: `make check`.

## 3. Constraints

**Must**:
- Re-use the existing `EscalatingRetry` struct + builder; do not introduce a
  new struct or a parallel walker. The flag is already there - this spec
  wires it.
- Apply `redact_secrets` to **every** byte that lands in the rendered
  envelope, not just the response excerpt. The verifier reason and the
  backend error message can also leak secrets if the verifier dumped a
  request body. Three-line check: redact verify_reason, redact
  response_excerpt, redact the final assembled header text once more as a
  belt-and-suspenders pass.
- Lock the envelope format at the byte level. The `<previous-attempt>` and
  `<original-prompt>` tags are part of the public contract. Tests use
  `insta` snapshots so any change here is immediately visible in PR review.
- Pass the rendered envelope to the *backend's* `query()` call, not to the
  template engine. The header is appended after templating, by design - it
  must not be subject to template substitution (a `{{previous_response}}`
  inside the failure body would be evaluated by mistake otherwise).
- Default `pass_failure_context` to `false`. Both the struct field
  initialiser and the serde `#[serde(default)]` must agree.
- Track only the **most recent** failure when building the next rung's
  context. Do not accumulate the full failure history into one envelope -
  that scales linearly with rung count and was not asked for. Document
  this in rustdoc on `FailureContext`.

**Must-not**:
- Modify `Strategy` trait, `Prompt`, `PhaseContext`, or `StrategyOutput`.
  This is purely additive inside `escalating_retry.rs`.
- Touch `src/workflow.rs`, `src/config.rs`, the workflow `.toml` files
  under `src/workflows/`, or any of `src/backend/`. The flag plumbs only
  inside `src/strategy/escalating_retry.rs` and its test file. Wiring
  through workflow loading is CLO-261 / T-029.
- Add a new dependency. Use the regex crate iff already in `Cargo.toml`;
  otherwise hand-write the secret patterns with `str::contains` /
  `char_indices` (the four patterns are simple enough to do by hand).
  Check `Cargo.toml` first; in this repo `regex` is likely already a
  transitive dep but may not be a direct one - prefer no new direct dep.
- Build a centralised secret-scrubbing service. The task description's
  "reuse the same filter" presumes a filter that does not exist; we
  intentionally scope to a module-local helper here. A future task may
  centralise it; that future task absorbs *this* helper.
- Chain failure history beyond the most-recent rung. If a future task
  needs full history, it adds a new field; this PR does not pre-empt.
- Touch `loker-design.md`. The design doc already documents this in §11
  ("Close as decided") and §6.

**Prefer**:
- Constants for byte budgets at the top of `src/strategy/escalating_retry.rs`,
  immediately above the `FailureContext` struct, with a one-line comment
  explaining the choice ("4 KiB excerpt fits inside an 8 KiB envelope while
  leaving headroom for verifier reason and backend error class").
- `insta` snapshots over hand-rolled `assert_eq!` for the envelope tests -
  matches the file's existing convention and keeps reviewer cost low when
  intentional changes happen.
- A free function `build_failure_envelope(ctx: &FailureContext, body:
  &str) -> String` so the assembly logic is unit-testable in isolation
  (the AC7 budget test calls this directly, not through `execute()`).
- A separate module `src/strategy/failure_context.rs` if `escalating_retry.rs`
  grows past ~350 lines (currently 202). Use judgement: split if the file
  starts to bloat, otherwise keep co-located.

**Escalate when**:
- The redaction patterns reject (false-positive) substantial portions of
  legitimate response bodies. Stop and surface for guidance - we may need
  to relax the heuristic-4 rule. Symptom: a normal "the user's name is
  John" produces `[REDACTED]`.
- A rung's response body contains binary / non-UTF-8 bytes that
  `truncate_excerpt`'s char-boundary logic cannot handle gracefully.
  Today `Backend::query` returns `String` so this should not happen, but
  if a backend ever streams non-UTF-8, we must decide whether to
  lossy-decode or skip injection. Stop and ask.
- The 8 KiB / 4 KiB budgets turn out to clash with a real model's input
  limit on integration testing. Bump constants and re-snapshot, but only
  with a written justification - this is a wire-format change.

## 4. Decomposition

Four sub-tasks, ordered by dependency. Each is independently testable.

1. **ST1: Land the value types and helpers (no execute() change yet).**
   In `src/strategy/escalating_retry.rs`:
   - Add `MAX_RESPONSE_EXCERPT_BYTES`, `MAX_FAILURE_CONTEXT_BYTES` constants.
   - Add `FailureContext` struct + the two named constructors.
   - Add `redact_secrets`, `truncate_excerpt`, `build_failure_envelope`
     free functions.
   - Add unit tests at the bottom of the file (`#[cfg(test)] mod tests`)
     covering: redaction patterns, truncation char-boundary safety,
     envelope assembly under-budget and over-budget.
   Does NOT modify `EscalatingRetry::execute()`. Done when `cargo test
   --lib strategy::escalating_retry` is green.
   Files: `src/strategy/escalating_retry.rs`.

2. **ST2: Add the serde-config round-trip test.**
   - If a `*Config` shape exists for `EscalatingRetry`, ensure it has
     `#[serde(default)] pub pass_failure_context: bool`.
   - If no such shape exists yet, define a minimal local
     `EscalatingRetryConfig` next to the struct, derive `Serialize +
     Deserialize`, and write a round-trip test using `toml`.
   Done when the round-trip test is green for both flag values. Independent
   of ST1 (no shared code path).
   Files: `src/strategy/escalating_retry.rs`.

3. **ST3: Wire `pass_failure_context` into `execute()`.**
   - Track `previous_failure: Option<FailureContext>` across the rung loop.
   - At the start of each rung iteration after the first, if the flag is
     on AND `previous_failure.is_some()`, build the envelope and pass it
     to `backend.query()` instead of the bare rendered prompt.
   - When a verify fails or a backend errors, replace `previous_failure`
     with a fresh `FailureContext` built from this attempt.
   - Otherwise leave `previous_failure = None` (so a successful rung does
     not poison a hypothetical later one - though by spec a successful
     rung returns immediately).
   Done when manual eyeball of `escalating_retry.rs:execute()` shows the
   flag has a behavioural effect. Hard depends on ST1.
   Files: `src/strategy/escalating_retry.rs`.

4. **ST4: Integration tests in `tests/strategy_escalating_retry.rs`.**
   - Six new `#[test]` functions covering AC8's six bullets.
   - Use `insta::assert_snapshot!` for envelope-format assertions.
   - Re-use the existing `MockBackend`, `SequenceVerify`, `AlwaysPass`
     test helpers - do not duplicate.
   - Add the rustdoc `# When to enable` block on
     `with_pass_failure_context` (AC10).
   Done when `cargo test --test strategy_escalating_retry` is green and
   `cargo insta review` shows no unintended snapshot changes. Hard depends
   on ST3.
   Files: `tests/strategy_escalating_retry.rs`, `src/strategy/escalating_retry.rs`
   (rustdoc only).

**Dependency order**: ST1 → ST3 → ST4. ST2 is independent and can be done
in parallel with ST1 / ST3 / ST4.

## 5. Evaluation

| # | Test | Expected Result | How to Run |
|---|------|-----------------|------------|
| 1 | `redact_secrets` strips the four secret shapes | Patterns 1-4 each replaced with `[REDACTED]`; bytes around them preserved | `cargo test --lib strategy::escalating_retry::tests::redaction` |
| 2 | `truncate_excerpt` is multibyte-safe | A 5 KiB string with multibyte chars at the boundary truncates to ≤4 KiB without panic | `cargo test --lib strategy::escalating_retry::tests::truncate` |
| 3 | `build_failure_envelope` enforces 8 KiB cap | Input excerpt of 100 KiB + verify reason of 10 KiB → output ≤ 8192 bytes | `cargo test --lib strategy::escalating_retry::tests::envelope_budget` |
| 4 | Flag-off identity (no diff vs. CLO-258) | Rung 2 receives bare rendered prompt; insta snapshot matches CLO-258 baseline | `cargo test --test strategy_escalating_retry pass_failure_context_off_passes_bare_prompt` |
| 5 | Flag-on after verify-fail injects envelope | Rung 2 prompt starts with `<previous-attempt>` block containing the verify reason | `cargo test --test strategy_escalating_retry pass_failure_context_on_after_verify_fail` |
| 6 | Flag-on after backend-error injects envelope | Rung 2 prompt's envelope shows `backend_error: "Timeout"` and `response_excerpt: null` | `cargo test --test strategy_escalating_retry pass_failure_context_on_after_backend_error` |
| 7 | Truncation visible in envelope | 100 KiB rung-1 body → rung-2 envelope ≤ 8 KiB and ends with `…[truncated, N bytes elided]` | `cargo test --test strategy_escalating_retry pass_failure_context_truncates_large_body` |
| 8 | Secrets scrubbed before injection | Rung-1 body with embedded API key → rung-2 envelope contains `[REDACTED]`, not the literal secret | `cargo test --test strategy_escalating_retry pass_failure_context_redacts_secrets` |
| 9 | Three-rung chain references most-recent only | Rung-3 envelope describes rung-2 failure, NOT rung-1 | `cargo test --test strategy_escalating_retry pass_failure_context_three_rung_chain` |
| 10 | serde round-trip preserves flag | TOML with `pass_failure_context = true` round-trips; default emits `false` | `cargo test --lib strategy::escalating_retry::tests::config_round_trip` |
| 11 | Default behaviour unchanged | `pass_failure_context_defaults_false` (existing test) still green | `cargo test --test strategy_escalating_retry pass_failure_context_defaults_false` |
| 12 | Pre-merge gate green | fmt + clippy + all tests | `make check` |

**Edge cases to verify**:
- Empty rung-1 response body with `pass_failure_context = true` →
  envelope contains `response_excerpt: ""` (or `null`, pick one and
  document) and does not panic.
- `verify.verify()` returning `VerifyError` (the hook itself blew up,
  not just `VerifyResult::Fail`) → envelope's `verify_reason` carries
  the error message string, redacted.
- Multibyte char on the 4 KiB truncation boundary (e.g. emoji at byte
  4094-4097) → truncation finds the previous char boundary, no panic.
- Three-rung ladder where rung-1 errors, rung-2 verify-fails, rung-3
  succeeds → rung-3's envelope describes rung-2 only (verify-fail), not
  rung-1's backend error. Most-recent-only contract enforced.
- Flag is `true` but the ladder is single-rung → no envelope is ever
  built (no "previous" exists). Walker behaves identically to flag-off.
- Concurrent / re-entrant calls to `execute()` on the same
  `EscalatingRetry` instance with the flag on → no shared state leaks
  between calls (the `previous_failure` lives on the stack of one
  `execute()` call, never on `self`).
