# Design: CLO-294 — Run Directory Layout (`runs/<workflow>-<timestamp>-<short-uuid>/`)

| Field | Value |
|-------|-------|
| Author | pi (design phase) |
| Status | Draft |
| Created | 2026-05-03 |
| Task | CLO-294 / T-030 |
| Depends on | CLO-292 (PhaseRunner) — provides `PhaseRunner::run` that consumes `run_dir` |
| Blocks | T-031 (resumability), T-032 (summary), T-035 (prompt templates) |
| PRD | FR-22 (run dir layout) |

## 1. Problem

The `PhaseRunner` (CLO-292) currently receives a bare `PathBuf` as its `run_dir` argument. There is no abstraction that creates, owns, or validates run directories. Every invocation of `loker run <workflow>` needs a fresh, collision-resistant directory at a well-known path under the project root (or configurable base). Without `RunDir`, callers must manually construct the `runs/<slug>-<timestamp>-<uuid>/` path, pre-create the scaffolding (`manifest.json`, `trace.jsonl`, `attempts/`), and pass the path through — leading to ad-hoc path construction, potential naming collisions, and missing scaffolding files.

The desired behaviour is a `RunDir::create(workflow_name)` builder that:

1. Generates a unique, deterministic directory name from the workflow name, current UTC timestamp, and a short UUID fragment.
2. Creates the directory atomically (retry once on collision).
3. Pre-creates the scaffolding: `manifest.json` with the initial schema and `attempts/` subdirectory. `trace.jsonl` is not pre-created here — the trace writer creates it on first append.
4. Exposes typed accessors for the canonical paths consumers need (manifest, trace, attempt directories).
5. Is the **single source of truth** for run directory paths — no other code constructs them manually.

## 2. Goals / Non-goals

### Goals

- New module `src/run_state/run_dir.rs` exposing `RunDir::create(workflow_name) -> Result<RunDir, RunDirError>`.
- Directory name format: `runs/<workflow_slug>-<YYYYMMDD>-<HHMMSS>-<short_uuid>/` (slug = kebab-case ascii, timestamp = UTC, short_uuid = first 8 hex chars of v4 UUID).
- Workflow slug derived via `slug::slugify` (kebab-case, ascii-only, deterministic for the same input string).
- Pre-creates `manifest.json` with initial shape `{"loker.run_id": "<uuid>", "schema_version": 1, "entries": []}`.
- Pre-creates `attempts/` subdirectory (empty, for per-phase attempt archives).
- `trace.jsonl` is NOT created by `RunDir::create` — the trace writer (T-029) creates it on first append. The `trace_path()` accessor provides the canonical path for the trace writer to use.
- Atomic creation: parent `runs/` mkdir-p via `create_dir_all`, then `mkdir` of the leaf directory. Retry once on collision (extremely unlikely with 8-char UUID + second-precision timestamp).
- Public accessors:
  - `RunDir::path() -> &Path` — the run root.
  - `RunDir::manifest_path() -> PathBuf` — `path().join("manifest.json")`.
  - `RunDir::trace_path() -> PathBuf` — `path().join("trace.jsonl")`.
  - `RunDir::attempt_dir(phase, n) -> AttemptDir` — delegates to `AttemptDir::new(self.path(), phase, n)`.
  - `RunDir::run_id() -> uuid::Uuid` — the UUID used in the directory name and manifest.
  - `RunDir::workflow_slug() -> &str` — the slug that went into the directory name.
- CLI plumbing: `loker run <workflow>` creates a `RunDir` and passes `RunDir::path()` to `PhaseRunner` as the `run_dir` field.
- All five TDD tests pass under `make check` (see §5).

### Non-goals

- Symlink to "latest" run (post-v0 feature).
- Pruning old runs (post-v0 `loker gc`).
- Custom `runs/` location via env var (post-v0).
- Lock files or daemon detection for concurrent writers.
- Migration of existing legacy run directories.
- CLI flag for `--run-dir` override.

## 3. Architecture

### 3.1 Module layout

```
src/
├── run_state/
│   ├── mod.rs           # UPDATED — add `pub(crate) mod run_dir;`, re-export `RunDir`
│   ├── run_dir.rs       # NEW — RunDir struct, create(), accessors
│   ├── attempt_dir.rs   # existing (unchanged)
│   ├── latest.rs        # existing (unchanged)
│   ├── atomic.rs        # existing (unchanged)
│   ├── markers.rs       # existing (unchanged)
│   └── ...
├── lib.rs               # UPDATED — add `RunDir` re-export from run_state if public
├── main.rs              # UPDATED — `loker run <workflow>` creates RunDir
├── config.rs            # UPDATED — optionally add runs_dir config (deferred to post-v0)
Cargo.toml               # UPDATED — add `slug = "0.1.6"` dependency
tests/
└── run_dir_layout.rs    # NEW — TDD contract tests
```

### 3.2 `RunDir` struct

```rust
use std::path::{Path, PathBuf};

/// A run directory: `runs/<workflow_slug>-<YYYYMMDD>-<HHMMSS>-<short_uuid>/`.
///
/// Created via `RunDir::create()` and is the canonical filesystem root
/// for all run artefacts: manifest, markers, attempt directories, and trace.
///
/// # Invariants
///
/// - `path()` exists on disk after `create()`.
/// - `manifest.json` exists at `manifest_path()` with the initial schema.
/// - `attempts/` exists at `path().join("attempts")`.
/// - No other code constructs these paths manually — `RunDir::create` is
///   the single producer.
pub struct RunDir {
    path: PathBuf,
    run_id: uuid::Uuid,
    workflow_slug: String,
}

impl RunDir {
    /// Create a new run directory under `base_dir/runs/`.
    ///
    /// 1. Generates a UTC timestamp and short UUID.
    /// 2. Derives the workflow slug from `workflow_name` via `slug::slugify`.
    /// 3. Constructs `base_dir/runs/<slug>-<YYYYMMDD>-<HHMMSS>-<short_uuid>/`.
    /// 4. Creates `base_dir/runs/` if absent (mkdir -p).
    /// 5. Creates the leaf directory with `mkdir` (fails atomically if collides).
    /// 6. On collision (extremely rare): retries once with a new UUID.
    /// 7. Writes the initial `manifest.json` and creates `attempts/`.
    ///    On failure during step 7, the leaf directory is removed to prevent
    ///    orphaned empty directories.
    ///
    /// Returns `RunDirError::Collision` if both attempts collide (retry-exhausted).
    pub fn create(base_dir: &Path, workflow_name: &str) -> Result<Self, RunDirError> { ... }

    /// Convenience wrapper: creates a run directory in the current working directory.
    /// Equivalent to `RunDir::create(&std::env::current_dir()?, workflow_name)`.
    pub fn create_in_cwd(workflow_name: &str) -> Result<Self, RunDirError> { ... }

    /// The run root directory.
    pub fn path(&self) -> &Path { &self.path }

    /// Path to `manifest.json` within this run directory.
    pub fn manifest_path(&self) -> PathBuf { self.path.join("manifest.json") }

    /// Path to `trace.jsonl` within this run directory.
    /// The file is NOT created by `RunDir::create` — the trace writer
    /// (T-029) creates it on first append. This accessor provides the
    /// canonical path for the trace writer to use.
    pub fn trace_path(&self) -> PathBuf { self.path.join("trace.jsonl") }

    /// Create an `AttemptDir` handle scoped to this run directory.
    pub fn attempt_dir(&self, phase: &str, attempt: u32) -> AttemptDir {
        AttemptDir::new(self.path(), phase, attempt)
    }

    /// The UUID that identifies this run (also embedded in `manifest.json`).
    pub fn run_id(&self) -> uuid::Uuid { self.run_id }

    /// The workflow slug used in the directory name.
    pub fn workflow_slug(&self) -> &str { &self.workflow_slug }
}
```

### 3.3 Name generation

```rust
/// Generate the run directory name.
///
/// Format: `<slug>-<YYYYMMDD>-<HHMMSS>-<short_uuid>`
///
/// Example: `design-review-20260503-104215-a1b2c3d4`
fn generate_dir_name(slug: &str, now: &chrono::DateTime<chrono::Utc>, run_id: &uuid::Uuid) -> String {
    let uuid_short = &run_id.to_string()[..8];
    format!(
        "{}-{}-{}",
        slug,
        now.format("%Y%m%d-%H%M%S"),
        uuid_short
    )
}
```

### 3.4 Initial manifest shape

The `manifest.json` written by `RunDir::create` uses the existing `Manifest::new(run_id)` constructor which produces:

```json
{
  "loker.run_id": "<uuid>",
  "schema_version": 1,
  "entries": []
}
```

No new schema fields are introduced. The `run_id` stored here must match the `RunDir::run_id()` getter and the UUID in the directory name. This is enforced by the constructor.

### 3.5 Error type

```rust
#[derive(Debug, thiserror::Error)]
pub enum RunDirError {
    #[error("run directory collision after retry: {0}")]
    Collision(PathBuf),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("manifest write failed: {0}")]
    Manifest(#[from] crate::manifest::ManifestError),
}
```

### 3.6 Atomic creation protocol

```
1. now = Utc::now()
2. run_id = Uuid::new_v4()
3. slug = slug::slugify(workflow_name)
4. base_dir: &Path = caller-provided (cwd in production, temp dir in tests)
5. dir_name = format!("{slug}-{YYYYMMDD}-{HHMMSS}-{short_uuid}", ...)
6. path = base_dir / "runs" / dir_name
7. std::fs::create_dir_all(base_dir / "runs/")       // parent exists now
8. match std::fs::create_dir(&path) {     // atomic: fails if path exists
       Ok(()) => { /* proceed */ },
       Err(e) if e.kind() == AlreadyExists => {
           // Retry once with a fresh uuid
           run_id = Uuid::new_v4()
           dir_name = updated with new short_uuid
           path = updated
           std::fs::create_dir(&path)?    // fail if collides again
       },
       Err(e) => return Err(e),
   }
9. // Establish cleanup guard: if any subsequent step fails, remove the
   // leaf directory to prevent orphaned empty directories.
   // In Rust this is a Drop guard or manual remove_dir on error branches.
10. Write manifest.json (via atomic_write)
11. std::fs::create_dir_all(path / "attempts/")
12. Return RunDir { path, run_id, workflow_slug: slug }
```

Note: the `mkdir` (step 8, not `mkdir -p`) provides the atomicity guarantee. `create_dir_all` on the parent `runs/` is safe because it's a shared parent, and the leaf mkdir ensures we never silently merge into an existing run directory.

On failure after step 8 (e.g., manifest write fails, or `attempts/` creation fails), the leaf directory MUST be removed before propagating the error. Implementations should use a `Drop`-based cleanup guard (e.g., `struct CleanupGuard(PathBuf)` with `Drop` impl that removes the directory) that is `.disarm()`-ed on success before returning.

### 3.7 CLI integration (`loker run <workflow>`)

The `loker run <workflow>` command currently goes through `run_workflow` → `WorkflowRunner`. The integration point is in `src/main.rs` where the workflow is loaded and executed.

**Current flow** (simplified):
```
main.rs: Commands::Run { name, ... }
  → run_workflow(&name, &dir, ...)
    → workflow::find_workflow(name)
    → workflow::load_workflow_from_source(source)
    → WorkflowRunner::new(config, cwd, args).run(&wf)
```

**Proposed flow**:
```
main.rs: Commands::Run { name, ... }
  → let run_dir = RunDir::create(&name)?;     // NEW: create the run directory
  → run_workflow(&name, &dir, ..., Some(run_dir))
    → WorkflowRunner::new(config, cwd, args)
        .with_run_dir(run_dir)                 // NEW: pass RunDir to runner
        .run(&wf)
    → WorkflowRunner internally extracts run_dir.path()
      and passes it to PhaseRunner via PhaseInputs
```

The `run_dir` is surfaced to workflow steps via the execution context (`PhaseContext::run_id` is already populated by tests; making it come from `RunDir` ensures the run_id is consistent across steps).

### 3.8 PhaseRunner integration

`PhaseRunner::run` receives `run_dir` as a `PathBuf` in `PhaseInputs`. The integration is:

```rust
let run_dir = RunDir::create("design-doc-tdd")?;

let phase_inputs = PhaseInputs {
    backends: &backends,
    prompt: Prompt::new("..."),
    ctx: PhaseContext {
        run_id: run_dir.run_id(),       // consistent run_id
        cwd: run_dir.path().to_path_buf(),
        ..Default::default()
    },
    verify: None,
    run_dir: run_dir.path().to_path_buf(), // the PathBuf PhaseRunner expects
};
```

No changes to the `PhaseRunner` API are needed — it already accepts a `PathBuf` for `run_dir`. The `RunDir` abstraction wraps the creation and provides the path.

## 4. Public API Surface Changes

### New public items (in `loker::run_state`)
- `pub use run_dir::RunDir;`
- `pub use run_dir::RunDirError;`

### New crate-private items
- `pub(crate) mod run_dir;` in `src/run_state/mod.rs`

### Updated items
- `src/main.rs` — `Run` command handler creates `RunDir` and passes it through the workflow runner.

### Cargo.toml
- Add: `slug = "0.1.6"`

## 5. Test Plan (`tests/run_dir_layout.rs`)

Matches the issue-body TDD spec exactly. All tests are self-contained (no network, no external state).

1. **created dir matches expected regex**: `RunDir::create("design-doc-tdd")` produces a path matching `runs/[a-z0-9-]+-\d{8}-\d{6}-[0-9a-f]{8}/`. The slug is deterministic: `slug::slugify("design-doc-tdd") == "design-doc-tdd"`.

2. **two back-to-back creates produce distinct paths**: Calling `RunDir::create` twice with the same name produces two different paths. The slug portion is the same; the timestamp/uuid differ.

3. **manifest.json exists with correct shape**: After `create`, `manifest.json` exists at `manifest_path()`, contains valid JSON with `loker.run_id`, `schema_version: 1`, `entries: []`.

4. **attempts/ subdirectory exists**: After `create`, `path().join("attempts")` exists as a directory.

5. **accessors return paths under the run dir root**: Each accessor path starts with `path()`.

6. **collision retry succeeds**: Simulate an existing directory with the same name as the first attempted create; assert the retry path is different (white-box: verify the `RunDir::create` retry logic works when the first uuid collides).

7. **workflow slug matches expected format**: `RunDir::create("My Workflow!")` produces a slug `my-workflow` in the directory name.

8. **run_id is consistent**: The UUID returned by `RunDir::run_id()` matches the UUID embedded in `manifest.json` under `loker.run_id`.

9. **RunDir::attempt_dir returns correct AttemptDir**: `run_dir.attempt_dir("design", 0)` returns `AttemptDir::new(run_dir.path(), "design", 0)`.

## 6. Integration with existing code

### 6.1 PhaseRunner (CLO-292)

No API changes needed. PhaseRunner already accepts `run_dir: PathBuf` in `PhaseInputs`. The calling code creates a `RunDir` and extracts the path:

```rust
let run_dir = RunDir::create(&workflow_name)?;
let inputs = PhaseInputs {
    run_dir: run_dir.path().to_path_buf(),
    // ... other fields
};
```

### 6.2 Workflow runner (src/workflow/)

The `WorkflowRunner` currently doesn't pass a `run_dir` to strategies — it's a simpler code path used for `.lok/workflows/` automation scripts. For the initial integration, only the `loker run <workflow>` CLI command (which targets PhaseRunner workflows) creates a `RunDir`. The existing `.lok/workflows/` code path continues to work unchanged.

### 6.3 Config (src/config.rs)

No config changes for v0. The `runs/` directory is always at the current working directory root. A future `runs_dir` config field can be added post-v0 when the need for custom paths arises.

## 7. Risks

| Risk | Mitigation |
|------|-----------|
| Directory name collision (two creates at the same microsecond with same UUID) | Short UUID (8 hex chars = 4 billion values) + second-precision timestamp = astronomically unlikely. Retry once with fresh UUID as safety net. |
| `slug` crate API changes | Pin `slug = "0.1.6"` in Cargo.toml. The function is simple: the design could also inline a minimal slug implementation (lowercase, replace non-alphanumeric with `-`, collapse, trim) to eliminate the dependency. |
| Existing tests rely on specific `run_dir` paths | No existing tests construct run directories from scratch — they use `tempfile::tempdir()`. No regression. |
| `loker run <workflow>` currently goes through `WorkflowRunner` which doesn't use PhaseRunner | PhaseRunner workflow integration is new code — the old path is preserved. No behavioral change to existing workflows. |
| Cross-device rename issues with `atomic_write` | Already handled by `atomic_write` in `src/run_state/atomic.rs` (uses `NamedTempFile::persist` which has cross-device fallback in tempfile). |

## 8. Migration / Rollout

### Backward compatibility

- **No existing run directories**: This is a greenfield feature. No migration needed.
- **Existing `loker run <workflow>` callers**: This new code path is only active when PhaseRunner workflows are used. The existing `.lok/workflows/` automation path is unchanged.
- **Existing tests**: All use `tempfile::tempdir()` — no impact.

### Rollout order

1. Add `slug` dependency to `Cargo.toml`.
2. Create `src/run_state/run_dir.rs` with `RunDir` struct and `create()`.
3. Wire `RunDir` into `src/run_state/mod.rs` re-exports.
4. Create `tests/run_dir_layout.rs` with all 9 tests.
5. Wire `loker run <workflow>` CLI command to create `RunDir`.
6. `make check` — all existing tests pass, all new tests pass.

## 9. Open Questions

1. **Should `slug` be an external dependency or inlined?** The `slug::slugify` function is trivially replaceable with ~5 lines of custom code (`.to_lowercase()`, regex replace `[^a-z0-9]+` with `-`, trim). A minimal inline implementation avoids adding a dependency for a single function call. Decision deferred to implementation — either approach is acceptable.

2. **Should `runs/` location be configurable?** Not in v0. The PRD says "Custom runs/ location via env (post-v0)". If the need arises, a `RunDirConfig` struct with a `base_dir: Option<PathBuf>` can be added to `Config::run_state`.

3. **Should `RunDir::create` take a `&Path` base dir or default to `cwd/runs/`?** Default to `cwd/runs/` for v0 simplicity. The `WorkflowRunner` already has access to `cwd` from the `loker run` directory argument.

## 10. Acceptance Criteria

- [ ] `RunDir::create(workflow_name)` creates `runs/<slug>-<timestamp>-<uuid>/` with correct format.
- [ ] Two back-to-back creates produce distinct paths.
- [ ] `manifest.json` exists with correct initial shape.
- [ ] `attempts/` subdirectory exists.
- [ ] Accessors return correct paths.
- [ ] `RunDir::run_id()` matches `manifest.json`'s `loker.run_id`.
- [ ] Collision retry mechanism works (tested via simulated collision).
- [ ] Slug is deterministic and kebab-case.
- [ ] `make check` is green.
- [ ] `tests/run_dir_layout.rs` TDD contract passes (9 tests).
