# Plan: CLO-286 Implement attempt directories

**Context:**
- **Design**: `docs/designs/clo-286-attempt-directories.md`
- **Discovery**: `docs/discovery/clo-286.md`
- **Linear**: https://linear.app/cloud-ai/issue/CLO-286/implement-attempt-directories
- **Task type**: development
- **Depends on**: CLO-284 (markers + `next_attempt` helper)

---

## Sub-tasks

### ST1 — Implement `AttemptDir` helper
**Files:** `src/run_state/attempt_dir.rs`, `src/run_state/mod.rs`
**Acceptance:** `cargo test --test run_state_attempts attempt_dir` passes
**Estimate:** M

Create the `AttemptDir` struct with:
- `new(run_dir, phase, attempt)` — computes `attempts/<phase>/<n>/` path
- `create()` — idempotent mkdir
- `promote_to_canonical(canonical_dir)` — atomic `rename(2)`, `CrossesDevices` fallback to recursive copy+remove
- `path() -> &Path` — accessor

Export from `run_state/mod.rs` (`pub use attempt_dir::AttemptDir;`).

### ST2 — Implement `LatestPointer`
**Files:** `src/run_state/latest.rs`, `src/run_state/mod.rs`
**Acceptance:** `cargo test --test run_state_attempts latest_pointer` passes
**Estimate:** S

Best-effort latest-attempt convenience pointer:
- `LatestPointer::update(run_dir, phase, attempt) → io::Result<()>`
- Unix: create symlink `run_dir/<phase>/latest → ../attempts/<phase>/<n>/` (best-effort; replace on update)
- Fallback: write `run_dir/<phase>/latest.json` with `{ attempt, path, updated_at }`

Export from `run_state/mod.rs` (`pub use latest::LatestPointer;`).

### ST3 — Enhance `next_attempt` to scan attempt directories
**Files:** `src/run_state/markers.rs`, `tests/run_state_markers.rs`
**Acceptance:** `cargo test --test run_state_markers` passes
**Estimate:** M

- Change `next_attempt(markers_dir, phase)` → `next_attempt(run_dir, phase)`
- Add `next_attempt_from_dirs(attempts_dir)` helper that returns `0..N` based on directory names under `attempts/<phase>/`
- Return `max(marker_max, dir_max)` (the +1 logic applied uniformly by caller)
- Update all existing tests in `tests/run_state_markers.rs` that call `next_attempt(markers_dir, phase)` to pass `tmp.path()` instead

### ST4 — Add `AttemptRetention` config stub
**Files:** `src/config.rs`
**Acceptance:** `cargo test config::` passes (all existing config unit tests)
**Estimate:** S

- Add enum `AttemptRetention { Unbounded, Keep(usize) }` with `Default = Unbounded`
- Add `RunStateConfig { keep_attempts: AttemptRetention }` struct
- Add `#[serde(default)] pub run_state: RunStateConfig` to root `Config`
- Ensure `deny_unknown_fields` still resolves `run_state` correctly via `#[serde(default)]` (no existing TOML configs set this key, so backward-compatible)

### ST5 — Write full `tests/run_state_attempts.rs` TDD contract
**Files:** `tests/run_state_attempts.rs` (new)
**Acceptance:** `cargo test --test run_state_attempts` passes (8 tests)
**Estimate:** L

Pull together everything from ST1–ST4 into the 8-test TDD contract:

1. **First attempt**: `next_attempt("design")` returns `0`, dir created at `attempts/design/0/`, write succeeds.
2. **Second attempt after failure**: write `design.failed` for attempt 0, `next_attempt` returns `1`, attempt-1 dir gets new file, attempt-0 untouched.
3. **Manifest entry pins attempt**: create a `ManifestEntry` with `attempt: Some(1)`, verify round-trip.
4. **Latest pointer**: after attempts 0,1,2 with attempt-2 completing, `latest` resolves to `attempts/design/2/`.
5. **Attempt counter survives restart**: derive `next_attempt` from disk only (wipe in-memory state).
6. **Cross-phase isolation**: `design` and `review` attempts numbered independently.
7. **Promotion is atomic**: attempt-0 file promoted to canonical path, attempt-0 dir is gone.
8. **Archive on failure**: failed attempt leaves files in `attempts/design/0/` intact.

### ST6 — Final gate: `make check` green
**Files:** (none new)
**Acceptance:** `make check` exits 0 (fmt + clippy + all tests)
**Estimate:** S

- Run `make check`
- Fix any clippy warnings
- Fix any compilation errors from feature flags or platform differences

---

## Ordering & Dependency Graph

```
ST1 ──┐
ST2 ──┼───> ST5 ───> ST6
ST3 ──┘
ST4 ──────────────────> ST6
```

- ST1, ST2, ST3, ST4 can start in parallel (no code dependencies between them)
- ST5 depends on ST1 + ST2 + ST3 (needs the primitives)
- ST6 depends on ST4 + ST5 (compilation gate)

---

## Pre-merge gate

- `make check` green
- All existing tests pass (`cargo test`)
- `cargo clippy` clean
- `cargo fmt` already applied

---

## Risks

| Risk | Mitigation |
|------|-----------|
| `next_attempt` signature change breaks callers outside tests | Search codebase for all `next_attempt(` usages and update in ST3 |
| `#[serde(deny_unknown_fields)]` rejects `run_state` key in tests | Add explicit `RunStateConfig` default + `#[serde(default)]` in ST4 |
| Platform-specific symlink code causes compilation errors on Windows | Use `#[cfg(unix)]` guard; json fallback is always compiled |
| TDD test #7 (atomic promotion) may flake on CI filesystem | Use `tempfile::tempdir()` which guarantees same filesystem |

---

## Done criteria

- [ ] All sub-tasks completed and acceptance commands pass
- [ ] `make check` is green
- [ ] `tests/run_state_attempts.rs` TDD contract passes (8 tests)
- [ ] Existing run smoke tests still pass
