# Migrating from lok to loker

This guide maps legacy `lok` concepts and commands to current `loker` behavior for M9 users.

## At a glance

`loker` is a hard fork of `lok` with a familiar single-shot experience (`ask`, `hunt`, `audit`, `diff`) plus first-class workflow orchestration (`workflow`, `run`, `resume`, `trace`, etc.).

## Concept mapping

| lok concept | loker equivalent | Notes |
|---|---|---|
| Command surface | legacy `ask`/`hunt`/`audit`/`diff` + expanded orchestration verbs | The legacy single-command ergonomics remain; orchestration gained new commands. |
| Config file | `lok.toml` | Same discovery model remains: `loker` reads project `lok.toml` (and ancestor config fallbacks). No hard rename yet. |
| Workflow definitions | `.lok/workflows/` (project) and `~/.config/lok/workflows/` (user) + embedded defaults | Same lookup order and override pattern as legacy `lok`, preserved in `loker` for compatibility. |
| Workflow phases | Implicit one-shot execution in legacy `lok` | `design`, `plan`, `implement`, and `verify` phases with per-phase resume/restart semantics |
| Backend model family | `[backends]` in `lok.toml`, including subprocess backends and optional `tensorzero/<id>` families | See the backend strategy examples in `README.md` (CLI + TensorZero notes). |
| Run artifacts | `runs/<run_id>/...` | `run_id` directories hold planner/output traces, manifests, and checkpoints used by resume and trace workflows. |

## Legacy command translation

| Legacy lok command | loker equivalent | Notes |
|---|---|---|
| `lok ask "<prompt>"` | `loker ask "<prompt>"` | Preserved. |
| `lok hunt .` | `loker hunt .` | Preserved. |
| `lok audit .` | `loker audit .` | Preserved. |
| `lok diff main..HEAD` | `loker diff main..HEAD` | Preserved (supports same positional diff spec style). |
| `lok workflow run` | `loker workflow run` | Preserved path for workflow execution in modern CLI structure. |
| `lok workflow list` | `loker workflow list` | Preserved listing command. |
| `lok workflow validate <path>` | `loker workflow validate <path>` | Preserved file validation entrypoint. |

## New in loker (no one-to-one legacy `lok` equivalent)

- `loker run <workflow>` — shorthand orchestration entrypoint (`workflow run`).
- `loker workflow` (as a command family with `run`/`list`/`validate`).
- `loker resume <run_id>` — continue a partially completed run.
- `loker trace <run_id>` — pretty-print `trace.jsonl` (with `--json`, `--color <auto|always|never>`).
- `loker explain`, `loker backends`, `loker doctor`, `loker context`, `loker report`, `loker fix`, `loker ci`, `loker pr`, `loker conduct`, `loker debate`, `loker suggest`, `loker smart`, `loker team`, `loker spawn`, `loker ls --blocked`, `loker init`, `loker spec`, `loker implement`.

## Breaking changes and compatibility notes

- No breaking changes were introduced for the canonical `lok` workflow commands in this migration scope (`ask`, `hunt`, `audit`, `diff`, and workflow validation/list/run commands above).
- New orchestration commands (`run`, `resume`, `trace`, `workflow`, etc.) are additive and intentionally scoped to modern `loker` pipelines.
- Migration behavior and syntax are validated from live CLI help output because option names and positional arguments can drift across releases.

## Not ported (or intentionally undefined)

The following legacy-era behaviors are not documented as guaranteed compatibility in this PR:

- Any non-command surface assumption about third-party shell aliases or wrappers outside the documented `loker` commands.
- Behavior not explicitly validated via command help in this document.
- Commands introduced after the lok era and listed above under “New in loker”.

## Compatibility and rollout posture

### Compatibility surface

- `lok.toml`: still in place and read for project/workflow behavior.
- `.lok/workflows/`: still in place and looked up for local workflow overrides.

### Deprecation window

`lok.toml` and `.lok/workflows/` are still in use. The handoff notes that these names are kept during a config-rename milestone with a planned deprecation window after that milestone.
This task only documents the current state and does not invent concrete end-of-life dates.

## Verification appendix

Commands were validated against this branch using `cargo run -- <command> --help` (local binary built from source):

- `loker --help`
- `loker ask --help` — `Usage: loker ask [OPTIONS] <PROMPT>`
- `loker hunt --help` — `Usage: loker hunt [OPTIONS] [DIR]`
- `loker audit --help` — `Usage: loker audit [OPTIONS] [DIR]`
- `loker diff --help` — `Usage: loker diff [OPTIONS] [SPEC]`
- `loker run --help` — `Usage: loker run [OPTIONS] <NAME> [ARGS]...`
- `loker workflow --help` — `Usage: loker workflow [OPTIONS] <COMMAND>`
- `loker workflow run --help` — `Usage: loker workflow run [OPTIONS] <NAME> [ARGS]...`
- `loker workflow list --help` — `Usage: loker workflow list [OPTIONS]`
- `loker workflow validate --help` — `Usage: loker workflow validate [OPTIONS] <PATH>`
- `loker resume --help` — `Usage: loker resume [OPTIONS] <RUN_ID>`
- `loker trace --help` — `Usage: loker trace [OPTIONS] <RUN_ID>`
- `loker explain --help` — `Usage: loker explain [OPTIONS] [TARGET]`
- `loker backends --help` — `Usage: loker backends [OPTIONS]`
- `loker doctor --help` — `Usage: loker doctor [OPTIONS]`

> This list is source-of-truth for the examples below; run these commands again before each migration-bridge release if needed.

Verified artifacts (non-executable):
- `README.md` contains the discoverability callout (ST3).

## Quick upgrade summary

1. Continue using `ask`, `hunt`, `audit`, `diff` first.
2. For orchestrated work, prefer `loker run` / `loker workflow run`.
3. Re-check signatures with `<command> --help` whenever upgrading across milestones.
