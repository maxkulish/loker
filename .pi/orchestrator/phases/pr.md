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

## Step 3.5 - Wait for bot reviews then fetch and address all comments

**Do this immediately after CI goes green** - bot reviewers (gemini-code-assist,
copilot-pull-request-reviewer) post their inline comments within minutes of a
green run. PRs merged without this step miss all bot feedback.

### 3.5.1 - Wait for bot reviewers to post

After `ci_passed` is logged, poll for inline review comments instead of a
single long blocking sleep (long sleeps may be truncated by the agent
runtime, and bots usually post within 1-3 minutes anyway):

```bash
PR=<n>
REPO=maxkulish/loker

for i in $(seq 1 60); do
  count=$(gh api repos/${REPO}/pulls/${PR}/comments --paginate --jq 'length' 2>/dev/null || echo 0)
  if [ "$count" -gt 0 ]; then
    echo "Found ${count} inline review comment(s) after ${i} poll(s)"
    break
  fi
  sleep 10
done
```

If after 10 minutes no inline comments exist, proceed anyway - some PRs
get only PR-level (issue) comments or no bot review at all. Step 3.5.2
fetches both endpoints.

### 3.5.2 - Fetch all inline comments

```bash
PR=<n>
REPO=maxkulish/loker

# All inline review comments (paginated - do not omit --paginate)
gh api repos/${REPO}/pulls/${PR}/comments --paginate \
  --jq '.[] | {id, path, line: .original_line, body, user: .user.login, commit_id: .original_commit_id}'

# General PR-level comments
gh pr view ${PR} --json comments \
  --jq '.comments[] | {id: .databaseId, body, author: .author.login}'
```

### 3.5.3 - Categorize comments

| Reviewer | Severity signal | Priority |
|----------|----------------|----------|
| `gemini-code-assist` | `**Severity**: high/medium/low` in body | Parse it; default medium |
| `copilot-pull-request-reviewer` | None | Treat as medium |
| Human | CHANGES_REQUESTED state | High; COMMENTED = medium |

High-severity and CHANGES_REQUESTED comments are blocking - must be addressed
before merge. Medium/low may be addressed or declined with rationale.

### 3.5.4 - Stale comment detection

For each inline comment, check if the referenced code has changed since it was
posted:

```bash
git diff <original_commit_id>..HEAD -- <path>
```

If lines within 5 of the commented line changed, flag as `[STALE?]` and confirm
with user before acting. Do not auto-skip stale comments.

### 3.5.5 - Address feedback, commit, push

Group comments by file. Address all comments on a file together, then commit:

```bash
git add <modified files>
git commit -m "$(cat <<'EOF'
fix(CLO-XX): address PR review feedback

- <file>: <change> (<reviewer>)

Resolves <N> review comments
EOF
)"
git push origin feat/clo-XX-<slug>
```

Push **before** replying so commit SHAs are live on GitHub when reviewers read
the replies.

### 3.5.6 - Reply or resolve each thread

For each thread, check its current state before acting:

**Fetch thread state (GraphQL node IDs required to resolve):**

```bash
REPO=maxkulish/loker
PR=<n>

gh api graphql -f query='
query($owner:String!, $repo:String!, $pr:Int!) {
  repository(owner:$owner, name:$repo) {
    pullRequest(number:$pr) {
      reviewThreads(first:100) {
        nodes {
          id
          isResolved
          comments(first:20) {
            nodes { author { login } body }
          }
        }
      }
    }
  }
}' -f owner=maxkulish -f repo=loker -F pr=<n>
```

**Decision per thread:**

| Thread state | Action |
|---|---|
| Already resolved | Skip |
| Gemini's latest comment approves the fix ("looks good", "this is sound", "no further action") | Resolve only - no reply |
| Awaiting author fix (no author reply yet) | Post reply with `/gemini review`, then resolve after Gemini approves |
| Author replied but Gemini hasn't re-reviewed | Post `/gemini review` reply to trigger re-review |
| Declined suggestion | Post "Intentionally kept as-is: `<rationale>`" reply |

**CRITICAL: one reply per thread, maximum. NEVER post a second standalone comment
to add the trigger after the fact.**

**Resolve a thread (no reply needed when Gemini already approved):**

```bash
gh api graphql -f query='
mutation($id:ID!) {
  resolveReviewThread(input:{threadId:$id}) {
    thread { id isResolved }
  }
}' -f id="<thread_graphql_id>"
```

**Reply when fix needs Gemini re-validation:**

```bash
COMMIT_SHA=$(git rev-parse --short HEAD)

gh api repos/${REPO}/pulls/${PR}/comments/<comment_id>/replies \
  -X POST -f body="Fixed in ${COMMIT_SHA}. <one-line explanation>

/gemini review"
```

**Reply for declined suggestions:**

```bash
gh api repos/${REPO}/pulls/${PR}/comments/<comment_id>/replies \
  -X POST -f body="Intentionally kept as-is: <rationale>.

/gemini review"
```

The `/gemini review` trailer asks Gemini to re-evaluate after the
rationale. If Gemini accepts, the thread can be resolved; if it pushes
back, escalate via Step 4.

Track reply count in state update.

### 3.5.7 - Re-check for new comments

After pushing and replying, check for new unresolved threads (bots re-review
after the `/gemini review` trigger):

```bash
gh pr view ${PR} --json reviews,reviewDecision
gh api repos/${REPO}/pulls/${PR}/comments --paginate \
  --jq '.[] | select(.created_at > "<push_timestamp>") | {id, user: .user.login, body}'
```

If new comments exist in unresolved threads, return to 3.5.3 and repeat.
Threads already resolved by Gemini approval can be skipped. Otherwise proceed.

### 3.5.8 - Log state

```ts
update_workflow_state({
  task_id: "CLO-XX",
  phase: "pr",
  action: "review_addressed",
  details: "<N> threads resolved; replies posted N/N; /gemini review trailer on all inline replies.",
  phase_updates: { reviews_addressed: true }
})
```

## Step 4 - Address escalated review comments

If Step 3.5 surfaces a comment that requires a design change or contradicts the
existing plan, surface the conflict in the PR thread rather than silently
complying. Options:

- Post a PR comment explaining the tension and asking for guidance.
- Link to the relevant design doc or ADR.
- Tag the user for a decision if blocking.

When all threads are resolved and `reviews_addressed: true` is set, proceed.

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
