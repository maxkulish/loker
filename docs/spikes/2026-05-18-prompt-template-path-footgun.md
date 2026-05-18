# prompt_template path-vs-inline footgun (task-kickoff run failure)

Date: 2026-05-18
Status: diagnosed, no code change yet
Repro: `~/Code/mentis--feat-menti-68-leadership/runs/task-kickoff-20260516-214909-741e72df/`

## TL;DR

`prompt_template` in `[[phases]]` is the **literal template source string**, not a path
to a `.tmpl` file. Loker never reads `.tmpl` files. Pointing it at a path makes the
path itself the prompt; backends then improvise from a bare filename. Every visible
symptom in the mentis MENTI-68 kickoff run traces to this one mistake.

## What was observed

Run dir `runs/task-kickoff-20260516-214909-741e72df/` produced three semantically
broken artifacts:

- `design.md` (610 B): a meta description of the design template ("This is the
  design phase prompt template for the task-kickoff workflow..."). No MENTI-68
  content.
- `review.md` (164 B, picked from `attempts/1-parallel.txt`): empty preamble,
  "I'll open the referenced kickoff review template...".
- `plan.md` (5052 B): recognized the chain was broken and pivoted to a smoke-test
  plan **of the workflow itself**, not a plan for MENTI-68.

`manifest.json` chose `review.md` from attempt 3 (ollama, 3373 B, generic
hallucinated template). The review fan-out shows what each backend did with the
mystery input:

- claude → opened the .tmpl file via its filesystem tools and described it.
- codex → returned a one-liner about the file path.
- gemini → similar meta response.
- ollama (`glm-5.1:cloud`) → no FS access, hallucinated a generic review template
  from the filename. The only attempt long enough to win the aggregator.

## Root cause

The workflow at `.loker/workflows/task-kickoff.toml` was authored assuming
`prompt_template` could be a relative path:

```toml
prompt_template = "../prompts/task-kickoff/design.md.tmpl"
```

Loker does not support that. The data flow:

1. `src/workflow/grammar.rs:73` — `pub prompt_template: String`. Doc comment
   line 71: "The prompt template source string." Doctest at line 18:
   `prompt_template = "Analyze {{ spec }}"`. TOML deserializer stores the path
   verbatim as the field value.

2. `src/workflow/phase_bridge.rs:185` — `Template::render(&phase.prompt_template,
   &tmpl_ctx)`. The regex engine scans for `{{ ... }}`. A path has none, so
   render returns the string **unchanged**. No error, no warning.

3. `src/phase_runner/dispatch.rs` — `cfg.prompt_template` (still the literal
   path string) is passed into `SingleModel::new` / `ParallelFanOut::new` and
   dispatched as the prompt to every backend.

No code path reads `.tmpl` files. The mental model "prompt_template is a path"
is consistently wrong; every existing test in `tests/workflow_grammar.rs` and
the phase_bridge unit tests uses inline strings (`"Design {{ spec }}"`,
`"Judge"`, `"Code"`, etc.). The wrong-shape input has no coverage, so the
silent no-op rendering is invisible until a real run blows up.

## Why the symptoms map exactly

Each backend received the string `"../prompts/task-kickoff/design.md.tmpl"`
(or the review/plan equivalent) as its entire prompt:

- claude has filesystem tools → opened the file, meta-described it.
- codex/gemini partially read the file or stubbed out a response.
- ollama hallucinated from the filename only.
- The plan phase saw `{{ phase.design.output }}` = "meta description" and
  `{{ phase.review.output }}` = "empty preamble", recognized the chain was
  broken, and silently pivoted to a workflow smoke-test plan.

## Fix — proposed direction

Ship **Option 1 + Option 3 together**:

### Option 1 (recommended): add `prompt_template_file`

Add a sibling field in `grammar::Phase`:

```rust
#[serde(default)]
pub prompt_template_file: Option<String>,
```

At workflow load (where the TOML file path is known), if `prompt_template_file`
is set, resolve it relative to the workflow file's directory, read it, and
populate `prompt_template` from disk. Reject the case where both are set.
Error clearly if the path escapes the workspace.

Author UX:

```toml
[[phases]]
name = "design"
strategy = { single = {} }
backends = ["claude/"]
prompt_template_file = "../prompts/task-kickoff/design.md.tmpl"
inputs = ["spec"]
output = "design.md"
```

### Option 3: validate and reject path-shaped `prompt_template`

In grammar validation, reject `prompt_template` values that:

- contain no `{{ ... }}` placeholders, AND
- match a path heuristic (relative or absolute, ends with `.tmpl`, or
  contains `/`).

Emit a clear error: *"prompt_template is the inline template source, not a
file path. Inline the contents, or use prompt_template_file."*

### Option 2 (rejected): auto-detect path semantics

Same field, auto-decide based on path heuristics. Ambiguous (what if someone
wants the literal string `"foo/bar"` as a prompt?). Don't ship.

### Bonus lint

A workflow-load-time warning when a phase's rendered `prompt_template` has
zero `{{ ... }}` placeholders despite declaring `inputs = [...]` would have
caught this in one second.

## Immediate unblock (no loker change)

Either:

1. Inline the template content into `.loker/workflows/task-kickoff.toml`
   using TOML triple-quoted strings (`prompt_template = """..."""`). Today's
   behavior matches grammar; the run will succeed.
2. Or wait for Option 1 to ship.

Discard the broken `runs/task-kickoff-20260516-214909-741e72df/` first —
none of its artifacts are usable for MENTI-68.

## Key file refs

- `src/workflow/grammar.rs:60-89` — `Phase` struct, `prompt_template: String`.
- `src/workflow/phase_bridge.rs:185` — `Template::render` call site.
- `src/workflow/template.rs` — strict regex engine for `{{ spec }}`,
  `{{ phase.NAME.output }}`, `{{ var.X }}`.
- `src/phase_runner/dispatch.rs` — where `cfg.prompt_template` is forwarded
  to strategies.
- `tests/workflow_grammar.rs:111-295` — every existing test uses inline strings.

## Next actions

- [ ] File a CLO ticket for Option 1 + Option 3.
- [ ] Add a regression test that exercises a real `.tmpl` file (either via
      `prompt_template_file` once it lands, or as a should-fail test for the
      Option 3 validator).
- [ ] Add the missing-placeholder lint warning.
