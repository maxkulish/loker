# CLO-273 Validation Synthesis

## Verdict
approve_with_changes

## Must Fix Before PR
- `High:` `TestRunner::to_verify_result()` did not convert non-zero exit statuses into failure when parsed counts indicated all tests passed. I fixed this by adding an explicit non-zero exit check before success return and returning `VerifyResult::Fail` with `SandboxViolation::NonZeroExit { code }`.
- `Medium:` `Test-runner output excerpt truncation` was implemented with UTF-8-safe truncation (`truncate_excerpt`) and covered by a focused unit test.
- `Medium:` `Pytest parser fallback` now handles noisy/multiline stdout by parsing the first JSON object from the first `{` rather than requiring each line to start with `{`.

## Out of Scope / Deferred
- Exposure of `CommandRun` / `CapturedOutput` as `pub` for integration tests remains from earlier implementation to support existing integration coverage. It is outside the stated scope of ST1–ST6, but can be revisited in a follow-up to reduce public surface area if desired.

## False Positives / Tooling Artifacts
- Codex F1 (sandbox defaults mismatch): `SandboxOpts` hardcodes 120s/8192 bytes intentionally. Test suites typically run longer and produce more output than generic `RunCommand` shells; the `RunCommand` defaults (30s/4096) are for arbitrary commands. The design doc does not mandate identical defaults; it says SandboxOpts "Maps to RunCommand fields but is kept as a separate struct." Different defaults for test-runner context is a design choice, not a bug.
- Codex F2 (NonZeroExit missing when `failed > 0`): `SandboxViolation::NonZeroExit` is meant for runner crashes / unexpected non-zero exits, not for the normal case where tests failed and the runner correctly reports that via non-zero exit code. When `result.failed > 0`, the failure reason is "tests failed," not a sandbox violation. The code correctly attaches `NonZeroExit` only when `code != 0 && result.failed == 0` (runner crash / no-test scenario per design §4.4).
- Codex F3 (public API expansion): Acknowledged above as out-of-scope. The `pub` visibility on `CommandRun`/`CapturedOutput` was pre-existing and is used by integration tests. Reducing visibility is a valid follow-up but not required for CLO-273.
- Codex environment failure: The Codex checker could not execute `cargo test` or `make check` in its sandbox, leaving checklist items unchecked. Host-level `make check` was run separately and is green.

## Recommendation
All synthesis-approved fixes from the first iteration have been applied and verified. The new Codex findings are working-as-designed or out-of-scope. Proceed to PR after confirming `make check` green and file-existence checklist.

## Re-validation
After the initial fix iteration, `make check` was re-run and remains green. All 9 `verify_test_runner` tests and 8 `verify_run_command` tests pass. No additional code changes are required.

## Synthesis Method
This report synthesizes:
- `docs/reviews/clo-273-codex-validation.md` (fresh run, verdict: `rework` — findings classified as false positives above)
- `docs/reviews/clo-273-gemini-validation.md` (verdict: `approve_with_changes`)
- git diff against main + host-level test results
- Manual review by orchestrator against design doc `docs/designs/clo-273-test-runner.md`
