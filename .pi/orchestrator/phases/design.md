# Phase: design

Translate the chosen discovery approach into a concrete design document.
Run an AI design review, classify feedback, and finalize.

Mirrors `.claude/commands/task/phases/design.md`.

## Required exit state

```yaml
phases:
  design:
    status: complete
    design_doc: "docs/designs/clo-XX-<slug>.md"
    draft_ready: true
    discovery_context_used: true
    review_completed: true
    review_gemini: "docs/reviews/clo-XX-design-gemini.md"
    review_synthesis: "docs/reviews/clo-XX-design-synthesis.md"
    review_verdict: "approve" | "approve_with_changes" | "rework"
    finalized: true
    applied_suggestions: []
    flagged_suggestions: []
```

History events required: `design_draft_ready`, `design_review_complete`,
`design_finalized`.

## Step 1 - Draft the design

Read the discovery report and the canonical design at
`/Users/mk/Work/investigations/sakana-fugu/loker-design.md` for any
relevant section references.

Write `docs/designs/clo-XX-<slug>.md` with:

- Problem (1 paragraph, citing discovery)
- Goals / Non-goals
- Architecture (modules, data flow, concrete types)
- Public API surface (Rust trait/struct signatures)
- Test plan (unit, integration, manual)
- Migration / rollout
- Open questions

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "design",
  action: "design_draft_ready",
  details: "Design doc draft at docs/designs/clo-XX-<slug>.md",
  phase_updates: {
    status: "in_progress",
    design_doc: "docs/designs/clo-XX-<slug>.md",
    draft_ready: true,
    discovery_context_used: true
  }
})
```

## Step 2 - AI design review

If `.lok/workflows/design-review.toml` exists:

```bash
lok run .lok/workflows/design-review.toml \
  docs/designs/clo-XX-<slug>.md \
  CLO-XX \
  "<task title>" \
  --dir . --verbose
```

This produces gemini + synthesis review files in `docs/reviews/`.

If lok tooling is unavailable in this repo, you may invoke the persona
directly via the agent: `pi run gemini-architect --input
docs/designs/clo-XX-<slug>.md`. If neither path works, set:

```yaml
review_completed: true
review_skip_reason: "<concrete reason>"
```

## Step 3 - Apply review feedback

Classify each suggestion:

| Class | Action |
|---|---|
| **Additive** (new test, doc clarification, edge case) | Apply immediately. Add to `applied_suggestions`. |
| **Refinement** (renames, signature tweaks) | Apply if cheap, else defer. |
| **Contradicts** the chosen approach | Do NOT apply. Add to `flagged_suggestions` with reason. |

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "design",
  action: "design_review_complete",
  details: "Review verdict: approve_with_changes. <n> applied, <m> flagged.",
  phase_updates: {
    review_completed: true,
    review_gemini: "docs/reviews/clo-XX-design-gemini.md",
    review_synthesis: "docs/reviews/clo-XX-design-synthesis.md",
    review_verdict: "approve_with_changes",
    applied_suggestions: ["..."],
    flagged_suggestions: [{ id: "...", reason: "..." }]
  }
})
```

## Step 4 - Finalize

Re-read the design doc end-to-end. Make sure:

- Public API signatures compile mentally
- Test plan is concrete enough for `plan` phase to enumerate sub-tasks
- All open questions are resolved or moved to follow-up issues

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "design",
  action: "design_finalized",
  details: "Design ready for plan phase",
  phase_updates: {
    status: "complete",
    finalized: true
  }
})

transition_phase({
  task_id: "CLO-XX",
  from_phase: "design",
  to_phase: "plan"
})
```

## Notes

- Loker has no separate `review` phase. The codex+gemini validation
  gate runs inside `implement.md` step 5 - that gate is for *code*, this
  step is for *design*. Both are required.
- If the design fundamentally changes scope, return to `discovery` via
  the user rather than forcing a transition.
