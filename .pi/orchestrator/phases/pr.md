# Phase: pr

Open the pull request, run pre-flight checks, and shepherd reviews until
CI is green. Mirrors `.claude/commands/task/phases/pr.md`.

## Required exit state

```yaml
phases:
  pr:
    status: complete
    pr_url: "https://github.com/maxkulish/loker/pull/<n>"
    pr_number: <n>
    ci_passed: true
    reviews_addressed: true
    merged_at: "<ISO-8601>"
    merge_commit: "<sha>"
```

History events required: `pre_flight_checks_passed`, `pr_created`.
Optional: `review_addressed`, `pr_merged`.

## Step 4.0 - Pre-flight checks (MANDATORY)

These run before opening the PR. They must all pass:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo clippy --tests
cargo test
```

If `make check` already covers fmt+clippy+test, `make check` is sufficient.

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "pr",
  action: "pre_flight_checks_passed",
  details: "make check green: fmt + clippy + test all pass",
  phase_updates: { status: "in_progress" }
})
```

## Step 1 - Push the branch

```bash
git push -u origin feat/clo-XX-<slug>
```

## Step 2 - Open the PR

```bash
gh pr create \
  --title "feat(CLO-XX): <one-line summary>" \
  --body "$(cat <<'EOF'
## Summary
<2-3 bullets describing the change>

## Plan
- docs/plans/clo-XX-<slug>.md

## Validation
- Codex: docs/reviews/clo-XX-codex-validation.md (verdict: approve)
- Gemini: docs/reviews/clo-XX-gemini-validation.md (verdict: approve)
- make check green locally

Closes CLO-XX
EOF
)"
```

Capture the URL and number, then:

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "pr",
  action: "pr_created",
  details: "PR #<n> opened: <url>",
  phase_updates: {
    pr_url: "<url>",
    pr_number: <n>
  }
})
```

Update Linear:

```
mcp__linear__save_issue(id="CLO-XX", state="In Review")
mcp__linear__save_comment(issueId="CLO-XX", body="PR #<n>: <url>")
```

## Step 3 - Wait for CI

Poll until CI completes:

```bash
gh pr checks <n> --watch
```

If CI fails, fix locally, push, repeat. Update state on each iteration:

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "pr",
  action: "ci_iteration",
  details: "<what failed>; <how fixed>; pushed <sha>"
})
```

When CI is green:

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "pr",
  action: "ci_passed",
  details: "All required checks passing",
  phase_updates: { ci_passed: true }
})
```

## Step 4 - Address review comments

For each comment from a human reviewer or `gemini-code-assist[bot]` /
`Copilot`:

1. Make the requested change locally.
2. Push.
3. Reply to the comment.

**CRITICAL**: replies to `gemini-code-assist[bot]` MUST end with
`/gemini review` on its own line, otherwise the bot will not re-evaluate.
Replies to other reviewers do not need that trailer.

```bash
gh api repos/maxkulish/loker/pulls/<n>/comments \
  -F body="Addressed in <sha>: <one-line how>.

/gemini review"
```

When all review threads are resolved:

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "pr",
  action: "review_addressed",
  details: "<n> threads resolved. /gemini review trailer used on all bot replies.",
  phase_updates: { reviews_addressed: true }
})
```

## Step 5 - Approval checkpoint

Auto Mode may merge once:

- `ci_passed: true`
- `reviews_addressed: true`
- All required reviewers approved (or no reviewers required)

Otherwise wait for the user.

## Step 6 - Transition

```ts
transition_phase({
  task_id: "CLO-XX",
  from_phase: "pr",
  to_phase: "complete"
})
```

The actual merge happens in `complete.md` (squash + cleanup are coupled).

## Notes

- Never force-push to a shared PR branch without warning the user.
- If a reviewer requests changes that contradict the design, surface
  the conflict in the PR thread rather than silently complying.
