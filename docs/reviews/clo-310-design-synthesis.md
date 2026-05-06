# Design Review Synthesis: CLO-310

## Verdict

approve_with_changes

## Applied suggestions

1. **F1 — `resolve_run_dir` should walk up to `lok.toml` for bare names**: Apply. The design doc's Open Question Q2 should be resolved in favor of `lok.toml` ancestor walking, consistent with how `loker run` resolves the project root. Update `resolve_run_dir` to walk ancestors looking for `lok.toml` before resolving bare names relative to `$PWD/runs/<run_id>`.

2. **F2 — Gate round-trip test behind opt-in env var**: Apply. The round-trip integration test spawns a child process with SIGTERM and adds ~30s to test time. Gate it behind `LOKER_RESUME_INTEGRATION=1` to match the existing `LOKER_TZ_INTEGRATION=1` convention.

3. **F3 — Document SHA-verification dependency in all-complete guard**: Apply. Add a comment explaining that `RunState::load()` implicitly validates manifest SHA integrity before the guard runs, so the guard ordering is correct. This prevents future refactors from breaking the invariant silently.

## Flagged suggestions

1. **F4 — `run_id` rename breaks backward compatibility**: Do not apply. The positional argument rename from `PathBuf` to `String` is transparent to CLI callers (clap parses by position, not type). No scripted callers are affected. The observation is correct but requires no action — add a brief note in the design doc if desired.

## Final recommendation

Proceed to the plan phase after applying the three suggestions above. The design is additive (CLI surface only), does not touch internal plumbing, and the implementation can follow the standard `make check` gate.
