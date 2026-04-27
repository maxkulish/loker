---
name: codex-task-lifecycle
description: Run a Codex-native task lifecycle from task intake through design, design review, implementation, pre-PR review, PR creation, PR fixes, and done. Use when the user asks Codex to orchestrate or resume a tracked engineering task end to end.
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

Persist progress in `docs/status/<task-id>-codex-workflow.yaml`.

Use lowercase file IDs, for example `clo-247-codex-workflow.yaml`. If a state
file exists, resume from its `workflow.current_phase` unless the user asks to
restart.

Initial state template: `assets/workflow-state.yaml`.

Update state after every meaningful action:

- `workflow.current_phase`
- `workflow.status`
- current phase fields
- append a timestamped `history` entry

Statuses are `pending`, `in_progress`, `checkpoint`, `complete`, and `blocked`.
If blocked, record the blocker and stop with the exact next action needed.

## Phase 1: Task

Goal: establish scope, inputs, branch, and working state.

1. Extract the task ID from the user, branch, PR, issue, or local docs.
2. Read project context first: `AGENTS.md`, `README.md`, `docs/handoff.md`,
   relevant specs/plans, and the current branch diff.
3. Gather task details from local docs and available connectors. If the tracker
   is unavailable, continue from user-provided context and record that gap.
4. Create or resume `docs/status/<task-id>-codex-workflow.yaml`.
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

- `design_review.status: complete`
- `design_review.review_file`
- `design_review.verdict: approve | approve_with_changes | revise`
- `design_review.applied_changes`

## Phase 4: Implementation

Goal: implement the approved design.

1. Read the design, status file, relevant source files, and tests.
2. Make scoped edits with `apply_patch`.
3. Add or update tests proportional to risk.
4. Run focused checks first, then broader checks as the blast radius grows.
5. Keep the status file current after each coherent implementation slice.

Do not overwrite unrelated user changes. If uncommitted changes are present,
separate user changes from Codex changes before editing.

Checkpoint fields:

- `implementation.status: complete`
- `implementation.changed_files`
- `implementation.tests_run`
- `implementation.open_risks`

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

- `pre_pr_review.status: complete`
- `pre_pr_review.review_file`
- `pre_pr_review.tests_run`
- `pre_pr_review.verdict: pass | pass_with_notes | fail`

## Phase 6: PR

Goal: publish the completed branch for review.

1. Check `git status --short` and confirm the diff belongs to this task.
2. Commit with an intentional message that includes the task ID when available.
3. Push the branch.
4. Create a draft PR unless the user requests ready-for-review.
5. PR body must include summary, validation, design/review links, and known
   risks or follow-ups.

Prefer the GitHub plugin skills when available. Use `gh` only when plugin
coverage is insufficient.

Checkpoint fields:

- `pr.status: complete`
- `pr.pr_url`
- `pr.pr_number`
- `pr.validation`

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

- `pr_fixes.status: complete`
- `pr_fixes.rounds`
- `pr_fixes.checks`
- `pr_fixes.unresolved_threads`

## Phase 8: Done

Goal: close out the task after merge or after the user confirms completion.

1. Verify PR state.
2. Update task/status docs with final summary.
3. Record merged PR, final validation, and follow-ups.
4. Mark workflow complete.

Checkpoint fields:

- `done.status: complete`
- `done.completed_at`
- `done.summary`
- `workflow.current_phase: done`
- `workflow.status: complete`

## Response Style

During execution, keep the user informed with concise progress updates. In the
final response, report changed files, validation run, PR link if created, and
any remaining risks.
