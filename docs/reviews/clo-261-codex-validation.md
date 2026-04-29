# Codex pre-PR validation - CLO-261

## Context
- Branch: `feat/clo-261-back`
- Plan / Spec: `docs/plans/clo-261-tensorzero-create-backend-wiring.md`
- Design: `docs/designs/clo-261-tensorzero-create-backend-wiring.md`

## Checklist
- [x] cargo fmt --check
- [ ] cargo clippy -D warnings
- [ ] cargo test
- [ ] make check green
- [ ] All ACs covered
- [ ] No unintended public surface
- [ ] Error handling
- [ ] Tests
- [ ] Schema / docs

## Findings
### F1 [blocker] Pre-merge lint gate is red, so the branch is not push-safe
**Where:** `examples/tensorzero_spike.rs:158`; `tests/strategy_parallel_fanout.rs:164`; `src/strategy/parallel_fanout.rs:479`  
**What:** `cargo clippy --all-targets --all-features -- -D warnings` fails before the review can clear item 1 of the checklist. The reported errors are `clippy::ptr_arg` on `&PathBuf`, `clippy::len_zero` on `len() >= 1`, and `unused_imports` in `parallel_fanout` tests. These files are outside the CLO-261 diff, but they still block the required gate and therefore block PR readiness for this branch.  
**Suggested fix:** Change `write_fixture(dir: &PathBuf, ...)` to `&Path`, replace the lower-bound length check with `!out.attempts.is_empty()`, and remove the unused `BranchFailure` / `BranchSuccess` imports; then rerun the full gate.

## Verdict
rework

Checklist item 1 failed at `cargo clippy --all-targets --all-features -- -D warnings`, so I stopped there per the validation gate. Until the lint gate is clean, I cannot sign off `cargo test`, `make check`, or the spec/coverage/public-surface review as complete.
