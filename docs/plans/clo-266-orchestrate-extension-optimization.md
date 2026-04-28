# Plan: CLO-266 Orchestrate Extension Optimization

## Objective
Refactor `.pi/extensions/orchestrate/` to align with Pi extension best practices, fix race/robustness bugs, and improve LLM operability/visibility.

## Proposed changes

### P0 (Critical)
1. Fix phase transition execution flow
   - Ensure `transition_phase` persists state updates and always awaits downstream phase dispatch.
2. Serialize workflow-state mutations
   - Wrap all YAML workflow writes in `withFileMutationQueue(statePath, ...)`.
   - Remove the custom lock-based queue that swallows errors.
3. Improve tool error signaling
   - Throw `Error(...)` for validation/parsing failures; do not return `{ isError: true }`.

### P1 (High)
4. Adopt typed extension imports and schemas
   - Import `ExtensionAPI` from `@mariozechner/pi-coding-agent`.
   - Replace raw JSON schema definitions with `Type.Object(...)` and `StringEnum(...)` where applicable.
5. Improve discoverability
   - Add `promptSnippet` and `promptGuidelines` to custom tools.
   - Add `prepareArguments` compatibility shim if needed.
6. Add persistence and discovery hooks
   - Restore workflow context in `session_start` from extension custom entries.
   - Register phase prompt path(s) in `resources_discover`.

### P2 (Medium)
7. Add proactive context injection
   - Use `before_agent_start` to inject current task/phase context.
8. Add output safety
   - Truncate long phase instructions with `truncateHead`; save full content to temp file when truncated.
9. Add status visibility
   - Use `ctx.ui.setStatus` / `setWidget` for active task and phase.

### P3 (Nice)
10. Add custom tool rendering
    - Add compact `renderCall`/`renderResult` with status transitions and history.
11. Improve Linear bridge resilience
    - Add prompt metadata (`promptSnippet`) to re-registered Linear tools.
    - Refresh linear tool registrations on session events (and guard duplicates).

## Acceptance Criteria
- Workflow tool calls never race when invoked in parallel tool turns.
- Tool failures are surfaced as tool errors (`isError=true`) via thrown exceptions.
- `update_workflow_state`/`transition_phase` appear clearly in tool guidance and tool result rendering.
- Session restarts/reloads preserve current-orchestrate context in extension state.
- Long phase instructions do not overflow default output limits without truncation notices.
- Linear bridge tools are discoverable and register reliably at startup/session starts.
