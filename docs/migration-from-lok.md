# Migrating from lok to loker

This guide maps legacy `lok` concepts and commands to current `loker` equivalents for M9 users.

## At a glance

`loker` is a hard fork of `lok` and keeps the original command surface for common workflows (`ask`, `hunt`, `audit`, `diff`) while adding new workflow orchestration commands (`run` + `workflow`, `resume`, `explain`, `context`, `report`, etc.).

As of this writing, `lok.toml` and `.lok/workflows/` are still in use; no hard deprecation switch has shipped yet.

## Concept mapping

| lok concept | loker equivalent | Notes |
|---|---|---|
| Command surface | `ask`, `hunt`, `audit`, `diff`, `doctor`, `fix`, `ci`, `pr`, `report` | `ask/hunt/audit/diff` are preserved; other commands are added by the fork. |
| Workflow config file | `lok.toml` | Still read by `loker` via workspace/project config search. |
| Workflow definitions | `.lok/workflows/` | Still expected as workflow lookup location and command path. |
| Backend model family | Configured backends (e.g. `claude`, `codex`, `gemini`, `ollama`, `tensorzero/...`) | Existing subprocess backends are still supported; TensorZero family-named backends are available as part of migration. |
| Orchestration entrypoint | Workflow runner (`loker run` / `loker workflow run`) | New orchestration capabilities beyond legacy `lok` command set. |
| Artifacts | `runs/<id>/...` directory (CLI generated) | Artifacts are run-scoped with manifest and trace files where supported by workflow definitions. |

## Command translation

| Legacy lok command | loker equivalent | Notes |
|---|---|---|
| `lok ask "<prompt>"` | `loker ask "<prompt>"` | Preserved. |
| `lok hunt .` | `loker hunt .` | Preserved. |
| `lok audit .` | `loker audit .` | Preserved. |
| `lok diff main..HEAD` | `loker diff main..HEAD` | Preserved; behavior now aligned to current `loker` argument conventions. |
| `lok run ...` | `loker run ...` | New in loker: shorthand to `loker workflow run`. |
| `lok workflow run` | `loker workflow run` | Preserved in form and intent. |
| `lok workflow list` | `loker workflow list` | Preserved in form and intent. |
| `lok workflow validate <path>` | `loker workflow validate <path>` | Preserved in form and intent. |
| `lok resume <run_dir>` | `loker resume <run_dir>` | Added in loker docs/CLI as resume path for run scaffolding. |
| `loker init` | `loker init` | Initializes/creates `lok.toml` in the current project. |
| `loker backends` | `loker backends` | New command to list detected backend plugins. |
| `loker explain <dir-or-spec>` | `loker explain` | New orchestration-era command for architecture/codebase explanation. |

## Breaking changes and compatibility notes

- **`loker trace` is not present** in the currently installed CLI in this branch; command migration should use available commands only.
- **Resume/run behavior now runs through workflow execution paths** rather than only static review-like helpers; users should treat `loker run/workflow` as the primary path.
- **Read more than command names from `--help` output at migration time**: syntax and flags are source-of-truth and may drift across releases.

## Not ported (or intentionally changed)

The following legacy `lok`-era behaviors were intentionally narrowed to documented, shipped `loker` behavior:

- `lok`-era trace command parity is explicitly not documented here because the CLI in this environment exposes `ask/hunt/audit/diff/workflow` and related orchestration commands without `trace`.
- Any direct behavior assumptions not reflected in `loker --help` or current docs are intentionally omitted until the command is present and verified.

## Compatibility and rollout posture

### Compatibility surface

- `lok.toml`:
  - still used/config-driven behavior remains.
- `.lok/workflows/`:
  - still used for workflow files.
- Migration is documentation-only for this task; no runtime semantics change is included.

### Deprecation window

`lok.toml` and `.lok/workflows/` are still in use. The repo handoff notes that these names are kept during a config-rename milestone with a planned deprecation window for both names afterward. This task intentionally documents the current names without inventing new cut-off dates.

## Verification appendix

All command examples below were checked against the current binary help output in this branch:

- `loker --help`
- `loker ask --help`
- `loker hunt --help`
- `loker audit --help`
- `loker diff --help`
- `loker run --help`
- `loker workflow --help`
- `loker workflow run --help`
- `loker resume --help`
- `loker explain --help`
- `loker backends --help`
- `loker doctor --help`
- `loker workflow list` (verifies workflow registry surface)

Verified artifacts (non-executable):
- `docs/status/clo-316-workflow.yaml` remains the source-of-truth task metadata.
- `README.md` references this document (added in ST3) as a single discoverability link.

## Quick upgrade mapping summary

If you are a `lok` user coming to `loker`:

1. Keep using familiar commands first (`ask`, `hunt`, `audit`, `diff`).
2. Move to workflow execution for larger tasks using `loker run` / `loker workflow run`.
3. Confirm command syntax from `loker <command> --help` during upgrades, as this is the most reliable compatibility signal.
