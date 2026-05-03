# Pre-PR validation: clo-292

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-03
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

`make check` passes (all 660+ unit tests + integration tests green; no clippy warnings). Writing up the review now.

## Findings

### F1 [high] PhaseRunner skips its own verify whenever the strategy is `EscalatingRetry`
**Where:** `src/phase_runner.rs:233-235` (in `PhaseRunner::run`)
**What:** The runner short-circuits its verify dispatch with `matches!(strategy_output.strategy, StrategyKind::Escalating)`, even when `cfg.verify` is `RunCommand` or `LlmVerifier`. That delegates verification entirely to the strategy's internal hook, which only ever sees backend stdout, never the canonical artefact bytes the runner is about to commit. PRD requirement "Dispatch verify hooks `run_command` and `llm_verifier`" and design §6 ("PhaseRunner runs the configured verify hook against the canonical bytes") are partially honored; an escalating phase configured with `RunCommand` will silently never run that command against the artefact.
**Suggested fix:** Drop the `StrategyKind::Escalating` branch from the skip predicate and let the runner re-verify the canonical bytes. If the intent is to avoid double-verifying, gate on whether the strategy's last attempt already carries a passing `VerifyOutcome` of the same hook kind, and document the contract.

### F2 [high] Per-attempt `<phase>.started.<n>` markers promised by the design are never written for retries
**Where:** `src/phase_runner.rs:196` (only call site of `persist::start_attempt`)
**What:** `start_attempt` is invoked exactly once with attempt=0, before strategy dispatch. Design §3.1 and the resumability narrative ("recover the live attempt from disk alone", PRD requirement 5) require `markers/<phase>.started.<n>` for every attempt — the integration test for escalating recovery only checks `markers/implement.completed`, masking the gap. After a crash mid-rung-2, the on-disk state would say "attempt 0 started, no completion" with no record of the rungs the strategy already burned.
**Suggested fix:** Either (a) thread an attempt-callback into the strategies so they can call `markers.write_started(phase, n)` per rung/branch, or (b) document explicitly that started markers are coarse phase-level and adjust the design + resumability story to match.

### F3 [medium] `parallel + all_pass` test never exercises the all_pass aggregator path
**Where:** `tests/phase_runner_integration.rs:163-192`
**What:** The test wires `targets = [a, missing]` but only registers backend `"a"`. `ParallelFanOut` resolves backends up-front, so the missing backend produces a `BackendNotFound` → `StrategyError` → `error_class() == "strategy_failed"` before the all_pass aggregator is ever asked to fold branches. The assertion passes for the wrong reason: the test name promises "collects failures" but no aggregation happens. PRD acceptance test 3 (escalating retry over `0..2` with `all_pass` + LLM verifier) is also not covered — the retry test uses `LlmVerifier` only.
**Suggested fix:** Make the test register both backends, have `missing` return a `BackendError` from `query()` (so a real branch failure flows into the aggregator), and assert `error_class() == "aggregator_failed"` plus the failed-marker reason mentions branch indices. Add a separate test exercising the `concat + any_fail` PRD acceptance case.

### F4 [medium] Missing PRD acceptance test: `parallel + concat + any_fail` happy-path
**Where:** `tests/phase_runner_integration.rs` (no test for this combination)
**What:** PRD §"Acceptance tests" lists "Parallel + concat + any_fail with a passing command verifier merges three replicas, captures verify exit, and writes `review.completed`." The integration suite covers `parallel + concat + RunCommand` (two replicas) but not the `any_fail` aggregator inside a parallel run, which is its own dispatch path in `parallel_fanout.rs`. With it untested, no integration coverage proves PhaseRunner correctly bridges the inline `any_fail_evaluate` failure mode into a `failed` marker with a meaningful reason.
**Suggested fix:** Add `phase_runner_parallel_concat_any_fail_three_replicas_completed` matching the PRD wording (3 mock backends, all stdouts pass `any_fail_evaluate`, RunCommand verifier returning pass), plus a negative variant where one branch fails and produces `aggregator_failed`.

### F5 [medium] `archive_failed_attempt` writes a summary but no actual debris
**Where:** `src/phase_runner/persist.rs:14-25, 58-83`
**What:** The function creates `attempts/<phase>/<n>/` and `record_terminal_failure` drops `failure-summary.json` there, but no stdout/stderr/rendered-prompt/aggregator-input is ever copied. Design §7.1 promised "failed retry debris" is recoverable from disk; today only a 3-field JSON summary survives. For escalating retry the strategy already wrote per-attempt outputs to `<cwd>/attempts/<n>.txt`, so `attempts/<phase>/<n>/` ends up holding only the summary, which is confusing on a post-mortem.
**Suggested fix:** Either (a) move/copy the strategy's per-attempt output files into `attempts/<phase>/<n>/` and add `aggregate-input.json` for parallel runs, or (b) write a `README` in the archive directory documenting that per-attempt artefacts live under `<run_dir>/attempts/<n>.txt` and the summary is the canonical entry-point. Option (a) matches design intent.

### F6 [medium] `winning_attempt` returns a meaningless index for parallel strategies
**Where:** `src/phase_runner.rs:298-304`
**What:** `winning_attempt` does `rposition(|a| !verify.fail)`. For `ParallelFanOut`, attempts are concurrent branches in arbitrary order; the "last non-fail" is just whichever branch happened to land at the tail. The manifest entry persisted via `commit_success(...attempt=winning_attempt(...))` will record an attempt index that doesn't correspond to the branch whose bytes were actually selected by the aggregator (e.g. `Aggregator::First` in `dispatch::winning_success_bytes` reverse-iterates, but Vote/AnyFail use `first_success_bytes` which forward-iterates). Manifest attribution can disagree with the canonical bytes.
**Suggested fix:** Have `dispatch::canonical_bytes` return `(Vec<u8>, usize selected_attempt_index)` and pass that index to `commit_success`, so the manifest's `attempt` field always points at the branch whose bytes are on disk.

### F7 [low] Marker-write failures during terminal-failure paths are swallowed with `let _ =`
**Where:** `src/phase_runner.rs:206-213, 222-228, 265-271`
**What:** Three call sites do `let _ = persist::record_terminal_failure(...);` before propagating the original error. If the marker/manifest write itself fails (disk full, permissions), the operator sees only the upstream error and on-disk state silently lacks a `failed` marker — exactly the resumability scenario the design tries to prevent.
**Suggested fix:** At minimum log the inner error via `tracing::error!` (the crate already pulls in `tracing` indirectly) so it shows up in CLI output. Optionally wrap the original error with a `marker_persist_failed` variant when both fail.

### F8 [low] Synthesized `VerifyContext` loses fidelity expected by `RunCommand`
**Where:** `src/phase_runner.rs:245-257`
**What:** The runner builds a `VerifyContext` with `stdout = String::from_utf8_lossy(&canonical_bytes)`, `stderr = None`, `exit_code = None`, `duration = Duration::ZERO`. `RunCommand` and `LlmVerifier` were exercised in their own crates with realistic backend output; passing aggregated artefact bytes as if they were a backend's stdout is a contract drift that may surprise hooks doing exit-code checks.
**Suggested fix:** Either re-issue the verify hook against the *canonical artefact path* (read it from disk, give the hook a `cwd` + path) rather than synthesizing a fake stdout, or document on `VerifyContext` that runner-level verify always sees `exit_code = None` and `stdout = artefact`.

### F9 [low] `pass_failure_context` always set even when constructor default would suffice
**Where:** `src/phase_runner/dispatch.rs:54`
**What:** `EscalatingRetry::new(...).with_pass_failure_context(cfg.pass_failure_context)` is unconditional. The default is `false` and `PhaseConfig::single` initializes the field to `false`. Calling the builder with `false` forces the field to its default — harmless, but a future `EscalatingRetry::new` whose default flips would be silently overridden by the runner.
**Suggested fix:** `if cfg.pass_failure_context { builder.with_pass_failure_context(true) } else { builder }` — or just add a comment that the runner takes ownership of this flag intentionally.

### F10 [low] `dispatch::canonical_bytes` reads `aggregate_output_path` then re-runs the aggregator anyway
**Where:** `src/phase_runner/dispatch.rs:74-102`
**What:** When the strategy already wrote `aggregate_output_path`, the function returns those bytes — good. But the subsequent `if matches!(aggregator, Aggregator::First)` branch and the per-aggregator match below it call `aggregator.aggregate(...)` *again* (re-reading every per-branch file from disk) for cases where parallel didn't write an aggregate. For Vote/LLMJudge that re-aggregation is then *thrown away* (only `first_success_bytes` is returned). It's wasted IO and slightly misleading code: the only branches whose aggregator output is actually consumed are `Concat` and `AllPass`.
**Suggested fix:** Drop the unused `aggregator.aggregate(...)` call from the Vote/LLMJudge/AnyFail arms (just call `first_success_bytes(...)`), and only invoke the aggregator for `Concat` / `AllPass`.

## Verdict

**approve_with_changes**

Core composition compiles, `make check` is green (660+ unit tests, 9 phase-runner integration assertions, no clippy noise), the new dispatch/persist split is clean, and the additive `Aggregator::First`/`AllPass` schema labels do not regress existing strategies. However, two design-level promises are partially honored — the runner's own verify hook is bypassed entirely for escalating retries (F1), and per-attempt `started.<n>` markers required by the resumability story aren't emitted (F2) — and the most load-bearing acceptance test (`all_pass` collects branch failures, F3) currently passes for the wrong reason. F1/F2/F3 should be addressed before merge; F4-F10 can land as a follow-up if the CLO-292 scope needs to ship now, but I'd want them at least filed as Linear sub-issues against M5 so they don't sink into the manifest/tracing work.
