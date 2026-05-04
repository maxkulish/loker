# Pre-PR validation: clo-291

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-04
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [high] Resume idempotency is not actually tested
**Where:** tests/m6_design_doc_tdd_e2e.rs:652-701
**What:** The ST8 test claims to verify resume-idempotency but never re-runs the pipeline against the same `RunDir`. It only inspects markers after a single run. The plan (ST8, line 105) and design (ST7, lines 167, 257-259) both explicitly require: "Re-running the same `RunDir` produces no new manifest entries, no new markers." The current code does not satisfy that acceptance criterion. Worse, the marker assertions are wrapped in `if markers_path.exists()` (line 687), so if the `markers/` directory layout ever changes the assertions silently pass.
**Suggested fix:** After the first call to `run_mocked_design_doc_tdd()`, snapshot the manifest entries and marker files, then re-run the same workflow against the existing `RunDir` (need a helper variant that takes an existing `RunDir`), and assert: same manifest entry count, same sha256s, same marker contents, zero backend invocations on the second pass. Drop the `if markers_path.exists()` guard — assert the path exists.

### F2 [high] ST9 live-mode test panics with `unimplemented!()` if invoked
**Where:** tests/m6_design_doc_tdd_e2e.rs:707-711
**What:** ST9 (live mode) is a listed deliverable in both the design (lines 169-173) and plan (ST9, lines 111-119). The shipped test is a panicking stub. If a maintainer ever runs `cargo test -- --ignored` or sets `LOKER_M6_INTEGRATION=1`, they get `unimplemented!()` instead of either a real run or a graceful skip.
**Suggested fix:** Either (a) implement the live path that pivots on the env var and skips silently when unset, or (b) remove the stub function entirely and update the plan/design to defer ST9 to a follow-up ticket. Shipping a panicking stub claims completeness without delivering it.

### F3 [medium] `Box::leak` in `PhaseFixtures::load` leaks per test invocation
**Where:** tests/m6_design_doc_tdd_e2e.rs:166-175
**What:** `Box::leak` is used 4× per `PhaseFixtures::load()` to coerce fixture content into `&'static str`. Each of the 6 tests invokes `run_mocked_design_doc_tdd()`, leaking 24 strings per `cargo test` run. This is acknowledged as a leak by Rust semantics. The reason it's there is `M6MockBackend.output: &'static str`. Tests are short-lived so it's not catastrophic, but it's a code smell that signals a wrong API choice and will surprise anyone running with leak-checking tools.
**Suggested fix:** Change `M6MockBackend.output` to `Arc<str>` or `String` and clone-on-construct. Drop the `Box::leak`. Same applies to `name` and `model` fields.

### F4 [medium] Template substitution is rendered but never asserted on
**Where:** tests/m6_design_doc_tdd_e2e.rs:122-143, 203-207, 238-242, 288-292, 331-335
**What:** The test renders templates via `Template::render()` and stores the result in `PhaseRun.rendered_prompt`, but no test asserts the rendered prompt contains the calculator spec content or upstream phase outputs. If `Template::render()` silently returned its input unchanged (or stripped placeholders), every test still passes. This defeats the integration coverage goal — template substitution is the CLO-289 dependency and the ST5 design plan calls for asserting `{{ spec }}`, `{{ phase:design.output }}` resolve.
**Suggested fix:** Add an assertion in the smoke test (or a new ST) that `phase_runs[0].rendered_prompt` contains a recognizable substring of the calculator spec, and that `phase_runs[1].rendered_prompt` contains the design fixture content (proving upstream substitution works).

### F5 [medium] `make check` is broken on this branch (clippy + test compile errors)
**Where:** src/resume/sweep.rs:130, tests/run_state_markers.rs:42/65/93/113
**What:** Pre-merge gate per CLAUDE.md is `make check` = fmt + clippy + test. Running `cargo clippy --all-targets -- -D warnings` fails with `clippy::needless_borrows_for_generic_args` on `src/resume/sweep.rs:130` and `unused_variables` errors on `tests/run_state_markers.rs`. These errors exist on `main` as well (verified via `git stash`), so they are pre-existing, not introduced by this PR — but they will block this PR's merge if the gate is enforced.
**Suggested fix:** Either fix in this PR (small, mechanical) so the gate passes, or open a separate trivial cleanup PR ahead of merge. Do not merge while the gate is red.

### F6 [low] Design vs implementation mismatch on `changes/` artefact shape
**Where:** docs/designs/clo-291-m6-e2e-integration-test.md:74; tests/m6_design_doc_tdd_e2e.rs:298, 627-632
**What:** The design table (line 74) says the implement phase produces `changes/` (a directory). The actual `commit_success` in `src/phase_runner/persist.rs:35-41` writes the bytes to a single file at `run_dir/changes`. The test correctly asserts on the file (not a directory), but neither the design nor the test flags the discrepancy. This is a latent issue — the ChangesDir kind doesn't currently produce a directory.
**Suggested fix:** Either update the design to reflect that `Kind::ChangesDir` currently writes a flat artefact (with a TODO for actual directory persistence), or open a follow-up ticket. At minimum, add a comment in the test next to the `changes` assertion explaining the shape.

### F7 [low] Loose trace assertions allow silent regression
**Where:** tests/m6_design_doc_tdd_e2e.rs:533-537, 580-583
**What:** ST6 asserts `spans.len() >= 6` and `!backend_spans.is_empty()`. The actual deterministic count is 4 phase_started + 4 backend + 4 aggregator_fold + 1 verify + 4 phase_finished = 17 spans, with 5 backend calls (1 design + 2 review + 1 implement + 1 verify). A regression that drops spans (e.g., aggregator stops emitting) won't be caught.
**Suggested fix:** Tighten to exact counts, or at minimum specific minimums per category: `phase_starts == 4`, `backend_spans >= 5`, `aggregator_spans == 4`, `verify_spans == 1`.

### F8 [low] Dead/unused fields in test scaffolding
**Where:** tests/m6_design_doc_tdd_e2e.rs:150-156, 159-164
**What:** `PhaseRun.outcome`, `PhaseRun.rendered_prompt`, and the entire `PhaseFixtures` return value are unused (every test pattern-matches them as `_`). `#[allow(dead_code)]` is on `PhaseRun`. This is dead scaffolding that violates the project rule "Don't add features beyond what the task requires."
**Suggested fix:** Drop the unused struct fields and stop returning `PhaseFixtures`. If F4 is addressed by asserting on `rendered_prompt`, keep that one field.

### F9 [low] Numbering inconsistency between design, plan, and code
**Where:** docs/designs/clo-291-...md (ST1-ST8), docs/plans/clo-291-...md (ST1-ST9), tests/m6_design_doc_tdd_e2e.rs (ST3-ST9 in comments), commit message ("ST2-ST8")
**What:** The sub-task numbering drifts across artefacts. The design uses ST1=smoke, the plan uses ST3=smoke. This will confuse anyone tracing requirements.
**Suggested fix:** Pick one numbering (the plan's, since the implementation comments match it) and update the design to match, or add a mapping table at the top of one file.

## Verdict
**rework**

The wiring works and the mock-backend pipeline executes end-to-end (all six mocked tests pass in ~0.68s, well under the 5s budget), so the foundation is sound. But two stated deliverables are missing in substance: ST8 does not actually test resume-idempotency (no second run, no comparison of pre/post state), and ST9 is a panicking stub rather than either a real implementation or a clean deferral. Together with the unverified template substitution (F4) — which is the whole reason CLO-291 was gated on CLO-289 — these gaps mean the test does not yet prove what the M6 exit gate requires. The `Box::leak` and broken `make check` on `main` are smaller but should be addressed before this lands. Recommend a focused follow-up that adds the second-run assertion, replaces or deletes the live-mode stub, and asserts on rendered prompts; once those land this is straightforward to approve.
