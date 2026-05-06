# Pre-PR validation: clo-310

**Reviewer**: Gemini (gemini-3.1-pro-preview)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
---

Error: Shell command failed: TASK="clo-310"
BRANCH="feat/clo-310-resume"
PRIMARY_MODEL="${GEMINI_MODEL:-gemini-3.1-pro-preview}"
FALLBACK_MODEL="${GEMINI_FALLBACK_MODEL:-gemini-2.5-pro}"

PERSONA=$(cat .pi/agents/gemini-architect.md)
PROMPT=$(cat <<EOF
$PERSONA

---

You are a senior code reviewer. Review all changes on this branch against this task's design document and implementation plan.

Inputs:
- Task: $TASK
- Branch: $BRANCH
- Design: read docs/designs/${TASK}-*.md
- Plan: read docs/plans/${TASK}-*.md
- Diff: \`git diff main...HEAD\`

Check design fidelity, correctness, API ergonomics, test coverage, Rust idioms, unintended public surface, scope creep.

End your output with:

## Verdict
approve | approve_with_changes | rework
EOF
)

OUTPUT=$(timeout 300 gemini --model "$PRIMARY_MODEL" -y --sandbox \
  --include-directories docs,src,tests \
  -p "$PROMPT" -o text 2>/tmp/lok-gemini-impl-stderr.log)

if [ -z "$OUTPUT" ] || [ $(printf '%s' "$OUTPUT" | wc -c) -lt 100 ]; then
  OUTPUT=$(timeout 300 gemini --model "$FALLBACK_MODEL" -y --sandbox \
    --include-directories docs,src,tests \
    -p "$PROMPT" -o text 2>/tmp/lok-gemini-impl-stderr.log)

  if [ -z "$OUTPUT" ] || [ $(printf '%s' "$OUTPUT" | wc -c) -lt 100 ]; then
    STDERR=$(head -5 /tmp/lok-gemini-impl-stderr.log 2>/dev/null || echo "no stderr")
    echo "REVIEW_FAILED: Empty output from both $PRIMARY_MODEL and $FALLBACK_MODEL (stderr: $STDERR)"
    exit 0
  fi
fi

echo "$OUTPUT"
sh: -c: line 38: unexpected EOF while looking for matching `''
sh: -c: line 46: syntax error: unexpected end of file

