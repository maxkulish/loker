# Gemini design / implementation review - CLO-273

## Context
- Branch: feat-clo-273-test-runner
- Design: docs/designs/clo-273-test-runner.md
- Plan / Spec: docs/discovery/clo-273.md

## Findings
### F1 [minor] Pytest parser fallback is fragile to early JSON logs
**Where:** `src/strategy/verify/test_runner.rs` (in `parse_pytest_output`)
**What:** The fallback for parsing pytest output searches for the first `{` and assumes that block is the pytest report. If `stdout` contains any valid JSON dictionary *before* the actual pytest report (e.g., `INFO: loaded config {"verbose": true}`), the deserializer will successfully parse it, fail to find the `"summary"` field, and return an empty failure immediately without checking subsequent lines.
**Why it matters:** The design doc (§4.3) explicitly specifies "fallback to line-by-line JSON candidate parsing" to handle noisy output. The current approach will break if tests emit a JSON-like log line before pytest's JSON report.
**Suggested fix:** Implement a true line-by-line scan (or search for subsequent `{` boundaries) and only return upon successfully finding a payload with a `"summary"` field, rather than halting at the first successfully parsed JSON block.

### F2 [nit] Unnecessary intermediate allocation in `truncate_excerpt`
**Where:** `src/strategy/verify/test_runner.rs` (`truncate_excerpt` function)
**What:** The truncation logic collects characters into a `String`, immediately converts it back to `chars()`, chains the ellipsis, and collects into a `String` a second time: `normalized.chars().take(max_chars).collect::<String>().chars().chain(Some('…')).collect()`.
**Why it matters:** It is an unidiomatic way to append a character and causes an unnecessary intermediate heap allocation.
**Suggested fix:** Simplify to avoid the intermediate `String`: `let mut s: String = normalized.chars().take(max_chars).collect(); s.push('…'); s`

### F3 [nit] Missing `timed_out` integration test
**Where:** `tests/verify_test_runner.rs`
**What:** The integration tests cover `Pass`, `Fail`, malformed outputs, and non-zero exits, but lack a test simulating a `timed_out: true` state on the `CommandRun` mock.
**Why it matters:** Section 4.4 of the design doc mandates specific `SandboxViolation::Timeout` handling, which is implemented but untested locally in the parser integration layer.
**Suggested fix:** Add a short test case injecting a `timed_out = true` mock and asserting the returned `VerifyResult::Fail` carries a `SandboxViolation::Timeout`.

## Strengths
- The `TestRunner` builder ergonomics are extremely clean and fit well within the existing orchestration abstractions.
- The cargo JSON-lines parsing accurately maps to the cargo test format, specifically handling the `ignored: true` and compiler message edge cases seamlessly.
- Reusing the `RunCommand` internals strictly respects the established sandbox guarantees (timeouts, capabilities, wall limits) while keeping `TestRunner` focused purely on test output parsing.
- Excellent separation of parser logic from execution overhead, allowing `verify_test_runner.rs` to comprehensively test the parsing paths via fixture-style mock runs without shelling out.

## Verdict
approve_with_changes

The implementation strongly aligns with the CLO-273 design document and safely leverages the existing `RunCommand` sandboxing primitives. Resolving the pytest parsing fallback to be a true line-by-line candidate scan will ensure it remains robust against noisy logger output, after which the implementation is fully sound and ready to merge.
