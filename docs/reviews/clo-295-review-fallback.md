# Design Review: clo-295

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-03
**Pipeline**: lok design-review
**Note**: External reviewers failed; this is the fallback review produced by the agent harness.

---

## 1. Completeness Check

| Section | Status | Notes |
|---------|--------|-------|
| Problem | ✓ Present | Clear 1-paragraph framing with D3 protocol citation. |
| Goals / Non-goals | ✓ Present | 9 goals, 4 non-goals. Well scoped. |
| Architecture | ✓ Present | Module layout, data flow diagram, concrete types, lock/sweep submodules. |
| Public API surface | ✓ Present | CLI addition + note that ResumeRunner is crate-internal. |
| Test plan | ✓ Present | Unit + integration (5 TDD scenarios) + manual. |
| Migration / rollout | ✓ Present | No migration needed; pre-merge gate noted. |
| Open questions | ✓ Present | 4 questions with resolutions. |

All 7 required sections are present and substantive.

## 2. Architecture Assessment

**Strengths:**
- Clean separation of concerns: `ResumePlanner` (pure logic) vs `ResumeRunner` (side effects). Mirrors the D3 protocol's read/write split.
- `PhaseAction` enum is minimal and covers all three states (Skip/Resume/RunFresh) without ambiguity.
- Lock + sweep are isolated into private submodules — good boundary discipline.
- Explicit decision to keep `ResumeRunner` crate-internal (not in `lib.rs`) is correct; this is a binary concern.

**Concerns:**
- `ResumeRunner::execute()` takes `workflow: &Workflow` but the plan is built from `Vec<PhaseConfig>`. The design admits in Open Question #2 that `Workflow` and `PhaseRunner` are not yet unified. This adapter gap is the biggest architectural risk. If `PhaseConfig` cannot be derived from a `Workflow` today, the `resume` subcommand cannot operate on workflow files — only on pre-constructed phase lists. **Recommendation:** Either (a) document that `resume` is for programmatic phase lists only in v0, or (b) add a lightweight `Workflow::to_phase_configs()` adapter in the same PR.
- `ResumeError` duplicates `ArtefactCorrupt` and `ArtefactMissing` that already exist in `crate::run_state::LoadError`. The design says `Load(#[from] LoadError)` is a variant, so these should surface through that path, not as first-class variants. The PRD has them as first-class variants but the design's `RunState::load()` path would return `LoadError` directly. **Minor inconsistency.**

## 3. Alignment with Handoff & Roadmap

- Fits M1 (TensorZero backend) scope: no backend changes, pure orchestration on top of existing primitives.
- Aligns with `docs/handoff.md` TDD-first mandate: the 5-scenario integration test contract is concrete and testable.
- Does not violate "New primitives land as new modules" — `resume.rs` is a new module, existing `WorkflowRunner` untouched.
- Pre-merge gate `make check` is explicitly mentioned.

## 4. Security Review

- Advisory lock prevents concurrent writer corruption — good.
- No network calls in resume logic; all IO is local filesystem.
- No secrets or env vars introduced.
- Stale tmp sweep moves (not deletes) files — safe for recovery.
- **Minor:** `RunLock` uses `flock` which is advisory-only (other processes can ignore it). The design correctly calls it "advisory lock" but the error variant `LockInUse` implies stronger semantics than the OS provides. Document that this only protects against cooperative loker instances.

## 5. Implementation Concerns

- **Heartbeat TTL coupling:** The design uses `DEFAULT_TTL` but does not say where it comes from. It should match the TTL used by `HeartbeatWriter` when the run was originally created. If the user overrides TTL at `run` time, `resume` must use the same value. The heartbeat file does not currently store the TTL. **Fix:** Either (a) add `ttl_seconds` to `heartbeat.json` schema, or (b) require the user to pass `--ttl` to `resume` and default to 300s.
- **Attempt archive on Resume:** The design says "archive current attempt" but does not describe the concrete operation. `AttemptDir::promote_to_canonical()` is for promoting, not archiving. We need either a new `AttemptDir::archive()` method or clarification that archiving is a rename of the canonical phase directory into `attempts/<phase>/<n>/`.
- **Manifest upstream artefact passing:** The design says "passes upstream artefacts through manifest entries" but `PhaseRunner::run()` does not currently accept upstream manifest entries. We need to verify whether `PhaseRunner` loads its own inputs or receives them. If the latter, the signature may need a new parameter.

## 6. Concurrency & Async

- `sweep_stale_tmp` is correctly kept synchronous.
- `ResumeRunner::execute()` is async (matches existing `PhaseRunner::run()` and `WorkflowRunner::run()`).
- Lock is held across the entire async execute — this is fine because the lock is a file descriptor and `flock` is not async-sensitive.
- Cancellation safety: if the process is killed while `execute()` is in progress, the partially-written attempt directory and `.started` marker will be left behind. On next resume, `RunState::load()` will see the stale heartbeat and archive the attempt. **This is correct per D3.**

## 7. Blind Spots

- **Phase ordering:** The design assumes workflow phases are a linear `Vec<PhaseConfig>`. If workflows ever have branching or conditional phases, the resume plan becomes ambiguous. Not a v0 concern but worth a one-line note.
- **Disk-full during sweep:** If `attempts/_orphan_tmp/` is on a full disk, the sweep rename fails. The design does not say whether this is a hard error or logged warning. Recommend: hard error (resume should not proceed if we cannot clean up stale tmp).
- **Marker race on fast restart:** If a process is killed and restarted faster than the TTL, the heartbeat is still live and resume is rejected. This is correct behavior, but the user experience could be improved with a `--force` flag (post-v0).
- **No mention of `trace.jsonl`:** The design does not say whether resumed phases append to the existing `trace.jsonl` or create a new one. The PRD says "modulo timestamps in trace.jsonl" for idempotency, implying trace is not expected to be identical. But we should document whether trace entries from resumed phases are appended or if a new trace file is started.

## 8. Verdict

**APPROVE_WITH_SUGGESTIONS**

The architecture is sound and the TDD contract is comprehensive. The design is ready for implementation once the following items are addressed (all are minor):

1. Resolve `ResumeError` duplication with `LoadError` (remove redundant variants or document why they are separate).
2. Clarify how `ResumeRunner` obtains `PhaseConfig` from `Workflow` — add adapter or document v0 limitation.
3. Specify how the original heartbeat TTL is recovered (add to heartbeat schema or require CLI flag).
4. Define the concrete "archive current attempt" operation in `AttemptDir` terms.
5. Verify `PhaseRunner::run()` can receive upstream manifest entries (or document the gap).

## 9. Actionable Feedback (Prioritized)

| Priority | Item | Location | Action |
|----------|------|----------|--------|
| P1 | Heartbeat TTL recovery | `resume.rs` execution flow step 2 | Add `ttl_seconds` to `heartbeat.json` or CLI flag. |
| P1 | PhaseConfig from Workflow | Open Question #2 | Add `Workflow::to_phase_configs()` adapter. |
| P2 | Archive operation definition | Architecture §3.3 | Clarify `AttemptDir::archive()` or equivalent. |
| P2 | ResumeError variant cleanup | Public API surface | Remove `ArtefactCorrupt`/`ArtefactMissing` from `ResumeError` (use `LoadError` path). |
| P3 | trace.jsonl behaviour | Test plan / Open questions | Document append vs new file for resumed phases. |
| P3 | Disk-full sweep failure | Blind spots | Define as hard error. |
