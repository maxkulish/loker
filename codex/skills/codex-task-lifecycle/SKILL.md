---
name: codex-task-lifecycle
description: Run a Codex-native task lifecycle from task intake through design, design review, implementation, pre-PR review, PR creation, PR fixes, and done. Use when the user asks Codex to orchestrate or resume a tracked engineering task end to end, creating the same docs/ artifacts as the legacy workflow.
metadata:
  short-description: Codex task-to-PR lifecycle
---

# Codex Task Lifecycle

Use this skill to run the repository's task lifecycle in Codex. Preserve only
this phase order:

`task -> design -> design review -> implementation -> review before PR -> PR -> pr fixes -> done`

Do not dispatch through repo command files for other assistants. Use Codex's
normal tools: read files, edit with `apply_patch`, run checks, use GitHub
capabilities when available, and keep state in repo files.

## State

Persist progress in `docs/status/<task-id>-workflow.yaml`.

Use lowercase file IDs, for example `clo-247-workflow.yaml`. If a state file
exists, resume from its `workflow.current_phase` unless the user asks to
restart. If an older Codex-specific state file exists at
`docs/status/<task-id>-codex-workflow.yaml`, migrate its content into
`docs/status/<task-id>-workflow.yaml` and continue with the shared docs/status
path.

Initial state template: `assets/workflow-state.yaml`.

Compatibility is mandatory. The YAML must use the same root shape as the
legacy workflow state:

- `task_id`, `linear_url`, `title`, `branch`, `task_type`, `created`
- `workflow.current_phase`
- `workflow.status`
- `phases.design`
- `phases.plan`
- `phases.implement`
- `phases.pr`
- `phases.complete`
- `history`

Do not create an incompatible top-level Codex-only shape such as
`design_review`, `implementation`, `pre_pr_review`, `pr_fixes`, or `done` at
the root. Codex-specific details may be added inside the existing `phases.*`
objects, using fields prefixed with `codex_` when there is no legacy field.

When resuming:

| Existing `workflow.current_phase` | Codex resumes at |
|---|---|
| missing file | task |
| `design` | design or design review, based on `phases.design.*` fields |
| `plan` | implementation plan creation/review |
| `implement` | implementation or review before PR |
| `pr` | PR creation or PR fixes |
| `complete` | done/finalization |

Codex may describe the user-facing flow as
`task -> design -> design review -> implementation -> review before PR -> PR -> pr fixes -> done`,
but it must persist state using the compatible phase names above so Claude can
continue later.

Never persist `workflow.current_phase` as `task`, `design_review`,
`pre_pr_review`, `pr_fixes`, or `done`. Use `design`, `plan`, `implement`,
`pr`, or `complete`.

Update state after every meaningful action:

- `workflow.current_phase`
- `workflow.status`
- current phase fields
- append a timestamped `history` entry

Statuses are `pending`, `in_progress`, `checkpoint`, `complete`, and `blocked`.
If blocked, record the blocker and stop with the exact next action needed.

## Docs Artifact Contract

Create the same `docs/` artifact classes as the legacy lifecycle, but keep the
content Codex-specific:

| Artifact | Path | Required |
|---|---|---|
| Workflow YAML state | `docs/status/<task-id>-workflow.yaml` | Always, created in `task` before other phase work |
| Design document | `docs/design-docs/<task-id>-<slug>.md` | Always for development work |
| Design review | `docs/reviews/<task-id>-codex-design-review.md` | Always after design |
| Implementation plan | `docs/plans/<task-id>-<slug>.md` | Always before code edits |
| Pre-PR review | `docs/reviews/<task-id>-codex-pre-pr-review.md` | Always before PR |

Create missing directories before writing artifacts:
`docs/status`, `docs/design-docs`, `docs/reviews`, and `docs/plans`.

Keep all generated artifact paths in the YAML state. PR bodies should link the
design document, implementation plan, workflow YAML, and pre-PR review.

## Phase 1: Task

Goal: establish scope, inputs, branch, and working state.

1. Extract the task ID from the user, branch, PR, issue, or local docs.
2. Read project context first: `AGENTS.md`, `README.md`, `docs/handoff.md`,
   relevant specs/plans, and the current branch diff.
3. Gather task details from local docs and available connectors. If the tracker
   is unavailable, continue from user-provided context and record that gap.
4. Create or resume `docs/status/<task-id>-workflow.yaml` from
   `assets/workflow-state.yaml`.
5. Ensure a task branch exists or record the current branch if the user wants to
   stay on it.
6. Move to `design` only when the problem statement, non-goals, acceptance
   criteria, and validation command are clear enough to implement.

Checkpoint fields:

- `task.status: complete`
- `task.problem`
- `task.acceptance_criteria`
- `task.validation`

## Phase 2: Design

Goal: create a focused implementation design that Codex can execute.

Create or update `docs/design-docs/<task-id>-<slug>.md`. Keep it practical:

- Summary
- Context
- Non-goals
- Affected files/modules
- Proposed approach
- Data/API/CLI/config changes
- Error handling and rollback
- Security and compatibility notes
- Acceptance criteria
- Validation plan

Prefer existing architecture and local patterns. If the task is small, the
design can be short, but it still needs enough detail to constrain
implementation.

Checkpoint fields:

- `design.status: complete`
- `design.design_doc`
- `design.draft_ready: true`

## Phase 3: Design Review

Goal: review the design before code changes.

Perform a Codex review of the design against project context and likely code
touchpoints. Write the review to
`docs/reviews/<task-id>-codex-design-review.md`.

Review for:

- missing acceptance criteria
- incorrect architecture assumptions
- unhandled failures and edge cases
- security or compatibility regressions
- test gaps
- scope creep

If findings require design changes, apply them to the design doc, record what
changed, and re-check the affected sections. Ask the user only for decisions
that cannot be inferred safely.

Checkpoint fields:

- `phases.design.review_completed: true`
- `phases.design.review_verdict: approve | approve_with_changes | revise`
- `phases.design.codex_review: docs/reviews/<task-id>-codex-design-review.md`
- `phases.design.review_applied: true | false`
- `phases.design.applied_suggestions`

## Phase 4: Implementation

Goal: implement the approved design.

1. Read the design, status file, relevant source files, and tests.
2. Create or update `docs/plans/<task-id>-<slug>.md` before code edits. The
   plan should contain architecture context, phased checkbox tasks, module
   structure, validation commands, and status indicators (`[ ]`, `[~]`, `[x]`,
   `[!]`).
3. Record the plan path in `phases.plan.plan_file`, set
   `phases.plan.status: complete`, set `phases.plan.approved: true` when the
   plan is ready to execute, and append `plan_created` to history.
4. Execute the plan phase-by-phase. Mark tasks `[~]` when started and `[x]`
   when completed.
5. Make scoped edits with `apply_patch`.
6. Add or update tests proportional to risk.
7. Run focused checks first, then broader checks as the blast radius grows.
8. Keep the status YAML current after each coherent implementation slice.

Do not overwrite unrelated user changes. If uncommitted changes are present,
separate user changes from Codex changes before editing.

Checkpoint fields:

- `phases.plan.status: complete`
- `phases.plan.plan_file`
- `phases.implement.status: complete`
- `phases.implement.codex_changed_files`
- `phases.implement.codex_tests_run`
- `phases.implement.codex_open_risks`

## Phase 5: Review Before PR

Goal: catch issues before publishing the PR.

Run the repo pre-merge gate unless the user says otherwise. For this repo, the
default gate is `make check`.

Also perform a Codex review of the branch diff against the design:

- inspect `git diff main...HEAD` or the appropriate base
- verify every acceptance criterion has coverage
- check for regressions, dead code, and missing docs
- write `docs/reviews/<task-id>-codex-pre-pr-review.md`

Fix blocking findings before moving to PR. Record non-blocking follow-ups.

Checkpoint fields:

- `phases.implement.codex_validated: true`
- `phases.implement.codex_verdict: pass | pass_with_notes | fail`
- `phases.implement.codex_report: docs/reviews/<task-id>-codex-pre-pr-review.md`
- `phases.implement.codex_tests_run`

## Phase 6: PR

Goal: publish the completed branch for review.

1. Check `git status --short` and confirm the diff belongs to this task.
2. Commit with an intentional message that includes the task ID when available.
3. Push the branch.
4. Create a draft PR unless the user requests ready-for-review.
5. PR body must include summary, validation, design/review links, plan link,
   workflow YAML link, and known risks or follow-ups.

Prefer the GitHub plugin skills when available. Use `gh` only when plugin
coverage is insufficient.

Checkpoint fields:

- `phases.pr.status: in_progress | complete`
- `phases.pr.pr_url`
- `phases.pr.pr_number`
- `phases.pr.codex_validation`

## Phase 7: PR Fixes

Goal: address CI and review feedback completely.

1. Inspect failing checks and review comments.
2. Categorize feedback as blocking, suggestion, question, or stale.
3. Fix blocking issues first.
4. Run relevant checks.
5. Commit and push fixes.
6. Reply to every actionable review thread with what changed or why it was not
   changed.
7. Repeat until CI is green and review is no longer blocked.

Checkpoint fields:

- `phases.pr.reviews_addressed`
- `phases.pr.codex_fix_rounds`
- `phases.pr.codex_checks`
- `phases.pr.codex_unresolved_threads`

## Phase 8: Done

Goal: close out the task after merge or after the user confirms completion.

1. Verify PR state.
2. Update task/status docs with final summary.
3. Record merged PR, final validation, and follow-ups.
4. Mark workflow complete.

Checkpoint fields:

- `phases.complete.status: complete`
- `phases.complete.merged_at` when merged, or `phases.complete.codex_completed_at`
  when the user explicitly marks a non-merged task complete
- `phases.complete.codex_summary`
- `workflow.current_phase: complete`
- `workflow.status: complete`

## Response Style

During execution, keep the user informed with concise progress updates. In the
final response, report changed files, validation run, PR link if created, and
any remaining risks.
