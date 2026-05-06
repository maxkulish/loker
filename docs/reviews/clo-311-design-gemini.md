# Gemini Design Review: CLO-311

## Verdict

approve_with_changes

## Summary

The design is sound and aligns with the discovery decision: implement `loker explain <workflow>` as workflow-first behavior on the existing top-level `explain` command while preserving codebase explanation fallback. It correctly avoids backend initialization and reuses existing phase grammar validation.

## Findings

### G1 - Clarify phase-based vs legacy step-based workflow handling

The design should explicitly state that `loker explain <workflow>` targets phase-based TOML (`[[phases]]`) for FR-34. If a workflow source resolves but parses only as the legacy step-based format, the command should produce a clear unsupported/invalid message rather than silently rendering an empty or misleading summary.

### G2 - Resolve lookup-root decision before implementation

The design leaves `find_workflow_in(name, dir)` as an open implementation choice. Since deterministic `--dir` behavior is important and async `set_current_dir` is unsafe, this should be settled in the design: add `find_workflow_in`/`find_workflow_source_in` and use it from explain.

### G3 - Make source text loading explicit

`workflow::load_workflow_from_source()` currently returns the legacy `workflow::Workflow`, so the design should emphasize that explain reads the raw source text and parses it with `workflow::grammar::Workflow` directly.

### G4 - Keep fallback tests narrow

A full codebase-explain fallback snapshot may be brittle because backend-selected explanation output can vary. Prefer a narrow CLI assertion that a directory target does not enter workflow mode, or defer to existing tests if available.
