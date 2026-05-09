# Pre-PR validation: clo-324

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-09
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [minor] DRY: run_id validation duplicated across 5 handlers
**Where:** `src/ui/routes.rs` (~lines 113, 203, 288, 448, 559)
**What:** The same defensive check `if run_id.is_empty() || run_id.contains("..") || run_id.contains('/') || run_id.contains('\\')` appears in five handlers. Drift between copies is likely if any future handler tightens validation (e.g. NUL byte, length cap), and the new `artefact.rs::resolve_artefact` already has its own stricter validation that includes `\0` — so the rules are already inconsistent.
**Suggested fix:** Extract `fn validate_run_id(s: &str) -> Result<&str, StatusCode>` (or reuse the helper now living in `artefact.rs`) and call it from every handler. Add `\0` to match the artefact-route check.

### F2 [nit] Unreachable `canonical.exists()` after canonicalize succeeded
**Where:** `src/ui/artefact.rs:98-100`
**What:** `std::fs::canonicalize` already returns `ErrorKind::NotFound` when the path doesn't exist (handled at line 85). The subsequent `if !canonical.exists()` branch is dead code and a TOCTOU re-stat for no benefit.
**Suggested fix:** Delete lines 98-100.

### F3 [minor] T-BIND-1 doesn't actually exercise non-loopback rejection
**Where:** `tests/ui_threat_model.rs:389-399`
**What:** The test name promises "Default bind address rejects non-loopback," but the body only confirms that the test fixture (which always binds `127.0.0.1:0`) responds 200 to GET `/`. It is tautologically green and does not exercise the WARN-on-non-loopback path in `src/ui/serve.rs`. The trailing comment even concedes this.
**Suggested fix:** Either (a) bind a second listener to `0.0.0.0:0` and assert a WARN is emitted (capture via `tracing-test` or a custom subscriber), or (b) rename to `t_bind_1_loopback_path_serves` and document the WARN-log path as covered by manual review only.

### F4 [minor] T-ENTROPY-1 doesn't measure entropy
**Where:** `tests/ui_threat_model.rs:587-613`
**What:** Test name implies entropy measurement (design §5 referenced ≥128 bits Shannon estimate), but the body merely posts an approval to a long run_id and checks the response file is written. There is no entropy assertion and no verification that short / guessable run_ids would be refused (because v0 has no such guard — the threat-model doc acknowledges this with `T-ENTROPY-1 (run_id is the entropy source; no per-gate token in v0)`).
**Suggested fix:** Rename to `t_entropy_1_run_id_is_entropy_source` to match what it actually verifies, and add a code comment pointing to §5 of the detailed threat model where the v0 acceptance is justified.

### F5 [nit] Scope creep — implement-gate workflow tweaks bundled with security work
**Where:** `.lok/workflows/implement-gate.toml`
**What:** Codex timeout 600000→1200000ms, `timeout 570→900`, Gemini `--skip-trust`, and a backtick→single-quote prompt-fence change ride along with the security hardening. Unrelated to CLO-324 but justified — these are exactly the changes needed to unblock the implement-gate that just failed for both Codex and Gemini on this branch.
**Suggested fix:** Accept as-is for this PR; mention the workflow tuning in the PR description so reviewers don't need to reverse-engineer the motivation.

### F6 [minor] Content-Type 415 path overlaps with axum `Form` extractor
**Where:** `src/ui/security.rs::check_post_origin`, called from `src/ui/routes.rs::hitl_approve`/`hitl_reject` and the equivalent `src/hitl_server/routes.rs` handlers
**What:** axum's `Form<DecisionForm>` extractor already returns 415 if Content-Type isn't form-urlencoded. The explicit guard runs first so it's the source of the response, but the test for "missing CT" (T-CSRF-3 / T-CSRF-4) becomes brittle if extractors are ever reordered or replaced. This is genuine defence-in-depth and worth keeping; just call it out so future refactors don't strip it as redundant.
**Suggested fix:** Add a one-line comment above `check_post_origin` invocations: `// Defence-in-depth: explicit CT/Origin check runs before axum's Form extractor.`

### F7 [nit] `Content-Disposition: attachment` forces download for every artefact
**Where:** `src/ui/routes.rs::get_artefact`
**What:** All artefacts download instead of rendering, by design (consistent with the no-inline-render posture in the threat model). Worth surfacing as a UX choice in `docs/threat-model.md` so it's not later "fixed" without re-evaluating XSS exposure.
**Suggested fix:** Add a one-liner under §1 trust boundaries: "Artefact responses are always served as `Content-Disposition: attachment` to keep them off the rendering surface."

## Verdict

**approve_with_changes**

The core security work is complete and well-tested: all five required response headers are applied uniformly via `add_security_headers`, the artefact route enforces both pre-canonicalize symlink walking and post-canonicalize prefix checks (proper defence-in-depth against TOCTOU and intermediate symlinks), POST handlers reject foreign Origins (403) and wrong Content-Types (415), SSE has a best-effort cross-origin guard documented to its known limitation, and the advisory lock is honored under concurrent approve attempts. The threat-model summary doc is coherent with the dated detailed model, and `Cargo.toml` only gains `mime_guess` (a low-risk dependency). The two issues that prevent a clean approve are both in `tests/ui_threat_model.rs`: T-BIND-1 and T-ENTROPY-1 are placeholder tests whose names overpromise relative to their assertions — they should either be hardened or renamed before merge so the test ledger doesn't claim coverage it doesn't have. F1 (DRY of run_id validation) and F2 (dead branch) are easy follow-ups; F5/F6/F7 are documentation nits. After F3+F4 are addressed, this is ready to ship.
