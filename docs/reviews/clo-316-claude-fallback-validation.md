# Pre-PR validation: clo-316

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-07
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [critical] Committed migration doc falsely claims `loker trace` is absent
**Where:** `docs/migration-from-lok.md:35` (committed at d409a22), section "Breaking changes and compatibility notes"
**What:** The committed file states `**loker trace is not present** in the currently installed CLI in this branch`. This is factually wrong: `loker trace` was shipped in commit 28c477d (CLO-312) and is currently exposed by `loker --help` (verified by running the binary). The "Verification appendix" claims commands were checked against the binary, but this contradiction shows that step did not actually happen. Shipping this document would mislead migrating users and damage the doc's source-of-truth credibility.
**Suggested fix:** Commit the local working-tree edits — the unstaged version of `docs/migration-from-lok.md` already removes the false claim and lists `loker trace <run_id>` correctly under "New in loker". Without committing, the PR would ship the broken text.

### F2 [medium] Uncommitted working-tree edits required for accuracy
**Where:** `git status` shows `M docs/migration-from-lok.md`
**What:** Substantive corrections to concept mapping, command translation table, and verification appendix exist only in the working tree. If a PR is opened from current HEAD (9b5dbc7), reviewers would see the older inaccurate copy. The pre-merge gate ran logically against the wrong artifact.
**Suggested fix:** `git add docs/migration-from-lok.md && git commit -m "docs(CLO-316): correct trace claim and reorganize migration table"` (or fold into an existing CLO-316 commit) before opening the PR.

### F3 [low] Verification appendix incomplete vs. "New in loker" list
**Where:** `docs/migration-from-lok.md:65-83` (uncommitted version)
**What:** The "New in loker" bullet at line 37 lists `context`, `report`, `fix`, `ci`, `pr`, `conduct`, `debate`, `suggest`, `smart`, `team`, `spawn`, `ls`, `init`, but the appendix only shows `--help` evidence for a subset. Either trim the new-commands list to those actually verified, or add the missing `--help` lines for symmetry with the doc's stated verification policy.
**Suggested fix:** Add the remaining `<cmd> --help` Usage lines to the appendix, or note "see `loker --help` for the full command list" and remove the unverified bullet detail.

### F4 [low] Concept-mapping row drops design-doc-mandated "phases"
**Where:** `docs/migration-from-lok.md:11-17`; `docs/designs/clo-316-migration-from-lok.md:13`
**What:** Design Goal 1 explicitly requires concept mapping for "workflows, phases, backends, run artefacts, and config paths." The current table covers all but **phases**. Plan acceptance for ST1 also references phases. Minor scope gap relative to acceptance criteria.
**Suggested fix:** Add a one-row entry mapping legacy `lok`'s implicit single-phase model to loker's per-phase orchestration (`design`, `plan`, `implement`, etc., with `--rerun phase=` and resume semantics), or explicitly note phases are out of scope here.

### F5 [low] README link placement
**Where:** `README.md:155`
**What:** Single-line callout placed under "Design docs & roadmap" works but is slightly buried. Open question 2 in the design doc was never resolved. Not blocking; placement is reasonable.
**Suggested fix:** Optional — consider moving the callout closer to the install/quickstart section so legacy users hit it earlier in the README scan path.

## Verdict
approve_with_changes

The diff is correctly scoped (docs-only, two files, no code changes) and aligns with M9 / CLO-316 intent. The pre-merge gate (`make check`) is unaffected because no Rust code is touched. However, the **committed** document on the branch contains a factually false statement about a shipped command (`loker trace`), which the local uncommitted edits already fix — those edits must be committed before the PR is opened. After committing F1/F2, the remaining items (F3/F4/F5) are low-severity polish that can ship in this PR or be tracked as follow-ups.
