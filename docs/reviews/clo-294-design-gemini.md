# Design Review: CLO-294 — Run Directory Layout

**Reviewer**: Manual (Gemini-style review — automated pipeline failed)
**Reviewed**: 2026-05-03
**Pipeline**: Manual (lok design-review failed: Gemini trust/approval, Ollama/opencode error)

---

# Gemini design / implementation review - CLO-294

## Context
- Design: docs/designs/clo-294-run-dir-layout.md

## Findings

### F1 [minor] Implicit `cwd` dependency in `create` signature
**Where:** design doc §3.2, `RunDir::create(workflow_name: &str)`
**What:** The `create` function implicitly uses `std::env::current_dir()` to resolve the `runs/` parent directory. There's no way for callers (especially test code) to specify a different base directory. Test code in `tests/run_dir_layout.rs` will need to create run dirs inside `tempfile::tempdir()` paths, which won't work if `create` hardcodes `cwd/runs/`.
**Why it matters:** Tests cannot use this API without either patching env vars or accepting run dirs outside their temp dirs.
**Suggested fix:** Add a `base_dir: &Path` parameter to `create`, or add a `RunDir::create_in(base_dir, workflow_name)` builder. Test code passes `tmp.path()`, production code passes `std::env::current_dir()`.

### F2 [minor] `trace.jsonl` "reserved" vs "created on demand" mismatch
**Where:** design doc §2 (Goals): "Reserves `trace.jsonl` (zero-length file, created on demand by trace writer later)."
**What:** The goal says the file should be created during `RunDir::create`, but §3.6 (Atomic creation protocol) only mentions writing `manifest.json` and creating `attempts/`. `trace.jsonl` is not created. The goals and implementation protocol are inconsistent.
**Why it matters:** If the trace writer expects the file to exist but it doesn't, or if `RunDir::create` pretends to create it but doesn't, consumers will be confused.
**Suggested fix:** Either (a) add `File::create(trace_path())?` to the creation protocol in §3.6, or (b) remove the "reserves trace.jsonl" goal and let the trace writer create the file on first append. Option (b) is simpler and avoids an empty file sitting around.

### F3 [minor] Cleanup on partial creation failure
**Where:** design doc §3.6 (Atomic creation protocol)
**What:** If `create_dir_all("runs/")` succeeds and `mkdir(directory)` succeeds, but writing `manifest.json` fails, the empty run directory remains on disk as debris.
**Why it matters:** Orphaned empty directories accumulate in `runs/` and confuse users.
**Suggested fix:** Add a cleanup step: if manifest write or `attempts/` creation fails, remove the leaf directory before propagating the error. Use a `build_cleanup` guard pattern — or since `RunDir::create` is a free function, use `defer!` or manual `remove_dir` on error.

### F4 [nit] `make check` must pass without `slug` crate pre-installed
**Where:** design doc Cargo.toml change
**What:** Adding `slug = "0.1.6"` requires `cargo build` to download and compile it. The crate is tiny but its transitive dependencies might fail under MSRV 1.80.
**Why it matters:** If `slug 0.1.6` has a dependency that requires a newer Rust, the build breaks. The MSRV is 1.80.
**Suggested fix:** Either (a) test `cargo add slug@0.1.6 && cargo check` before accepting the dependency, or (b) inline the ~5-line slug function to avoid the dependency entirely.

### F5 [nit] `RunDir::attempt_dir` allocates a new `AttemptDir` on every call
**Where:** design doc §3.2
**What:** `attempt_dir(phase, attempt)` calls `AttemptDir::new(...)` and returns a new owned struct. This is fine for a convenience accessor, but the design should note whether `AttemptDir::create()` is expected to be called by the caller or is implicit.
**Why it matters:** Callers need to know if `attempt_dir("design", 0)` returns a handle to a non-existent directory, or if `create` pre-creates the attempt directory.
**Suggested fix:** Document in the docstring that `AttemptDir::create()` must be called separately. This is consistent with the existing `AttemptDir` API.

### F6 [nit] Missing `run_id` consistency assertion
**Where:** design doc §3.4 (Initial manifest shape)
**What:** The design says "The `run_id` stored here must match the `RunDir::run_id()` getter" but doesn't add a runtime assertion or test for this invariant. If someone modifies the manifest construction code and introduces a mismatch, the bug won't be caught until a consumer reads and compares.
**Why it matters:** Silent inconsistency between `RunDir::run_id()` and `manifest.json`'s `loker.run_id` would cause confusing failures in downstream consumers.
**Suggested fix:** Add a `debug_assert_eq!` after writing the manifest, or add test #8 (which is proposed) to the TDD contract in §5.

## Strengths
- Clean module layout following the existing `attempt_dir.rs` pattern in `src/run_state/`.
- Atomic creation protocol with collision retry is well-designed and documented.
- Error type is minimal yet complete (`Collision`, `Io`, `Manifest`).
- Accessors follow the same pattern as `AttemptDir` — consistent API surface.
- Strong TDD contract with 9 well-specified tests.
- No API changes needed to PhaseRunner — clean integration point.
- Non-goals are clearly stated and reasonable.

## Verdict
approve_with_changes

Solid design. The concerns are minor (implicit cwd dependency, trace.jsonl semantics, cleanup on partial failure). Address F1, F2, and F3 before implementation; F4, F5, and F6 can be addressed during implementation.
