# Pre-PR validation: clo-319

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc parse error (unmatched `'` from backtick-in-heredoc); model never invoked |
| Gemini | REVIEW_FAILED | Same shell heredoc parse error in invocation script; model never invoked |
| Claude (fallback) | OK | Full review delivered, `make check` confirmed green; verdict `approve_with_changes` with 7 findings |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 (medium) - Lock-file lifecycle contract.** `src/run_state/phase_lock.rs:55-67, 188-194, 214-221` removes the lock file on `release`/`Drop`, but `docs/designs/clo-319-advisory-lock.md:147-152, 174-176, 287-289` resolved the open question in favor of *truncating to 0 bytes*, and the struct rustdoc claims a third behavior ("stays on disk until next acquire overwrites it"). T-044 (`loker ls --blocked`) is the next consumer of this on-disk schema, so the divergence needs to land before downstream work builds on it. Pick one behavior, align doc + code + design note. Cheapest path: replace `remove_file` in `Drop`/`release` with `OpenOptions::new().write(true).truncate(true).open(&self.path)` (or `set_len(0)` on the held fd), then drop the redundant cleanup in `release`.

## Out of Scope / Deferred
- **F2 - Corrupt JSON propagation in `acquire` early-exit path** (`phase_lock.rs:111-123`). Design resolved open question #3 with "lenient reading -> not held," but `acquire` returns `PhaseLockError::Json` instead of falling through to `try_lock_exclusive`. The OS lock is still authoritative, so safety isn't compromised; address alongside F1 if the lifecycle change is reopened, otherwise as a follow-up.
- **F3 - `is_dir()` follows symlinks** (`phase_lock.rs:97-103`). Design §3 called for symlink hardening; current check passes for symlinked run dirs. Loker creates run dirs itself, so the practical risk is near-zero. Either swap to `symlink_metadata().file_type().is_dir()` later or remove the misleading comment - not a blocker.
- **F4 - Unused `lock_dir` field** (`phase_lock.rs:66, 182`). Dead state, derive-Debug masks it from clippy. Trivial cleanup.
- **F5 - Unused `PhaseLockError::StaleReclaimFailed` variant** (`phase_lock.rs:37-38`). Public-API cruft; either remove or annotate as reserved.
- **F6 - Phase-name validation omits `\`** (`phase_lock.rs:86-95`). Unix-only project today; defensive add when the validator is next touched.
- **F7 - `release` + `Drop` both call `remove_file`** (`phase_lock.rs:188-194, 214-221`). Cosmetic; second call is a swallowed ENOENT.

## False Positives / Tooling Artifacts
- Codex and Gemini invocations both died from a shell-escaping bug in their wrapper scripts (heredoc containing a backtick was interpreted by the outer shell). Not a code defect - the runner scripts under `.pi/` need quoting fixes before the next validation cycle. No findings from those reviewers were ever produced.

## Recommendation
PROCEED_WITH_FIXES. The branch is mergeable shape-wise (`make check` green, scope matches plan, no API/schema regressions) but the lock-file lifecycle contract (F1) needs to land in this PR so that T-044 builds against the schema the design committed to. Bounded one-iteration fix: align `Drop`/`release` to truncate-on-release, update the struct rustdoc, optionally fold F2's lenient-read into the same touch since both are "honor the resolved design open questions." Defer F3-F7 as low-priority follow-ups. Separately, flag the `.pi/agents/codex-pre-pr.md` / `.pi/agents/gemini-architect.md` runner scripts to the user for a quoting fix - the synthesis step ran on a single reviewer this cycle.

## Re-validation

**Fix iteration 1 applied.** Commit `b89cb63`.

| # | Finding | Status | Details |
|---|---------|--------|---------|
| F1 | Lock-file lifecycle contract | ✅ FIXED | `Drop`/`release` now truncates to 0 bytes via `set_len(0)` on the held fd. Struct rustdoc aligned. Design doc open question updated. |
| F2 | Corrupt JSON propagation | 🔲 DEFERRED | Low risk — OS lock is still authoritative. Follow-up if downstream T-044 consumption encounters it. |
| F3 | `is_dir()` follows symlinks | 🔲 DEFERRED | Not a blocker. |
| F4 | Unused `lock_dir` field | 🔲 DEFERRED | Trivial cleanup. |
| F5 | Unused `StaleReclaimFailed` variant | 🔲 DEFERRED | Reserved for future use; add comment when touched. |
| F6 | Phase-name validation omits `\` | 🔲 DEFERRED | Add when validator is next touched. |
| F7 | `release` + `Drop` both call `remove_file` | ✅ RESOLVED | Both now share truncation path via `Drop`; `release` delegates to `Drop`. |

`make check` green on HEAD (`b89cb63`). Scope matches plan — additive `locks/` directory only, no schema changes. Ready for PR.
