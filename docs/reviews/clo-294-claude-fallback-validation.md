# Pre-PR validation: clo-294

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-03
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [medium] CLI silently swallows `RunDir::create` failure
**Where:** src/main.rs:1183-1192
**What:** The `Run` handler logs and continues if `RunDir::create` fails, instead of returning an error. The workflow then proceeds without a run directory. Since the `RunDir` isn't yet plumbed into `WorkflowRunner`, the value is currently only printed and dropped — but a silent fallthrough on a feature whose whole purpose is "single source of truth for run paths" sets a bad precedent and hides storage/permission problems from the user.
**Suggested fix:** `let run_dir = RunDir::create(&cwd, name).context("failed to create run directory")?;` — propagate the error via `anyhow`. If the v0 intent is "best-effort breadcrumb", at minimum log via `eprintln!` only and document the temporary nature.

### F2 [medium] `runs/` not added to `.gitignore`
**Where:** .gitignore (root)
**What:** Running `loker run <workflow>` from the project root creates `cwd/runs/<…>/` and pollutes the working tree. `git status` already shows `?? runs/` from local exercises. Without an ignore entry, every developer will see noise in `git status` and risk committing run artefacts.
**Suggested fix:** Add `/runs/` to `.gitignore` (matches existing pattern style like `/target/`).

### F3 [low] Retry branch maps non-`AlreadyExists` errors to `Collision`
**Where:** src/run_state/run_dir.rs:61
**What:** `std::fs::create_dir(&path).map_err(|_| RunDirError::Collision(path.clone()))?` discards the underlying `io::Error` — e.g., `PermissionDenied` or `NotFound` on the second attempt is reported as a collision, masking the real cause.
**Suggested fix:** Match on the error kind: only convert `AlreadyExists` to `Collision`; forward any other `io::Error` via `?` (the existing `From<std::io::Error>` impl).

### F4 [low] Retry path is not exercised by any test
**Where:** tests/run_dir_layout.rs:108-145
**What:** The `collision_retry_succeeds` test is explicitly documented as not actually simulating a collision (timestamp/UUID can't be predicted). The unit tests in `run_dir.rs` also don't cover it. The retry branch (line 56-62) is dead code from a test perspective. Acceptance criterion #6 in the design is therefore unmet.
**Suggested fix:** Refactor `create` to take an injected name-generator (or expose an internal helper that takes precomputed `now`/`run_id`) so a test can pre-create the first attempted path and assert the retry branch ran. Alternatively, factor out a `try_create_with(slug, now, run_id)` helper and unit-test it directly.

### F5 [low] Redundant `Json` variant in `RunDirError`
**Where:** src/run_state/run_dir.rs:188-194
**What:** `RunDirError` has both `Json(serde_json::Error)` and `Manifest(ManifestError)`. `ManifestError` already wraps `serde_json::Error` via `#[from]`. The `Manifest` variant is currently unreachable because no call in `create` returns a bare `ManifestError`. Either drop one or route manifest writes through `Manifest::append`/an API that returns `ManifestError`.
**Suggested fix:** Remove the `Json` variant and convert `manifest.to_json()?` via `.map_err(ManifestError::from)?`, or keep `Json` and delete the unreachable `Manifest` variant.

### F6 [low] `RunDir: Clone` allows multiple owners of one on-disk dir
**Where:** src/run_state/run_dir.rs:20
**What:** `#[derive(Clone)]` lets callers duplicate a struct that the design describes as "the canonical filesystem root … single producer". No `Drop` exists today so it's harmless, but it weakens the invariant and would silently break if anyone later adds cleanup-on-drop.
**Suggested fix:** Drop the `Clone` derive unless a concrete consumer needs it; pass `&RunDir` instead.

### F7 [info] `RunDirError::Manifest` import path is `crate::manifest::ManifestError`
**Where:** src/run_state/run_dir.rs:193
**What:** Verified `ManifestError` exists with `#[from] serde_json::Error`. No issue — note for completeness.
**Suggested fix:** None.

## Verdict
approve_with_changes

The implementation matches the design closely: 9 contract tests pass, `cargo check`/`clippy`/`fmt` are clean, the cleanup-guard pattern is correctly placed, and module wiring is minimal. Two issues warrant a quick follow-up before merge: the silent CLI fallthrough on `create` failure (F1) and the missing `/runs/` `.gitignore` entry (F2) — the latter will dirty every developer's tree the moment this lands. The retry-branch coverage gap (F4) and the lossy error mapping (F3) are smaller but worth addressing now since the retry path is the only safety net against UUID collisions; otherwise the design's acceptance criterion #6 isn't truly verified. F5/F6 are polish.
