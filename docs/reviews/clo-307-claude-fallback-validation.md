# Pre-PR validation: clo-307

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-04
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [high] `deploy/tensorzero/.env` not gitignored — secrets-leak risk
**Where:** `.gitignore:3`
**What:** The documented quick-start (root README, `deploy/tensorzero/README.md`, and the new compose comment header) tells users to `cd deploy/tensorzero && cp ../../tensorzero/.env.example .env`. The resulting `deploy/tensorzero/.env` will hold real `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` values but is not in `.gitignore` — only the old `tensorzero/.env` is. A `git add -A` after the documented flow stages real secrets.
**Suggested fix:** Add `deploy/tensorzero/.env` (or a broader `**/.env` / `**/tensorzero/.env`) to `.gitignore` in this PR.

### F2 [medium] Orphan `deploy/tensorzero/.env.example` — confusing dead file
**Where:** `deploy/tensorzero/.env.example` vs. `README.md:82`, `deploy/tensorzero/README.md:11`, `deploy/tensorzero/docker-compose.yml:7`
**What:** A new `deploy/tensorzero/.env.example` is added (byte-identical to `tensorzero/.env.example`) but every documented copy command points at the *old* path `../../tensorzero/.env.example`. The new file is never referenced. Two sources of truth for env templates means future drift, and a confused user who edits the local one wonders why nothing happened.
**Suggested fix:** Pick one. Either (a) delete `deploy/tensorzero/.env.example` and keep the cross-tree references, or (b) keep the local one and switch all three docs to `cp .env.example .env`. Option (b) is the cleaner outcome since `deploy/tensorzero/` is now canonical per the roadmap (T-038).

### F3 [low] Stale path in integration-test docstring
**Where:** `tests/tensorzero_integration.rs:25`
**What:** The docstring still says `cd tensorzero && docker compose up -d` while every other touched doc has been updated to `deploy/tensorzero`. Developers reading the test header will land on the soon-to-be-deprecated path.
**Suggested fix:** Update the inline run instructions to `cd deploy/tensorzero && docker compose up -d` to match the new canonical path. (The handoff already calls the old path "backward compatibility".)

### F4 [low] Two near-identical compose files with no deprecation plan
**Where:** `tensorzero/docker-compose.yml`, `deploy/tensorzero/docker-compose.yml`
**What:** The new file diverges from the old by exactly two lines (header comment + `volumes:` path). `docs/handoff.md:82–83` keeps the old one alive "for backward compatibility" but nothing schedules its removal. This is fine for one cycle but will silently drift (e.g., next image bump touches one but not both).
**Suggested fix:** Either reduce the old file to a one-line README pointer/tombstone now, or open a follow-up Linear ticket and reference it from `docs/handoff.md:82` so the "backward compat" caveat has an owner and an exit.

### F5 [info] Workflow tracker committed to repo
**Where:** `docs/status/clo-307-workflow.yaml`
**What:** The 150-line workflow tracker is included in the PR. `docs/status/` already contains 40+ such files, so the convention is established — flagging only so reviewers know it was an intentional pattern, not noise.
**Suggested fix:** None unless the team wants to revisit the convention.

## Verdict
**approve_with_changes**

The change is scoped correctly to T-038's acceptance criteria (compose with gateway+ClickHouse+UI under `deploy/tensorzero/`, plus root README pointer), `docker compose -f deploy/tensorzero/docker-compose.yml config -q` parses cleanly, and there are no Rust-code changes so `make check` is unaffected. The blocker is F1: the documented quick-start writes a real-secret `.env` to a path that isn't gitignored — that's a one-line fix but should land before merge. F2 (orphan `.env.example`) is also worth resolving in this PR to avoid two divergent templates from day one. F3/F4 are clean-up nits that can ride along or land as a follow-up.
