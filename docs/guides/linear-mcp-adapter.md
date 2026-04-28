# Linear MCP Adapter — API Contract & Policy

**Status**: draft  
**Applies to**: `mcp__linear__*` (server `linear`, not `linear-server`)  
**Last updated**: 2026-04-28

## 1. Problem Statement

The `linear` MCP server exposes **30 tools**. The Loker task lifecycle uses
**7** of them. The remaining 23 add noise to the tool surface, increase prompt
token overhead, and make agent tool-selection less predictable. We need a
canonical adapter layer that:

1. Declares the approved 7-tool subset.
2. Defines the exact API contract for each tool (params, returns, errors).
3. Adds caching rules for slow/static entities.
4. Enforces which lifecycle phases may use which tools.
5. Provides a thin conceptual wrapper that agent instructions reference.

## 2. Approved Tool Subset

### 2.1 Core 7

| # | Tool | Purpose | Phase(s) |
|---|------|---------|----------|
| 1 | `mcp__linear__list_issues` | Discover/find issues by query or filter | init, discovery, status |
| 2 | `mcp__linear__get_issue` | Fetch full issue detail (attachments, branch) | init, discovery, implement, pr |
| 3 | `mcp__linear__save_issue` | Create or update an issue (upsert) | init, implement, pr, complete |
| 4 | `mcp__linear__list_comments` | Read comment thread for an issue | discovery, implement, pr |
| 5 | `mcp__linear__save_comment` | Create or update a comment (upsert) | All phases |
| 6 | `mcp__linear__list_issue_statuses` | Discover valid status names for state transitions | init, implement, pr |
| 7 | `mcp__linear__list_projects` | Resolve project name → ID for issue creation | init |

### 2.2 Conditional 8th (on-demand only)

| # | Tool | Purpose | Trigger |
|---|------|---------|---------|
| 8 | `mcp__linear__get_team` | Resolve team name → ID | Only when `CLO` team key is ambiguous |

### 2.3 Explicitly excluded

Everything else — `list_cycles`, `list_documents`, `save_document`,
`list_users`, `get_user`, `list_teams`, `list_milestones`,
`save_milestone`, `get_milestone`, `list_issue_labels`,
`create_issue_label`, `get_issue_status`, `search_documentation`,
`extract_images`, `get_attachment`, `create_attachment`,
`delete_attachment`, `delete_comment`, `save_project`, `get_project`,
`list_project_labels`.

If a task genuinely needs one of these, escalate to the user with the
specific tool name and rationale. Do not call it automatically.

## 3. API Contract

### 3.1 `list_issues` — Find issues

```
mcp__linear__list_issues(
    team:       "CLO",            // required — team key or ID
    query?:     string,           // search title/description
    state?:     string,           // state type/name/ID
    assignee?:  "me" | user_id,   // filter by assignee
    project?:   string,           // project name/ID/slug
    priority?:  0|1|2|3|4,        // 0=None, 1=Urgent, 2=High, 3=Normal, 4=Low
    limit?:     number,           // default 50, max 250
    orderBy?:   "createdAt" | "updatedAt",
    updatedAt?: string,           // ISO-8601 filter (e.g., "-P1D" for last 24h)
)
→ { issues: Issue[] }   // Issue: { id, identifier, title, state, ... }
```

**Contract rules**:
- Always pass `team="CLO"`.
- Prefer `query` for text search; combine with `state`/`assignee` for narrowing.
- Use `updatedAt` for incremental polling (avoids re-fetching stale data).
- Max 2 calls per phase. Use `limit` tuning before paginating.

### 3.2 `get_issue` — Fetch full issue

```
mcp__linear__get_issue(
    id:                   string,  // "CLO-NNN" or UUID
    includeRelations?:    boolean, // default false
    includeCustomerNeeds?: boolean, // default false
)
→ Issue (with attachments, branch name, relations)
```

**Contract rules**:
- Always pass the `CLO-NNN` identifier (not UUID).
- Set `includeRelations: true` only when a blocker chain is suspected.
- Cache result for 5 minutes within the same phase.

### 3.3 `save_issue` — Create or Update

```
// CREATE (omit `id`)
mcp__linear__save_issue(
    title:        string,        // required
    team:         "CLO",         // required
    description?: string,        // markdown — literal newlines, no escape sequences
    project?:     "Loker",       // project name/ID/slug
    priority?:    1|2|3|4,
    assignee?:    "me" | user_id,
    labels?:      string[],
    parentId?:    string,        // parent issue for subtasks
    state?:       string,        // initial state (default: team default)
)
→ Issue (includes auto-assigned identifier like "CLO-310")

// UPDATE (include `id`)
mcp__linear__save_issue(
    id:           "CLO-NNN",     // required for update
    title?:       string,
    description?: string,
    state?:       string,        // "Backlog", "Todo", "In Progress", "In Review", "Done"
    priority?:    1|2|3|4,
    assignee?:    "me" | user_id,
    labels?:      string[],
    // links, blocks, blockedBy, relatedTo: append-only arrays
)
→ Issue (updated)
```

**Contract rules**:
- Capture the returned `identifier` (e.g., `CLO-310`) immediately after create.
- For status transitions, call `list_issue_statuses` first if the canonical
  status name is unknown (cache the result).
- Default priority: 3 (Normal). Bump to 2 if deadline is at risk.
- Markdown in `description`: use literal newlines. Never escape as `\n`.

### 3.4 `list_comments` — Read comment thread

```
mcp__linear__list_comments(
    issueId:   string,           // "CLO-NNN"
    limit?:    number,           // default 50, max 250
    orderBy?:  "createdAt" | "updatedAt",
)
→ { comments: Comment[] }   // Comment: { id, body, createdAt, user }
```

**Contract rules**:
- Use `orderBy: "createdAt"` for chronological read.
- Cache for 2 minutes within a phase to avoid re-fetching.
- When checking for new comments, use `updatedAt` on `get_issue` instead
  of polling `list_comments`.

### 3.5 `save_comment` — Create or Update

```
// CREATE (omit `id`)
mcp__linear__save_comment(
    issueId:    string,          // "CLO-NNN" (required)
    body:       string,          // markdown — literal newlines
    parentId?:  string,          // reply to existing comment
)
→ Comment

// UPDATE (include `id`)
mcp__linear__save_comment(
    id:     string,              // comment ID
    body:   string,
)
→ Comment (updated)
```

**Contract rules**:
- Always include a `## Section` heading in the body for scannability.
- When updating status, add a comment explaining the _why_ before
  changing the state.
- Use `parentId` for threaded replies to review feedback.

### 3.6 `list_issue_statuses` — Discover status names

```
mcp__linear__list_issue_statuses(
    team: "CLO",                 // required
)
→ { statuses: IssueStatus[] }   // IssueStatus: { id, name, type }
```

**Contract rules**:
- Call once per session, cache for the session lifetime.
- Use returned `name` values directly in `save_issue(state: ...)`.
- Expected names for CLO: `"Backlog"`, `"Todo"`, `"In Progress"`,
  `"In Review"`, `"Done"`, `"Canceled"`. Verify on first call.

### 3.7 `list_projects` — Resolve project

```
mcp__linear__list_projects(
    query?:  string,       // search by name
    team?:   "CLO",        // filter by team
)
→ { projects: Project[] }
```

**Contract rules**:
- Call once, cache for session.
- Hardcoded project name `"Loker"` is acceptable if the ID is known
  and stable. Otherwise resolve via this tool on first create.

## 4. Cache Strategy

| Entity | TTL | Scope |
|--------|-----|-------|
| Issue status list | Session | Global |
| Project list / Loker project ID | Session | Global |
| Issue detail (get_issue) | 5 min | Per-phase |
| Comment list (list_comments) | 2 min | Per-phase |

**Cache invalidation**: Flush per-entity cache after any `save_*` call
that mutates that entity. E.g., after `save_issue`, flush the issue
detail cache for that issue ID.

## 5. Phase-Action Matrix

Which tools are allowed in which lifecycle phases:

| Phase | Allowed tools |
|-------|--------------|
| init | `list_issues`, `get_issue`, `list_issue_statuses`, `list_projects`, `save_comment` |
| discovery | `get_issue`, `list_comments`, `save_comment` |
| spec | `save_comment` |
| operational | `save_comment` |
| design | `save_comment` |
| plan | `save_comment` |
| implement | `get_issue`, `save_issue`, `list_comments`, `save_comment` |
| pr | `get_issue`, `save_issue`, `list_comments`, `save_comment` |
| execute | `save_comment` |
| document | `save_issue`, `save_comment` |
| complete | `save_issue`, `save_comment` |
| status | `list_issues`, `get_issue`, `list_comments` |
| blocked | `get_issue`, `save_issue`, `save_comment` |

## 6. Adapter Interface (Conceptual)

This is the thin wrapper that agent instructions reference. It is not
compiled code — it is a pattern contract for how the agent composes
Linear calls.

```
// --- Finders ---

/// Find CLO issues matching a search term.
find_issues(query: string, opts?: { state?, assignee?, limit? }) → Issue[]

/// Fetch full context for a task ID (issue + recent comments + valid statuses).
get_task_context(task_id: string) → TaskContext {
    issue:    Issue,
    comments: Comment[],
    statuses: IssueStatus[],
}

// --- Mutators ---

/// Create a new issue. Returns the assigned CLO-NNN identifier.
create_issue(params: CreateIssueParams) → string  // returns "CLO-NNN"

/// Update issue state with an explanatory comment.
transition_issue(task_id: string, to_state: string, reason: string) → void

/// Post a phase checkpoint comment.
post_progress(task_id: string, body: string) → Comment

// --- Sync helpers ---

/// Ensure the branch name includes the task ID.
sync_branch_name(task_id: string) → void
```

### 6.1 Usage in task lifecycle

```
// Phase: init — gather context
ctx = get_task_context("CLO-266")

// Phase: implement — update status
transition_issue("CLO-266", "In Progress", "Starting implementation per plan")

// Phase: implement — checkpoint
post_progress("CLO-266", """## Phase 1 complete
- [x] Aggregator module skeleton
- [x] ConcatAggregator struct + tests
""")

// Phase: pr — create PR
transition_issue("CLO-266", "In Review", "PR #42 ready for review")
```

## 7. Migration: `linear-server` → `linear`

### 7.1 Current state

The `.claude/settings.json` allowlist references the old `linear-server`
namespace with 11 tools:
`create_comment`, `create_issue`, `create_issue_label`, `get_issue`,
`list_comments`, `list_issue_labels`, `list_issue_statuses`,
`list_issues`, `list_projects`, `list_teams`, `update_issue`.

### 7.2 Migration steps

1. **Add** `linear` as a new MCP server in `.mcp.json` (already present).
2. **Update** `.claude/settings.json` to use the `mcp__linear__*` namespace.
3. **Reduce** allowlist to the 7 approved tools.
4. **Update** `docs/guides/linear-mcp.md` to reference this adapter.
5. **Remove** `linear-server` from `.mcp.json` after a 1-week burn-in.

### 7.3 New allowlist (`.claude/settings.json`)

```json
{
  "permissions": {
    "allow": [
      "mcp__linear__list_issues",
      "mcp__linear__get_issue",
      "mcp__linear__save_issue",
      "mcp__linear__list_comments",
      "mcp__linear__save_comment",
      "mcp__linear__list_issue_statuses",
      "mcp__linear__list_projects"
    ]
  }
}
```

## 8. Telemetry & Enforcement

### 8.1 What to log

Per MCP call, record:
- tool name
- phase from `workflow.current_phase`
- latency (ms)
- success/failure
- cache hit/miss

### 8.2 Enforcement

- If an agent attempts a tool outside the 7-tool subset, it should
  refuse and suggest the correct tool or escalate to the user.
- If a tool is called in a phase where it is not allowed (see §5),
  log a warning and proceed (soft enforcement) or block (hard
  enforcement — configurable).

### 8.3 Review cadence

After every 20 tasks, review the call log and prune any tool that
was used < 2 times.
