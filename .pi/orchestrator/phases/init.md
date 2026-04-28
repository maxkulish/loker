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

Pi has no `/project:sync` slash command, and the loker repo currently has
no `PROJECT.md` / `ROADMAP.md` / `DEPENDENCIES.md` at the root. Record the
skip and move on:

```ts
update_workflow_state({
  task_id: "CLO-42",
  phase: "init",
  action: "project_sync_skipped",
  details: "No PROJECT.md/ROADMAP.md/DEPENDENCIES.md exist in this repo."
})
```

If those aggregation files are added later, update the equivalent Claude
flow at `.claude/commands/task/phases/init.md` first - this pi script
mirrors it.

## Step 3 - Dispatch first phase

The `task:orchestrate` slash command auto-dispatches the phase named in
`workflow.current_phase`. Once `update_workflow_state` set it correctly
in 2.4, no extra step is needed - the runtime sends the matching
`.pi/orchestrator/phases/<phase>.md` as a follow-up prompt.

Do NOT call `transition_phase` from `init`: there is no `init` entry in
`ALLOWED_TRANSITIONS`. Init is a virtual phase that ends the moment the
workflow file is written.

## Runtime contract for every later phase

After this point, the loop is:

1. The agent runs the dispatched phase file's steps.
2. The phase file ends with a `transition_phase({...})` call.
3. The runtime auto-dispatches the next phase's file as a follow-up.

If `transition_phase` does not auto-dispatch (older builds of the
orchestrator extension), re-run `/task:orchestrate CLO-XX` to resume from
the new `current_phase`.

## Notes

- Branch creation happens in the first real phase (discovery/spec/ops),
  not here. The init phase only records `branch_suggested`.
- Never overwrite an existing workflow file unless `--force-restart` is
  passed.
- Always read Linear via `mcp__linear__*` tools. In pi these come from
  the `.pi/extensions/linear/` bridge; in Claude they come from the
  global Linear MCP config. The names match either way.
