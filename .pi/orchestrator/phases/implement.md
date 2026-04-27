# Phase: implement

Land the plan's sub-tasks one by one. Run the codex+gemini validation
gate as the final pre-PR step. The validation gate is **embedded in this
phase** - loker has no separate `review` phase.

Mirrors `.claude/commands/task/phases/implement.md`.

## Required exit state

```yaml
phases:
  implement:
    status: complete
    commits: ["abc123", "def456"]   # optional but recommended
    codex_validated: true
    codex_verdict: "approve" | "approve_with_changes" | "rework"
    codex_report: "docs/reviews/clo-XX-codex-validation.md"
    gemini_validation_report: "docs/reviews/clo-XX-gemini-validation.md"
```

History events required: `implementation_complete`,
`codex_validation_complete`.

## Step 1 - Land sub-tasks

For each sub-task ST1..STN in `docs/plans/clo-XX-<slug>.md`:

1. Implement the changes in the named files.
2. Run the sub-task's acceptance command (usually `cargo test ...`).
3. If green, commit:
   ```
   git add -A
   git commit -m "feat(CLO-XX): <ST verb> <component>"
   ```
4. Append the commit SHA to `phases.implement.commits`:
   ```ts
   update_workflow_state({
     task_id: "CLO-XX",
     phase: "implement",
     action: "subtask_complete",
     details: "ST1 landed: <description>. Commit <sha>",
     phase_updates: { commits: [...existing, "<sha>"] }
   })
   ```

If a sub-task fails after a reasonable attempt, set `workflow.status =
blocked` and dispatch `phases/blocked.md`.

## Step 2 - Run the pre-merge gate

```bash
make check    # fmt + clippy + test
```

It must be green before proceeding.

## Step 3 - Record implementation complete

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "implement",
  action: "implementation_complete",
  details: "All sub-tasks landed. make check green. <n> commits.",
  phase_updates: { status: "complete" }
})
```

Note: do NOT `transition_phase` yet. Run the validation gate first.

## Step 4 - Codex + Gemini validation gate (MANDATORY)

This is the loker equivalent of the mentis `review` phase. Run BOTH
validators in parallel against the implementation. Outputs go to
`docs/reviews/`.

### 4.1 Run Codex

```bash
codex exec -m gpt-5.4 \
  --persona .pi/agents/codex-pre-pr.md \
  --input "branch: feat/clo-XX-...; design: docs/designs/clo-XX-<slug>.md; plan: docs/plans/clo-XX-<slug>.md" \
  > docs/reviews/clo-XX-codex-validation.md
```

### 4.2 Run Gemini

```bash
gemini --model gemini-3.1-pro-preview \
  --persona .pi/agents/gemini-architect.md \
  --input "branch: feat/clo-XX-...; design: docs/designs/clo-XX-<slug>.md; plan: docs/plans/clo-XX-<slug>.md" \
  > docs/reviews/clo-XX-gemini-validation.md
```

Run them in parallel (the orchestrator should background one and wait on
both). If either binary is unavailable in this environment, document the
skip with an explicit reason in `phases.implement.codex_report` /
`gemini_validation_report` (e.g. `"skipped: codex not installed"`).

### 4.3 Parse verdicts

Each report ends with a `## Verdict` line: `approve`, `approve_with_changes`,
or `rework`.

| Codex | Gemini | Action |
|---|---|---|
| approve | approve | proceed |
| approve_with_changes | approve | apply suggested fixes |
| approve | approve_with_changes | apply suggested fixes |
| approve_with_changes | approve_with_changes | apply union of fixes |
| rework | * | re-enter Step 1 with fixes |
| * | rework | re-enter Step 1 with fixes |

After applying fixes, re-run `make check` and update the same review
files (or append a `## Re-validation` section).

### 4.4 Record verdict

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "implement",
  action: "codex_validation_complete",
  details: "Codex: <verdict>. Gemini: <verdict>. <fixes> applied.",
  phase_updates: {
    codex_validated: true,
    codex_verdict: "<approve|approve_with_changes|rework>",
    codex_report: "docs/reviews/clo-XX-codex-validation.md",
    gemini_validation_report: "docs/reviews/clo-XX-gemini-validation.md"
  }
})
```

## Step 5 - Transition to PR

```ts
transition_phase({
  task_id: "CLO-XX",
  from_phase: "implement",
  to_phase: "pr"
})
```

## Notes

- The validation gate is non-negotiable for development tasks. For
  specification tasks the gate is recommended but may be skipped if the
  spec author opts out (record the decision in `details`).
- If validation surfaces a fundamental design issue, do not paper over
  it - return to `design` via user confirmation.
