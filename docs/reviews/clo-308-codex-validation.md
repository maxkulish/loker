# Pre-PR validation: clo-308

**Reviewer**: Codex (gpt-5.5)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
---

Error: Shell command failed: TASK="clo-308"
BRANCH="feat/clo-308-doctor"
PRIMARY_MODEL="${CODEX_MODEL:-gpt-5.5}"

PERSONA=$(cat .pi/agents/codex-pre-pr.md)
PROMPT=$(cat <<EOF
$PERSONA

---

You are a senior code reviewer. Review all changes on this branch against this task's design document and implementation plan.

Inputs:
- Task: $TASK
- Branch: $BRANCH
- Design: see docs/designs/${TASK}-*.md (read it first)
- Plan: see docs/plans/${TASK}-*.md (read it first)
- Diff: \`git diff main...HEAD\`

Check correctness, completeness, regressions, code quality, security, schema/API compatibility, and scope creep.

End your output with:

## Verdict
approve | approve_with_changes | rework
EOF
)

OUTPUT=$(timeout 570 codex exec -m "$PRIMARY_MODEL" -s read-only "$PROMPT" 2>/tmp/lok-codex-stderr.log)

if [ -z "$OUTPUT" ] || [ $(printf '%s' "$OUTPUT" | wc -c) -lt 100 ]; then
  STDERR=$(head -10 /tmp/lok-codex-stderr.log 2>/dev/null || echo "no stderr")
  echo "REVIEW_FAILED: Empty or trivially short output from Codex model $PRIMARY_MODEL (stderr: $STDERR)"
  exit 0
fi

echo "$OUTPUT"
sh: -c: line 30: unexpected EOF while looking for matching `''
sh: -c: line 37: syntax error: unexpected end of file

