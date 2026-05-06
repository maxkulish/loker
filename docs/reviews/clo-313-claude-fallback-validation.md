# Pre-PR validation: clo-313

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [Low] Display path can disagree with the file the existence check used
**Where:** `src/commands/ls_blocked.rs:117-130` (and the existence check at line 68)
**What:** The existence check at line 68 builds `response_path` from the **filesystem** (`run_dir` directory name + pending file stem). But `response_display_path` at line 127 is built from the **pending JSON's** `run_id` / `phase` fields (with fallback to disk values only when the JSON field is empty). If `runs/run-A/pending/review.json` contains `"run_id": "run-B"`, the listing tells the operator to write `runs/run-B/responses/review.json`, but the unblock file the scanner actually checks is `runs/run-A/responses/review.json` — so the row never disappears even after the operator writes the file at the displayed path. HumanVerifier writes both consistently today, but the two-source pattern is fragile.
**Suggested fix:** Use a single source for run_id / phase. Either (a) always use the disk path stems for run_id, phase, and the display path (parse JSON only for severity / timestamps); or (b) when JSON values are present and differ from the disk path, treat the file as malformed (warn + skip), matching how schema mismatches are handled.

### F2 [Trivial] Dead branch in response_path normalization
**Where:** `src/commands/ls_blocked.rs:139-143`
**What:** `response_path` passed into `parse_pending` is always `run_dir.join("responses")...` where `run_dir = runs_dir.join(...)` and `runs_dir = root.join("runs")`. It is absolute whenever `root` is absolute (the only realistic call path; `find_project_root` returns an absolute path, and `current_dir()` does too). The `else` branch joining onto `root` cannot fire from `scan_blocked` and there's no other caller. Minor noise.
**Suggested fix:** Drop the `if response_path.is_absolute()` branch and just store `response_path` as-is, or take a `&Path` and copy it. Removes a couple of lines and a parameter.

### F3 [Info] `response_path` field on `BlockedEntry` is currently unused
**Where:** `src/commands/ls_blocked.rs:21`
**What:** `response_path` (absolute) is computed and stored on every `BlockedEntry` but never read by `render_table`, the snapshot test, or any caller; only `response_display_path` is shown. Not wrong — public field for future callers — but currently it carries no test coverage.
**Suggested fix:** Either remove it until a consumer needs it, or add an assertion in `scan_blocked_lists_unmatched_pending` that it equals `<tmp>/runs/run-1/responses/review.json` so the field has a contract.

## Verdict
**approve_with_changes**

The implementation matches the design and PRD acceptance criteria, mirrors the `trace` command pattern as agreed in synthesis, and the pre-merge gate (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, the new `ls_blocked` and `ls_blocked_cli` test targets) all pass cleanly. Test coverage is solid: 10 unit tests + 1 inline-snapshot integration test + 2 CLI smoke tests covering empty, malformed, multi-phase, sort, and the bare-`ls` error path. Schema usage is read-only and gated on `schema_version == 1`, matching how `HumanVerifier` itself rejects mismatched schemas, so this is safe to ship. F1 is the only concern worth addressing before merge — it is a small data-integrity inconsistency that won't bite under normal HumanVerifier writes today, but the dual-source pattern will silently mislead operators in any drift scenario; the fix is a few-line tightening. F2 / F3 are trivial cleanup that can land in the same PR or be deferred.
