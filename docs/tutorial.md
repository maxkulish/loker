# loker Tutorial — From Clone to First Run

This guide takes you from `git clone` through a successful first workflow run to inspecting the results. You'll explore a workflow without LLMs, run a simple workflow that reads the calculator example spec, find the run directory, and inspect `manifest.json`. It should take under 10 minutes.

## Prerequisites

- **Rust** toolchain (`cargo` ≥ 1.80)
- **Git**
- *(Optional)* **Ollama** for LLM-backed steps

## Install

```bash
git clone https://github.com/maxkulish/loker.git
cd loker
cargo build --release
```

```text
   Compiling loker v20260504.0.0
    Finished release [optimized] target(s) in 23.86s
```

Verify the CLI works:

```bash
cargo run -- --help
```

```text
Usage: loker [OPTIONS] <COMMAND>

Commands:
  ask        Ask LLM backends a question
  audit      Run a security audit on a codebase
  run        Run a workflow
  explain    Explain a workflow DAG
  trace      Pretty-print the trace.jsonl from a run directory
  doctor     Check which backends are available and ready
  ...
```

## Check your setup

```bash
cargo run -- doctor
```

```text
Lok Doctor
==================================================

Checking backends...

  ✓ codex - ready
  ✓ gemini - ready
  ✓ claude - ready

Checking API keys...

  ○ ANTHROPIC_API_KEY - not set (claude backend)
  ○ GOOGLE_API_KEY - not set (gemini backend)

✓ 3 backend(s) ready.
```

No API keys are needed — the first workflow is shell-only.

## Explore a workflow (no backends needed)

`loker explain` prints a workflow's structure without executing it.

```bash
cargo run -- explain design-doc-tdd
```

```text
Workflow: design-doc-tdd
Description: Four-phase design → review → implement → verify pipeline.

Phase order:
  1. design
  2. review (depends on: design)
  3. implement (depends on: design, review)
  4. verify (depends on: implement, design)

Phases:
  design    strategy: single     output: design.md
  review    strategy: parallel   output: review.md
  implement strategy: escalating output: changes
  verify    strategy: parallel   output: verify.json
```

A workflow is a DAG of phases, each with a strategy, backends, and inputs/outputs.

## Run your first workflow

The tutorial workflow reads the calculator example spec.

```bash
cargo run -- run examples/workflows/calculator-tutorial.toml
```

```text
✓ Run directory: runs/examples-workflows-calculator-tutorial-toml-20260507-074325-6baf6064
Running workflow: calculator-tutorial
==================================================

[step] read_spec
  ✓ (0.0s)
[step] count_sections
  ✓ (0.0s)

Results:

[OK] read_spec (0.0s)
  --- Calculator Spec ---
  # Calculator Library Specification
  A minimal calculator library providing pure, deterministic arithmetic operations.
  ...
  --- End of Spec ---

[OK] count_sections (0.0s)
  Sections found:
  ## Problem Statement
  ## Requirements
  ## Constraints
  ## Out of Scope
  ## Acceptance
```

*(Output truncated for brevity.)*

The spec at `examples/specs/calculator.md` defines a minimal Rust calculator library. It was written in [CLO-290](https://linear.app/cloud-ai/issue/CLO-290) and serves as the canonical small input for loker's workflow pipeline.

## Locate the run directory

Every `loker run` creates a directory under `runs/`:

```bash
ls runs/
```

```text
examples-workflows-calculator-tutorial-toml-20260507-074325-6baf6064
```

Naming: `runs/<workflow_slug>-<YYYYMMDD-HHMMSS>-<short_uuid>/`.

Read the manifest:

```bash
cat runs/examples-workflows-calculator-tutorial-toml-20260507-074325-6baf6064/manifest.json
```

```json
{
  "loker.run_id": "6baf6064-dc68-46d0-a047-c6396a23a567",
  "schema_version": 1,
  "workflow_name": "examples/workflows/calculator-tutorial.toml",
  "entries": []
}
```

- **`loker.run_id`** — UUID that uniquely identifies this run
- **`schema_version`** — format version
- **`workflow_name`** — the workflow that produced this run
- **`entries`** — artefacts from each phase (populated by phase-based workflows)

## Read the trace

`trace.jsonl` contains OpenTelemetry GenAI spans: backend calls, token usage, phase outcomes. It is written by **phase-based** workflows. The step-based runner creates the run directory and manifest, but does not yet emit `trace.jsonl` — wiring lands in [T-041](https://linear.app/cloud-ai/issue/CLO-315).

When present, read it with:

```bash
cargo run -- trace <run_directory_name>
```

Example pretty-printed output:

```text
[20:45:00] design phase 12.4s
[20:45:01] design backend anthropic 9.9s 1.8k+712
[20:45:02] design verify make_check 320ms [verify_passed]
[20:45:04] implement phase 5.6s [strategy_failed]
[20:45:05] implement backend openai 1.5s 512+256
[20:45:06] implement backend anthropic 3.8s 1.0k+512
```

Stream raw JSONL with `cargo run -- trace <run_id> --json`.

## Next steps

- Read [`docs/handoff.md`](docs/handoff.md) for project intent and architecture.
- List workflows with `cargo run -- workflow list`.
- Run a security audit with `cargo run -- audit .` (requires an API key).
- See `cargo run -- --help` for every command and flag.

*Completed the tutorial? You now know how to clone loker, run a workflow, and inspect the run directory. Welcome aboard.*
