# Design Review: clo-286

**Reviewer**: Gemini 3.1 Pro (fallback: manual review by pi)
**Reviewed**: 2026-05-03
**Pipeline**: lok design-review (external reviewers failed; manual fallback)
**Note**: Gemini CLI failed due to trust directory restrictions; Ollama failed due to missing model. This review was produced by the primary agent (pi) acting as reviewer.

---

## 1. Completeness Check

| Section | Status |
|---------|--------|
| Problem statement | Present (Section 1) |
| Goals / Non-goals | Present (Section 2) but merged with scope |
| Architecture | Present (Section 3) |
| Public API surface | Present (Section 4) |
| Test plan | Present (Section 5) |
| Migration / rollout | **MISSING** — no explicit migration section |
| Open questions | **MISSING** — no explicit open-questions section |

## 2. Architecture Assessment

**Strengths**:
- Clean separation: `AttemptDir` for directory management, `LatestPointer` for convenience links, `markers.rs` enhanced for dual-source attempt counting.
- D3 Approach A reconciliation is well-reasoned and documented.
- `promote_to_canonical` handles cross-device fallback gracefully.
- Symlink/json dual-mode for `LatestPointer` accounts for Windows portability.

**Concerns**:
- `AttemptDir::promote_to_canonical` renames the *entire* attempt directory to the canonical phase directory, but the design note later claims "individual files are renamed." These are contradictory. Directory rename is simpler and aligns with D3; per-file rename adds complexity without clear benefit for a single-artefact phase.
- `next_attempt_from_dirs` has an **off-by-one bug**: it stores `n + 1` in `max_attempt`, but `next_attempt_from_markers` returns `max_attempt.map_or(0, |m| m + 1)` where `max_attempt` is the raw attempt number. The dir scanner should store `n` (raw), and the caller should apply `+1` uniformly.
- No mention of `run_state/mod.rs` changes needed to export `AttemptDir` and `LatestPointer`.
- `AttemptRetention` is added to `src/config.rs` but `Config` currently has no `run_state` field. Need a new `RunStateConfig` sub-struct.

## 3. Alignment with Handoff & Roadmap

Design aligns with D3 (`docs/run-state.md`) and T-027 roadmap description. The explicit rejection of Approach B is correct per ADR. Non-goals section properly defers cleanup and HITL to later milestones.

## 4. Security Review

- No hardcoded secrets.
- `atomic_write` primitive already fsyncs before rename.
- Symlink creation on Unix is unprivileged; falls back safely on failure.
- No user-controlled path components are joined without sanitization (`phase` is validated by marker system).

## 5. Implementation Concerns

- `AttemptDir::copy_tree` is recursive but lacks symlink handling and permission preservation. For the scope of this task (all files in `attempts/` are regular files written by `atomic_write`), this is acceptable but should be documented.
- The `chrono` dependency is already present, so `LatestPointer` timestamp generation is fine.
- `serde_json` is already a dependency.
- Test contract (8 tests) is concrete and comprehensive.

## 6. Concurrency & Async

- `AttemptDir` operations are all synchronous filesystem calls. Since they run during the phase execution critical path, blocking calls on local filesystem are acceptable (microseconds).
- No async code needed in this subsystem.

## 7. Blind Spots

- **Missing Migration / Rollout section**: How do existing run directories without `attempts/` interact with the new code?
- **Missing Open Questions section**: E.g. "Should `latest` point to the latest completed or latest started attempt?"
- **Config wiring**: `Config` struct needs a new `run_state: RunStateConfig` field; this is non-trivial because `Config` uses `serde(deny_unknown_fields)`.
- **Orphan sweep interaction**: `load.rs` orphan sweep may delete marker files but leave attempt dirs. Should orphan sweep also clean attempt dirs? Deferred per non-goals, but worth noting.

## 8. Verdict

**APPROVE_WITH_SUGGESTIONS**

## 9. Actionable Feedback

1. **[CRITICAL]** Fix `next_attempt_from_dirs` off-by-one: store raw directory name `n` in `max_attempt`, not `n + 1`. The `+1` logic should be applied uniformly by the caller.
2. **[MEDIUM]** Clarify `promote_to_canonical`: document that directory rename is the chosen mechanism and remove the contradictory per-file-rename design note.
3. **[MEDIUM]** Add `run_state/mod.rs` changes to the design: export `AttemptDir` and `LatestPointer`, add `pub mod attempt_dir; pub mod latest;`.
4. **[MEDIUM]** Add `Config` wiring: introduce `RunStateConfig { keep_attempts: AttemptRetention }` and add `#[serde(default)] pub run_state: RunStateConfig` to `Config`.
5. **[LOW]** Add Migration / Rollout section: backward-compatible with existing runs (no attempt dirs), `next_attempt` degrades gracefully to marker-only.
6. **[LOW]** Add Open Questions section.
