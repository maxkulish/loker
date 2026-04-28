**Must-Fix**
- None. `LLMJudge` no longer short-circuits on `min_responses`; it waits for all candidate futures and uses all successful candidates in the judge ballot flow.
- Aggregate artifact is now written to `aggregate_output_path` (`aggregated.txt`) in `ParallelFanOut` before returning success.
- `require_judge_different_family=false` now emits a warning via stderr.

**Should-Fix**
- None identified in working implementation.
