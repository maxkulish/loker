# Design: Pi orchestrator flow improvements

**Date:** 2026-05-01
**Scope:** loker `.pi/` (Pi extension) only. The `.claude/commands/task/` command set is out of scope.
**Status:** Draft for user review.

## Problem

The current loker orchestrator auto-dispatches every phase end-to-end after `init`. That works when one model can do every phase, but it breaks two things the user actually wants to do:

1. **Switch models per phase.** Discovery and plan/implement are fast-and-cheap work; design needs a slower, smarter model. Today there is no boundary at which the orchestrator pauses long enough for the user to switch Pi's active model.
2. **Human-review the final design doc.** Today design ends with two AI review iterations (Gemini single-shot, plus a synthesis pass) that auto-apply feedback, then the orchestrator transitions straight into `plan` and `implement`. The user has no built-in checkpoint to read the post-AI-review design doc and approve, annotate, or reject it before code starts landing.

Both problems share one fix surface: the orchestrator needs declarative pause points at phase transitions, and design needs an explicit human-review gate before `finalized: true`.

## Goals

- Add per-phase pause flag in `PHASE_CONFIG`. When set, `transition_phase` records the transition but does **not** auto-dispatch the next phase. The orchestrator prints an instruction banner instead and exits. The user resumes manually via `/task:orchestrate CLO-XX`.
- Wire two pause points: `discovery → design` and `design → plan`. These are the model-switch boundaries.
- Add a plannotator review gate inside `design.md` between "apply AI feedback" and "finalize". On approval, design completes normally. On denial, orchestrator captures annotations to YAML, prints them, and stops the design phase.
- Print pause banners that name the boundary and the resume command, but do **not** name a specific model. The user is empirically discovering which model is "smart enough" for design.

## Non-goals

- **No automatic model switching.** The orchestrator never edits Pi's model config or invokes a model-swap tool. The pause is purely declarative; the human flips the model.
- **No `next_model_hint` field yet.** That comes in a follow-up after the user has tested several "smart enough" candidates and picked one.
- **No auto-fix loop on plannotator denial.** Denial = print annotations, exit phase, user edits the design doc by hand and re-fires the gate. (Q3 answer: option b.)
- **No new pause point at `implement → pr`.** The validation gate already exists inside `implement.md` step 5; adding another pause there is out of scope.
- **No symmetric reviewer roster change in this spec.** Design currently runs Gemini single-shot + synthesis; implement runs Codex + Gemini + synthesis. Aligning them is a separate concern flagged in "Follow-ups", not folded in here.
- **No `ctx.compact()` plumbing.** Loker may already have it; verifying and wiring it explicitly between phases is a separate spec if needed.

## Architecture

### Pause mechanism

`PHASE_CONFIG` in `.pi/extensions/orchestrate/index.ts` gains an optional `auto_dispatch_after_transition` boolean (default `true`).

```ts
const PHASE_CONFIG: Record<string, {
  requiredFields: string[];
  historyEvents: string[];
  auto_dispatch_after_transition?: boolean;
}> = {
  discovery: {
    requiredFields: ["status"],
    historyEvents: ["discovery_approved"],
    auto_dispatch_after_transition: false,
  },
  design: {
    requiredFields: ["status", "design_doc", "draft_ready", "finalized", "review_completed", "human_review_completed"],
    historyEvents: ["design_draft_ready", "design_review_complete", "design_human_review_complete", "design_finalized"],
    auto_dispatch_after_transition: false,
  },
  // other entries unchanged
};
```

Inside the `transition_phase` tool handler, after the state mutation succeeds, the dispatcher consults the **outgoing** phase's flag:

```ts
const fromConfig = PHASE_CONFIG[params.from_phase];
const shouldAutoDispatch = fromConfig?.auto_dispatch_after_transition !== false;

if (shouldAutoDispatch) {
  await dispatchPhase(pi, params.task_id, params.to_phase, state, statePath, workspaceRoot);
} else {
  await emitPauseBanner(pi, params.task_id, params.from_phase, params.to_phase);
}
```

`emitPauseBanner` calls `pi.sendUserMessage` with a templated, self-contained banner that does not rely on conversation memory:

```
============================================================
PAUSE: <from_phase> → <to_phase> boundary
============================================================
Phase <from_phase> is complete. Workflow YAML is updated;
current_phase is now <to_phase>.

This boundary is a model-switch point. Switch to your
preferred model for <to_phase> work, then resume:

  /task:orchestrate <task_id>

The next phase will not run until you do.
============================================================
```

The banner is intentionally model-agnostic in v1. A future iteration adds an optional `next_model_hint` field to the config; the banner template will surface it as "Recommended: <hint>" when present.

### Resume path

`/task:orchestrate CLO-XX` already reads the workflow YAML, sees `workflow.current_phase` (which `transition_phase` already updated to `to_phase`), and dispatches that phase's markdown. No change needed in the resume path. The pause is purely a one-shot skip of the inline `dispatchPhase` call inside `transition_phase`.

### Plannotator gate inside design

Plannotator integrates via its slash commands (`/plannotator-review <file>`), which are run by the model executing `design.md`. We do **not** use plannotator's plan-mode takeover or the shared event API in v1 — slash commands are the simplest surface and match how the user already runs annotations.

The current `design.md` has 4 steps: draft → AI review → apply feedback → finalize+transition. The new flow inserts a human-review gate as a new Step 4, pushing finalize to Step 5:

```
Step 1 - Generate the design draft (claude-designer)
Step 2 - AI design review (gemini + synthesis)
Step 3 - Apply review feedback (classify + apply non-contradicting)
Step 4 - HUMAN REVIEW via plannotator (NEW)
Step 5 - Finalize and transition
```

Step 4 instructs the model to:

1. Print a one-line summary of what changed in Step 3 (which suggestions applied, which flagged).
2. Run `/plannotator-review docs/designs/clo-XX-<slug>.md`.
3. Read the result. Plannotator returns either approved or denied-with-annotations.
4. **If approved:** record `human_review_completed: true`, add history event `design_human_review_complete`, advance to Step 5.
5. **If denied:** capture the annotation text into `phases.design.plannotator_annotations` (string), record `human_review_completed: false`, add history event `design_human_review_denied` with the annotation summary in `details`, then **stop**. Do not transition. Print the annotations and tell the user to edit the design doc and re-run `/task:orchestrate CLO-XX`. The phase is idempotent: re-running design.md sees `draft_ready: true` + `review_completed: true` + `human_review_completed: false` and re-enters at Step 4 (skipping draft and AI review).

`PHASE_CONFIG.design.requiredFields` gains `"human_review_completed"`. `historyEvents` gains `"design_human_review_complete"`. The transition out of design is therefore blocked until the gate passes.

### Data flow

```
                       discovery
                           │
                           ▼
                  transition_phase()
                  validatePhaseTransition()
                  set current_phase = design
                           │
                  PHASE_CONFIG[discovery]
                  .auto_dispatch=false?  ───── yes ──┐
                           │                         │
                           no                        ▼
                           │                emitPauseBanner()
                           ▼                         │
                  dispatchPhase(design)              │
                                              user switches model
                                              user runs /task:orchestrate
                                                     │
                                                     ▼
                                              dispatchPhase(design)

                            design.md
                                │
            Step 1 draft ──► Step 2 AI review ──► Step 3 apply
                                                       │
                                                       ▼
                                          Step 4 plannotator review
                                                       │
                                       ┌───────────────┴───────────────┐
                                  approved                          denied
                                       │                               │
                                       ▼                               ▼
                                  Step 5 finalize             record annotations
                                       │                       print + stop phase
                                       ▼
                              transition_phase()
                              PHASE_CONFIG[design]
                              .auto_dispatch=false → banner
```

## Public API surface

This is a TypeScript extension, not a Rust crate, so "API surface" means the orchestrator's tool schemas and the workflow YAML schema.

### Workflow YAML — new fields

Under `phases.design`:

```yaml
phases:
  design:
    status: complete
    design_doc: docs/designs/clo-XX-<slug>.md
    draft_ready: true
    discovery_context_used: true
    review_completed: true
    review_gemini: docs/reviews/clo-XX-design-gemini.md
    review_synthesis: docs/reviews/clo-XX-design-synthesis.md
    review_verdict: approve | approve_with_changes | rework
    applied_suggestions: []
    flagged_suggestions: []
    human_review_completed: true            # NEW
    plannotator_annotations: ""             # NEW (only populated on denial)
    finalized: true
```

History events under `history[]`:

- `design_human_review_complete` (NEW) - emitted when plannotator approves.
- `design_human_review_denied` (NEW, optional) - emitted on each denial; details should contain a one-line summary of the annotations.

### TypeScript — `PHASE_CONFIG` shape

`PHASE_CONFIG[phase]` adds an optional `auto_dispatch_after_transition?: boolean`. Default behaviour (when absent or `true`) is unchanged: `transition_phase` calls `dispatchPhase`. When `false`, the dispatcher skips and emits the banner.

### TypeScript — `emitPauseBanner` helper

```ts
async function emitPauseBanner(
  pi: ExtensionAPI,
  taskId: string,
  fromPhase: string,
  toPhase: string,
): Promise<void>;
```

Sends a `pi.sendUserMessage` with `deliverAs: "followUp"` containing the banner template above. Self-contained — does not reference workflow state by closure, only the four arguments.

### Phase markdown — `design.md` Step 4 contract

The phase file gains a section before the existing finalize step:

```
## Step 4 - Human review via plannotator

Run `/plannotator-review docs/designs/clo-XX-<slug>.md`.

Read the result. If plannotator returns approved:

  update_workflow_state({
    task_id: "CLO-XX",
    phase: "design",
    action: "design_human_review_complete",
    details: "Plannotator review approved.",
    phase_updates: { human_review_completed: true }
  })

If plannotator returns denied with annotations:

  update_workflow_state({
    task_id: "CLO-XX",
    phase: "design",
    action: "design_human_review_denied",
    details: "<one-line summary of annotations>",
    phase_updates: {
      human_review_completed: false,
      plannotator_annotations: "<full annotation text>"
    }
  })

Print the annotations to the user, then STOP. Tell the user
to edit docs/designs/clo-XX-<slug>.md and re-run
`/task:orchestrate CLO-XX` when ready. Do NOT call
transition_phase.
```

The existing Step 4 (finalize + transition) is renumbered to Step 5 with no other changes.

## Test plan

Unit-test the orchestrator extension where feasible; otherwise integration-test by exercising a real CLO-task workflow end-to-end.

### Unit tests (`.pi/extensions/orchestrate/`)

Add to whatever test harness already exists for the extension. If there is none, add a minimal Node test runner.

- `transition_phase_pauses_when_flag_false` — when `PHASE_CONFIG[from].auto_dispatch_after_transition === false`, `transition_phase` updates state and emits a banner via `pi.sendUserMessage` (via mock) but does **not** call `dispatchPhase`.
- `transition_phase_dispatches_when_flag_true_or_absent` — default behaviour preserved for phases without the flag.
- `transition_phase_blocks_design_exit_without_human_review` — `validatePhaseTransition` returns errors when `phases.design.human_review_completed` is missing or false; existing `validation_override` still bypasses.
- `pause_banner_template_is_self_contained` — banner text contains task ID, both phase names, and the resume command without referencing any other state.

### Integration test (manual, end-to-end)

1. Pick a small task, e.g. CLO-99 dummy.
2. Run init → discovery normally with the fast model.
3. Verify orchestrator pauses after discovery completes, prints the banner, does not auto-dispatch design.
4. Switch Pi's model manually, run `/task:orchestrate CLO-99`.
5. Verify `design.md` runs through Step 1-3, then halts at Step 4 to run plannotator.
6. Approve via plannotator. Verify `human_review_completed: true` lands in YAML and `design_human_review_complete` history event is emitted.
7. Verify orchestrator pauses again at design → plan boundary, prints banner.
8. Switch model back, resume, and verify plan/implement run normally to completion.
9. Repeat the design phase with a denial to confirm the stop-and-print path: annotations land in `plannotator_annotations`, `design_human_review_denied` is in history, and the next `/task:orchestrate` run re-enters at Step 4 rather than redoing the draft.

### Manual verification checklist

- Banner is readable in the Pi terminal UI (no markdown swallowing, line breaks correct).
- `validation_override: true` still allows forcing the transition out of design even with `human_review_completed: false` (escape hatch must keep working).

## Migration / rollout

- This is additive. Existing workflow YAML files for completed tasks will not have `human_review_completed` or `plannotator_annotations`, which is fine because the new required-field check only applies on the **next** transition out of `design`. Tasks already past design are unaffected.
- For tasks currently in design phase when the change ships: they will hit the new gate on their next `transition_phase(from=design)` call. If the user wants to skip plannotator on those (because they already eyeballed the doc), they can set `phases.design.human_review_completed: true` in the YAML manually and add a `design_human_review_complete` history entry, or use `validation_override: true`.
- No feature flag. The pause flags and gate are part of `PHASE_CONFIG` and the phase markdown; flipping them on is the rollout.
- No dependency additions. Plannotator is invoked via slash command, which the user already has installed globally.

## Open questions

- **Banner channel.** `pi.sendUserMessage` with `deliverAs: "followUp"` is what `dispatchPhase` already uses. That should make the banner appear as the next assistant turn. If it ends up looking like a tool message in the UI rather than a clear pause indicator, we may need a notification/toast channel instead. Verify during the manual integration test.
- **Plannotator non-zero exit / unavailable.** If `/plannotator-review` is not installed or returns an unparseable result, what does Step 4 do? Proposed default: treat as denial with annotations = "plannotator unavailable; fall back to manual review". User can then either `validation_override` past or fix the install. To confirm during integration test.
- **What counts as a "denial summary" in the history event details field?** The annotations themselves can be long. Proposal: truncate to first 200 chars + count of annotations. Confirm when implementing.

## Follow-ups (out of scope for this spec)

These were flagged during brainstorming but explicitly deferred:

- **Symmetric reviewer rosters.** Design phase runs Gemini single-shot + synthesis; implement phase's validation gate runs Codex + Gemini + synthesis. Worth a separate spec to align them so design gets the same dual-perspective second look implement gets.
- **`ctx.compact()` between phases.** Verify whether loker's orchestrator already calls it; if not, evaluate whether self-contained dispatch prompts already cover the post-compaction-amnesia failure mode (mentis lesson) or if explicit compaction at phase boundaries is still warranted.
- **`next_model_hint` field on pause config.** After the user has tested several "smart enough" models for design and picked one, add the field and surface it in the pause banner.
- **`/plannotator-review` on the implement-phase diff.** Could add a similar human gate before `pr` creation. Out of scope for v1 since it would introduce a third pause point and the validation gate already provides automated coverage there.
- **Pause point at `implement → pr`.** Currently auto-dispatched. If users want to model-switch back to "smart" for PR description writing, that's a one-line `auto_dispatch_after_transition: false` add later.
