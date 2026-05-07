# Design Review: CLO-322

**Reviewer**: Gemini (via Claude fallback — external Gemini returned empty output)
**Reviewed**: 2026-05-07
**Pipeline**: lok design-review (fallback)
**Document**: docs/designs/clo-322-sessions-ui.md

---

## 1. Completeness Check

All 7 required sections present and substantive:

| Section | Status | Notes |
|---------|--------|-------|
| Problem | ✅ Present | Clear 1-paragraph summary linked to discovery |
| Goals / Non-goals | ✅ Present | 6 goals, 4 non-goals, well-scoped |
| Architecture | ✅ Present | Module layout, routes, AppState, data flow |
| Public API surface | ✅ Present | Types, functions, handler signatures |
| Test plan | ✅ Present | 10 unit + 10 snapshot + manual + regression |
| Migration / rollout | ✅ Present | Dependencies, file changes, rollout path |
| Open questions | ✅ Present | 5 questions, all with decisions |

## 2. Architecture Assessment

**Strengths**:
- Clean layering: `gate_discovery.rs` is a new focused module, not mixed into `discovery.rs`
- AppState extension is minimal (just `max_trace_events`)
- Data flow diagrams are concrete — no hand-waving
- Askama integration via `with-axum` feature avoids extra crate dependency

**Concerns**:
- `POST /gates/:run_id/:phase/approve` uses format `Path((run_id, phase))` — need to verify axum 0.7 supports this destructuring. Actually, axum supports `Path<(String, String)>` via serde. Verified.
- No explicit mention of how `DecisionForm` from `hitl_server` is shared with `routes.rs`. The form type needs to be re-exported or duplicated.
- `build_phase_timeline` needs access to the workflow definition to know which phases exist. The design assumes phase names come from markers only, which means phases with no markers won't appear. Clarified.

## 3. Alignment with Handoff & Roadmap

- ✅ FR-24: Sessions list + per-run trace + pending panel — all three views covered
- ✅ FR-27: Shared route handlers between daemon and one-shot — daemon mounts T-051 POST routes
- ✅ FR-28: Localhost-only bind — unchanged from T-052
- ✅ Roadmap Phase 12: T-053 blocks T-054 (SSE) — this design doesn't expand scope into SSE

## 4. Security Review

- ✅ Path traversal: `run_id` should be sanitized — the `Path(run_id)` in axum doesn't validate. Need to reject `..` and `/` in the `run_id` parameter. **Add this to the handler.**
- ✅ No secrets in rendered HTML — data comes from manifest/trace files already on disk
- ✅ Forms POST to daemon routes — same origin as the browser, XSS not a concern with server-rendered HTML
- ✅ The `pending_file_path` in `PendingGate` includes the full path. This is only used server-side to read JSON; never rendered in templates.

## 5. Implementation Concerns

- The `tail_trace_file` function reads entire file + splits lines. For large trace files (100k+ events), this is inefficient. Consider `rev_lines` crate or simple byte-scan from end. **Low priority for v0 — trace files are bounded.**
- `discover_pending_gates` re-scans the entire runs directory on every `GET /pending`. For many runs, consider caching with a short TTL. **Defer to post-v0.**
- `read_manifest_entries` duplicates some logic from `discovery.rs::load_run_summary`. Consider extracting shared manifest parsing into a utility.

## 6. Concurrency & Async

- All handlers are `async` but do sync filesystem I/O. The `discover_runs` and `discover_pending_gates` functions are sync — they should be called in `tokio::task::spawn_blocking` for production. **Add to implementation notes.**
- No shared mutable state — `AppState` is `Clone + immutable`. Safe.
- The approve/reject handlers do `PhaseLock::acquire` which is sync. Same concern — should be in `spawn_blocking`.

## 7. Blind Spots

1. **Missing error page template**: The design mentions 404 for unknown runs but doesn't define an error template. Need a simple `error.html` or inline error HTML.
2. **Run directory naming collisions**: `run_id` from URL maps to directory basename. If two projects have the same run ID, this won't cause issues since the daemon is project-scoped, but worth noting.
3. **No health check endpoint**: Useful for integration tests. `/health` returning 200 would help T-054 SSE tests later. **Nice-to-have.**
4. **CSS specificity**: Inline `<style>` in layout template could conflict if we add per-view styles later. The design addresses this by keeping CSS minimal, but a `{% block styles %}` approach in Askama would be more flexible.

## 8. Verdict

**APPROVE_WITH_SUGGESTIONS**

The design is complete, well-structured, and covers all required views. Minor gaps (path sanitization, error template, blocking I/O in async handlers) should be addressed during implementation. None are blocking.

## 9. Actionable Feedback

| # | Priority | Category | Suggestion |
|---|----------|----------|------------|
| 1 | High | Security | Sanitize `run_id` in `run_detail` handler — reject values containing `..` or `/` |
| 2 | High | Completeness | Add error template (`error.html`) for 404 and 500 responses |
| 3 | Medium | Performance | Use `tokio::task::spawn_blocking` for sync filesystem I/O in async handlers |
| 4 | Medium | Code quality | Extract shared manifest parsing between `discovery.rs` and new `run_detail` logic |
| 5 | Low | Architecture | Add `/health` endpoint returning 200 for integration test readiness |
| 6 | Low | Performance | Consider `rev_lines` or byte-scan for `tail_trace_file` on large files |
