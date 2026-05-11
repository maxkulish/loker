# Review: CLO-327 Design - Wire Phase-Based Runner into `loker run`

**Reviewer**: Claude (senior architect)
**Document**: `docs/designs/clo-327-wire-phase-based-runner.md`
**Date**: 2026-05-10

## 1. Completeness Check

All seven required sections are present (Problem, Goals/Non-goals, Architecture,
Public API surface, Test plan, Migration/Rollout, Open questions). Section
headings differ slightly from the review template's "Summary, Background,
Architecture, Detailed Design, Implementation Plan, Acceptance Criteria" but
the loker designer template (per `task:phases:design`) is the authoritative
schema and is followed.

| Section | Present | Quality |
|---|---|---|
| Problem | yes | Clear; cites discovery report and baseline |
| Goals / Non-goals | yes | Goals concrete; non-goals explicit |
| Architecture | yes | Has ASCII diagram; data-flow per phase listed |
| Public API surface | yes | Real Rust signatures shown |
| Test plan | yes | Unit + integration named; manual verify step |
| Migration / Rollout | yes | Implementation order + back-compat note |
| Open questions | **no** | All five have inline `Decision:` answers — they are not actually open |

## 2. Architecture Assessment

**Strengths**
- Single decision point in `run_workflow()` keeps step-based path untouched.
  Aligns with discovery's recommended Approach A and the `src/main.rs:1458`
  TODO comment.
- Reuses existing `PhaseRunner` + `PhaseConfig` rather than re-implementing
  phase execution. This is correct — those abstractions are already proven via
  `ResumeRunner` and `tests/phase_runner_integration.rs`.
- Backend resolution via `create_backend()` is consistent with how
  `WorkflowRunner` and `conductor.rs` resolve backends today.

**Concerns**
- **Duplicates `TemplateEngine`**: `src/template/mod.rs` already exports a
  `TemplateEngine` backed by MiniJinja with `{{ steps.X.output }}` /
  `{{ env.X }}` / `{{ arg.N }}` syntax (registered filters, strict-undefined
  behavior, parse-error tests). The design proposes building a new
  `TemplateEngine` from scratch with custom `{{ spec }}` /
  `{{ phase.NAME.output }}` / `{{ var.X }}` syntax. This is the largest
  unaddressed concern — see Blind Spots #1.
- **Run directory layout in §Architecture is wrong**: the diagram shows
  `attempts/<phase>/<n>/<output>` (e.g. `attempts/design/0/design.md`) but
  `phase_runner::persist::commit_success()` writes successful artefacts to
  `run_dir.join(&cfg.artefact_name)` — i.e. `run_dir/design.md` directly. The
  `attempts/<phase>/<n>/` tree is only created on failure
  (`archive_failed_attempt`) for `failure-summary.json`. The design inherits
  this misunderstanding from the discovery report; both must be corrected
  before implementation.
- **`artefact_kind` mapping is unspecified**: `PhaseConfig.artefact_kind: Kind`
  determines manifest classification. `PhaseConfig::single()` hardcodes
  `Kind::DesignMd`. The `build_phase_config()` builder must map
  `phase.output` filenames (`design.md`, `review.md`, `plan.md`,
  arbitrary-name) to `Kind` variants. The design says nothing about this. AC #1
  in the PRD requires "design.md, review.md, plan.md" — what `Kind` does each
  get?
- **`Producer` mapping is unspecified**: `PhaseConfig.producer: Producer` is
  also hardcoded to `Producer::Single` in `PhaseConfig::single`. With
  `ParallelFanOut` and `EscalatingRetry` strategies, the producer must change.
  The builder needs an explicit mapping table.
- **Detection mechanism is ambiguous**: §Architecture shows
  `text.parse::<grammar::Workflow>()` first, falling through on failure. But
  `grammar::Workflow::FromStr::Err = Vec<WorkflowError>` includes TOML parse
  errors, malformed-backend errors, and empty-phases errors. A mistyped
  step-based file ("forgot the `[[steps]]` table") would produce
  `WorkflowError::NoPhases` from the grammar parser — that error is not
  diagnostic that "this is the wrong path; try step-based". The design needs
  to commit to either:
  (a) peek for a `[[phases]]` array (fast, unambiguous; suggested by discovery),
  or (b) pick a discriminator that distinguishes "wrong grammar" from
  "wrong content" (e.g. check `phases` field is non-empty and `steps`
  field is absent in the parsed TOML value).
  Falling through on any grammar error masks real syntax errors in
  phase-based files behind a "no phases found" step-based fallback.

## 3. Alignment with Handoff & Roadmap

- **Active milestone mismatch**: Project CLAUDE.md says **"v0 shipped (tag
  `v20260509.0.0`); M1-M11 complete; no active milestone — awaiting v1
  scope"**. The review template's stated milestone "M1 - TensorZero backend"
  is stale — that shipped a fortnight ago. CLO-327 is not in
  `docs/plans/002-v1-backlog-draft.md` as drafted, so this task currently
  has no milestone home. Suggest asking the user whether CLO-327 belongs in
  the v1 backlog or is a v0 follow-up bug fix.
- **Handoff intent alignment**: Design respects "New primitives land as new
  modules" — `PhaseWorkflowRunner` + `run_phase_workflow()` are additive.
  Step-based `WorkflowRunner` is untouched.
- **TDD-first**: §Test plan lists failing-test contracts before implementation
  in §Migration order (test → implement). Good.
- **Backend mocking**: Test plan uses mock backends and references the
  existing `tests/phase_runner_integration.rs` pattern. Aligns with handoff
  rule "mock the HTTP layer (`wiremock`) before writing the impl".
- **PRD acceptance-criteria contradiction**: PRD AC #7 = "Honour `--resume` for
  phase-based workflows". Design moves this to non-goals as "deferred". This
  is a scope shrink from the approved PRD and must either be (a) re-approved
  with the user or (b) re-instated. The design lists this as Open Question #1
  with a `Decision: Defer` answer, but the PRD already gave the answer
  (do it). The designer cannot unilaterally override an approved AC.

## 4. Security Review

The design does not introduce backend secrets or shell execution. Spec/var
inputs flow through templates into prompts, which is the existing pattern.
Two security-adjacent observations:

- **Prompt injection via spec content**: `{{ spec }}` substitutes raw spec
  bytes into the prompt template. If a malicious spec contains text designed
  to alter the LLM's behavior, that's the user's risk to manage (same as
  step-based runner today). Worth a one-line non-goal note.
- **Path traversal on `phase.output`**: `phase.output` is a string from the
  workflow file. If the value is `"../etc/passwd"` or absolute, the persist
  path would escape the run dir. `commit_success()` joins via
  `run_dir.join(&cfg.artefact_name)` — `Path::join()` with an absolute path
  would replace the prefix. The grammar should validate `phase.output` is a
  relative, non-traversing filename. Verify whether `grammar.rs::validate()`
  already does this; if not, add it.
- **Template-engine sandbox**: if the design adopts the existing MiniJinja
  engine (recommended), strict-undefined behavior is already configured. If
  it ships a custom engine, the security review must include a list of
  allowed/disallowed substitution shapes.

## 5. Implementation Concerns

- **`make check` testability**: design's named tests are unit tests in
  `src/workflow/` and integration in `tests/`, both runnable under
  `cargo test` with no env vars. `make check` will exercise them. Good.
- **Phasing**: 6-step implementation order is reasonable, but step 1 ("phase
  detection in `run_workflow()`") is dependent on step 2 ("template engine")
  and step 3 ("PhaseConfig builder") to be useful — a wired-but-empty
  detection is dead code. Recommend reordering: (a) builder, (b) template,
  (c) `run_phase_workflow()`, (d) detection wiring in `run_workflow()`,
  (e) integration test, (f) manual verify.
- **`load_workflow_from_source` refactor**: the design says "modify
  `load_workflow_from_source` ... to return raw text alongside the parsed
  AST." This function is called from non-`run_workflow` sites (e.g.
  `loker explain`). Either change all call sites or add a new
  `load_workflow_text` that returns the raw text, leaving the existing
  function alone. The design should specify which.
- **Test count**: 12 unit tests + 4 integration tests is appropriate. Missing:
  a test for the detection-disambiguation case (a phase-based file with a
  syntactic error — does it correctly report a phase-grammar error rather
  than silently falling through to step-based?).

## 6. Concurrency & Async

- **Phase iteration**: design states "walk phases sequentially". Correct —
  later phases depend on earlier outputs. No parallelism intended.
- **Cancellation safety not addressed**: if the user `Ctrl-C`s mid-phase,
  what state is left on disk? `PhaseRunner` writes `started.<n>` markers and
  archives failures. The design should explicitly say: "interrupted runs
  leave a `started.<n>` marker without a matching `completed`; `--resume`
  (deferred) would resume from the next attempt; today, the user re-runs
  fresh and a new run dir is created." This is implicit in current behavior
  but should be called out.
- **Blocking calls in async path**: design imports `tokio::fs` for spec
  reading (existing pattern in `run_workflow`) but does not specify whether
  the new `run_phase_workflow()` is `async fn`. The signature in §Public API
  surface says `pub async fn`. Verify all internal file-IO uses
  `tokio::fs::*` or is acceptable to do via blocking `std::fs` (the existing
  `phase_runner::persist::commit_success` uses blocking `std::fs` —
  acceptable since it's a small write under spawn_blocking semantics in
  practice, but design should acknowledge).
- **No `select!` / cancellation tokens are introduced**, which matches the
  current pattern. Good.

## 7. Blind Spots

1. **Existing `TemplateEngine` ignored** (highest priority). The design
   reinvents a template engine that already exists at `src/template/mod.rs`
   with MiniJinja, custom filters, semi-strict undefined behavior, and a
   complete test suite. The grammar's documented syntax (`{{ spec }}`,
   `{{ phase.NAME.output }}`, `{{ var.X }}`) can be expressed as a MiniJinja
   context shape — the existing engine handles render, error mapping, and
   parse errors. Building a new custom engine duplicates work and creates
   inconsistency between step-based templates (`{{ steps.x.output }}`) and
   phase-based templates (`{{ phase.x.output }}`). Recommend: extend the
   existing `TemplateContext` with `spec`, `phase`, `var` keys; reuse
   `TemplateEngine::render()`. If the grammar truly mandates a different
   template language, raise that explicitly as an open question.
2. **`Kind` and `Producer` mapping for builder**: how does
   `build_phase_config` pick `Kind::DesignMd` vs `Kind::ReviewMd` vs
   `Kind::PlanMd` vs other? Design is silent.
3. **Multi-attempt persistence**: `PhaseRunner` returns one `PhaseOutcome` per
   call. For `ParallelFanOut` and `EscalatingRetry`, multiple backend
   attempts happen inside one `PhaseRunner::run` call. The design's
   manifest entries section says "manifest entries appended" (plural) but
   `commit_success()` produces one entry per phase call. If the AC requires
   one entry per attempt (rather than one per phase), the design must say
   so and `PhaseRunner` may need an enhancement.
4. **Trace integration**: `PhaseInputs::trace: Option<&dyn TraceSink>` exists
   on the runner. Design says "no tracing in v1" and passes `None`. The
   broader project ships `trace.jsonl` per run dir; switching it off here
   creates a regression vs. step-based runs that emit traces. Recommend
   wiring `trace_writer::JsonlSink` (or whatever the step-based runner uses)
   so `runs/<id>/trace.jsonl` is consistent across both runner paths.
5. **`--rerun` semantics**: design accepts `rerun_phases: Vec<String>` but
   doesn't specify how it influences a fresh (non-resume) phase walk. Does
   `--rerun design` mean "skip everything except design"? "force re-run of
   design only, even if outputs exist"? In a fresh run there's nothing to
   skip. Either treat `--rerun` as resume-only (and validate) or define the
   fresh-run semantics.
6. **Workflow-level `defaults` field**: `grammar::Workflow.defaults` is
   parsed but not consumed in the design. Either explicit non-goal or a
   wiring note.
7. **`extends` field**: step-based `Workflow` has `extends: Option<String>`.
   Phase-based `grammar::Workflow` does not. Design should call out that
   workflow inheritance is unavailable for phase-based grammars (or whether
   it should be added).
8. **Open-question template violation**: per the designer template hard
   rules, "Leave open questions genuinely open. Do not fabricate
   resolutions." All 5 open questions have inline `Decision:` answers.
   Either move resolved items into the body of the design (where they
   belong) or genuinely leave them open with the tradeoff stated.

## 8. Verdict

**NEEDS_REVISION**

The architectural shape is correct (Approach A from discovery, additive,
backward-compatible), but four issues must be resolved before plan/implement:
template-engine duplication, the wrong run-dir-layout diagram, missing
`Kind`/`Producer` mapping for the phase-config builder, and the unilateral
deferral of `--resume` despite PRD AC #7. Open questions are also closed,
which violates the designer template.

## 9. Actionable Feedback

In priority order:

1. **(P0) Reuse existing `TemplateEngine`**. Replace the new `TemplateEngine`
   in §Public API with an extension to `crate::template::TemplateContext`
   that exposes `spec`, `phase.<name>.output`, and `var.<name>`. If the
   design must introduce a separate engine, document the reason and surface
   it as a true open question.
2. **(P0) Fix the run-directory-layout diagram**. Describe what
   `phase_runner::persist::commit_success()` actually does:
   `run_dir/<artefact_name>` for success, `run_dir/attempts/<phase>/<n>/`
   only for failure-summary archives. Update the discovery report
   accordingly to prevent the next reader from inheriting the same error.
3. **(P0) Resolve `--resume` contradiction with PRD AC #7**. Either
   (a) include phase-based `--resume` in scope and update the design with a
   `ResumeRunner`-integration sketch, or (b) get the PRD amended (with the
   user) before deferring. Do not silently drop an approved AC.
4. **(P1) Specify `Kind` and `Producer` mapping** in
   `build_phase_config()`: how is `Kind` derived from `phase.output`
   filename (extension-based? phase-name-based?), and how is `Producer`
   derived from `Strategy` (Single → `Producer::Single`,
   ParallelFanOut → `Producer::Parallel`, EscalatingRetry → ?).
5. **(P1) Commit to a detection mechanism**. Either peek for `[[phases]]`
   string presence (cheap and unambiguous, recommended by discovery) or
   parse the TOML once into `toml::Value`, inspect for `phases` vs `steps`
   keys, then deserialize into the appropriate type. Document why the chosen
   mechanism doesn't mask phase-grammar syntax errors as "wrong dispatch".
6. **(P1) Validate `phase.output` filename**: confirm
   `grammar::Workflow::validate()` rejects absolute paths and `..`
   components in the `output` field; if not, add a validation rule.
7. **(P2) Wire trace.jsonl** to phase-based runs to match step-based
   behavior, or explicitly note the regression and a follow-up issue.
8. **(P2) Specify `--rerun` semantics for fresh phase-based runs** (or
   make it resume-only).
9. **(P2) Re-open the open questions**. Move "decided" items into the
   relevant body section (resume → migration; template syntax → public
   API). Anything still genuinely open (e.g. milestone home for CLO-327)
   stays in the section.
10. **(P3) Mention prompt-injection-via-spec as a non-goal** for clarity,
    matching the existing step-based posture.
11. **(P3) Confirm milestone home with user**: CLAUDE.md says no active
    milestone; CLO-327 is not in `docs/plans/002-v1-backlog-draft.md`.
    Either add to v1 backlog or treat as v0 follow-up bug-fix and document.
