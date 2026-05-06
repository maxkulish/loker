# Pre-PR validation: clo-319

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [medium] Lock-file lifecycle diverges from design and own doc-comment

**Where:** `src/run_state/phase_lock.rs:55-67`, `src/run_state/phase_lock.rs:188-194`, `src/run_state/phase_lock.rs:214-221` vs. `docs/designs/clo-319-advisory-lock.md:147-152, 174-176, 287-289`

**What:** The design's resolved open question (line 289) and the public `release()` rustdoc (line 174 of the design) commit to *truncating* the lock file to 0 bytes on release; the struct-level rustdoc on `PhaseLock` (lines 56-60 of the impl) says "The lock body stays on disk until the next acquire overwrites it." The actual `Drop` and `release` impls call `std::fs::remove_file`. Three sources, three different stories. Functionally fine ("not held" maps to `Ok(None)` either way), but it's a contract that downstream T-044 (`loker ls --blocked`) consumes - the design promised a discoverable schema, and a missing file vs. an empty file vs. a truncated body matter when troubleshooting.

**Suggested fix:** Pick one. Cheapest: replace `remove_file(&self.path)` with `OpenOptions::new().write(true).truncate(true).open(&self.path)` (or `set_len(0)` on the held fd before drop in `release`), and update the struct rustdoc to match the design ("Drop truncates the body to 0 bytes; `inspect` returns `Ok(None)` for empty files"). Also delete the redundant `let _ = remove_file(&self.path)` in `release` once `Drop` does the truncate.

### F2 [low] Corrupt JSON in lock body propagates instead of being treated as stale

**Where:** `src/run_state/phase_lock.rs:111-123` and `src/run_state/phase_lock.rs:229-242`

**What:** `acquire` calls `read_lock_body(&lock_path)?` in the optimisation path; a malformed body returns `Err(PhaseLockError::Json(_))` and aborts the acquire entirely. Design open question #3 explicitly resolved with "lenient reading... a parse error... indicates 'not held'". This also creates a real-but-tiny race: a second process that reads while the holder is mid-`set_len(0) + write_all` (those aren't atomic) sees partial JSON and returns `Json` instead of `LockInUse`. The integration test `corrupt_body_does_not_panic` accepts either outcome, masking the divergence.

**Suggested fix:** In the early-exit path inside `acquire`, change the `?` to swallow `PhaseLockError::Json` and fall through to `try_lock_exclusive` (the OS lock is the real guard). `inspect` should keep returning the structured error since callers are explicitly asking for the body.

### F3 [low] `run_dir.is_dir()` follows symlinks - design called for "symlink hardening"

**Where:** `src/run_state/phase_lock.rs:97-103` vs. `docs/designs/clo-319-advisory-lock.md:90-92`

**What:** Design §3 "Symlink hardening" says: "validate that `run_dir` is a real directory (not a symlink)". `is_dir()` returns `true` for a symlink that resolves to a directory, so the check passes for symlinked run dirs. Low risk in practice (loker creates run dirs itself) but the comment on line 97 claims "Symlink hardening: ensure run_dir is a real directory" while the code doesn't enforce that.

**Suggested fix:** `std::fs::symlink_metadata(run_dir)?.file_type().is_dir()` (rejects when the entry itself is a symlink), or remove the misleading comment and skip the check entirely.

### F4 [trivial] `lock_dir` field is stored but never read

**Where:** `src/run_state/phase_lock.rs:66, 182`

**What:** `PhaseLock { file, path, lock_dir }` retains `lock_dir: PathBuf`, but no method ever reads it. Clippy doesn't flag it because `derive(Debug)` formally counts as a read. It's just cruft.

**Suggested fix:** Drop the field. `path.parent()` covers any future need.

### F5 [trivial] `PhaseLockError::StaleReclaimFailed` is defined but never constructed

**Where:** `src/run_state/phase_lock.rs:37-38`

**What:** Variant exists in the public error enum with no construction site. If it's reserved for a future code path, that's fine, but it's currently dead surface area on a public API.

**Suggested fix:** Either remove it or add a `// reserved for ...` comment; otherwise it's an unused public symbol that locks future error refactors.

### F6 [info] Phase name validation misses `\` (Windows path separator)

**Where:** `src/run_state/phase_lock.rs:86-95`

**What:** Validation rejects `/`, `\0`, `.`, and `..` but not `\`. A phase name like `..\foo` would pass on Unix (literal filename) but acts as a path separator on Windows, enabling traversal of `runs/<id>/locks/..\foo.lock`. The design Open Q #4 flagged exactly this. Loker is single-machine and effectively Unix-only today, so risk is low.

**Suggested fix:** Add `phase.contains('\\')` to the rejection conditions, mirroring `AttemptDir`'s `path_segment_is_safe` if it exists.

### F7 [info] `release(self)` then `Drop` both call `remove_file` (cosmetic)

**Where:** `src/run_state/phase_lock.rs:188-194, 214-221`

**What:** `release` removes the file, then dropping `self` runs `Drop` which removes it again. The second `remove_file` is a guaranteed `ENOENT`, swallowed by `let _ =`. Harmless, but it betrays that `release` and `Drop` aren't sharing the same path.

**Suggested fix:** Have `release` consume `self` and rely solely on `Drop` (or factor the cleanup into a private `cleanup(&mut self)` called by both - matters once F1's truncate/remove decision lands).

## Verdict

**approve_with_changes**

`make check` is green: `cargo fmt --all --check` clean, `cargo clippy --all-targets -- -D warnings` clean, full `cargo test` passes (810 + 837 unit tests, 4 phase_lock integration tests, all existing resume tests unmodified). The scope matches the plan exactly - no creep, no schema changes to markers/manifest, additive `locks/` directory only. The one substantive issue worth resolving before downstream T-044 ships is F1: the design promised a specific lock-file lifecycle that the implementation contradicts and the struct rustdoc contradicts in a third direction. F2 and F3 are minor robustness items that align the code with the design's resolved open questions. F4-F7 are cleanup that can ride along or land separately. Nothing here blocks merging the PR; address F1/F2 in this PR or as an immediate follow-up before T-044 builds against the on-disk schema.
