# Phase: spec

Specification-task path. Produce a focused 5-section spec document at
`specs/YYYY-MM-DD-clo-XX-<slug>.md`, run AI review (lok pipeline), and
transition straight to `implement` (no design / plan phase).

Mirrors `.claude/commands/task/phases/spec.md`.

## Required exit state

```yaml
phases:
  spec:
    status: complete
    spec_file: "specs/YYYY-MM-DD-clo-XX-<slug>.md"
    approved: true
    auto_approved: true | false
    auto_approval_reason: "..."        # if auto_approved
    review_completed: true
    review_skip_reason: "..."          # only if review tooling unavailable
    review_gemini: "docs/reviews/clo-XX-spec-gemini.md"   | null
    review_ollama: "docs/reviews/clo-XX-spec-ollama.md"   | null
    review_synthesis: "docs/reviews/clo-XX-spec-synthesis.md" | null
    review_verdict: "approve" | "approve_with_changes" | "rework" | null
    review_applied: true | false
    applied_suggestions: []
    flagged_suggestions: []
```

History events required: `spec_approved`. Optional:
`phase_completed`, `spec_review_complete`.

## Step 1 - Branch

If `linear.branch_actual` is empty:

```bash
git checkout main && git pull
git checkout -b feat/clo-XX-<slug>
```

Record via `update_workflow_state` as in `discovery.md` Step 1.

## Step 2 - Write the spec

Path: `specs/<today>-clo-XX-<slug>.md`. Use this 5-section structure:

```markdown
# CLO-XX <title>

**Status:** draft
**Type:** specification
**Linear:** https://linear.app/cloud-ai/issue/clo-xx/...
**Design context:** /Users/mk/Work/investigations/sakana-fugu/loker-design.md §<n>

## 1. Problem and goal
<3-5 sentences: what we are building and why>

## 2. Acceptance criteria
- [ ] AC1 ... (verifiable: `<command>`)
- [ ] AC2 ... (verifiable: `<command>`)
... (target ~10 ACs, every one with an explicit verification command)

## 3. Sub-tasks
### ST1 <verb> <component>
**Files:** src/...
**Tests:** tests/...
**Estimate:** S | M | L

### ST2 ...

## 4. Evaluation table
| # | Scenario | Input | Expected | Verification |
|---|---|---|---|---|
| 1 | ... | ... | ... | `cargo test ...` |

## 5. Edge cases
- Edge 1: ... -> handled by ...
- Edge 2: ... -> handled by ...
```

Save and record:

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "spec",
  action: "spec_drafted",
  details: "Spec at specs/<today>-clo-XX-<slug>.md (<n> ACs, <m> sub-tasks)",
  phase_updates: {
    status: "in_progress",
    spec_file: "specs/<today>-clo-XX-<slug>.md"
  }
})
```

## Step 3 - AI spec review (if available)

If `.lok/workflows/spec-review.toml` exists:

```bash
lok run .lok/workflows/spec-review.toml \
  specs/<today>-clo-XX-<slug>.md \
  CLO-XX \
  "<task title>" \
  "<short description>" \
  "<labels>" \
  --dir . --verbose
```

Outputs:

- `docs/reviews/clo-XX-spec-gemini.md`
- `docs/reviews/clo-XX-spec-ollama.md`
- `docs/reviews/clo-XX-spec-synthesis.md`

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "spec",
  action: "spec_review_complete",
  details: "Verdict: <verdict>. Applied: <n>. Flagged: <m>.",
  phase_updates: {
    review_completed: true,
    review_gemini: "docs/reviews/clo-XX-spec-gemini.md",
    review_ollama: "docs/reviews/clo-XX-spec-ollama.md",
    review_synthesis: "docs/reviews/clo-XX-spec-synthesis.md",
    review_verdict: "<verdict>",
    review_applied: true,
    applied_suggestions: [...],
    flagged_suggestions: [...]
  }
})
```

If `.lok/workflows/spec-review.toml` is not present (current loker
state), record the skip:

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "spec",
  action: "spec_review_skipped",
  details: "AI review tooling unavailable; marking review_completed=true",
  phase_updates: {
    review_completed: true,
    review_skip_reason: "No .lok/workflows/spec-review.toml present"
  }
})
```

## Step 4 - Approval checkpoint

Auto Mode auto-approves if:

- All required exit fields populated
- Every AC has an explicit verification command
- All sub-tasks reference real files / modules
- The task is mechanically testable (no architecture decisions hiding
  inside the spec)

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "spec",
  action: "spec_approved",
  details: "<auto_approval_reason or human approval note>",
  phase_updates: {
    status: "complete",
    approved: true,
    auto_approved: true,
    auto_approval_reason: "..."
  }
})
```

## Step 5 - Transition

Specification tasks skip `plan` entirely - the spec's sub-tasks ARE the
plan.

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "spec",
  action: "phase_completed",
  details: "Transitioning spec -> implement. Skipping plan phase."
})

transition_phase({
  task_id: "CLO-XX",
  from_phase: "spec",
  to_phase: "implement"
})
```

The `implement` phase will still run the codex+gemini validation gate.

## Notes

- If the spec turns out to require architecture decisions, abort: set
  `task_type = development` and route to `discovery`/`design`.
- The spec file lives under `specs/`, not `docs/specs/` - that path
  matches existing repo convention (see CLO-247/248/249/251/257).
