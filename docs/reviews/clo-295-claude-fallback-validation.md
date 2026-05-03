# Pre-PR validation: clo-295

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-03
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [critical] CLI `resume` subcommand is a stub — never invokes ResumeRunner
**Where:** `src/main.rs:864-898` (the `Commands::Resume` arm)
**What:** The CLI command acquires the lock, sweeps tmp, loads `RunState`, prints status, and exits. It never builds a `ResumePlan` and never calls `ResumeRunner::execute()`. Comment in code says "Phase configs would be derived from workflow here. For now, print status and exit." This is the user-facing entry point of CLO-295 and it does not resume anything. Open question #2 in the design contemplated a fallback (`--phases` from a workflow config) but neither path is wired.
**Suggested fix:** Either (a) wire `Workflow::to_phase_configs()` and call `ResumePlanner::plan()` + `ResumeRunner::execute()` end-to-end, or (b) explicitly scope CLO-295 to "library-only" in plan/PRD and gate the CLI behind a hidden flag with a clear "not yet executable" message — and update Linear to split runner-wiring into a follow-up. Don't ship a CLI that silently does nothing.

### F2 [critical] Integration tests don't exercise ResumeRunner — only the planner
**Where:** `tests/resume.rs:113-263`
**What:** Design §5.2 Test 1 asserts "Resume re-runs phase 2 in `attempts/phase2/2/`, phase 3 runs, manifest has exactly 3 entries". The actual test only asserts the per-phase `PhaseAction` enum values from the planner. `ResumeRunner` is imported (line 8) but never invoked; `fake_backends()` and `write_failed_marker()` are dead-code warnings (confirmed by compiler). Tests 1, 2, 5 silently skip the runner; Test 4 only checks that `RunState::load()` classifies a heartbeat as Live. The TDD contract from the design isn't met.
**Suggested fix:** Add a `mock` Backend that returns deterministic strings; build a 3-phase `Vec<PhaseConfig>` with the mock; call `ResumeRunner::new(...).execute(&plan)`; assert manifest entry count, `attempts/phase2/<n>/` contents, and that the design's "no-op when all complete" exit is reached. Or, again, descope the runner integration to a follow-up issue and remove the now-unused imports.

### F3 [high] `archive_current_attempt` labeled with the *next* attempt counter, not the current one
**Where:** `src/resume.rs:170-175` (caller) + `src/resume.rs:45-75` (callee)
**What:** Planner sets `PhaseAction::Resume { attempt: next }` where `next = next_attempt(...)` (i.e. `current+1`). `ResumeRunner::execute` passes that value straight to `archive_current_attempt(run_dir, phase, *attempt)`, archiving partial output of attempt N as `attempts/<phase>/<N+1>/`. Design §3.5 explicitly says "`attempt` is the *current* attempt number". This collides with the new attempt directory and mislabels postmortem debris. In practice it's masked because partial work usually lives in `attempts/<phase>/<n>/` rather than the canonical `<phase>/` (so the function early-returns), but the bug surfaces the moment a completed phase is being re-attempted (e.g. corrupted-after-completion edge case).
**Suggested fix:** Either rename `PhaseAction::Resume.attempt` to `next_attempt` and pass `next_attempt - 1` to `archive_current_attempt`, or compute the current attempt independently inside `ResumeRunner::execute` (read the highest existing `<phase>.started.<n>` marker). Add a regression test that creates a canonical `<phase>/` directory + `.started.1` marker and verifies the archived path is `attempts/<phase>/1/`.

### F4 [high] `ResumeRunner::run_phase` ignores attempt counter, builds an empty Prompt and uses no upstream manifest
**Where:** `src/resume.rs:189-209`
**What:** `_attempt: u32` is unused; `Prompt::new()` produces an empty prompt; `verify: None`; `trace: None`; backends come from a `Vec<Arc<dyn Backend>>` constructor with no resolution against `cfg.backend`/`cfg.targets`. Even if F1/F2 were addressed, the runner cannot reproduce a real phase invocation. This is dead orchestration glue.
**Suggested fix:** Either reuse the existing `WorkflowRunner` plumbing (preferred — DRY) or document explicitly that ResumeRunner is a stub for now. The current half-implementation is the worst of both worlds: it compiles and looks complete, but a kill-then-resume would silently produce empty artefacts.

### F5 [medium] Cross-filesystem fallback specified in design §3.5 is missing
**Where:** `src/resume.rs:67`
**What:** Design §3.5: "If the run directory and attempts directory are on different mount points, the operation falls back to `fs_extra::dir::move_dir`". Implementation only does `std::fs::rename` and surfaces `EXDEV` as a hard error. Low risk in practice (run_dir + attempts share parent) but the design contract isn't met and the failure mode is silent.
**Suggested fix:** Either drop the fallback from the design (preferable — `fs_extra` is heavy for one edge case) or implement it. Either way, align design and code.

### F6 [low] Sweep TTL boundary off-by-one vs. `is_stale`
**Where:** `src/resume/sweep.rs:46`
**What:** `if mtime <= cutoff` sweeps files at exactly TTL age, while `heartbeat::is_stale` (`heartbeat.rs:212`) treats exactly-TTL as *not* stale (`tick_at < cutoff`). Two staleness predicates with different boundary semantics is a recipe for confusion in tests and logs.
**Suggested fix:** Use `<` consistently in `sweep_stale_tmp` to match `is_stale`, or extract a shared `is_older_than(ttl_seconds)` helper.

### F7 [low] `RunLock` field marked `#[allow(dead_code)]` masks intent
**Where:** `src/resume/lock.rs:14-16`
**What:** The `file: File` field is the lock holder — dropping it releases the OS lock. `#[allow(dead_code)]` works around the warning but obscures that the field is load-bearing. A future refactor could remove it and silently break the lock.
**Suggested fix:** Remove the allow and add a one-line `// dropped on Drop, releasing the OS lock` comment, or implement an explicit `Drop` (even if a no-op) so a reader sees the lifetime contract.

### F8 [low] `clippy::result_large_err` blanket-allowed instead of fixed
**Where:** `src/resume.rs:1`
**What:** Pre-merge cleanup commit added `#![allow(clippy::result_large_err)]` to silence clippy. `ResumeError` contains `LoadError` (an enum with many heap-y variants) by value — fix by `Box`-ing the heaviest variants rather than disabling the lint.
**Suggested fix:** `Load(Box<crate::run_state::LoadError>)` + `Phase(Box<...>)` and drop the file-level allow.

## Verdict
rework

The branch ships a polished planner with green unit tests, but the user-visible contract — `loker resume <run-dir>` actually resumes a partially-completed run — is not met. The CLI subcommand is a status printer (F1), the integration tests covering Design §5.2's "Resume re-runs phase 2 / manifest has 3 entries / phase 3 runs" assert only the planner's enum output (F2), and `ResumeRunner::run_phase` plus the attempt-numbering in `archive_current_attempt` (F3, F4) cannot drive a real phase. Either wire the runner end-to-end with a mock-backend integration test, or explicitly descope CLO-295 to "planner + library scaffolding" in PRD/Linear and split runner-wiring into a follow-up issue before merging. Don't merge in the current state — it would hand a non-functional `resume` command to users while looking like it works.
