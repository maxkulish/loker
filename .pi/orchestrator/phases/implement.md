# Phase: implement

Land the plan's sub-tasks one by one. Run the codex+gemini validation
gate as the final pre-PR step. The validation gate is **embedded in this
phase** - loker has no separate `review` phase.

Mirrors `.claude/commands/task/phases/implement.md`.

## Required exit state

**Every field below is mandatory.** `status: complete` is only legal once
the validation gate (Step 4) has produced all three review files AND the
synthesis verdict is `approve` or `approve_with_changes` with the single
fix iteration applied. See Step 4.6 for the hard checklist that gates
Step 5.

```yaml
phases:
  implement:
    status: complete
    commits: ["abc123", "def456"]   # optional but recommended
    codex_validated: true
    codex_verdict: "approve" | "approve_with_changes" | "rework"
    codex_report: "docs/reviews/clo-XX-codex-validation.md"
    gemini_validation_report: "docs/reviews/clo-XX-gemini-validation.md"
    validation_synthesis_report: "docs/reviews/clo-XX-validation-synthesis.md"
    validation_synthesis_verdict: "approve" | "approve_with_changes" | "pivot" | "rework"
    validation_fix_iteration_count: 0 | 1
```

History events required: `implementation_complete`,
`codex_validation_complete`.

**Anti-pattern:** opening the PR before Step 4.6 passes. If you find
yourself thinking "I'll just push the PR and address validation comments
in review", stop. That is the failure mode this gate exists to prevent.
Codex/Gemini findings are pre-PR blockers, not review-cycle suggestions.

## Step 1 - Land sub-tasks

For each sub-task ST1..STN in `docs/plans/clo-XX-<slug>.md`:

1. Implement the changes in the named files.
2. Run the sub-task's acceptance command (usually `cargo test ...`).
3. If green, commit:
   ```
   git add -A
   git commit -m "feat(CLO-XX): <ST verb> <component>"
   ```
4. Append the commit SHA to `phases.implement.commits`:
   ```ts
   update_workflow_state({
     task_id: "CLO-XX",
     phase: "implement",
     action: "subtask_complete",
     details: "ST1 landed: <description>. Commit <sha>",
     phase_updates: { commits: [...existing, "<sha>"] }
   })
   ```

If a sub-task fails after a reasonable attempt, set `workflow.status =
blocked` and dispatch `phases/blocked.md`.

## Step 2 - Run the pre-merge gate

```bash
make check    # fmt + clippy + test
```

It must be green before proceeding.

## Step 3 - Record implementation complete

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "implement",
  action: "implementation_complete",
  details: "All sub-tasks landed. make check green. <n> commits.",
  phase_updates: { status: "validating" }
})
```

Note: `status` is `validating`, NOT `complete`. The phase only completes
after Step 4 produces all three review files and the synthesis verdict
permits it (Step 4.5). Setting `status: complete` here would let an agent
open a PR while skipping the validation gate - that is the bug this
ordering prevents. Do NOT call `transition_phase` until Step 4.6 passes.

## Step 4 - Two-reviewer validation + synthesis gate (MANDATORY)

This is the loker equivalent of the mentis `review` phase. It is a
**bounded** gate:

1. Run Codex and Gemini concurrently as independent raw reviewers.
2. Save both raw reports.
3. Run a third model to synthesize the two reports, classify scope, and
   decide what (if anything) must be fixed.
4. Apply at most **one** synthesis-approved fix iteration.
5. If the synthesis recommends a pivot or fundamental rework, stop and ask
   the user instead of auto-fixing.

The roster is intentionally asymmetric: design-review uses Gemini + Ollama + Claude
fallback during iteration, while implement-gate uses Codex + Gemini for final PR
decisions.

Never loop indefinitely on reviewer suggestions. Raw reviewer reports are
inputs; only the synthesis report drives fixes.

### 4.1 Build validation prompt

Use the same prompt for Codex and Gemini:

```text
You are a senior code reviewer. Review all changes on this branch against
this task's design document and implementation plan.

Inputs:
- Branch: feat/clo-XX-...
- Design: docs/designs/clo-XX-<slug>.md
- Plan: docs/plans/clo-XX-<slug>.md
- Diff: git diff main...HEAD

Check for correctness, completeness, regressions, code quality, security,
schema/API compatibility, and scope creep.

Output markdown with findings grouped by severity. End with:
## Verdict
approve | approve_with_changes | rework
```

### 4.2 Run Codex and Gemini concurrently

```bash
# Codex validation (background)
{
  cat .pi/agents/codex-pre-pr.md
  printf '\nYou are a senior code reviewer. Review all changes on this branch against this task'\''s design document and implementation plan.\n\n'
  printf 'Inputs:\n'
  printf '- Branch: feat/clo-XX-...\n'
  printf '- Design: docs/designs/clo-XX-<slug>.md\n'
  printf '- Plan: docs/plans/clo-XX-<slug>.md\n'
  printf '- Diff: git diff main...HEAD\n'
  printf '\n'
} | codex exec -m gpt-5.4 > docs/reviews/clo-XX-codex-validation.md &
PID_CODEX=$!

# Gemini validation (background)
GEMINI_VALIDATE_PROMPT=$(
cat .pi/agents/gemini-architect.md
printf '\nYou are a senior code reviewer. Review all changes on this branch against this task'\''s design document and implementation plan.\n\n'
printf 'Inputs:\n'
printf '- Branch: feat/clo-XX-...\n'
printf '- Design: docs/designs/clo-XX-<slug>.md\n'
printf '- Plan: docs/plans/clo-XX-<slug>.md\n'
printf '- Diff: git diff main...HEAD\n'
)

gemini --model gemini-3.1-pro-preview \
  -p "$GEMINI_VALIDATE_PROMPT" \
  > docs/reviews/clo-XX-gemini-validation.md &
PID_GEMINI=$!

wait $PID_CODEX; CODEX_EXIT=$?
wait $PID_GEMINI; GEMINI_EXIT=$?
```

If either binary is unavailable or fails due tooling/sandbox limitations,
write a concrete skip/failure reason into that report file. Do not treat a
tooling failure as a code finding; the synthesis must account for it.

### 4.3 Run synthesis reviewer

Run a third model after both raw reports exist. It reads the design, plan,
diff, and both raw reports, then writes:

`docs/reviews/clo-XX-validation-synthesis.md`

Synthesis prompt:

```text
You are the validation synthesis reviewer. Combine the Codex and Gemini
reports for CLO-XX.

Read:
- Design: docs/designs/clo-XX-<slug>.md
- Plan: docs/plans/clo-XX-<slug>.md
- Codex report: docs/reviews/clo-XX-codex-validation.md
- Gemini report: docs/reviews/clo-XX-gemini-validation.md
- Diff: git diff main...HEAD

Decide which findings are:
- Must fix before PR (in-scope correctness/regression/security/schema issue)
- Nice-to-have / out of scope
- False positive / tooling artifact
- Pivot/fundamental scope issue requiring user decision

Output:
## Verdict
approve | approve_with_changes | pivot | rework

## Must Fix Before PR
- ...

## Out of Scope / Deferred
- ...

## False Positives / Tooling Artifacts
- ...

## Recommendation
Proceed, apply one fix iteration, or stop for user decision.
```

Use an available third model/provider. If no third model is available,
synthesize manually from the two reports and clearly state that in the
synthesis report.

### 4.4 Act on synthesis verdict

| Synthesis verdict | Action |
|---|---|
| `approve` | Proceed to Step 5. |
| `approve_with_changes` | Apply only `Must Fix Before PR` items, once. Run `make check`, commit fixes, update synthesis with `## Re-validation`, then proceed. Do not rerun a new unbounded review loop. |
| `pivot` | Stop. Set workflow blocked/pending human action and ask the user with synthesis recommendations. Do not transition to PR. |
| `rework` | Stop or ask the user before returning to implementation/design. Do not auto-loop. |

Maximum validation fix iterations: **1**. If fixes reveal more issues,
record them in the synthesis report and ask the user whether to continue.

### 4.5 Record validation result

Only set `status: complete` when the synthesis verdict is `approve` or
`approve_with_changes` AND (for `approve_with_changes`) the single fix
iteration has been applied and `make check` is green again. For `pivot`
or `rework`, leave `status: validating` and stop - the phase is not
complete; jump to Step 4.4's escalation path.

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "implement",
  action: "codex_validation_complete",
  details: "Codex: <verdict>. Gemini: <verdict>. Synthesis: <verdict>. <fixes> applied.",
  phase_updates: {
    status: "complete",   // ONLY for approve / approve_with_changes (after fix)
    codex_validated: true,
    codex_verdict: "<approve|approve_with_changes|rework>",
    codex_report: "docs/reviews/clo-XX-codex-validation.md",
    gemini_validation_report: "docs/reviews/clo-XX-gemini-validation.md",
    validation_synthesis_report: "docs/reviews/clo-XX-validation-synthesis.md",
    validation_synthesis_verdict: "<approve|approve_with_changes|pivot|rework>",
    validation_fix_iteration_count: 0 | 1
  }
})
```

`codex_verdict` remains for backward compatibility. Use the synthesis
verdict as the decision source for PR transition.

### 4.6 Pre-transition checklist (MANDATORY)

Before calling `transition_phase` in Step 5, every item below MUST hold.
If any check fails, the validation gate has not passed - either return
to Step 4.4 (apply the single permitted fix) or stop and escalate to the
user. Do NOT open a PR, do NOT call `transition_phase`, do NOT mark the
phase complete.

Run the file-existence check verbatim:

```bash
TASK=clo-XX
for f in \
  docs/reviews/${TASK}-codex-validation.md \
  docs/reviews/${TASK}-gemini-validation.md \
  docs/reviews/${TASK}-validation-synthesis.md
do
  if [ ! -s "$f" ]; then
    echo "GATE FAIL: missing or empty $f"
    exit 1
  fi
done
echo "GATE OK: all three validation reports present"
```

Then verify each item by reading the workflow YAML and the synthesis
report:

- [ ] `phases.implement.status == "complete"` (not `validating`,
      `in_progress`, or unset).
- [ ] `phases.implement.codex_validated == true`.
- [ ] `phases.implement.codex_report` points to an existing,
      non-empty file with a final `## Verdict` section.
- [ ] `phases.implement.gemini_validation_report` points to an existing,
      non-empty file with a final `## Verdict` section.
- [ ] `phases.implement.validation_synthesis_report` points to an
      existing, non-empty file.
- [ ] `phases.implement.validation_synthesis_verdict` is `approve` or
      `approve_with_changes`. Anything else is a stop.
- [ ] If verdict is `approve_with_changes`,
      `phases.implement.validation_fix_iteration_count == 1` AND every
      "Must Fix Before PR" item from the synthesis report is reflected
      in the diff (re-read the synthesis to confirm).
- [ ] `make check` is green on the current HEAD (re-run if any commits
      landed since the last green run).
- [ ] History contains both `implementation_complete` and
      `codex_validation_complete` events.

If any synthesis "Must Fix" item is unaddressed, the gate fails -
returning to it later as PR-review feedback is not acceptable. Codex and
Gemini are pre-PR reviewers; the human and bot reviewers in Step 3.5 of
`pr.md` are not a substitute.

## Step 5 - Transition to PR

Only after every Step 4.6 box is checked:

```ts
transition_phase({
  task_id: "CLO-XX",
  from_phase: "implement",
  to_phase: "pr"
})
```

## Notes

- The validation gate is non-negotiable for development tasks. For
  specification tasks the gate is recommended but may be skipped if the
  spec author opts out (record the decision in `details`).
- If validation surfaces a fundamental design issue, do not paper over
  it - return to `design` via user confirmation.
