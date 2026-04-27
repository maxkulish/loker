# Loker pi Orchestrator - Implementation Summary

This `.pi/` tree adds a pi-CLI surface for the loker task lifecycle that
mirrors the existing Claude flow at `.claude/commands/task/`. The two
sides share the same YAML schema, same phase set, same allowed
transitions, and same Linear integration model so a task can move
freely between Claude and pi.

## Layout

```
.pi/
├── IMPLEMENTATION_SUMMARY.md          (this file)
├── extensions/
│   ├── orchestrate/
│   │   ├── index.ts                   pi extension: state machine + tools
│   │   ├── package.json
│   │   └── README.md                  extension-level docs
│   └── linear/
│       ├── index.ts                   pi extension: Linear MCP bridge
│       ├── package.json
│       └── README.md                  extension-level docs
├── orchestrator/
│   ├── README.md                      phase-script index, conventions
│   └── phases/
│       ├── init.md
│       ├── discovery.md
│       ├── design.md
│       ├── plan.md
│       ├── implement.md               (embedded codex+gemini gate)
│       ├── pr.md
│       ├── complete.md
│       ├── spec.md
│       ├── operational.md
│       ├── status.md
│       └── blocked.md
└── agents/
    ├── gemini-architect.md            design / impl architecture review
    ├── codex-pre-pr.md                pre-PR validation gate
    └── ollama-rust-reviewer.md        local-only Rust footgun pass
```

## What the extension exposes

`.pi/extensions/orchestrate/index.ts` registers:

- `task:orchestrate` slash command - dispatches the right phase based
  on flags (`--status`, `--spec`, `--ops`, `--skip-discovery`).
- `update_workflow_state` tool - merges phase / workflow / linear /
  root updates into the YAML and appends a history event. Concurrency
  is guarded by per-task write locks.
- `transition_phase` tool - validates the requested transition against
  `ALLOWED_TRANSITIONS`, `TYPE_ALLOWED_PHASES`, and `PHASE_CONFIG`,
  then advances `workflow.current_phase`.

Validation rules enforced at transition time:

1. `from_phase` must equal current `workflow.current_phase`.
2. `to_phase` must be in `ALLOWED_TRANSITIONS[from]`.
3. `to_phase` must be permitted for `task_type`.
4. Outgoing phase must have `status: complete` or `status: skipped`.
5. All required fields and history events for the outgoing phase must
   be present (skipped phases bypass this).

`validation_override: true` exists for manual unblocking but should
be a last resort.

## Schema parity with Claude

Top-level keys that must remain identical across Claude and pi:

```yaml
task_id: CLO-XX
task_title: ...
task_url: https://linear.app/cloud-ai/issue/clo-xx/...
task_type: development | specification | operational
classification_reason: ...

linear:
  team: Cloud-ai
  project: Loker
  status_at_start: ...
  priority: ...
  branch_suggested: ...
  branch_actual: feat/clo-xx-...
  blocks: []
  blocked_by: []

workflow:
  current_phase: ...
  status: active | blocked | paused | complete | in_progress | checkpoint
  created_at: ...
  updated_at: ...

phases:
  discovery: { status, approved, ... }
  spec: { status, spec_file, approved, review_completed, ... }
  design: { status, design_doc, draft_ready, finalized, review_completed, ... }
  plan: { status, plan_file, approved }
  implement: { status, commits[], codex_validated, codex_verdict, codex_report, gemini_validation_report }
  pr: { status, pr_url, pr_number, ci_passed, reviews_addressed, merged_at, merge_commit }
  complete: { status, aggregation_files_updated, merged_at }

history:
  - { timestamp, action, phase, details }
```

The complete phase block reference lives in
`extensions/orchestrate/README.md`.

## Differences vs the mentis pi orchestrator

The structural skeleton is borrowed from `~/Code/mentis/.pi/`. The
loker version diverges in these places:

| Aspect | Mentis | Loker |
|---|---|---|
| Task ID | `MENTI-XX` | `CLO-XX` |
| Tracker | Plane.so | Linear (MCP) |
| Tracker tools | `mcp__plane__*` | `mcp__linear__*` |
| Status file | `docs/status/menti-XX-workflow.yaml` | `docs/status/clo-XX-workflow.yaml` |
| Review phase | Separate `review` phase before `pr` | None - validation gate is inside `implement.md` step 5 |
| Stack | Tauri v2 + Rust + React | Pure Rust + TensorZero |
| Pre-merge gate | `cargo fmt --manifest-path src-tauri/...` | `make check` |

The "no review phase" choice matches the existing Claude flow for
loker (see CLO-247 / 248 / 249 / 251 / 257 history) and keeps the
codex+gemini gate close to the code it validates.

## Auto Mode behaviour

Auto Mode is the default expected operating mode for both Claude and
pi. Each phase file's "approval checkpoint" section names the exact
preconditions that allow auto-approval. When those preconditions hold,
pi auto-approves and records the reason in
`phases.<phase>.auto_approval_reason`. When they do not hold, pi
prompts the user.

## Linear integration

Pi does NOT inherit Claude's MCP server configuration. Each pi
extension that needs MCP must establish its own client connection. So
the loker pi setup ships a thin bridge extension at
`.pi/extensions/linear/` that connects to Linear's hosted MCP and
re-registers every Linear tool under the `mcp__linear__` prefix - the
exact names Claude already uses.

This keeps phase scripts identical across Claude and pi:

- `mcp__linear__get_issue` to ingest the issue at init.
- `mcp__linear__save_issue` to update status (`Todo -> In Progress ->
  In Review -> Done`).
- `mcp__linear__save_comment` at every phase transition.

The bridge is a 70-line MCP-client wrapper modelled on
`~/Code/mentis/.pi/extensions/plane/`. It:

- Reads `LINEAR_API_KEY` from env.
- Connects to `https://mcp.linear.app/mcp` via Streamable HTTP
  (or `https://mcp.linear.app/sse` if `LINEAR_MCP_TRANSPORT=sse`).
- Lists Linear's tools, prefixes each with `mcp__linear__`, and
  registers them with pi.

See `extensions/linear/README.md` for setup and
`docs/guides/linear-mcp.md` for the project context (team
`Cloud-ai`, project `Loker`, identifier `CLO`) and full tool reference.

## Installation

Both extensions install the same way. Install both - the orchestrator
calls Linear tools, so the linear bridge must be active for end-to-end
runs.

```bash
# orchestrator (state machine + phase dispatcher)
cd .pi/extensions/orchestrate
npm install
ln -s $(pwd) ~/.pi/agent/extensions/loker-task-orchestrator

# linear bridge (mcp__linear__* tools)
cd ../linear
npm install
ln -s $(pwd) ~/.pi/agent/extensions/loker-linear

# required env
export LINEAR_API_KEY=lin_api_...

# temporary load (one-shot, both extensions)
pi -e .pi/extensions/orchestrate/index.ts -e .pi/extensions/linear/index.ts
```

## Usage

```bash
/task:orchestrate CLO-42                  # start or resume
/task:orchestrate CLO-42 --status         # show current state
/task:orchestrate CLO-42 --spec           # specification task
/task:orchestrate CLO-42 --ops            # operational task
/task:orchestrate CLO-42 --skip-discovery # development, skip discovery
```

## Maintenance rules

- Any change to a phase file or schema rule must be mirrored on the
  Claude side under `.claude/commands/task/`.
- Any change to required fields / history must be mirrored in
  `PHASE_CONFIG` inside `extensions/orchestrate/index.ts`.
- Any change to the YAML schema must be reflected in the phase file's
  "Required exit state" section AND in the extension README.

## See also

- `extensions/orchestrate/README.md` - extension-level docs
- `orchestrator/README.md` - phase-script index
- `.claude/commands/task/orchestrate.md` - canonical Claude flow
- `docs/handoff.md` - project WHY/Intent/HOW
- `docs/guides/linear-mcp.md` - Linear MCP usage
- `/Users/mk/Work/investigations/sakana-fugu/loker-design.md` -
  canonical loker design
