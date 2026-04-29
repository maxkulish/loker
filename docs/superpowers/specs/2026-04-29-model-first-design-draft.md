# Spec: model-first design drafting

**Date:** 2026-04-29
**Status:** draft
**Scope:** Add a Claude-powered first-draft step to the design phase, with a soft gate before the existing two-reviewer pipeline.

---

## Problem and goal

The `design.md` phase currently expects the human to write the design document in Step 1 before the AI review pipeline runs. This creates unnecessary friction: the human must translate discovery outputs (PRD, approach chosen, discovery report) into the 7-section structure before getting any model feedback.

The goal is to have Claude generate a first draft from discovery artifacts, give the human a soft gate to inspect and edit it, then hand off to the existing `design-review.toml` pipeline (Gemini review + Ollama review + Claude synthesis) unchanged.

---

## Architecture

```
discovery outputs (prd_file, approach_chosen, discovery_report in workflow YAML)
        │
        ▼
.lok/workflows/design-draft.toml
  ├── read_milestone      shell: grep CLAUDE.md (same regex as design-review.toml)
  ├── load_persona        shell: cat .pi/agents/claude-designer.md
  ├── read_discovery      shell: locate workflow YAML, extract field paths, cat files
  ├── generate_draft      backend: claude, prompt = persona + discovery context + task args
  └── write_draft         shell: lowercase task ID, write to docs/designs/<id>-<slug>.md
        │
        ▼ (file on disk)
design.md phase - SOFT GATE
  printf draft path + line count
  read -r _ (Enter to continue, Ctrl+C to abort)
        │
        ▼ (user presses Enter)
.lok/workflows/design-review.toml  (unchanged)
```

---

## Files

| File | Change |
|------|--------|
| `.pi/agents/claude-designer.md` | new - design drafter persona |
| `.lok/workflows/design-draft.toml` | new - draft generation pipeline |
| `.pi/orchestrator/phases/design.md` | updated - Step 1 calls draft workflow + soft gate |

`design-review.toml` and all existing personas are untouched.

---

## Component specs

### `.pi/agents/claude-designer.md`

Follows the same structure as `gemini-architect.md` and `ollama-rust-reviewer.md`.

**Sections:**

- **Role** - first-draft author from discovery outputs, not a reviewer. Produces the initial design doc that Gemini and Ollama will review.
- **Stack context** - same as `gemini-architect.md`: pure Rust, TensorZero backend, pre-merge gate `make check`, canonical design at `/Users/mk/Work/investigations/sakana-fugu/loker-design.md`.
- **Input contract** - the step provides: PRD content, `approach_chosen` text, discovery report content, `docs/handoff.md`, active milestone, task ID, task title.
- **Output format** - a single markdown document starting directly with `# Design: CLO-XX - <title>`, containing these sections in order:
  1. Problem (1 paragraph citing discovery)
  2. Goals / Non-goals
  3. Architecture (modules, data flow, concrete Rust types)
  4. Public API surface (trait/struct signatures)
  5. Test plan (unit, integration, manual)
  6. Migration / rollout
  7. Open questions
- **Hard rules:**
  - Do not write any preamble. Start directly with the `# Design:` heading. Do not write "I will now design", "Based on the discovery", "Here is my draft", or any sentence describing what you are about to do.
  - Do not include chain-of-thought, scratchpad, internal monologue, or `<think>` blocks. Output only the final design markdown.
  - Leave open questions genuinely open. Do not fabricate resolutions to unresolved discovery questions.
  - Do not invent implementation details not supported by the discovery outputs or the canonical design doc.
  - Never recommend abandoning the `approach_chosen` from discovery without flagging it as an open question.

---

### `.lok/workflows/design-draft.toml`

**Invocation:**
```bash
lok run .lok/workflows/design-draft.toml \
  CLO-XX \        # arg.1 - task ID (e.g. CLO-269)
  <slug> \        # arg.2 - slug (e.g. aggregator-vote)
  "<title>" \     # arg.3 - task title
  --dir . --verbose
```

**Steps:**

**`read_milestone`** (shell, 10s, `continue_on_error = true`)
Identical to `design-review.toml`: `grep -E '^Active milestone:' CLAUDE.md | head -1 | sed ...`. Outputs milestone text or "Unknown".

**`load_persona`** (shell, 5s)
```bash
cat .pi/agents/claude-designer.md
```
Outputs the full persona file content.

**`read_discovery`** (shell, 15s, `continue_on_error = false`)
- Locate workflow YAML: try `docs/status/{{ arg.1 }}-workflow.yaml`, then lowercase variant. Hard fail (exit 1) if neither exists.
- Extract `prd_file`, `approach_chosen`, `discovery_report` via `grep`/`sed`.
- Cat each referenced file if it exists; warn to stderr if missing but continue.
- Also cat `docs/handoff.md`.
- Outputs a single compiled context string (section headers + file contents).

**`generate_draft`** (`backend = "claude"`, 180s, 2 retries, `continue_on_error = false`)
```toml
depends_on = ["read_milestone", "load_persona", "read_discovery"]
prompt = """
{{ steps.load_persona.output }}

---

Active milestone: {{ steps.read_milestone.output }}
Task: {{ arg.1 }} - {{ arg.3 }}
Output path: docs/designs/<lowercase-id>-{{ arg.2 }}.md

DISCOVERY CONTEXT:
{{ steps.read_discovery.output }}

Produce the design document now.
"""
```

No validator sub-step. The draft is human-reviewed at the soft gate; machine validation would add latency with no benefit here.

**`write_draft`** (shell, 10s)
```bash
mkdir -p docs/designs
TASK_ID_LOWER=$(printf '%s' '{{ arg.1 }}' | tr '[:upper:]' '[:lower:]')
OUTPUT_PATH="docs/designs/${TASK_ID_LOWER}-{{ arg.2 }}.md"
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

# Single-quoted heredoc: lok substitutes {{ ... }} before the shell runs,
# but the shell itself does no further expansion - same pattern as write_reviews.
cat <<'LOKER_WF_DESIGNER_DRAFT_OUTPUT_EOF' > "$TMP"
{{ steps.generate_draft.output }}
LOKER_WF_DESIGNER_DRAFT_OUTPUT_EOF

LINE_COUNT=$(wc -l < "$TMP")
if [ "$LINE_COUNT" -lt 15 ]; then
  printf 'ERROR: Draft is only %d lines - generation likely failed\n' "$LINE_COUNT" >&2
  exit 1
fi

mv "$TMP" "$OUTPUT_PATH"
printf 'Draft written to %s (%d lines)\n' "$OUTPUT_PATH" "$LINE_COUNT"
```

---

### `design.md` phase - updated Step 1

Replace the current "Draft the design" instruction block with:

```markdown
## Step 1 - Generate the design draft

Run the draft workflow:

```bash
lok run .lok/workflows/design-draft.toml \
  CLO-XX \
  <slug> \
  "<task title>" \
  --dir . --verbose
```

On success the draft is at `docs/designs/clo-xx-<slug>.md`.

**Soft gate** - open the draft, read it, make any edits. Then:

```bash
DESIGN_DOC="docs/designs/clo-xx-<slug>.md"
printf '\nDraft at %s (%d lines).\nEdit it now, then press Enter to start the AI review pipeline, or Ctrl+C to abort: ' \
  "$DESIGN_DOC" "$(wc -l < "$DESIGN_DOC")"
read -r _
```

Only after the user presses Enter, record state:

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "design",
  action: "design_draft_ready",
  details: "Draft generated by claude-designer at docs/designs/clo-XX-<slug>.md",
  phase_updates: {
    status: "in_progress",
    design_doc: "docs/designs/clo-XX-<slug>.md",
    draft_ready: true,
    draft_source: "claude-designer",
    discovery_context_used: true
  }
})
```

Step 2 (AI review) follows unchanged.
```

The existing Steps 2-4 in `design.md` are renumbered but otherwise unchanged.

---

## Error handling

| Scenario | Behavior |
|----------|----------|
| Workflow YAML not found | `read_discovery` exits 1; lok exits non-zero; phase file surfaces the error; no draft written |
| Discovery fields missing or files absent | Warn to stderr; proceed with available context; user sees thin draft at soft gate |
| `generate_draft` produces < 15 lines | `write_draft` exits 1; no file written; phase file re-runs or aborts |
| User presses Ctrl+C at soft gate | `update_workflow_state` never called; phase stays `in_progress`; re-running the phase re-generates the draft (overwrites existing) |
| Draft file already exists | `write_draft` overwrites unconditionally - re-running always produces a fresh draft |
| `design-review.toml` unavailable | Unchanged behavior: `design.md` falls back to direct persona invocation or sets `review_skip_reason` |

---

## Acceptance criteria

1. `lok run .lok/workflows/design-draft.toml CLO-XX <slug> "<title>" --dir . --verbose` exits 0 and writes a non-empty file to `docs/designs/clo-xx-<slug>.md`.
2. The written file starts with `# Design: CLO-XX` and contains all 7 required sections.
3. The soft gate in `design.md` pauses for user input before `update_workflow_state` is called.
4. Ctrl+C at the soft gate leaves `phases.design.draft_ready` absent or false in the workflow YAML.
5. Running the draft workflow a second time overwrites the existing file without error.
6. If `docs/status/CLO-XX-workflow.yaml` does not exist, the workflow exits non-zero with a message identifying the missing file.
7. `design-review.toml`, `gemini-architect.md`, and `ollama-rust-reviewer.md` are byte-for-byte unchanged.

---

## Out of scope

- Slug sanitization (caller responsibility, same as today)
- Discovery phase completeness enforcement (enforced by `design.md` entry conditions before Step 1)
- Validation of AI draft quality beyond the 15-line minimum (the soft gate is the quality gate)
- T2.1 assertions for draft format (can be added to `lok-review-assertions.yaml` once stable)
- `.lok/workflows/spec-review.toml` (tracked separately)
