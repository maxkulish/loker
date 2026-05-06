# Workflow Spec Reference

## HumanVerifier severity ladder

Human-in-the-loop gates use a severity ladder to decide what happens when no human response file is present by the effective deadline.

| Severity | Default timeout | Default timeout action |
| --- | --- | --- |
| `low` | `1h` | `auto_approve` |
| `medium` | `24h` | `auto_fail` |
| `high` | none | `block` |

Explicit human responses always win over timeout policy. `approve` passes, `reject` fails, and `comment_only` fails because it is not approval.

Pending request files under `runs/<id>/pending/<phase>.json` persist the effective `timeout_at` for that gate instance. Re-running a blocked phase does not extend an existing deadline. The pending JSON schema allows `timeout_at` to be either a date-time string or `null` for every severity because workflow policy may override the defaults.

### Example

```toml
[phases.review.verify.human]
severity = "medium"

[phases.review.verify.human.timeout.medium]
duration = "24h"
on_timeout = "auto_fail"
```

### Override semantics

- `duration = "1h"` writes `timeout_at` as `opened_at + duration`.
- Omitting `duration` or setting an equivalent no-timeout policy writes `timeout_at = null`.
- `on_timeout = "auto_approve"` returns a passing verify result after timeout.
- `on_timeout = "auto_fail"` returns a failing verify result after timeout.
- `on_timeout = "block"` continues waiting even if a deadline exists.

Trace spans for HumanVerifier gates include `loker.hitl.severity`, `loker.hitl.timeout_at` when present, `loker.hitl.timeout_action`, and `loker.hitl.timeout_outcome`. Completed and failed phase markers include the same HITL context under an optional `hitl` object.
