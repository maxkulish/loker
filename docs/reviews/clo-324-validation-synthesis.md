# Pre-PR validation: clo-324

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-09
**Pipeline**: lok implement-gate
---

## Reviewer Status

| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc parse error in implement-gate wrapper (`unexpected EOF while looking for matching '`) — the persona file likely contains an unescaped single quote that breaks the `$(cat <<EOF ... EOF)` capture. No model output produced. |
| Gemini | REVIEW_FAILED | Same shell heredoc parse error pattern in the Gemini wrapper. Neither `gemini-3.1-pro-preview` nor the `gemini-2.5-pro` fallback was invoked because the script aborted before the `gemini` call. |
| Claude (fallback) | OK | Produced 5 findings (2 medium, 3 low) with verdict `approve_with_changes`. Used as the sole substantive review for this synthesis. |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 (doc drift, Content-Type):** Update `docs/designs/clo-324-threat-model-suite.md` §3 and the corresponding row in `docs/threat-model.md` to state `application/x-www-form-urlencoded` (matching the `axum::Form` extractor in `src/ui/security.rs:32-35`). Cheap text fix; eliminates a future-reader trap.
- **F2 (ST4 reconciliation):** Pick one — either implement the planned `heartbeat_deadline`/`force_expire` accessor on `src/run_state/phase_lock.rs` and add the named `lock_heartbeat_expiry_releases_lock` test, OR formally drop ST4 from `docs/plans/clo-324-*.md`, mark `T-LOCK-1…3` in `docs/threat-model.md` §4 as covered by the existing TTL/dead-PID tests, and note the trade-off in the design's open questions. The plan and threat-model currently overstate coverage; the docs must match what shipped.
- **F3 (symlink status code):** Reconcile `src/ui/routes.rs` `get_artefact` (`ArtefactError::Symlink` → 403) with the design's stated 404. Either flip the mapping to `NOT_FOUND` (matches `Traversal`, hides existence) or amend the design to record that 403 was chosen deliberately. One line of code or one line of doc.

All three are bounded, mechanical, and addressable in a single fix iteration without re-architecting.

## Out of Scope / Deferred
- **F4 (security headers inline vs layer):** The implementation's per-handler `add_security_headers(&mut response)` is functionally correct and tested on the routes that exist today. Migrating to `axum::middleware::from_fn` or adding a header-coverage enumeration test is a hardening follow-up, not a blocker. Recommend filing as a follow-up under M11.
- **F5 (CSP test depth):** `t_csp_1_*` validates the header value, which is what the threat-model row promises. Adding a "no inline `<script>`/`style=` in served templates" check or a Playwright smoke is a coverage upgrade for the M11 close-gate, not in-scope for this PR.

## False Positives / Tooling Artifacts
- **Codex review failure:** Tooling artifact — shell heredoc parse error in `.pi/agents/codex-pre-pr.md` ingestion, not a code defect. Should be tracked separately so the implement-gate wrapper escapes the persona payload (likely needs `printf '%s' "$PERSONA"` or single-quoted heredoc) before next run.
- **Gemini review failure:** Same tooling artifact — identical shell-quoting bug in the Gemini wrapper. Fix is mechanical (escape the persona content or pass via stdin/file).

## Recommendation
PROCEED_WITH_FIXES. Land three bounded fixes in one iteration before opening the PR: (1) flip the Content-Type string in the design doc and threat-model row to `application/x-www-form-urlencoded`; (2) reconcile ST4 — either implement `heartbeat_deadline` + the named test, or remove ST4 from the plan and mark the threat-model row as TTL-test-covered; (3) pick 404 or 403 for the symlink case and align doc + code. F4 and F5 are good follow-ups for M11 but should not gate this PR. Separately, the implement-gate wrapper's heredoc bug should be fixed so future Codex/Gemini reviews actually run — but that's an orchestrator-side issue, not a CLO-324 blocker.
