# Pre-PR validation: clo-307

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-04
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc quoting error in invocation script (`unexpected EOF while looking for matching '`) — script never reached the model. Tooling failure, not a code signal. |
| Gemini | REVIEW_FAILED | Same shell heredoc quoting error in invocation script — script never reached the model. Tooling failure, not a code signal. |
| Claude (fallback) | OK | Produced 5 findings (1 high, 1 medium, 2 low, 1 info) against design/plan and diff. |

## Verdict
approve_with_changes

## Must Fix Before PR
- **F1 — `deploy/tensorzero/.env` not gitignored (security/secrets-leak).** The documented quick-start in `README.md`, `deploy/tensorzero/README.md`, and the new compose header all instruct users to create `deploy/tensorzero/.env` populated with real `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` values. `.gitignore:3` only excludes the legacy `tensorzero/.env` path, so a routine `git add -A` after following the docs stages real secrets. One-line fix: add `deploy/tensorzero/.env` (or `**/.env`) to `.gitignore` in this PR.
- **F2 — Resolve the orphan `deploy/tensorzero/.env.example`.** A new env template was added but every documented `cp` command still points at `../../tensorzero/.env.example`, leaving two divergent sources of truth from day one. Pick one in this PR: either delete `deploy/tensorzero/.env.example` (keep cross-tree references), or switch the three doc sites to `cp .env.example .env`. Per the T-038 roadmap intent (canonicalize on `deploy/tensorzero/`), the latter is preferred. Bounded and lands cleanly with F1.

## Out of Scope / Deferred
- **F3 — Stale path in `tests/tensorzero_integration.rs:25` docstring.** Low-impact cleanup; can ride along with the F1/F2 fix iteration if convenient, otherwise defer.
- **F4 — Duplicate compose files with no deprecation owner.** `docs/handoff.md:82–83` already documents the legacy `tensorzero/docker-compose.yml` as kept "for backward compatibility". A follow-up Linear ticket to schedule removal is the right move; not a blocker for this PR.

## False Positives / Tooling Artifacts
- **Codex and Gemini reviews** — both failed due to shell heredoc quoting bugs in the invocation scripts (`.pi/agents/...` wrappers), not due to any signal about the change. The wrapper scripts should be patched, but that's orchestrator hygiene, not a finding against this branch.
- **F5 — Workflow tracker `docs/status/clo-307-workflow.yaml`.** Established repo convention (40+ siblings). No action.

## Recommendation
PROCEED_WITH_FIXES. Two bounded edits before opening the PR:
1. Add `deploy/tensorzero/.env` to `.gitignore` (F1, security).
2. Resolve the env-template duplication (F2) — preferred path: keep `deploy/tensorzero/.env.example` and update the three doc sites (`README.md:82`, `deploy/tensorzero/README.md:11`, `deploy/tensorzero/docker-compose.yml:7` comment) to `cp .env.example .env`.

Optionally fold F3 (one-line docstring) into the same iteration. F4 should be tracked as a follow-up Linear ticket but does not block. Codex/Gemini wrapper scripts have a heredoc bug that should be fixed separately so future syntheses aren't single-reviewer.

## Re-validation

Fix iteration applied (commit `d2d2908`):

| Finding | Status | Details |
|---------|--------|---------|
| F1 — `.gitignore` for `deploy/tensorzero/.env` | ✅ FIXED | Added `deploy/tensorzero/.env` to `.gitignore` |
| F2 — Orphan `.env.example` | ✅ FIXED | Updated all 4 doc sites (`README.md`, `deploy/tensorzero/README.md`, `deploy/tensorzero/docker-compose.yml` comment, `docs/handoff.md`) to reference local `cp .env.example .env` |
| F3 — Stale integration test docstring | ✅ FIXED | Updated `tests/tensorzero_integration.rs:25` to `cd deploy/tensorzero && docker compose up -d` |
| F4 — Duplicate compose files | 📋 DEFERRED | Create follow-up Linear ticket to schedule removal of legacy `tensorzero/docker-compose.yml` |
| F5 — Workflow tracker | ℹ️ NO ACTION | Existing repo convention |

`make check` — ✅ passes.

**Re-validation verdict**: `approve` — all Must Fix items resolved. Ready for PR.
