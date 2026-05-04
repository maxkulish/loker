# PRD: CLO-317 — HumanVerifier verify hook scaffold

| Field | Value |
|-------|-------|
| Author | pi (discovery phase) |
| Status | Draft |
| Created | 2026-05-04 |
| Task | CLO-317 |
| Depends on | CLO-020, CLO-244 |

## 1. Goal

Introduce a `HumanVerifier` verify hook that pauses workflow execution when a phase needs explicit human judgment. The hook must surface machine-readable pending state and wait for a human-authored response file so operators can review and unblock later.

## 2. Scope

### In scope
- Add a `VerifyHookName::HumanVerifier` dispatch path in the phase runner.
- Implement `HumanVerifier` as a `VerifyHook`:
  - On invocation, write `runs/<run_id>/pending/<phase>.json` using `docs/schemas/pending.schema.json`.
  - Return a non-pass verify result that causes the phase to halt until resolution.
- On resume/poll, read `runs/<run_id>/responses/<phase>.json` and map `decision` + comments to `VerifyResult`.
- Support `approve`, `reject`, and optional `comment_only` outcomes in the pending context.
- Add unit/integration tests covering:
  - marker creation format
  - response mapping
  - blocking behavior on missing response file
  - non-regression of existing `RunCommand`, `LLMVerifier`, `TestRunner` hooks

### Out of scope (deferred)
- Severity escalation ladder and timer-driven auto-fail (`T-049`).
- First-write-wins locking and concurrent response races (`T-050`, `T-051`).
- Per-gate fallback axum one-shot endpoint (`T-051`).

## 3. Acceptance criteria

1. `VerifyHookName` and phase runner dispatch can select the HumanVerifier variant.
2. HumanVerifier writes a valid pending marker conforming to `pending.schema.json`.
3. Phase execution halts (non-pass verify result) while pending response file is absent.
4. Presence of `runs/<run_id>/responses/<phase>.json` unblocks the phase deterministically.
5. Response `approve`/`reject` + optional inline comments map into `VerifyResult` as expected.
6. Existing verify hooks (`RunCommand`, `LLMVerifier`, `TestRunner`) remain behaviorally unchanged.
7. `make check` is green.

## 4. Design direction

### 4.1 API shape

- Add new verify hook variant under `src/strategy/verify/` (or shared `human_verifier.rs`).
- Extend `src/phase_runner.rs` and `src/phase_runner/dispatch.rs` to accept/configure the new hook by name.
- Reuse `VerifyContext` for context-aware mapping (including phase and model metadata).

### 4.2 Pending/response contract

- **Pending:** `runs/<run_id>/pending/<phase>.json`
  - Fields required by `pending.schema.json`.
  - Includes severity, opened/timeout timestamps, artefact path, and decision options.
- **Response:** `runs/<run_id>/responses/<phase>.json`
  - `response.schema.json`
  - `decision` drives verify outcome path.
  - `global_comment` and `inline_comments_path` (if present) are passed to downstream result mapping (or recorded in verify summary metadata).

### 4.3 Resume behavior (observability-first)

- `HumanVerifier` may return a synthetic verify result that marks the run as blocked/pending while keeping phase context intact.
- Phase runner should preserve pending marker and return a recoverable terminal phase error class distinguishable from hard backend failures.
- On subsequent runs, same phase attempts resume by re-entering verify stage and checking response file first.

### 4.4 Risks

- Blocking wait loops can stall long-running runs without explicit cancellation/timeout semantics.
- Response races and lock contention are intentionally out of scope for this task and should be documented as follow-up debt.
- Missing timeout policy for non-high severity may make stale pending states ambiguous; should be deferred to `T-049`.
