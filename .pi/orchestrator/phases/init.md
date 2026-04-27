# Phase: init

Initialize a new CLO-XX workflow or resume an existing one. This phase is
implicit - the orchestrator runs it before dispatching the actual phase
file.

Mirrors `.claude/commands/task/phases/init.md` so a task started in
Claude can resume in pi (and vice-versa). Schema must stay identical.

## Step 1 - Parse arguments

The user invokes:

```
/task:orchestrate CLO-42 [--status|--ops|--spec|--skip-discovery]
```

Flags:

| Flag | Effect |
|---|---|
| `--status` | Show current state, do not dispatch phase. |
| `--spec` | Force `task_type=specification`. Skip discovery+design. |
| `--ops` | Force `task_type=operational`. Skip discovery+design+plan. |
| `--skip-discovery` | Development task that skips discovery (rare). |

If `--status` is set, jump to `phases/status.md`.

## Step 2 - Init or resume

Status file: `docs/status/clo-XX-workflow.yaml` (lowercase `clo-`).

### 2.1 Resume (file exists)

Read the file. If `workflow.status == "complete"` print a summary and stop
unless the user passed `--force-restart`. Otherwise dispatch the phase
named in `workflow.current_phase`.

### 2.2 Init (file does not exist)

1. Fetch the Linear issue:
   ```
   mcp__linear__get_issue(id="CLO-42")
   ```
2. Capture: title, url, description, priority, current state, blocks,
   blocked_by, suggested branch.
3. Classify task type (Step 2.3) unless overridden by `--spec` / `--ops`.
4. Pre-create the workflow file by calling `update_workflow_state` with
   the phase set to `init` (the orchestrator runtime initialises the
   skeleton on first call - see `index.ts::initializeWorkflow`).
5. Update Linear status to `Backlog -> Todo` if it is still `Backlog`.

### 2.3 Classify task type

| Signal | Type |
|---|---|
| Issue body has `**Type:** Specification` or `[spec]` label | `specification` |
| Issue body has `**Type:** Operational` or `[ops]` label | `operational` |
| Title starts with `Investigate`, `Audit`, `Document`, `Migrate`, `Configure` | `operational` |
| Title starts with `Add`, `Implement`, `Fix`, `Refactor` and ACs are mechanical | `specification` |
| Otherwise | `development` |

Set `task_type` and write `classification_reason` describing the signal.

### 2.4 Pre-create workflow file

Call:

```ts
update_workflow_state({
  task_id: "CLO-42",
  phase: "init",
  action: "workflow_started",
  details: "Created from Linear issue CLO-42. Classified as <type>: <reason>",
  workflow_updates: {
    current_phase: <first phase>,
    status: "active"
  },
  linear_updates: {
    team: "Cloud-ai",
    project: "Loker",
    status_at_start: "<linear status>",
    priority: "<linear priority>",
    branch_suggested: "<from linear>",
    blocks: [...],
    blocked_by: [...]
  },
  root_updates: {
    task_title: "...",
    task_url: "https://linear.app/cloud-ai/issue/clo-42/...",
    task_type: "<type>",
    classification_reason: "..."
  }
})
```

First phase by task type:

| Task type | First phase |
|---|---|
| `development` (default) | `discovery` |
| `development` + `--skip-discovery` | `plan` |
| `specification` | `spec` |
| `operational` | `operational` |

### 2.5 Project sync start

If `PROJECT.md`, `ROADMAP.md`, or `DEPENDENCIES.md` exist at repo root,
run `/project:sync --start CLO-42`. If none exist (current state of the
loker repo), record the skip:

```ts
update_workflow_state({
  task_id: "CLO-42",
  phase: "init",
  action: "project_sync_skipped",
  details: "No PROJECT.md/ROADMAP.md/DEPENDENCIES.md exist in this repo."
})
```

## Step 3 - Dispatch first phase

Call `transition_phase` only if you need to leave `init`. The orchestrator
runtime treats `init` as a virtual phase: after the workflow file is
created with `current_phase` set, dispatch the named phase file directly.

Loader path: `.pi/orchestrator/phases/<phase>.md`.

## Notes

- Branch creation happens in the first real phase (discovery/spec/ops),
  not here. The init phase only records `branch_suggested`.
- Never overwrite an existing workflow file unless `--force-restart` is
  passed.
- Always read Linear via `mcp__linear__*` tools. In pi these come from
  the `.pi/extensions/linear/` bridge; in Claude they come from the
  global Linear MCP config. The names match either way.
