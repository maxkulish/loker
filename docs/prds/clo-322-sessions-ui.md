# CLO-322: Sessions List + Per-Run Trace + Pending Panel

## Problem Statement

Lok users currently have no visual interface to browse workflow run history
or resolve human-in-the-loop gates. The only way to inspect a run is to
`cat manifest.json` and manually tail trace files on disk. The T-052 daemon
provides a JSON `GET /` with run metadata, but there's no HTML rendering,
no per-run detail view, and no aggregated view of pending HITL gates.

We need server-rendered HTML views served by the existing axum daemon so
users can browse sessions, drill into individual runs, and act on pending
gates — all from a browser on localhost.

## Who is affected

Lok users who want to monitor, inspect, and control long-running or paused
workflow runs without CLI commands or filesystem navigation.

## Current state

- T-052 daemon runs axum on localhost.
- `GET /` returns JSON array of `RunSummary` objects (from `discover_runs()`).
- T-051 HITL server has `render_gate_view()` returning per-gate HTML forms.
- No HTML rendering for the run list. No per-run detail page. No pending
  gates aggregate panel.

## Desired state

Three server-rendered HTML views:

1. **Index (`/`)** — table of all runs: status, workflow, started/ended
   timestamps, current phase. Rendered from `discover_runs()` data.
2. **Run detail (`/runs/:id`)** — rendered manifest entries, phase timeline,
   last N trace events from `trace.jsonl`.
3. **Pending panel (`/pending`)** — every currently-paused HITL gate across
   all runs, with approve/reject forms POSTing to T-051 gate endpoints.

All views use server-rendered HTML (Askama), no JS framework, minimal CSS.
Snapshot tests on rendered HTML for stable views.

## Constraints

- Server-rendered HTML (Askama templates).
- No JavaScript framework.
- Minimal CSS.
- Localhost-only bind (127.0.0.1).
- Reuses T-051 HITL endpoints for approve/reject.
- Reuses T-052 discovery (`discover_runs()`, `RunSummary`).
- Reuses existing `minijinja` if needed for trace event formatting.

## Acceptance criteria

1. `GET /` returns HTML table of all runs with correct metadata.
2. `GET /runs/:id` renders manifest, phase timeline, last N trace events.
3. `GET /pending` shows active HITL gates across all runs with working
   approve/reject forms.
4. Pending panel transitions to empty when all gates are resolved.
5. Snapshot tests (insta) on rendered HTML for all three views against
   fixture data.
6. Existing T-052 JSON endpoint and T-051 gate routes continue to work.

## Non-goals

- Live updates via SSE (T-054).
- Auth, theming, mobile layout.
- Client-side JavaScript of any kind.

## Dependencies

- T-052: daemon mode (`src/ui/serve.rs`, `src/ui/routes.rs`, `src/ui/discovery.rs`).
- T-051: HITL gate routes (`src/hitl_server/routes.rs` — approve/reject POST).

## References

- PRD FR-24 — `loker ui` sessions list + per-run trace + pending panel.
- PRD FR-25 — SSE tail-f (T-054, out of scope here).
- Roadmap Phase 12.
- `docs/security/2026-04-25-ui-threat-model.md` (mitigations applied in T-055).
