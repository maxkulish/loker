# Design Review Synthesis: CLO-327

**Reviewer**: Gemini 3.1 Pro + Human (synthesis)
**Reviewed**: 2026-05-10
**Design Doc**: `docs/designs/clo-327-wire-phase-based-runner.md`

---

## Reviewer Status

| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini 3.1 Pro | OK | Full structured review produced (5648 bytes) |
| Claude (fallback) | TIMEOUT | 300s timeout exceeded |

## Key Findings

| # | Finding | Severity |
|---|---------|----------|
| 1 | **Reuse existing TemplateEngine**: Design proposed building a new `TemplateEngine`, but `src/workflow/template.rs` (CLO-289) already handles all required substitutions | High |
| 2 | **`--resume` scope contradiction**: PRD scope item 7 says "Honour `--resume`", design defers it as non-goal | High |
| 3 | **API signature mismatch**: `PhaseWorkflowRunner` uses `Arc<Config>`, `run_phase_workflow` uses `&Config` | Low |
| 4 | **Backend resolution timing**: Design claimed "same as step-based runner" but step-based resolves at runtime | Low |
| 5 | **`--rerun` interaction unclear**: Not clarified how `--rerun phase=<name>` works when resume is deferred | Medium |

## Verdict

APPROVE_WITH_SUGGESTIONS

## Applied Changes

All 5 findings were addressed in the design document:

1. ✅ **TemplateEngine**: Removed new `TemplateEngine` proposal. Design now reuses `crate::workflow::template` (CLO-289)
2. ✅ **`--resume` scope**: Updated Open Question #1 to explain that `loker run` (fresh) and `loker resume` (stateful) are different entry points. Deferral documented with a note to update PRD scope item 7
3. ✅ **API signatures**: Unified on `Arc<config::Config>` for both `PhaseWorkflowRunner` and `run_phase_workflow`
4. ✅ **Backend timing**: Fixed description — now correctly notes this differs from step-based runner
5. ✅ **`--rerun`**: Added Open Question #6 clarifying that `--rerun` is a no-op until resume is wired

## Flagged Items

None — all suggestions were additive or refinement, none contradicted the chosen approach.

## Priority Actions

1. Update PRD scope item 7 to match the design decision on `--resume` deferral
2. Proceed to Plan phase
