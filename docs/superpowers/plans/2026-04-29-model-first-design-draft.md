# Model-First Design Draft Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Claude-powered first-draft step to the design phase, gated by a human soft gate before the existing two-reviewer pipeline.

**Architecture:** Three deliverables - a new `claude-designer.md` persona file, a new `design-draft.toml` lok workflow (5 steps: read_milestone, load_persona, read_discovery, generate_draft, write_draft), and an updated `design.md` phase that calls the draft workflow then soft-gates before the existing review pipeline.

**Tech Stack:** TOML (lok workflow), Markdown (persona, phase file), bash (shell steps), Claude API via lok `backend = "claude"`.

**Spec:** `docs/superpowers/specs/2026-04-29-model-first-design-draft.md`

---

## File map

| File | Action | Responsibility |
|------|--------|----------------|
| `.pi/agents/claude-designer.md` | Create | Design drafter persona - role, output format, hard rules |
| `.lok/workflows/design-draft.toml` | Create | 5-step pipeline: milestone → persona → discovery context → Claude draft → write file |
| `.pi/orchestrator/phases/design.md` | Modify | Replace Step 1 (manual draft) with lok workflow invocation + soft gate |

---

## Task 1: Create the claude-designer persona

**Files:**
- Create: `.pi/agents/claude-designer.md`

- [ ] **Step 1.1: Write the persona file**

Create `.pi/agents/claude-designer.md` with the following exact content:

```markdown
# Persona: Claude designer (loker)

You are a senior Rust system designer producing the first draft of a design document
for the loker repository. Loker is a pure-Rust orchestration library that talks to
LLM backends through TensorZero.

Your job is to translate discovery outputs into a concrete design document that
Gemini and Ollama will review. You are a drafter, not a reviewer.

## Stack context

- Pure Rust (workspace at the repo root, no Tauri / no React / no JS).
- Core dependency: TensorZero gateway as the canonical LLM transport.
- Pre-merge gate: `make check` (fmt + clippy + test).
- Canonical design doc:
  `/Users/mk/Work/investigations/sakana-fugu/loker-design.md`.
- Public surface lives in `src/lib.rs`; private modules use
  `#![allow(dead_code)]` at the lib root only.

## Input contract

You will receive:

- Active milestone (from CLAUDE.md)
- Task ID and title
- PRD content (from the discovery phase)
- The chosen approach (text from `approach_chosen` in the workflow YAML)
- Discovery report (from `discovery_report` path in the workflow YAML)
- `docs/handoff.md` (project intent, constraints, conventions)

## Output format

Produce a single markdown document. Include all seven sections in this order:

1. **Problem** - 1 paragraph citing the discovery report. WHO is affected, WHAT
   is broken or missing, WHY it matters now.
2. **Goals / Non-goals** - bulleted lists. Goals are concrete deliverables;
   non-goals are explicit exclusions that prevent scope creep.
3. **Architecture** - modules, data flow, concrete Rust types. Include a
   block diagram in ASCII if it helps clarify relationships.
4. **Public API surface** - Rust trait and struct signatures exactly as they
   will appear in `src/lib.rs` or the relevant module. Use real Rust syntax.
5. **Test plan** - unit tests (wiremock for backend calls), integration tests,
   manual verification steps. Name the test functions.
6. **Migration / rollout** - backward compatibility notes, feature flags if
   needed, rollout order. If there is nothing to migrate, say so explicitly.
7. **Open questions** - unresolved decisions from discovery. Leave these
   genuinely open with a description of the tradeoff, not a fabricated answer.

## Hard rules

- Do not write any preamble. Start directly with `# Design: <task-id> - <title>`.
  Do not write "I will now design", "Based on the discovery", "Here is my draft",
  or any sentence describing what you are about to do.
- Do not include chain-of-thought, scratchpad, internal monologue, or `<think>`
  blocks. Output only the final design markdown.
- Leave open questions genuinely open. Do not fabricate resolutions to unresolved
  discovery questions.
- Do not invent implementation details not supported by the discovery outputs or
  the canonical design doc.
- Never recommend abandoning the `approach_chosen` from discovery without
  flagging it as an open question.
- Do not propose dependency additions unless strictly required by the design.
- Never paste the entire discovery report back at the reader; synthesize it.
```

- [ ] **Step 1.2: Verify the file structure matches the other personas**

```bash
head -3 .pi/agents/claude-designer.md
head -3 .pi/agents/gemini-architect.md
head -3 .pi/agents/ollama-rust-reviewer.md
```

Expected: all three start with `# Persona:` on line 1.

- [ ] **Step 1.3: Commit**

```bash
git add .pi/agents/claude-designer.md
git commit -m "feat(agents): add claude-designer persona for design drafting"
```

---

## Task 2: Create the design-draft lok workflow

**Files:**
- Create: `.lok/workflows/design-draft.toml`

- [ ] **Step 2.1: Write the workflow file**

Create `.lok/workflows/design-draft.toml` with the following exact content:

```toml
name = "design-draft"
description = "Claude-powered first draft of a design document from discovery outputs"

# Step 1: Resolve active milestone from CLAUDE.md
[[steps]]
name = "read_milestone"
timeout = 10000
continue_on_error = true
shell = """
ACTIVE_MILESTONE=$(grep -E '^Active milestone:' CLAUDE.md | head -1 | sed 's/^Active milestone: //; s/\*\*//g; s/\.$//')
if [ -z "$ACTIVE_MILESTONE" ]; then
  echo "Unknown"
else
  echo "$ACTIVE_MILESTONE"
fi
"""

# Step 2: Load designer persona from canonical file
[[steps]]
name = "load_persona"
timeout = 5000
shell = """
cat .pi/agents/claude-designer.md
"""

# Step 3: Compile discovery context from workflow YAML + referenced files
[[steps]]
name = "read_discovery"
timeout = 15000
continue_on_error = false
depends_on = ["read_milestone"]
shell = """
TASK_ID="{{ arg.1 }}"
TASK_ID_LOWER=$(printf '%s' "$TASK_ID" | tr '[:upper:]' '[:lower:]')

# Locate workflow YAML - try exact case first, then lowercase
YAML_PATH=""
for candidate in \
  "docs/status/${TASK_ID}-workflow.yaml" \
  "docs/status/${TASK_ID_LOWER}-workflow.yaml"; do
  if [ -f "$candidate" ]; then
    YAML_PATH="$candidate"
    break
  fi
done

if [ -z "$YAML_PATH" ]; then
  printf 'ERROR: No workflow YAML found for %s. Tried:\n' "$TASK_ID" >&2
  printf '  docs/status/%s-workflow.yaml\n' "$TASK_ID" >&2
  printf '  docs/status/%s-workflow.yaml\n' "$TASK_ID_LOWER" >&2
  exit 1
fi

# Extract field values - 4-space-indented YAML fields under phases.discovery
PRD_FILE=$(grep -E '^\s{4}prd_file:' "$YAML_PATH" | head -1 | sed 's/.*prd_file:[[:space:]]*//' | tr -d '"'"'"' ')
APPROACH=$(grep -E '^\s{4}approach_chosen:' "$YAML_PATH" | head -1 | sed 's/.*approach_chosen:[[:space:]]*//' | tr -d '"'"'"' ')
DISCOVERY_REPORT=$(grep -E '^\s{4}discovery_report:' "$YAML_PATH" | head -1 | sed 's/.*discovery_report:[[:space:]]*//' | tr -d '"'"'"' ')

printf '=== TASK: %s - {{ arg.3 }} ===\n' "$TASK_ID"
printf 'Active milestone: %s\n\n' "{{ steps.read_milestone.output }}"

printf '=== PRD ===\n'
if [ -n "$PRD_FILE" ] && [ -f "$PRD_FILE" ]; then
  cat "$PRD_FILE"
else
  printf '[PRD unavailable: %s]\n' "${PRD_FILE:-field not found in YAML}" >&2
  printf '[PRD unavailable]\n'
fi

printf '\n=== APPROACH CHOSEN ===\n'
if [ -n "$APPROACH" ]; then
  printf '%s\n' "$APPROACH"
else
  printf '[approach_chosen not found in %s]\n' "$YAML_PATH" >&2
  printf '[Approach not specified]\n'
fi

printf '\n=== DISCOVERY REPORT ===\n'
if [ -n "$DISCOVERY_REPORT" ] && [ -f "$DISCOVERY_REPORT" ]; then
  cat "$DISCOVERY_REPORT"
else
  printf '[Discovery report unavailable: %s]\n' "${DISCOVERY_REPORT:-field not found in YAML}" >&2
  printf '[Discovery report unavailable]\n'
fi

printf '\n=== PROJECT HANDOFF ===\n'
if [ -f docs/handoff.md ]; then
  cat docs/handoff.md
else
  printf '[docs/handoff.md not found]\n' >&2
fi
"""

# Step 4: Generate design draft via Claude
[[steps]]
name = "generate_draft"
backend = "claude"
timeout = 180000
retries = 2
continue_on_error = false
depends_on = ["load_persona", "read_discovery"]
prompt = """
{{ steps.load_persona.output }}

---

Active milestone: {{ steps.read_milestone.output }}
Task: {{ arg.1 }} - {{ arg.3 }}
Output path: docs/designs/<task-id-lowercase>-{{ arg.2 }}.md

DISCOVERY CONTEXT:
{{ steps.read_discovery.output }}

Produce the design document now.
"""

# Step 5: Write draft to docs/designs/ with injection-safe heredoc
[[steps]]
name = "write_draft"
timeout = 10000
depends_on = ["generate_draft"]
shell = """
mkdir -p docs/designs
TASK_ID_LOWER=$(printf '%s' '{{ arg.1 }}' | tr '[:upper:]' '[:lower:]')
OUTPUT_PATH="docs/designs/${TASK_ID_LOWER}-{{ arg.2 }}.md"
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

# Single-quoted heredoc: lok substitutes {{ ... }} before shell runs,
# but the shell itself does no further expansion (no $(...) execution).
# Same pattern as write_reviews in design-review.toml.
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
"""
```

- [ ] **Step 2.2: Verify the TOML parses (if lok is available)**

```bash
lok validate .lok/workflows/design-draft.toml --dir . 2>&1 || echo "lok not available - skip"
```

If lok is not in PATH, skip. The structure is correct by inspection against `design-review.toml`.

- [ ] **Step 2.3: Commit**

```bash
git add .lok/workflows/design-draft.toml
git commit -m "feat(workflows): add design-draft.toml for Claude-powered first draft"
```

---

## Task 3: Update design.md phase - replace Step 1

**Files:**
- Modify: `.pi/orchestrator/phases/design.md`

The current Step 1 block (lines 29-57 in `design.md`) asks the human to manually draft the document and record `draft_ready: true` immediately. Replace it with a workflow invocation, soft gate, and deferred state recording.

- [ ] **Step 3.1: Replace the Step 1 block**

In `.pi/orchestrator/phases/design.md`, replace the entire content of `## Step 1 - Draft the design` through its closing `update_workflow_state` call with:

```markdown
## Step 1 - Generate the design draft

Run the draft workflow. It reads discovery outputs from the workflow YAML and
produces a first draft at `docs/designs/clo-XX-<slug>.md`:

```bash
lok run .lok/workflows/design-draft.toml \
  CLO-XX \
  <slug> \
  "<task title>" \
  --dir . --verbose
```

If lok is unavailable, fall back to writing the draft manually using the
7-section structure below, then skip to the soft gate.

**Soft gate** - open the draft, read it end-to-end, make any edits needed.
Then run this in a terminal to confirm you are satisfied:

```bash
DESIGN_DOC="docs/designs/clo-xx-<slug>.md"
printf '\nDraft at %s (%d lines).\nEdit it now, then press Enter to start the AI review pipeline, or Ctrl+C to abort: ' \
  "$DESIGN_DOC" "$(wc -l < "$DESIGN_DOC")"
read -r _
```

Ctrl+C at this point leaves the workflow YAML unchanged. Re-running Step 1
overwrites the draft.

Only after pressing Enter, record state:

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

The 7-section structure the draft must contain (verify before pressing Enter):

- Problem (1 paragraph, citing discovery)
- Goals / Non-goals
- Architecture (modules, data flow, concrete types)
- Public API surface (Rust trait/struct signatures)
- Test plan (unit, integration, manual)
- Migration / rollout
- Open questions
```

- [ ] **Step 3.2: Verify the edit looks right**

```bash
grep -n "Step 1\|Step 2\|lok run\|soft gate\|draft_source" .pi/orchestrator/phases/design.md
```

Expected output should show:
- `## Step 1 - Generate the design draft`
- `lok run .lok/workflows/design-draft.toml`
- `soft gate`
- `draft_source: "claude-designer"`
- `## Step 2 - AI design review` (unchanged, still present)

- [ ] **Step 3.3: Verify Steps 2-4 are untouched**

```bash
grep -c "update_workflow_state" .pi/orchestrator/phases/design.md
```

Expected: `3` (one in Step 1 after soft gate, one in Step 3 for review_complete, one in Step 4 for finalized).

- [ ] **Step 3.4: Commit**

```bash
git add .pi/orchestrator/phases/design.md
git commit -m "feat(phases): replace manual design draft with claude-designer workflow + soft gate"
```

---

## Task 4: Smoke-test the end-to-end workflow

This task runs the new `design-draft.toml` against CLO-269 (which has complete discovery artifacts at `docs/discovery/clo-269.md` and `docs/prds/clo-269-aggregator-vote.md`).

- [ ] **Step 4.1: Verify discovery artifacts exist**

```bash
ls docs/status/clo-269-workflow.yaml
ls docs/prds/clo-269-aggregator-vote.md
ls docs/discovery/clo-269.md
```

All three should exist. If any are missing, use a different CLO task that has all three.

- [ ] **Step 4.2: Run the draft workflow**

```bash
lok run .lok/workflows/design-draft.toml \
  CLO-269 \
  aggregator-vote \
  "Aggregator: Vote" \
  --dir . --verbose
```

Expected terminal output (approximately):
```
[read_milestone] M1 - TensorZero backend
[load_persona]   (persona file content)
[read_discovery] === TASK: CLO-269 ...
[generate_draft] (Claude generating...)
[write_draft]    Draft written to docs/designs/clo-269-aggregator-vote.md (N lines)
```

Lok exits 0.

- [ ] **Step 4.3: Verify the draft file exists and has required sections**

```bash
ls -la docs/designs/clo-269-aggregator-vote.md
head -3 docs/designs/clo-269-aggregator-vote.md
grep -E "^## (Problem|Goals|Architecture|Public API|Test plan|Migration|Open questions)" \
  docs/designs/clo-269-aggregator-vote.md
```

Expected from `head -3`: first line is `# Design: CLO-269 - Aggregator: Vote` (no preamble).
Expected from `grep`: at least 5 of the 7 section headings present (some sections may be
named slightly differently - check for presence, not exact match).

- [ ] **Step 4.4: Verify the draft is >= 15 lines**

```bash
wc -l docs/designs/clo-269-aggregator-vote.md
```

Expected: a number >= 15. If it's < 15, the `write_draft` step would have exited 1 and
Step 4.2 would have failed.

- [ ] **Step 4.5: Test error case - missing workflow YAML**

```bash
lok run .lok/workflows/design-draft.toml \
  CLO-999 \
  fake-slug \
  "Nonexistent task" \
  --dir . --verbose 2>&1 | grep -i "error\|not found\|CLO-999"
```

Expected: a non-zero exit and an error message naming `CLO-999` and the paths that were tried.

- [ ] **Step 4.6: Commit the smoke-test artifact**

The draft file may overwrite an existing design doc for CLO-269. This is expected - the
file is regenerated content, not a regression.

```bash
git add docs/designs/clo-269-aggregator-vote.md
git commit -m "test(design-draft): smoke-test draft for CLO-269 via claude-designer workflow"
```

---

## Acceptance criteria checklist

From the spec - verify each before declaring done:

- [ ] `lok run .lok/workflows/design-draft.toml CLO-269 aggregator-vote "Aggregator: Vote" --dir . --verbose` exits 0 and writes a non-empty file to `docs/designs/clo-269-aggregator-vote.md`
- [ ] Written file starts with `# Design: CLO-269` (no preamble)
- [ ] Written file contains all 7 section headings
- [ ] `update_workflow_state` call in `design.md` Step 1 appears after the soft gate block, not before
- [ ] `design-review.toml` is byte-for-byte unchanged: `git diff HEAD~3 -- .lok/workflows/design-review.toml` shows no output
- [ ] `gemini-architect.md` and `ollama-rust-reviewer.md` are unchanged: same check
- [ ] Running the workflow a second time overwrites the file without error (test by re-running Step 4.2)
- [ ] Missing YAML test (Step 4.5) produces a non-zero exit and a message naming the missing file
