# Phase: Implement

**Purpose**: Execute the implementation plan phase by phase, tracking commits and pushing to remote. Run external model validation before transitioning to PR.

**Entry conditions**: `current_phase: implement`

---

## Status: pending or in_progress

1. Update state: `phases.implement.status: in_progress`
2. **Invoke**: `/plan:implement CLO-XX`

3. After each phase completion within `/plan:implement`:
   - Update workflow state:
     - `phases.implement.last_phase_completed: [phase name]`
     - Add commit SHA to `phases.implement.commits[]`
   - **Push to remote**:
     ```bash
     git push origin feat/clo-XX-short-desc
     ```
   - Add history entry: `phase_completed` with details of phase name
   - Add history entry: `pushed_to_remote`

4. When `/plan:implement` reaches 100%:
   - Add history entry: `implementation_complete`
   - **Continue to Validation Gate** (Step 5)

---

## Step 5: Two-reviewer validation + synthesis gate

**After implementation is complete, before creating a PR**, run external
model validation to catch issues Claude may have blind spots for. This is a
**bounded** gate:

1. Run Codex and Gemini concurrently as independent raw reviewers.
2. Save both raw reports.
3. Run a third model to synthesize the two reports, classify scope, and
   decide what (if anything) must be fixed.
4. Apply at most **one** synthesis-approved fix iteration.
5. If the synthesis recommends a pivot or fundamental rework, stop and ask
   the user instead of auto-fixing.

Never loop indefinitely on reviewer suggestions. Raw reviewer reports are
inputs; only the synthesis report drives fixes.

### Build Unified Validation Prompt

```
You are a senior code reviewer. Review all changes on this branch against the design
document and implementation plan.

FILES TO READ:
1. The design document: [path from phases.design.design_doc]
2. The implementation plan: [path from phases.plan.plan_file]
3. Run: git diff main...HEAD (to see all changes)
4. Read any new or significantly modified source files

CHECK FOR:
1. CORRECTNESS: Do the changes implement what the design doc specifies?
2. COMPLETENESS: Are all acceptance criteria from the design doc covered?
3. REGRESSIONS: Could any changes break existing functionality?
4. CODE QUALITY: Clean interfaces, proper error handling, no dead code
5. SECURITY: No hardcoded secrets, proper input validation, safe FFI usage
6. SCHEMA/API COMPATIBILITY: Existing schemas and public imports still work
7. SCOPE: Findings must be in-scope for this task

OUTPUT FORMAT:
## Verdict
approve | approve_with_changes | rework

## Findings
[List each finding with severity: CRITICAL / HIGH / MEDIUM / LOW]

## Missing Items
[Any acceptance criteria not yet implemented]

## Recommendations
[Specific actionable improvements]
```

### Run Raw Validation in Parallel

```bash
# Codex validation (background) - 10 minute timeout
timeout 600 codex exec -m gpt-5.4 \
  -c reasoning.effort='"high"' \
  -s read-only \
  -o docs/reviews/clo-XX-codex-validation.md \
  "[VALIDATION_PROMPT]" &
PID_CODEX=$!

# Gemini validation (background) - 5 minute timeout
(
  timeout 300 gemini --model gemini-3.1-pro-preview -y --sandbox \
    --include-directories docs,src \
    -p "[VALIDATION_PROMPT]" -o text \
    > docs/reviews/clo-XX-gemini-validation.md 2>&1
) &
PID_GEMINI=$!

wait $PID_CODEX; CODEX_EXIT=$?
wait $PID_GEMINI; GEMINI_EXIT=$?
```

If either binary is unavailable or fails due tooling/sandbox limitations,
write a concrete skip/failure reason into that report file. Do not treat a
tooling failure as a code finding; the synthesis must account for it.

### Run Synthesis Reviewer

Run a third model after both raw reports exist. It reads the design, plan,
diff, and both raw reports, then writes:

`docs/reviews/clo-XX-validation-synthesis.md`

Synthesis prompt:

```
You are the validation synthesis reviewer. Combine the Codex and Gemini reports.

Read:
- Design: [path from phases.design.design_doc]
- Plan: [path from phases.plan.plan_file]
- Codex report: docs/reviews/clo-XX-codex-validation.md
- Gemini report: docs/reviews/clo-XX-gemini-validation.md
- Diff: git diff main...HEAD

Decide which findings are:
- Must fix before PR (in-scope correctness/regression/security/schema issue)
- Nice-to-have / out of scope
- False positive / tooling artifact
- Pivot/fundamental scope issue requiring user decision

OUTPUT FORMAT:
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

### Decision Handling

| Synthesis verdict | Action |
|---|---|
| `approve` | Update state and continue to PR phase. |
| `approve_with_changes` | Apply only `Must Fix Before PR` items, once. Run `make check`, commit fixes, update synthesis with `## Re-validation`, then continue. Do not rerun a new unbounded review loop. |
| `pivot` | Stop. Set workflow blocked/pending human action and ask the user with synthesis recommendations. Do not transition to PR. |
| `rework` | Stop or ask the user before returning to implementation/design. Do not auto-loop. |

Maximum validation fix iterations: **1**. If fixes reveal more issues,
record them in the synthesis report and ask the user whether to continue.

### Fallback

- If Codex is unavailable: Warn and run Gemini only, then synthesize with
  the available report and the skip reason.
- If Gemini is unavailable: Warn and run Codex only, then synthesize with
  the available report and the skip reason.
- If both unavailable: Warn and let user decide (proceed or pause).
- If the synthesis model is unavailable: perform a manual synthesis and
  clearly mark it as manual.

### Update State

- `phases.implement.codex_validated: true`
- `phases.implement.codex_verdict: [approve|approve_with_changes|rework]`
- `phases.implement.codex_report: docs/reviews/clo-XX-codex-validation.md`
- `phases.implement.gemini_validation_report: docs/reviews/clo-XX-gemini-validation.md`
- `phases.implement.validation_synthesis_report: docs/reviews/clo-XX-validation-synthesis.md`
- `phases.implement.validation_synthesis_verdict: [approve|approve_with_changes|pivot|rework]`
- `phases.implement.validation_fix_iteration_count: 0 | 1`
- Add history entry: `codex_validation_complete`

### Transition to PR

- `phases.implement.status: complete`
- `workflow.current_phase: pr`
- `workflow.status: in_progress`
- **Continue to PR phase**


---

## YAML Checkpoint (Required before transition)

Before signaling completion to the dispatcher, verify:
- `phases.implement.status: complete`
- `phases.implement.commits` is non-empty
- History contains `implementation_complete`
- `phases.implement.codex_validated` is set (true if ran, false if skipped/unavailable)
- `phases.implement.validation_synthesis_report` is set
- `phases.implement.validation_synthesis_verdict` is set
- `phases.implement.validation_fix_iteration_count` is set (0 or 1)
