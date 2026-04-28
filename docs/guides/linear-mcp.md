# Linear MCP - usage guide

How to read, create, and update Linear work items for loker via the
`mcp__linear__*` tools. Source skill: `~/.claude/skills/linear/SKILL.md`.

## Project context

| Field | Value |
|---|---|
| Linear project | Loker (`081a3c1e-610c-4b54-b558-12c440559a88`) |
| URL | https://linear.app/cloud-ai/project/loker-ebd36c1903b4 |
| Team | Cloud-ai, key `CLO` |
| Issue identifier | `CLO-NNN` |
| Target | 2026-05-31 |

The active MCP server is `linear`; `linear-server` exists as a sibling
and exposes the same surface. Prefer `mcp__linear__*` for consistency.

## When to use

- Track milestone work that maps to issues (M1 backend, M2 strategies).
- Log progress on long-running tasks where the comment thread is the
  source of truth (decision changes, blockers, phase completions).
- Cross-reference issues from commits, branches, and PRs (`CLO-NNN`).

Skip it for: throwaway investigations, internal-only refactors with no
user-visible behavior change, and anything already tracked in
`docs/plans/`.

## Tool reference

Canonical adapter: [`linear-mcp-adapter.md`](./linear-mcp-adapter.md) —
defines the approved 7-tool subset, API contract, caching, and
phase-action matrix.

| Action | Tool | Approved |
|---|---|---|
| List projects | `mcp__linear__list_projects` | ✅ |
| List issues | `mcp__linear__list_issues` | ✅ |
| Get issue | `mcp__linear__get_issue` | ✅ |
| Create/update issue | `mcp__linear__save_issue` | ✅ |
| Comment | `mcp__linear__save_comment` | ✅ |
| List comments | `mcp__linear__list_comments` | ✅ |
| List statuses | `mcp__linear__list_issue_statuses` | ✅ |
| List labels | `mcp__linear__list_issue_labels` | ❌ (escalate) |
| Search docs | `mcp__linear__search_documentation` | ❌ (escalate) |

`save_issue` / `save_comment` are upsert-style: omit `id` to create,
include `id` to update.

## Common operations

### Find an issue

```
mcp__linear__list_issues(team="CLO", query="tensorzero", limit=10)
```

### Create an issue

```
mcp__linear__save_issue(
    team="CLO",
    project="Loker",
    title="Add aggregator for cross-family parallel calls",
    description="""## Goal

Parallel calls to anthropic/google/openai/zhipu, merged to reduce
correlated failures.

## Scope

- [ ] Aggregator trait
- [ ] Merge strategies (majority, llm-judge)
- [ ] Wiremock-backed unit tests

## Acceptance Criteria

- [ ] `cargo test` covers the merge paths
- [ ] Real-gateway test gated behind `LOKER_TZ_INTEGRATION=1`
""",
    priority=3,
)
```

Pass real newlines in `description` and comment bodies - never literal
`\n` escape sequences (MCP server constraint).

### Update status

```
mcp__linear__save_issue(id="CLO-12", state="In Progress")
```

Workflow: `Backlog -> Todo -> In Progress -> In Review -> Done`. Add a
comment whenever you change state so the thread explains the why.

### Comment on progress

```
mcp__linear__save_comment(
    issueId="CLO-12",
    body="""## Progress

- [x] Wiremock harness landed
- [ ] genai client wiring in flight

Blocker: TensorZero gateway image pin pending sakana-fugu sync.
""",
)
```

## Naming and references

- Title formula: `[verb] + [component] + [outcome]`. Example:
  `Add escalating retry with verify gates`.
- Commits: `feat(CLO-12): add tensorzero backend skeleton`.
- Branches: `feature/clo-12-tensorzero-backend`.
- PR titles mirror the commit subject. PR body links the issue with
  `Closes CLO-12`.

## Priority

| Level | When |
|---|---|
| 1 Urgent | Production down, security breach |
| 2 High | Blocks milestone, deadline risk |
| 3 Normal | Standard planned work |
| 4 Low | Backlog, nice-to-have |

Default to 3 for milestone tasks; bump to 2 once a deadline is at risk.

## Phase labels

The shared label set is broader than what loker needs today. For now use
no label or `Phase 2: Core Platform` for orchestration primitives. Check
`mcp__linear__list_issue_labels(team="CLO")` before inventing a new one.

## Pitfalls

- Don't pass team key without quotes - `team="CLO"` works,
  `team=CLO` does not.
- `save_issue` returns the full issue including the auto-assigned
  `CLO-NNN` identifier; capture it before composing commits.
- `state` accepts the human name (`"In Progress"`); use
  `list_issue_statuses(team="CLO")` to discover the canonical strings.
- Markdown content is rendered as-is; literal `\n` will show up as text.

## Reference

| Need | Read |
|---|---|
| Skill source | `~/.claude/skills/linear/SKILL.md` |
| Task templates | `~/.claude/skills/linear/task-templates.md` |
| Progress comment templates | `~/.claude/skills/linear/progress-tracking.md` |
| Tool reference | `~/.claude/skills/linear/reference.md` |
