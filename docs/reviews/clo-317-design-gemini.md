# Design Review: CLO-317

**Reviewer**: Gemini 3.1 Pro
**Reviewed**: 2026-05-04
**Pipeline**: lok design-review

---

## 1. Completeness Check

- All required sections are present and in order:
  1. Problem
  2. Goals / Non-goals
  3. Architecture
  4. Public API surface
  5. Test plan
  6. Migration / rollout
  7. Open questions
- API signatures and data flow are concrete enough for implementation.

## 2. Architecture Assessment

**Strengths**: Solid separation between phase orchestration and HITL policy. The doc narrows scope to `PhaseRunner` + new hook implementation and keeps existing hooks unchanged. It correctly models resume/retry behavior via pending-response flow and emphasizes idempotent pending creation + response consumption.

**Concerns**: Need explicit implementation policy on replay prevention for valid response files across retries, and explicit handling when response JSON is malformed (it must stay pending, not fail hard).

## 3. Alignment with Handoff & Roadmap

The design fits the active M1 process expectations and FR-17 in the roadmap: additive changes only, no premature lock/daemon work, and explicit mention that severity ladder/locking follow-ons are deferred to T-049/T-050/T-051. It is operationally consistent with existing `PhaseRunner`/resume architecture.

## 4. Security Review

No new subprocess or network execution is introduced. The hook consumes local JSON files only, so risks are malformed/poisoned files and replayability. The design's planned schema validation and strict phase/decision checks are appropriate for preventing pathologically invalid untrusted input.

## 5. Implementation Concerns

- High priority: consumed responses can otherwise cause stale decisions to re-apply across escalations/retries.
- High priority: malformed response files should not convert to terminal backend verification failures; they should keep the phase in a human-pending state.
- Medium priority: decide and document whether `comment_only` is pending-or-pass; task should prefer a deterministic contract to avoid silent bypass.

## 6. Concurrency & Async

Design is async-friendly and synchronous I/O is bounded to filesystem interactions already exercised through existing run-directory patterns. No blocking watch loops or daemon background tasks are introduced.

## 7. Blind Spots

- The exact mapping strategy for `comment_only` should be pinned in code comments/tests.
- Need to confirm whether run_id/workflow names in pending payload are derived from `run_dir` consistently (schema expects a workflow-like identifier).
- Auditability choice for consumed responses (rename vs delete) should be codified.

## 8. Verdict

APPROVE_WITH_SUGGESTIONS

## 9. Actionable Feedback

1. Add response consumption (rename/delete after parse) to prevent retry replay.
2. Keep malformed/mismatched response files in the pending state.
3. Define `comment_only` deterministically (recommend pending gate, not auto-pass).
4. Add explicit tests for all three cases above.