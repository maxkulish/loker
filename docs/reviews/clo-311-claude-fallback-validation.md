# Pre-PR validation: clo-311

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [minor] Workflow lookup error swallowed when target looks like a workflow name
**Where:** src/main.rs:1996-2005
**What:** When the user types `loker explain my-typo` (intended as a workflow name), `find_workflow_in` returns "Workflow 'my-typo' not found...". `run_explain_unified` silently discards that error and falls back to codebase mode with `codebase_dir = "my-typo"`, which then fails inside `run_explain` with a `canonicalize`/path error that doesn't mention "workflow not found." This degrades UX for a common typo case. The design explicitly chose codebase fallback when no workflow resolves, but the fallback should still hint at the workflow miss when the target isn't a directory.
**Suggested fix:** If `target` doesn't resolve as a workflow AND isn't an existing directory, surface the original `find_workflow_in` error instead of falling through. E.g., `if !codebase_dir.is_dir() { return Err(workflow_err); }` before delegating to `run_explain`.

### F2 [minor] `[[phases]]` substring heuristic is brittle and produces a misleading error
**Where:** src/workflow/explain.rs:44-49
**What:** The pre-parse check `text.contains("[[phases]]")` returns "supports phase-based workflows only" for any TOML lacking that literal substring — including commented-out `# [[phases]]`, a TOML file with `[[ phases ]]` (technically invalid), or a phase-based workflow that uses includes. Conversely, a Cargo.toml that happened to contain the substring inside a string would pass this gate and then fail with a more confusing TOML parse error. The intent was to distinguish legacy step-based workflows from phase-based ones.
**Suggested fix:** Drop the textual heuristic; attempt the phase-grammar parse first. If parsing succeeds with zero phases or with a `NoPhases`/`TomlParse` error AND the file has `[[steps]]`, return the "phase-based workflows only" message. Otherwise let the grammar's own error speak. This also avoids re-running the validate() pass already done by `FromStr`.

### F3 [minor] `explain_workflow` re-runs validation that `FromStr` already enforced
**Where:** src/workflow/explain.rs:86-89 and src/workflow/grammar.rs:317-326
**What:** `Workflow::from_str` calls `validate()` and only returns `Ok` if errors are empty. `explain_workflow` then immediately calls `workflow.validate()` again. Harmless but wasteful; both run twice on every successful `loker explain`. Defensive code is fine but should be commented as such, or skipped on the inner path.
**Suggested fix:** Remove the re-validation in `explain_workflow` (callers must pass a validated `&grammar::Workflow`), or split into `explain_workflow_unchecked` for the post-FromStr path and keep `explain_workflow` for direct callers.

### F4 [nit] Snapshot path is OS-dependent
**Where:** tests/explain_cli.rs:29 (`Source: ./.lok/workflows/design-doc-tdd.toml`)
**What:** The displayed source comes from `Path::new(".").join(".lok/workflows").join("design-doc-tdd.toml").display()`. On Windows this becomes `.\.lok\workflows\design-doc-tdd.toml` and the inline snapshot diverges. Plan risk #4 explicitly flagged this.
**Suggested fix:** Either (a) gate the snapshot test with `#[cfg(unix)]`, or (b) normalize separators in the source string before rendering, or (c) prefer a `WorkflowSource::display_name()` method that always uses forward slashes / a stable canonical form.

### F5 [nit] CLI flag arity changed silently in `--help`
**Where:** src/main.rs:439-454
**What:** The `Explain` subcommand previously had a positional `dir`; it is now a positional `target` plus `--dir/-d`. The doc-comment says "Workflow name/path to explain, or directory for codebase explanation," which is good, but anyone with an alias like `loker explain ./somedir` keeps working only because workflow lookup falls through to codebase mode. Worth a note in CHANGELOG/handoff.
**Suggested fix:** Add a one-line entry to docs/handoff.md or the M7/M8 status doc noting that `loker explain` now accepts a workflow name as the first positional, and that `--dir`/`-d` is the canonical way to point at a codebase. No code change required.

### F6 [nit] `WorkflowSource` derives `Clone` but no caller clones
**Where:** src/workflow/mod.rs:3453 (the new `#[derive(Debug, Clone)]`)
**What:** `Debug` is required by the new test's `panic!("got {other:?}")`. `Clone` is added but unused — adds API surface without need. The CLAUDE.md "don't add features beyond what the task requires" guideline applies.
**Suggested fix:** Drop `Clone` from the derive list unless a caller actually needs it. Keep `Debug`.

## Verdict
approve_with_changes

The implementation matches the approved design closely: `find_workflow_in` adds proper base-dir handling with a strictly-better `is_file()` check, the new `workflow::explain` module renders stable plain text, and unit + integration coverage exercises all five sub-task acceptance criteria. `cargo build`, `cargo clippy --all-targets`, the targeted `workflow::explain` unit tests, the new `find_workflow_in` test, and the full `explain_cli` integration suite all pass cleanly. No regressions to the legacy step-workflow path or to other `find_workflow` callers (the new `is_file()` gate only rejects directories that previously would have been mis-resolved). Findings F1–F6 are non-blocking polish: F1 and F2 are UX clarity improvements worth landing before close, F3 is a small efficiency cleanup, F4 prevents a future Windows-CI surprise, F5 is documentation, F6 is an API-surface trim. None require redesign; the branch is ready to merge once F1 and F2 are addressed (or explicitly waived).
