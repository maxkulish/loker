# pi-linear-mcp

Pi CLI bridge to Linear's hosted MCP server. Connects via SSE (default)
or Streamable HTTP, lists Linear's tools, and re-registers each one in
pi under the `mcp__linear__` prefix so the orchestrator phase scripts
can call them with the same names Claude uses.

## Why this exists

Pi extensions do NOT inherit Claude's MCP server configuration. Each
extension that needs MCP must establish its own client connection. The
loker orchestrator phase scripts call `mcp__linear__get_issue`,
`mcp__linear__save_issue`, `mcp__linear__save_comment`, etc. - this
bridge is what makes those names resolvable inside pi.

The bridge mirrors the pattern used by `~/Code/mentis/.pi/extensions/plane/`.

## Configuration

Required env var:

```bash
export LINEAR_API_KEY=lin_api_...
```

Optional:

```bash
# default: http (Streamable HTTP). SSE kept as fallback because the
# MCP SDK has deprecated SSE in favour of Streamable HTTP.
export LINEAR_MCP_TRANSPORT=http   # or 'sse'
```

If `LINEAR_API_KEY` is missing, the extension prints a follow-up
message and exits cleanly - the orchestrator will then fail fast on
the first `mcp__linear__*` call rather than silently no-op.

## Installation

```bash
cd .pi/extensions/linear
npm install

# temporary
pi -e .pi/extensions/linear/index.ts

# permanent (symlink)
ln -s $(pwd)/.pi/extensions/linear ~/.pi/agent/extensions/loker-linear
```

## Tool surface

Whatever Linear's MCP exposes, prefixed with `mcp__linear__`. The
orchestrator currently uses:

- `mcp__linear__get_issue`
- `mcp__linear__save_issue`
- `mcp__linear__save_comment`
- `mcp__linear__list_issues`
- `mcp__linear__list_comments`

If Linear ships new tools, they show up automatically on next pi start.

## Auth notes

Linear's hosted MCP at `https://mcp.linear.app/sse` accepts a Linear
personal API key as a Bearer token. If your workspace requires OAuth
instead, replace the `Authorization` header construction in `index.ts`
with the OAuth token flow.

## See also

- `../orchestrate/README.md` - extension-level docs for the orchestrator
- `../../IMPLEMENTATION_SUMMARY.md` - high-level pi flow overview
- `../../../docs/guides/linear-mcp.md` - tool reference and team context
