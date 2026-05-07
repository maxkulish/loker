# Design: CLO-316 - docs/migration-from-lok.md

## 1. Problem

Per `docs/discovery/clo-316.md`, existing `lok` users upgrading to `loker` in M9 lack one authoritative, concise mapping from legacy behavior to current `loker` behavior. The documentation is fragmented across `README.md`, `docs/old-readme.md`, and source (`src/main.rs`), so users cannot self-serve migration by command or concept alone. This is blocking task close-out for `T-045` and `CLO-314` follow-up work because M9 requires a dedicated migration note that is safe, trustworthy, and command-verified.

## 2. Goals / Non-goals

**Goals**

- Produce `docs/migration-from-lok.md` with a focused migration artifact containing:
  - Side-by-side concept mapping for workflows, phases, backends, run artefacts, and config paths.
  - Command translation rows for `lok`-era commands used by task owners (`ask`, `hunt`, `audit`, `diff`) and current `loker` equivalents.
  - Explicit breaking changes and compatibility notes.
  - A section for non-porting behaviors with rationale.
  - A deprecation-window statement for legacy entrypoints/config paths (`lok.toml`, `.lok/workflows/`).
- Verify all `loker` command examples against the real binary help or current behavior before publication.
- Add a short link from `README.md` to the new migration doc.

**Non-goals**

- Build automation tooling or auto-conversion scripts.
- Expand or change runtime compatibility behavior (no source-code migration logic changes).
- Promise semantic parity for every historical `lok` feature. This task documents what exists today.

## 3. Architecture

This is a documentation-only change; there is no runtime architecture change. The deliverable is the structure and source of truth for migration statements.

### File layout

```text

docs/
├── designs/
│   └── clo-316-migration-from-lok.md   (design for this issue)
├── discovery/
│   └── clo-316.md                       (input)
├── prds/
│   └── clo-316-migration-from-lok.md     (input)
├── migration-from-lok.md                 (new doc)
├── old-readme.md                         (legacy reference)
└── 
README.md                                (add one migration link)
```

### Data flow for migration source-of-truth

```text
Current CLI surface (CLI parsing + src/main.rs)
        └─► migration source context

`README.md` + `docs/old-readme.md` + `docs/prd/2026-04-25-loker.md`
        └─► discovery synthesis

Discovery + PRD context
        └─► docs/designs/clo-316-migration-from-lok.md

Approved design
        └─► docs/migration-from-lok.md + README link
```

The migration doc is the canonical downstream artifact; it does not introduce new behavior.

## 4. Public API surface

This task makes documentation-only changes. No Rust API additions are planned.

### Relevant CLI contracts (read-only references)

The migration table must reflect current command shapes as exposed by `loker`.

```rust
#[derive(Subcommand)]
enum Commands {
    Ask { prompt: String, backend: Option<String>, dir: PathBuf, no_cache: bool },
    Hunt { dir: PathBuf, issues: bool, issue_backend: String, yes: bool },
    Audit { dir: PathBuf },
    Diff { spec: String, dir: PathBuf, backend: Option<String>, unstaged: bool },
    Workflow(WorkflowCommands),
    Explain { target: Option<String>, dir: PathBuf, backend: Option<String>, focus: Option<String> },
    Run { name: String, spec: Option<PathBuf>, ... },
    Resume { run_id: String, ttl: Option<u64> },
    Trace { run_id: String, json: bool, color: Option<ColorChoice> },
    // ...
}
```

```rust
enum WorkflowCommands {
    Run { name: String, dir: PathBuf, output: Option<PathBuf>, explain_validation: bool, args: Vec<String> },
    List,
    Validate { path: PathBuf },
}
```

### Planned source edits

- New file: `docs/migration-from-lok.md`
- Existing file update: `README.md` (single-link addition)

No code modules or tests are modified in this phase.

## 5. Test plan

This is a docs deliverable, so validation is manual but concrete.

### Command verification (required)

For each command line added to `docs/migration-from-lok.md`:

1. Run `loker --help` and `loker <command> --help` to confirm command syntax.
2. Confirm the documented arguments/order match current CLI help output.
3. Confirm command examples are valid for the current major/minor version.

### Configuration compatibility checks

4. Validate legacy config path claims by confirming current code still discovers `lok.toml` and `.lok/workflows/` in current behavior.
5. Confirm any workflow path examples resolve under repo root and documented commands execute with minimal inputs.

### Documentation integrity checks

6. Verify the new link from `README.md` resolves to an existing file.
7. Verify formatting renders on GitHub and line-wraps cleanly.

### Manual gate

- `make check` remains unchanged for docs-only work but is still run as project gate for PR readiness.

## 6. Migration / rollout

Rollout is doc-only and single-PR.

1. Add `docs/migration-from-lok.md` with a canonical mapping table and deprecation note.
2. Update `README.md` with one forward link to the new migration doc.
3. Keep wording conservative: only documented behavior from current source docs + CLI help.
4. Do not add runtime behavior changes or feature flags.

Backward compatibility for users is **described**, not changed.

## 7. Open questions

1. **Deprecation window length:** `docs/handoff.md` commits to a config-rename milestone with a deprecation window but does not specify exact duration.
   - Tradeoff: longer window lowers migration risk, shorter window reduces dual-name debt.
2. **README link placement:** where to place the migration link (frontmatter vs docs section).
   - Tradeoff: maximum discoverability vs minimizing noise for non-migrating readers.
3. **Scope of "not ported" section:** exhaustive list vs grouped categories.
   - Tradeoff: precision vs maintenance effort over time.
4. **M10/M11 mention:** whether to include forward-looking notes to HITL/browser docs in this migration page.
   - Tradeoff: better context vs scope clarity for M9 readers.

These items will be tracked as follow-up doc clarifications if final scope expands beyond this task.