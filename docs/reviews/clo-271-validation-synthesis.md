# Validation Synthesis - CLO-271

## Reviews consulted
- Codex pre-PR validation: `docs/reviews/clo-271-codex-validation.md`
- Gemini validation: `docs/reviews/clo-271-gemini-validation.md`

## Verdict
approve_with_changes

## Must Fix Before PR
- **Both reports F1**: `is_secret_like_env_key` is defined but unused; allowlisted env vars with secret-shaped names (e.g., `MY_API_KEY=foobar`) leak their literal values into `FailureReason` because only generic regex heuristics run. Collect values of secret-shaped keys and explicitly redact those exact strings from captured stdout/stderr before constructing `FailureReason`. Add a test with a non-regex-shaped value (e.g., `CLO271_SECRET_TOKEN=plain-secret-value`) to prove the fix.
- **Gemini F2**: Timeout handler calls `kill_process_group` (no-op on Windows) but never calls `child.kill()`, causing `child.wait()` to hang indefinitely on non-Unix platforms. Add `let _ = child.kill().await;` after `kill_process_group` to ensure the direct child is terminated universally.

## Out of Scope / Deferred
- **Gemini F3**: Redundant double-redaction where `CapturedOutput::to_reason_text()` already redacts and the caller wraps again. Harmless and idempotent; clean up opportunistically but not blocking.

## False Positives / Tooling Artifacts
- None identified.

## Recommendation
Apply one fix iteration to address secret-value redaction for allowlisted env keys and the Windows timeout hang, then re-verify.

---

## Re-validation (post-fix)

**Commit**: 7b5bbcd

### Fixes applied

1. **F1 — Explicit secret value redaction**: `build_environment()` now returns environment variables as a local `Vec<(String, String)>` before iterating. Values from keys matching `is_secret_like_env_key()` are collected into `secret_values: Vec<String>` and stored in `CommandRun`. A new `redact_output()` helper first applies `redact_secrets()` (regex-based), then replaces any exact match of allowlisted secret values with `[REDACTED]`. The `verify()` method uses `redact_output()` instead of bare `redact_secrets()`. Test value changed from regex-shaped `AKIA...` literal to `"plain-secret-value-that-should-be-redacted"` to genuinely test the new path.

2. **F2 — Windows timeout fix**: Added `let _ = child.kill().await;` immediately after `kill_process_group(child.id())` in the wall-timeout branch. On Unix, this is a redundant second kill (harmless); on Windows, it ensures the child is terminated before `child.wait().await` is called.

3. **F3 — Double-redaction cleanup**: `CapturedOutput::to_reason_text()` no longer calls `redact_secrets()` internally — it returns the raw text with truncation marker. Redaction happens once in `redact_output()`.

### Pre-merge gate
`make check` — **green**. All 636 lib + 532 bin + 8 integration tests pass. 0 clippy warnings. 0 fmt issues.

### Verdict (re-validation)
**approve** — All must-fix items resolved. Ready for PR transition.
