# Pre-PR validation: clo-309

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-04
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [medium] Top-level `Run` help text misrepresents the engine
**Where:** src/main.rs:350-353
**What:** Doc comment claims "Run a workflow with the new phase-based engine (CLO-309)", but the command still routes to the step-based `WorkflowRunner` (the design and plan explicitly defer phase-based wiring to T-041). Users running `loker run --help` will be misled.
**Suggested fix:** Replace with something like "Run a workflow. Step-based today; phase-based wiring lands in T-041. Supports --spec, --var, --rerun (rerun forwarded for forward compatibility).”

### F2 [medium] `--rerun` prints "Forcing re-execution" while doing nothing
**Where:** src/main.rs:1342-1352 (the `for phase_name in rerun_phases { println!(...) }` block)
**What:** The CLI emits `↻ Forcing re-execution of phase '<X>'` for each `--rerun` arg, but no markers are cleared and the step-based runner is stateless, so absolutely nothing is forced. The inline `// NOTE:` comment further claims "for the legacy step-based runner, we clear status markers directly here" — that is false (no clearing happens). This is misleading UX and dead code with an incorrect comment.
**Suggested fix:** Either drop the loop entirely (silent forward-compat) or change the message to be honest, e.g. `↻ --rerun phase='X' accepted (no-op for step-based runner; effective once phase-based runner is wired in T-041)`. Delete the misleading inline note.

### F3 [low] `WorkflowRunner.rerun_phases` field is set but never read
**Where:** src/workflow/mod.rs:1546, 1581-1584
**What:** `with_rerun_phases()` stores `rerun_phases: Vec<String>` on `WorkflowRunner`, but no code path consumes the field. It's effectively dead state. clippy doesn't warn because the field is written through the builder, but it adds noise and an implicit "we did something" signal that isn't true.
**Suggested fix:** Annotate `rerun_phases` with `#[allow(dead_code)]` and a one-line `// Reserved for T-041 (phase-based runner marker deletion).` so reviewers understand the intent. Same applies to `spec`/`vars` if they end up unused in any path — they are used in `interpolate_with_fields`, so OK.

### F4 [low] Top-level `Run` lost `#[command(trailing_var_arg = true)]`
**Where:** src/main.rs:354 (was on the prior `Run` in main; still present on `WorkflowCommands::Run` at src/main.rs:462)
**What:** The pre-CLO-309 `Run` was a shorthand with `trailing_var_arg = true`, allowing `loker run wf arg1 --some-flag-arg arg2` style invocations to collect everything past `wf` as positional args. The new definition drops it; `allow_hyphen_values = true` on `args` only partially compensates. Behavior diverges between `loker run` and `loker workflow run` for hyphen-prefixed positionals.
**Suggested fix:** Re-add `#[command(trailing_var_arg = true)]` to keep parity with `WorkflowCommands::Run`, or document the intentional divergence. Quick parity check: run `loker run wf -- --foo bar` vs. `loker workflow run wf -- --foo bar` and confirm both still produce `args = ["--foo", "bar"]`.

### F5 [low] `test_run_rerun_flag` does not actually exercise rerun semantics
**Where:** tests/run_cli.rs:113-153
**What:** Both runs invoke the stateless step-based runner via fresh `RunDir::create`, so all three `phase_*` steps execute every time regardless of `--rerun`. The assertions on the second run would pass even if `--rerun` were entirely removed. This gives a false signal of working rerun behavior. Accepted by design (no-op for step-based) but the test name and assertions advertise more than they verify.
**Suggested fix:** Add a code comment in the test stating "smoke test only — rerun semantics deferred to T-041", and assert at minimum that the CLI accepts the flag without warning. Optionally rename to `test_run_rerun_flag_accepted`.

### F6 [low] Test workflows use phase-flavored names with `[[steps]]`
**Where:** tests/workflows/test_run_rerun.toml, src/main.rs:367 (`--rerun phase=design` doc example)
**What:** The rerun test workflow declares `[[steps]]` named `phase_one`, `phase_two`, `phase_three`. It is conceptually inconsistent (steps named like phases) and may confuse future maintainers reading the file alongside the actual `[[phases]]` grammar in `src/workflow/grammar.rs`.
**Suggested fix:** Rename steps to `step_one`, `step_two`, `step_three` and adjust the rerun arg + assertions accordingly, or add a top-of-file comment explaining the placeholder naming until T-041.

### F7 [low] Spec content is double-cloned per template render
**Where:** src/workflow/mod.rs:2910-2916, src/template/context.rs:64-69, 140-142
**What:** `interpolate_with_fields` calls `self.spec.clone()` (clones the `Option<String>`) and passes it by value. Inside `new_with_extras`, `Value::from(spec_content.clone())` clones again. For multi-step workflows with a multi-KB spec, this is two heap copies per step. Functionally correct; just wasteful.
**Suggested fix:** Change `new_with_extras` signature to `spec: Option<&str>` and pass `self.spec.as_deref()`. Use `Value::from(s)` (MiniJinja's `Value::from(&str)` already allocates internally once).

### F8 [low] No size guard on `--spec` file read
**Where:** src/main.rs:1325-1338
**What:** `tokio::fs::read_to_string(&abs_path)` will load arbitrarily large files into memory and then clone them into every template context. A user pointing `--spec` at a multi-GB file would OOM the process before any helpful error.
**Suggested fix:** Cap at, e.g., 1 MiB (or whatever value matches sibling tooling); on overflow return `anyhow::bail!("--spec file too large: {} bytes (max {})", ...)`. Low priority, but cheap to add.

### F9 [info] Documentation/safety: `{{ spec }}` in shell commands is injection-prone
**Where:** docs/designs/clo-309-run-cli.md §4.1, tests/workflows/test_run_spec*.toml
**What:** The example workflows interpolate raw spec content into single-quoted shell `echo` commands. A spec containing `'`, `$`, backticks, or `;` would break or hijack the shell command. Not a CLI bug per se (workflow author's responsibility), but the canonical examples teach the unsafe pattern.
**Suggested fix:** In the design doc, add a one-line note: "Prefer injecting `{{ spec }}` into LLM prompts or files written via `prompt_file`; avoid raw shell interpolation." No code change needed.

## Verdict
**approve_with_changes**

Implementation is correct and `make check` is green (fmt clean, clippy clean, 783 lib + 8 new integration tests pass). The diff matches the documented scope of CLO-309 (CLI flag surface only; phase-based marker deletion deferred to T-041 per the design). The blocking concerns are F1 (help text claims phase-based engine when only step-based is wired) and F2 (`↻ Forcing re-execution` prints a lie and ships a code comment that contradicts what the code does) — both are honesty/UX issues that will mislead the next reader. The remaining items are quality nits that can either land in this PR or in T-041's follow-up. Fix F1 and F2, optionally F4/F5, and this is mergeable.
