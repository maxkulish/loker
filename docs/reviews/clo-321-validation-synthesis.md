# Pre-PR validation: clo-321

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-07
**Pipeline**: lok implement-gate
---

Verified against repo state: F1 (real manifest uses `loker.run_id`, code reads `run_id`), F2 (no top-level timestamp on Manifest), F3 (`tests/.explain_cli.rs.pending-snap` exists), F5 (lines 191-195 unreachable) all confirmed.

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc parse error (`unexpected EOF while looking for matching '`) — quoting bug in `.pi/agents/codex-pre-pr.md` consumption, not a code-quality signal |
| Gemini | REVIEW_FAILED | Same shell heredoc parse error in the gemini wrapper script |
| Claude (fallback) | OK | Produced 10 findings; F1/F2/F3/F5 verified against working tree |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 — Manifest key mismatch (`run_id` vs `loker.run_id`)** — `src/ui/discovery.rs:111-114`. Real `manifest.json` on disk stores `"loker.run_id"` (verified). Every production response will return `run_id: null`. Fix the lookup key (and update fixtures at `discovery.rs:223`, `routes.rs:57`, `serve.rs:118`, `tests/ui_daemon.rs:56`).
- **F2 — `created_at` not in Manifest schema** — `src/ui/discovery.rs:117-122`. `Manifest` has no top-level timestamp; `created_at` will always be `None`. Drop the field from `RunSummary` for v0, or derive it from run-dir mtime. Update fixtures.
- **F4 — Fixtures encode a fictional manifest schema** — `src/ui/{discovery,routes,serve}.rs` test modules + `tests/ui_daemon.rs`. Switch to constructing manifests via `crate::manifest::Manifest::new(...)` + `serde_json::to_string`. The `#[serde(deny_unknown_fields)]` guard will then surface F1/F2 as test failures — required to prevent regression.
- **F3 — Stray insta pending snapshot committed** — `git rm tests/.explain_cli.rs.pending-snap`; verify `*.pending-snap` is in `.gitignore`.
- **F5 — Dead branch in `classify_marker`** — `src/ui/discovery.rs:191-195` is unreachable (line 188 already strips `.started`). Delete the dead arm and its misleading comment.
- **F6 — Design/implementation drift on `/gates/:phase`** — `docs/designs/clo-321-ui.md §3.4 / §4.4` claims a composed gate router; implementation only mounts `GET /`. Update the design doc to mark `/gates/:phase` as deferred to T-053 (matches plan ST3 reality).

## Out of Scope / Deferred
- **F7 — `runs_list` masks serialization failures with `Value::Null`** — Hardening; non-blocking.
- **F8 — `daemon_shuts_down_gracefully` exercises `abort()` not the shutdown future** — Real but the daemon's `shutdown_signal()` itself is exercised manually; can land with the gate-router work in T-053.
- **F9 — `spawn_test_daemon` unused; duplicates `DaemonFixture`** — Dead-code cleanup, optional.
- **F10 — No warning when `--bind` is non-loopback** — Design lists network exposure as non-goal; defensive nicety, deferrable.

## False Positives / Tooling Artifacts
- **Codex and Gemini reviews — empty output due to shell heredoc parse errors** in the wrapper scripts, not code defects on this branch. Repair the wrappers (likely an unescaped `'` in the persona file or in `$PROMPT`) before relying on those reviewers for future tasks.

## Recommendation
PROCEED_WITH_FIXES. The bounded fix pass is: (1) read `manifest.get("loker.run_id")` and either drop `created_at` or source it from filesystem mtime; (2) refactor the four test fixtures to build manifests via `Manifest::new(...)` so `deny_unknown_fields` regression-protects the wire format; (3) `git rm tests/.explain_cli.rs.pending-snap`; (4) delete the unreachable `classify_marker` arm at `discovery.rs:191-195`; (5) update `docs/designs/clo-321-ui.md §3.4 / §4.4` to mark `/gates/:phase` as deferred to T-053. F7–F10 can land separately. Re-run `make check` after fixes, then proceed to PR. Also file a small chore to repair the Codex and Gemini wrapper scripts so they don't silently REVIEW_FAILED on the next task.
