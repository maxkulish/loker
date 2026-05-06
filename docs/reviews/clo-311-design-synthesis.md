# Design Review Synthesis: CLO-311

## Verdict

approve_with_changes

## Applied suggestions

1. Clarify that workflow explain is for phase-based workflows and should error clearly on unsupported legacy step-only workflows.
2. Resolve the `--dir` lookup strategy in favor of a deterministic `workflow::find_workflow_in(name, dir)` helper instead of process-wide cwd changes.
3. Make explicit that the implementation reads raw workflow TOML and parses `workflow::grammar::Workflow` directly, not `load_workflow_from_source()`.
4. Narrow the fallback test recommendation to avoid brittle snapshots of existing codebase explanation output.

## Flagged suggestions

None.

## Rationale

The design is ready for planning after the above refinements. It scopes the implementation to static analysis and stable text rendering, reuses existing grammar validation, and preserves backward compatibility for the existing codebase `loker explain` command.
