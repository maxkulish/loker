# Pre-PR validation: clo-324

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-09
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Wrapper shell script unparsable — unmatched single quote in the heredoc-embedded prompt (`sh: -c: line 30: unexpected EOF while looking for matching '''`). Model never invoked. |
| Gemini | REVIEW_FAILED | Same wrapper bug as Codex (heredoc quoting error at line 38). Model never invoked. |
| Claude (fallback) | OK | Produced a full 7-finding review with `approve_with_changes` verdict against design + plan + diff. |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F3 — T-BIND-1 does not exercise non-loopback rejection** (`tests/ui_threat_model.rs:389-399`). The test name claims coverage of the WARN-on-non-loopback path, but the body only hits `127.0.0.1:0`. Either harden it (bind a second listener to `0.0.0.0:0` and assert the WARN log via a captured subscriber) or rename to `t_bind_1_loopback_path_serves` so the ledger doesn't overclaim.
- **F4 — T-ENTROPY-1 does not measure entropy** (`tests/ui_threat_model.rs:587-613`). The body only verifies a long run_id is accepted and writes its decision file; no entropy assertion is made and v0 has no short-id rejection. Rename to `t_entropy_1_run_id_is_entropy_source` and add an inline reference to §5 of the detailed threat model where the v0 "run_id is the entropy source" stance is justified.

Both fixes are bounded, single-file, ~30 LOC of test edits — addressable in one iteration without touching production code.

## Out of Scope / Deferred
- **F1 — run_id validation duplicated across 5 handlers** (`src/ui/routes.rs`). Real DRY issue with mild drift risk (`artefact.rs` already validates `\0` while routes.rs does not), but no current security gap because the artefact route is the only one that opens user-influenced filesystem paths. Track as a follow-up cleanup; bundling it here would expand the diff without changing security posture.
- **F5 — `.lok/workflows/implement-gate.toml` tuning bundled with security work**. Codex 600k→1200k ms, gate timeout 570→900s, Gemini `--skip-trust`, and prompt-fence change. Justified — these are exactly the gate-unblock tweaks the branch needs — but unrelated to CLO-324's threat-model surface. Mention in PR description so reviewers don't have to reverse-engineer the motive. (Ironically, both reviewers in *this* synthesis just failed because of a *different* quoting bug in the validation wrappers — the gate fix did not propagate to the validation scripts.)
- **F6 — Content-Type 415 overlap with axum `Form` extractor**. Defence-in-depth is correct and worth keeping; only deferred ask is the one-line comment above the call sites so a future refactor doesn't strip it as "redundant." Not blocking.
- **F7 — `Content-Disposition: attachment` UX choice**. Document under threat-model §1 trust boundaries so it isn't later "fixed" without re-evaluating XSS exposure. Doc-only.

## False Positives / Tooling Artifacts
- **Codex review failure** — wrapper script's heredoc has an unescaped apostrophe; not a content judgement, just a shell bug. Same root cause as F5's workflow tuning, but in `.pi/agents/codex-pre-pr.md` (or its caller). Worth fixing in the validation pipeline before relying on these gates again.
- **Gemini review failure** — identical heredoc/quote bug in the Gemini wrapper. Both wrappers need the same fix (probably switching from `cat <<EOF` to `cat <<'EOF'` or sanitizing the persona file of stray apostrophes).
- **F2 — `canonical.exists()` after successful canonicalize** (`src/ui/artefact.rs:98-100`). Technically dead but harmless; flagged as a nit, not a defect. Reasonable to delete in this PR if touching the file anyway, otherwise defer.

## Recommendation
**PROCEED_WITH_FIXES.** Apply the two test-honesty fixes (F3, F4) — either harden the test bodies or rename them to match their actual assertions, plus a comment pointing F4 readers to threat-model §5 — and ship. The core security work (uniform headers, symlink + canonicalize defence on the artefact route, POST Origin/CT guards, advisory lock under concurrency) is complete and consistent with the design. While addressing F3/F4, optionally fold in F2 (delete the dead `exists()` branch) and F6's one-line defence-in-depth comment if it's cheap; F1/F5/F7 are legitimate follow-ups but should not block this PR. Separately — but outside the scope of this PR — fix the heredoc quoting in the Codex and Gemini validation wrappers so the next gate run doesn't degrade to single-reviewer mode again.

---

## Re-validation (2026-05-09)

Fix iteration applied against the 2026-05-09 `approve_with_changes` verdict:

| Finding | Action | Status |
|---|---|---|
| F1 (Content-Type doc drift) | Updated design doc and threat-model to state `application/x-www-form-urlencoded` per actual implementation. | ✅ Fixed |
| F2 (ST4 reconciliation) | Dropped `force_expire`/`heartbeat_deadline` from design; updated plan to note existing TTL + PID-liveness unit tests cover the stale-lock surface. | ✅ Fixed |
| F3 (symlink status code) | Updated design doc to state `403 Forbidden` (matches code and detailed threat model §5 T-SYMLINK-1/2). | ✅ Fixed |
| F3 (bind test overclaim) | Renamed `t_bind_1_default_bind_is_loopback` → `t_bind_1_loopback_serves` with comment. | ✅ Fixed |
| F4 (entropy test overclaim) | Renamed `t_entropy_1_gate_url_uses_run_id` → `t_entropy_1_run_id_is_entropy_source` with comment. | ✅ Fixed |

`make check` green on HEAD `b020ad4`. No new unbounded review loop. The core security work (uniform headers, symlink + canonicalize defence on the artefact route, POST Origin/CT guards, advisory lock under concurrency) is complete and consistent with the design. While addressing F3/F4, optionally fold in F2 (delete the dead `exists()` branch) and F6's one-line defence-in-depth comment if it's cheap; F1/F5/F7 are legitimate follow-ups but should not block this PR. Separately — but outside the scope of this PR — fix the heredoc quoting in the Codex and Gemini validation wrappers so the next gate run doesn't degrade to single-reviewer mode again.
