# Design: `loker run <workflow>` CLI Subcommand Flags (CLO-309 / T-040)

## 1. Problem

The `loker run <workflow>` CLI command is the primary entry point for Slice B, but it lacks
three critical flags that the phase-based workflow engine expects:

1. **`--spec <path>`** — Injects a spec/requirements file into `{{ spec }}` template placeholders.
2. **`--var key=value`** — Injects template variables into `{{ var.<name> }}` placeholders (repeatable).
3. **`--rerun phase=<name>`** — Forces re-execution of a completed phase by clearing status markers (repeatable).

Without these flags, users cannot supply spec files to workflows, pass dynamic variables into
prompt templates, or re-run specific phases of a pipeline from the command line.

## 2. Goals / Non-goals

### Goals

- `loker run <workflow> --spec <path>` injects the spec file content into `{{ spec }}`.
- `loker run <workflow> --var key=value` injects `{{ var.<name> }}` into template contexts.
- `loker run <workflow> --rerun phase=<name>` forces re-execution of a phase.
- Malformed input is rejected with clear error messages.
- Integration tests cover every flag combination against a mock-backend workflow.
- All existing tests (`make check`) remain green.

### Non-goals

- Phase-based runner auto-detection (detecting `[[phases]]` vs `[[steps]]`). This is deferred to T-041.
- Marker deletion for `--rerun` in the phase-based conductor. This is deferred to T-041.
- `loker resume` improvements — covered by T-041.
- `loker explain` — covered by T-042.
- `loker trace` — covered by T-043.

## 3. Architecture

### 3.1 Module layout

All changes land in existing files:

```
src/
  main.rs                  ← MODIFIED: CLI flags + run_workflow() args
  workflow/
    mod.rs                 ← MODIFIED: with_spec/with_vars/with_rerun_phases builders
  template/
    context.rs             ← MODIFIED: spec + var namespaces in TemplateContext
tests/
  run_cli.rs               ← NEW: integration tests for all flag combinations
tests/workflows/
  test_run_spec.toml       ← NEW: spec-only test workflow
  test_run_vars.toml       ← NEW: var-only test workflow
  test_run_spec_var.toml   ← NEW: combined spec+var test workflow
  test_run_rerun.toml      ← NEW: rerun test workflow
tests/test_specs/
  test_spec.md             ← NEW: sample spec file
```

### 3.2 Data flow

```
CLI: loker run <workflow> [--spec <path>] [--var k=v]... [--rerun phase=<name>]...
  │
  ├─→ clap parses flags:
  │      spec: Option<PathBuf>
  │      var: Vec<KeyValue>
  │      rerun: Vec<String>
  │
  ├─→ run_workflow():
  │      ├─→ Read spec file if --spec provided
  │      ├─→ Build template_vars map from --var flags
  │      ├─→ Create WorkflowRunner with .with_spec().with_vars().with_rerun_phases()
  │      │
  │      ├─→ WorkflowRunner::run():
  │      │      ├─→ For each step:
  │      │      │      ├─→ interpolate_with_fields() uses TemplateContext::new_with_extras()
  │      │      │      │      └─→ spec and vars available as {{ spec }}, {{ var.<name> }}
  │      │      │      └─→ Execute step (shell or LLM backend)
  │      │      └─→ Return Vec<StepResult>
  │      │
  │      └─→ Print results / write to output file
  │
  └─→ Exit code: 0 on success, non-zero on any step failure
```

### 3.3 Concrete types

```rust
// Custom value parsers (src/main.rs)
struct KeyValue { key: String, value: String }

fn parse_key_val(s: &str) -> Result<KeyValue, String>
fn parse_rerun_phase(s: &str) -> Result<String, String>

// WorkflowRunner builder extensions (src/workflow/mod.rs)
impl WorkflowRunner {
    pub fn with_spec(self, spec: Option<String>) -> Self;
    pub fn with_vars(self, vars: HashMap<String, String>) -> Self;
    pub fn with_rerun_phases(self, phases: Vec<String>) -> Self;
}

// TemplateContext extension (src/template/context.rs)
impl TemplateContext {
    pub fn new_with_extras(
        steps: &HashMap<String, StepResult>,
        args: &[String],
        backends: &[String],
        spec: Option<String>,
        vars: &HashMap<String, String>,
    ) -> Self;
}
```

## 4. Template variable resolution

### 4.1 `{{ spec }}`

- Provided via `--spec <path>` flag.
- The file is read at startup in `run_workflow()`.
- Content is stored as `WorkflowRunner.spec: Option<String>`.
- Passed to `TemplateContext::new_with_extras()` which inserts it as a top-level `spec` key.
- If `--spec` is not provided, `{{ spec }}` in a template produces an undefined-variable error.

### 4.2 `{{ var.<name> }}`

- Provided via `--var key=value` flags (repeatable).
- Parsed by `parse_key_val()` which splits on the first `=`.
- Stored as `WorkflowRunner.vars: HashMap<String, String>`.
- Passed to `TemplateContext::new_with_extras()` which inserts a `var` namespace object.
- The `var` namespace is always present (even when empty) so `{{ var.foo }}`
  can be used with `default()` or `is defined` guards, rather than erroring
  when `var` itself is undefined.

### 4.3 `{{ phase.<name>.output }}`

- Phase-based template syntax (from `src/workflow/template.rs`).
- The step-based runner does NOT support this syntax — it uses `{{ steps.<name>.output }}`.
- Phase output references are handled by the phase-based workflow engine (T-028/T-029).

## 5. --rerun semantics

### 5.1 Step-based runner (current implementation)

The step-based `WorkflowRunner` is stateless — it always executes all steps from scratch.
The `--rerun` flag is accepted and validated at the CLI level but has no effect on the
step-based runner. It is valid syntax for forward compatibility with the phase-based runner.

### 5.2 Phase-based runner (future)

When the phase-based runner is wired to `loker run` (planned for T-041), `--rerun phase=X` will:
1. Delete `runs/<id>/markers/X.completed` if it exists.
2. Delete `runs/<id>/markers/X.failed` if it exists.
3. Delete `runs/<id>/markers/X.started.*` if any exist (clean slate).
4. Archive the current attempt directory as `runs/<id>/attempts/X/<n>/`.
5. Re-execute phase X and all downstream phases that depend on X's output.

## 6. Validation rules

| Input | Behavior |
|-------|----------|
| `--var key=value` | Valid. Injects `{{ var.key }}` = `"value"`. |
| `--var =value` | Rejected: "Empty key in --var '=value'" |
| `--var key` (no `=`) | Rejected: "Invalid --var format: 'key' (expected key=value)" |
| `--rerun phase=name` | Valid. Marks `name` for re-execution. |
| `--rerun badformat` | Rejected: "Invalid --rerun format: 'badformat' (expected phase=<name>)" |
| `--rerun phase=` | Rejected: "Empty phase name in --rerun 'phase='" |
| `--spec <missing-file>` | Rejected with IO error: "Failed to read spec file: <path>" |

## 7. Test contract

### Integration tests (`tests/run_cli.rs`)

| Test | Workflow | Flags | Expected |
|------|----------|-------|----------|
| `test_run_spec_flag` | `test_run_spec.toml` | `--spec <spec.md>` | `{{ spec }}` interpolated |
| `test_run_var_flag` | `test_run_vars.toml` | `--var foo=hello --var bar=world` | `{{ var.foo }}` and `{{ var.bar }}` interpolated |
| `test_run_spec_and_var_combined` | `test_run_spec_var.toml` | Both `--spec` and `--var` | Both `{{ spec }}` and `{{ var.* }}` work |
| `test_run_rerun_flag` | `test_run_rerun.toml` | `--rerun phase=phase_two` | All phases execute (rerun accepted) |
| `test_run_invalid_var_format` | Any | `--var invalidformat` | Error: "expected key=value" |
| `test_run_invalid_rerun_format` | Any | `--rerun badformat` | Error: "expected phase=" |
| `test_run_empty_var_key` | Any | `--var =value` | Error: "Empty key" |
| `test_run_empty_rerun_phase` | Any | `--rerun phase=` | Error: "Empty phase name" |

### Pre-merge gate

```
make check    # fmt + clippy + test (includes run_cli.rs)
```

## 8. Files modified

| File | Change |
|------|--------|
| `src/main.rs` | Added `--spec`, `--var`, `--rerun` flags to `Run` subcommand. Added `KeyValue` struct, `parse_key_val()`, `parse_rerun_phase()`. Updated `run_workflow()` signature. |
| `src/workflow/mod.rs` | Added `spec`, `vars`, `rerun_phases` fields to `WorkflowRunner`. Added `.with_spec()`, `.with_vars()`, `.with_rerun_phases()` builders. Updated `interpolate_with_fields()` to pass spec/vars to context. |
| `src/template/context.rs` | Added `new_with_extras()` constructor. Added `{{ spec }}` and `{{ var.<name> }}` namespaces. Always expose `var` namespace (even empty). |
| `tests/run_cli.rs` | NEW — 8 integration tests for all flag combinations. |
| `tests/workflows/test_run_spec.toml` | NEW — simple workflow with `{{ spec }}`. |
| `tests/workflows/test_run_vars.toml` | NEW — simple workflow with `{{ var.* }}`. |
| `tests/workflows/test_run_spec_var.toml` | NEW — workflow with both `{{ spec }}` and `{{ var.* }}`. |
| `tests/workflows/test_run_rerun.toml` | NEW — workflow for rerun test. |
| `tests/test_specs/test_spec.md` | NEW — sample spec file. |

## 9. Risks and mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| `{{ spec }}` and `{{ var.* }}` interfere with existing MiniJinja variable resolution | Low | Medium | `TemplateContext::new_with_extras()` extends the existing `new()` — existing callers use `new()` which delegates to `new_with_extras()` with `None`/empty defaults. No existing templates reference `spec` or `var.*`, so zero regression risk. |
| CLI flag parsing conflicts with existing `--var` usage patterns | Low | Low | The `--var` flag uses clap's `value_parser = parse_key_val` which integrates with clap's native error formatting. No existing loker flags use `--var`. |
