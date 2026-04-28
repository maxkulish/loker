# Phase: complete

Merge the PR, sync project aggregation files (if any), and close the
workflow. Mirrors `.claude/commands/task/phases/complete.md`.

## Required exit state

```yaml
phases:
  complete:
    status: complete
    aggregation_files_updated: true | false
    merged_at: "<ISO-8601>"

workflow:
  current_phase: complete
  status: complete
```

History events required: `workflow_complete`. Optional: `pr_merged`,
`project_sync_complete`.

## Step 1 - Merge the PR

```bash
gh pr merge <n> --squash --delete-branch
```

Capture the merge commit SHA from the output.

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "complete",
  action: "pr_merged",
  details: "PR #<n> merged. Merge commit <sha>.",
  phase_updates: {
    merged_at: "<ISO-8601>",
    merge_commit: "<sha>"
  }
})
```

Also update the `pr` phase block so both records agree:

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "pr",
  action: "pr_merged",
  details: "Merge commit <sha>",
  phase_updates: {
    merged_at: "<ISO-8601>",
    merge_commit: "<sha>"
  }
})
```

## Step 2 - Local cleanup

```bash
git checkout main
git pull
git branch -D feat/clo-XX-<slug>     # only if it still exists locally
```

## Step 3 - Project sync complete

Pi has no `/project:sync` slash command, and the loker repo currently has
no `PROJECT.md` / `ROADMAP.md` / `DEPENDENCIES.md`. Record the skip:

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "complete",
  action: "project_sync_skipped",
  details: "No PROJECT.md/ROADMAP.md/DEPENDENCIES.md exist in this repo",
  phase_updates: {
    aggregation_files_updated: false,
    aggregation_files_skip_reason: "No aggregation files in repo"
  }
})
```

If those aggregation files are added later, update the equivalent Claude
flow at `.claude/commands/task/phases/complete.md` first - this pi
script mirrors it.

## Step 4 - Linear

```
mcp__linear__save_issue(id="CLO-XX", state="Done")
mcp__linear__save_comment(
  issueId="CLO-XX",
  body="Merged in <sha>. Workflow YAML: docs/status/clo-XX-workflow.yaml"
)
```

## Step 5 - Mark workflow complete

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "complete",
  action: "workflow_complete",
  details: "Task CLO-XX fully completed. PR #<n> merged. <unblocks list>.",
  phase_updates: { status: "complete" },
  workflow_updates: { status: "complete" }
})
```

The orchestrator runtime treats `complete` as terminal - no further
`transition_phase` call is allowed.

## Step 6 - Commit the workflow YAML

```bash
git checkout main && git pull
git add docs/status/clo-XX-workflow.yaml
git commit -m "docs(CLO-XX): mark workflow complete after PR #<n> merge"
git push
```

## Notes

- If `aggregation_files_updated` is false, record a concrete reason -
  do not leave it null.
- For specification / operational tasks the same flow applies; only the
  earlier phases differ.
