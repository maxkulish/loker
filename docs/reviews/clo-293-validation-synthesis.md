# Pre-PR validation: clo-293

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-03
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc quoting error (unmatched `'` from backtick-wrapped `git diff main...HEAD` inside single-quoted heredoc) — never reached the model |
| Gemini | REVIEW_FAILED | Same shell heredoc quoting error in invocation script |
| Claude (fallback) | OK | Produced 9 findings (F1-F9), verdict approve_with_changes |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F2 — `loker.outcome` enum drift (schema rejects real failure paths).** `PhaseError::error_class()` returns `manifest_failed | marker_failed | io_failed | phase_failed`, none of which are in `docs/schemas/trace_span.schema.json:111-114`. With `additionalProperties: false`, any phase failing on manifest/marker/IO writes a span that fails strict validation. Fix: extend the schema enum to include those four values (cleaner — labels are useful diagnostic signal). Bounded edit to one schema file.
- **F1 — InMemorySink emits a different span shape than TraceWriter.** `InMemorySink` skips `timestamp` and `loker.run_id`, so it can't pass `schema_validation`. Trait makes no shape guarantee. Fix: route both sinks through `build_span_skeleton`, then add an `InMemorySink` round-trip to the schema test to lock parity in. Also retires ~150 lines of duplicated map-building (subsumes F8).
- **F5 — `duration_ms` plumbed but never emitted.** Both sinks ignore `BackendSpanResult.duration_ms` / `VerifySpanResult.duration_ms` despite the schema documenting the field and verify code measuring it. Latency is the primary reason traces exist. Fix: emit `duration_ms` in both sinks for backend + verify spans; add a phase-level `Instant` and pass duration through to `phase_finished`/`error`.
- **F6 — Backend attempt durations always 0.** `phase_runner.rs:266-276` hard-codes `duration_ms: 0`. After F5 the field will be present but useless. Fix: add `duration: Duration` to `Attempt`, populate inside the strategy, surface as `attempt.duration.as_millis() as u64`.

These four are bounded: one schema enum extension, one helper extraction across two sink files, two field-emission additions, one struct field threaded through the strategy layer. All within one fix iteration; no design change needed.

## Out of Scope / Deferred
- **F3 — `fsync` flag only flushes the BufWriter.** Misleading name and rustdoc, but test-only path; correctness of on-disk format is unaffected for the merge. Fix in a follow-up: either call `sync_all()` or rename to `flush_each_line` and update doc.
- **F4 — Backend-attempt errors not surfaced in `error.message`.** Real observability gap, but consumers can still read `gen_ai.response.finish_reasons`. Defer to a follow-up that may need an `Attempt.last_error` plumbing decision.
- **F7 — `verify_hook` casing inconsistency (snake_case on phase span, PascalCase on verify span).** Cosmetic/joinability concern; pick one canonical form in a follow-up.
- **F8 — Duplicated map-building between sinks.** Naturally absorbed into F1's fix; no separate action needed.
- **F9 — `phase_finished` called twice on failure paths.** Worth resolving alongside F2 if convenient, otherwise defer; design doc should specify which event is the terminal record.

## False Positives / Tooling Artifacts
- Codex and Gemini reviewer scripts both failed with `unexpected EOF while looking for matching '` because the heredoc body contains `\`git diff main...HEAD\`` inside a single-quoted `<<EOF` and the surrounding `$(cat <<EOF ... EOF)` is itself inside single quotes passed to `sh -c`. Tooling artifact, not a code issue. Recommend fixing the wrapper scripts (escape the backticks or switch to double-quoted heredoc with explicit escapes).

## Recommendation
PROCEED_WITH_FIXES. Land one fix iteration covering: (1) extend `loker.outcome` enum in `docs/schemas/trace_span.schema.json` to include `manifest_failed | marker_failed | io_failed | phase_failed`; (2) refactor `InMemorySink` onto `build_span_skeleton` and add an `InMemorySink` arm to `tests/trace_jsonl.rs::schema_validation`; (3) emit `duration_ms` in both sinks for backend + verify spans, plus phase-level duration; (4) add `duration: Duration` to `Attempt` and populate from the strategy so backend spans report real timings. F3, F4, F7, F9 can ship as follow-ups. Separately, fix the Codex/Gemini reviewer wrapper scripts so future syntheses aren't single-source.
