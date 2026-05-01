# Pi Orchestrator Flow Improvements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add per-phase pause boundaries to the Pi orchestrator so the user can switch models manually at `discovery → design` and `design → plan`, and add a plannotator human-review gate inside the design phase before finalize.

**Architecture:** Two deliverables - extend `PHASE_CONFIG` in the orchestrate extension with an optional `auto_dispatch_after_transition` flag and a new banner helper, and insert a plannotator gate as Step 4 in `.pi/orchestrator/phases/design.md` (renumbering finalize to Step 5). Scope is `.pi/` only; the Claude command set is out of scope.

**Tech Stack:** TypeScript (Pi extension, no tsc - runtime-loaded), Markdown (phase files), js-yaml workflow YAML, plannotator slash commands.

**Spec:** `docs/superpowers/specs/2026-05-01-pi-flow-improvements-design.md`

---

## File map

| File | Action | Responsibility |
|------|--------|----------------|
| `.pi/extensions/orchestrate/index.ts` | Modify | Add `auto_dispatch_after_transition` to `PHASE_CONFIG` type and entries; add new design required field and history event; implement `emitPauseBanner`; gate `transition_phase` dispatch on the flag. |
| `.pi/orchestrator/phases/design.md` | Modify | Insert Step 4 plannotator gate; renumber existing Step 4 finalize to Step 5; document idempotent re-entry on denial. |

No new test files: the orchestrate extension has no Node test harness (`package.json` `scripts: {}`). Verification is by smoke-loading the extension and running an end-to-end integration test on a throwaway CLO task. The integration test is Task 6.

---

## Task 1: Extend PHASE_CONFIG type with auto-dispatch flag

**Files:**
- Modify: `.pi/extensions/orchestrate/index.ts:233`

- [ ] **Step 1.1: Open the file and locate `PHASE_CONFIG`**

The declaration begins at line 233 and runs through line 270. The type is currently:

```ts
const PHASE_CONFIG: Record<string, { requiredFields: string[]; historyEvents: string[] }> = {
```

- [ ] **Step 1.2: Extend the inline type with the new optional field**

Replace the type signature line with:

```ts
const PHASE_CONFIG: Record<string, {
  requiredFields: string[];
  historyEvents: string[];
  auto_dispatch_after_transition?: boolean;
}> = {
```

Do not change any of the entries yet (Task 2 handles that).

- [ ] **Step 1.3: Smoke-load the extension to confirm syntax**

Run:

```bash
cd /Users/mk/Code/orchestrator/loker/.pi/extensions/orchestrate
node --input-type=module -e "import('./index.ts').catch(e => { console.error(e.message); process.exit(1); })"
```

Expected: command exits 0 (Node may warn about TS loader if not configured; that is OK as long as there is no syntax/parse error). If Node cannot load `.ts` directly, instead verify with:

```bash
node --check index.ts 2>&1 | head -20
```

If `--check` rejects TS syntax (it usually will), fall back to a minimal regex sanity grep:

```bash
grep -n "auto_dispatch_after_transition" .pi/extensions/orchestrate/index.ts
```

Expected: one match on the type line.

- [ ] **Step 1.4: Commit**

```bash
git add .pi/extensions/orchestrate/index.ts
git commit -m "orchestrate: add optional auto_dispatch_after_transition to PHASE_CONFIG type"
```

---

## Task 2: Mark discovery and design as pause boundaries

**Files:**
- Modify: `.pi/extensions/orchestrate/index.ts:234-245`

- [ ] **Step 2.1: Update the `discovery` entry**

Locate the existing entry (currently lines 234-237):

```ts
  discovery: {
    requiredFields: ["status"],
    historyEvents: ["discovery_approved"],
  },
```

Replace with:

```ts
  discovery: {
    requiredFields: ["status"],
    historyEvents: ["discovery_approved"],
    auto_dispatch_after_transition: false,
  },
```

- [ ] **Step 2.2: Update the `design` entry**

Locate the existing entry (currently lines 242-245):

```ts
  design: {
    requiredFields: ["status", "design_doc", "draft_ready", "finalized", "review_completed"],
    historyEvents: ["design_draft_ready", "design_review_complete", "design_finalized"],
  },
```

Replace with:

```ts
  design: {
    requiredFields: ["status", "design_doc", "draft_ready", "finalized", "review_completed", "human_review_completed"],
    historyEvents: ["design_draft_ready", "design_review_complete", "design_human_review_complete", "design_finalized"],
    auto_dispatch_after_transition: false,
  },
```

This adds the new required field `human_review_completed` and the new history event `design_human_review_complete`, and flags the phase as a pause boundary.

- [ ] **Step 2.3: Sanity-check the diff**

```bash
git diff .pi/extensions/orchestrate/index.ts
```

Expected output should show exactly two `auto_dispatch_after_transition: false` additions (one on `discovery`, one on `design`), plus the appended `human_review_completed` and `design_human_review_complete` strings inside the `design` arrays. No other entries should be touched.

- [ ] **Step 2.4: Commit**

```bash
git add .pi/extensions/orchestrate/index.ts
git commit -m "orchestrate: mark discovery and design as pause boundaries; require human_review_completed"
```

---

## Task 3: Implement the emitPauseBanner helper

**Files:**
- Modify: `.pi/extensions/orchestrate/index.ts` (add new function near `dispatchPhase`, around line 1100)

- [ ] **Step 3.1: Add the helper function above `dispatchPhase`**

Find `async function dispatchPhase(` (currently at line 1100). Insert the following helper **immediately above** it:

```ts
async function emitPauseBanner(
  pi: ExtensionAPI,
  taskId: string,
  fromPhase: string,
  toPhase: string,
): Promise<void> {
  const banner = [
    "============================================================",
    `PAUSE: ${fromPhase} -> ${toPhase} boundary`,
    "============================================================",
    `Phase ${fromPhase} is complete. Workflow YAML is updated;`,
    `current_phase is now ${toPhase}.`,
    "",
    "This boundary is a model-switch point. Switch to your",
    `preferred model for ${toPhase} work, then resume:`,
    "",
    `  /task:orchestrate ${taskId}`,
    "",
    "The next phase will not run until you do.",
    "============================================================",
  ].join("\n");
  pi.sendUserMessage(banner, { deliverAs: "followUp" });
}
```

The helper is intentionally self-contained: it only uses its four parameters and never reads workflow state by closure, so the banner stays correct even if the surrounding state changes between calls.

- [ ] **Step 3.2: Sanity-check the function lands in the right place**

```bash
grep -n "^async function emitPauseBanner\|^async function dispatchPhase" .pi/extensions/orchestrate/index.ts
```

Expected: two lines, with `emitPauseBanner` listed before `dispatchPhase`.

- [ ] **Step 3.3: Commit**

```bash
git add .pi/extensions/orchestrate/index.ts
git commit -m "orchestrate: add emitPauseBanner helper for model-switch boundaries"
```

---

## Task 4: Gate transition_phase on the auto-dispatch flag

**Files:**
- Modify: `.pi/extensions/orchestrate/index.ts:1040`

- [ ] **Step 4.1: Locate the dispatch call inside `transition_phase`**

The current code (around line 1040) is:

```ts
      persistRuntimeState(params.task_id, state);
      await dispatchPhase(pi, params.task_id, params.to_phase, state, statePath, workspaceRoot);

      return {
        content: [{ type: "text", text: `Transitioned to ${params.to_phase} phase and dispatched instructions` }],
        details: {
```

- [ ] **Step 4.2: Replace the unconditional dispatch with a flag-gated branch**

Replace those two lines (`persistRuntimeState(...)` and `await dispatchPhase(...)`) and the immediate `return` block with:

```ts
      persistRuntimeState(params.task_id, state);

      const fromConfig = PHASE_CONFIG[params.from_phase];
      const shouldAutoDispatch = fromConfig?.auto_dispatch_after_transition !== false;

      if (shouldAutoDispatch) {
        await dispatchPhase(pi, params.task_id, params.to_phase, state, statePath, workspaceRoot);
      } else {
        await emitPauseBanner(pi, params.task_id, params.from_phase, params.to_phase);
      }

      const replyText = shouldAutoDispatch
        ? `Transitioned to ${params.to_phase} phase and dispatched instructions`
        : `Transitioned to ${params.to_phase} phase. Paused at model-switch boundary; user must resume via /task:orchestrate ${params.task_id}.`;

      return {
        content: [{ type: "text", text: replyText }],
        details: {
```

The rest of the `details` block and the closing of the function are unchanged.

- [ ] **Step 4.3: Verify the edit**

```bash
grep -n "shouldAutoDispatch\|emitPauseBanner\|auto_dispatch_after_transition" .pi/extensions/orchestrate/index.ts
```

Expected: at least four matches - one in the type definition (Task 1), two in the entries (Task 2: `discovery`, `design`), one in the helper signature reference (Task 3 location), and the new branch in `transition_phase` (this task).

- [ ] **Step 4.4: Commit**

```bash
git add .pi/extensions/orchestrate/index.ts
git commit -m "orchestrate: gate transition_phase auto-dispatch on auto_dispatch_after_transition flag"
```

---

## Task 5: Insert plannotator gate as Step 4 of design.md (Pi side)

**Files:**
- Modify: `.pi/orchestrator/phases/design.md`

- [ ] **Step 5.1: Update the required-exit-state YAML at the top of the file**

Locate the YAML block at lines 10-24. The current `phases.design` block is:

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

Replace with (adds two fields between `flagged_suggestions` and `finalized`):

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
    applied_suggestions: []
    flagged_suggestions: []
    human_review_completed: true
    plannotator_annotations: ""
    finalized: true
```

- [ ] **Step 5.2: Update the `History events required` line**

Find the line (around line 26):

```
History events required: `design_draft_ready`, `design_review_complete`,
`design_finalized`.
```

Replace with:

```
History events required: `design_draft_ready`, `design_review_complete`,
`design_human_review_complete`, `design_finalized`.
```

- [ ] **Step 5.3: Insert the new Step 4 (plannotator gate) before the existing Step 4**

Locate the line `## Step 4 - Finalize` (currently line 136). Immediately before that heading, insert the following new section:

```markdown
## Step 4 - Human review via plannotator

The AI review iterations in Steps 2-3 caught what models can catch. This step
is the human gate on the post-AI-review design before code starts landing.

Print a one-line summary of what changed in Step 3 (count of applied vs
flagged suggestions), then run:

```bash
/plannotator-review docs/designs/clo-XX-<slug>.md
```

Read the result. Plannotator returns either approved, or denied with
inline annotations.

**On approval:**

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "design",
  action: "design_human_review_complete",
  details: "Plannotator review approved.",
  phase_updates: {
    human_review_completed: true
  }
})
```

Then proceed to Step 5.

**On denial with annotations:**

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "design",
  action: "design_human_review_denied",
  details: "<one-line summary, max 200 chars>",
  phase_updates: {
    human_review_completed: false,
    plannotator_annotations: "<full annotation text>"
  }
})
```

Then print the full annotations to the user and **STOP**. Do not call
`transition_phase`. Tell the user:

```
Plannotator denied the design. Annotations recorded in
phases.design.plannotator_annotations. Edit
docs/designs/clo-XX-<slug>.md by hand, then re-run
/task:orchestrate CLO-XX to re-fire the gate.
```

**Re-entry on resume.** When the user re-runs `/task:orchestrate CLO-XX`
after editing, the design phase will dispatch again. At that point
`draft_ready: true` and `review_completed: true` are already set, so
skip Steps 1-3 and re-enter at Step 4 (re-fire plannotator on the
edited doc).

If `/plannotator-review` is unavailable or returns an unparseable
result, treat it as a denial with annotations =
`"plannotator unavailable; manual review required"`. The user can
either install plannotator and resume, or pass `validation_override:
true` on the next `transition_phase` to bypass.

```

- [ ] **Step 5.4: Renumber the existing finalize step**

Change the heading `## Step 4 - Finalize` (now pushed below the new Step 4) to:

```markdown
## Step 5 - Finalize
```

The body of that section (lines 138-161) stays exactly the same.

- [ ] **Step 5.5: Sanity-check the renumbering**

```bash
grep -n "^## Step" .pi/orchestrator/phases/design.md
```

Expected output:

```
## Step 1 - Generate the design draft
## Step 2 - AI design review
## Step 3 - Apply review feedback
## Step 4 - Human review via plannotator
## Step 5 - Finalize
```

- [ ] **Step 5.6: Commit**

```bash
git add .pi/orchestrator/phases/design.md
git commit -m "design phase: add plannotator human-review gate as Step 4 before finalize"
```

---

## Task 6: Manual integration test on a throwaway task

This task has no automated test harness; verification is end-to-end on a real workflow YAML. Run this checklist after Tasks 1-5 are committed.

**Files:**
- Inspect (no edits): `docs/status/clo-99-workflow.yaml` (created by Pi during the test, deleted at the end)

- [ ] **Step 6.1: Pick a throwaway task ID**

Use `CLO-99` (or any unused number). Confirm it is not in use:

```bash
ls docs/status/clo-99-workflow.yaml 2>/dev/null && echo "EXISTS - pick another number" || echo "OK - clo-99 free"
```

- [ ] **Step 6.2: Smoke-load the orchestrate extension**

In a fresh Pi session in this repo, run any orchestrator tool (e.g., `update_workflow_state`) on the throwaway task. The extension should load without error. If it fails to load, fix the syntax error before continuing.

- [ ] **Step 6.3: Walk through init → discovery → pause**

Run init and discovery for `CLO-99` (use any small fake spec). After `discovery_approved` and the `transition_phase(from=discovery, to=design)` call, verify:

- The Pi terminal prints the pause banner with the text `PAUSE: discovery -> design boundary` and `/task:orchestrate CLO-99`.
- `docs/status/clo-99-workflow.yaml` shows `workflow.current_phase: design`.
- The design phase markdown is **not** dispatched. There is no follow-up message containing `# Phase: design` or its instructions.

- [ ] **Step 6.4: Resume into design and verify the plannotator gate**

Run `/task:orchestrate CLO-99`. Pi should now dispatch `design.md` and walk through Steps 1-3. At Step 4 it should run `/plannotator-review` on the draft.

Approve the doc in plannotator. Verify:

- `phases.design.human_review_completed: true` lands in YAML.
- `design_human_review_complete` is in `history`.
- The phase advances to Step 5 (finalize), then `transition_phase(from=design, to=plan)` fires.
- The pause banner prints again with `PAUSE: design -> plan boundary`.

- [ ] **Step 6.5: Test the denial path**

Repeat Step 6.3 for a fresh `CLO-98`, but this time at the plannotator gate, **deny** with annotations. Verify:

- `phases.design.human_review_completed: false` is in YAML.
- `phases.design.plannotator_annotations` contains the annotation text.
- `design_human_review_denied` is in `history`.
- `transition_phase` is **not** called.
- The user sees the print-and-stop message.

Then edit the design doc by hand (any small change), re-run `/task:orchestrate CLO-98`, and confirm the phase re-enters at Step 4 (no redrafting, no AI re-review) and re-fires plannotator.

- [ ] **Step 6.6: Test the validation_override escape hatch**

Manually call `transition_phase(from=design, to=plan, validation_override=true)` while `human_review_completed: false`. Confirm the transition succeeds and the pause banner still prints (the override skips the required-field check but the auto-dispatch flag still applies).

- [ ] **Step 6.7: Clean up**

Delete the throwaway YAMLs:

```bash
rm -f docs/status/clo-98-workflow.yaml docs/status/clo-99-workflow.yaml
```

- [ ] **Step 6.8: Commit a one-line note recording the integration result**

If the test passed, no code changes are needed; just record completion. If you found bugs, fix them, then commit fixes and re-run Step 6.3-6.6 until clean. End with:

```bash
git log --oneline -10
```

Verify the five commits from Tasks 1-5 land in order, plus any fix commits from this task.

---

## Self-review checklist

After all six tasks complete, run this final check:

- [ ] **Spec coverage.** Every Goal in the spec maps to a task: pause flag (Tasks 1, 2, 4), banner helper (Task 3), plannotator gate (Task 5), banner text constraints (Task 3 banner text matches spec). Every Non-goal in the spec is preserved (no auto model switching, no auto-fix loop, no implement→pr pause, no reviewer-roster change, no `ctx.compact()` plumbing).
- [ ] **No placeholders.** Each task step contains exact file paths, exact code, and exact commands. No "TBD" or "implement appropriate handling".
- [ ] **Type and naming consistency.** `auto_dispatch_after_transition` is spelled identically in the type, the entries, and the gate. `human_review_completed`, `plannotator_annotations`, `design_human_review_complete`, and `design_human_review_denied` are spelled identically across the orchestrate extension and `.pi/orchestrator/phases/design.md`.
- [ ] **Existing escape hatches preserved.** `validation_override` still bypasses the required-field check; the pause flag is independent and still applies on override (Task 6.6 confirms).
