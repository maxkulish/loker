# Design Review Synthesis: CLO-322

**Synthesized**: 2026-05-07
**Pipeline**: lok design-review synthesis
**Source**: Single reviewer (Gemini via Claude fallback — Ollama failed with model-not-found)

---

## Reviewer Status

| Reviewer | Status | Detail |
|----------|--------|--------|
| Gemini | FAILED | External Gemini returned empty output (MCP initialization warnings only) |
| Ollama | FAILED | ProviderModelNotFoundError — glm-5.1:cloud not available locally |
| Claude Fallback | OK | Produced full structured review |

## Source

Claude fallback review (docs/reviews/clo-322-review-gemini.md).

## Key Findings

| # | Finding | Severity |
|---|---------|----------|
| 1 | `run_id` path parameter lacks sanitization against `..` and `/` traversal | High |
| 2 | No error template defined for 404 / 500 responses | High |
| 3 | Sync filesystem I/O in async handlers should use `spawn_blocking` | Medium |
| 4 | Manifest parsing logic duplicated between `discovery.rs` and new `run_detail` code | Medium |
| 5 | Missing `/health` endpoint for integration test readiness | Low |
| 6 | `tail_trace_file` implementation may be slow on large trace files | Low |

## Consolidated Verdict

**APPROVE_WITH_SUGGESTIONS**

## Priority Actions

1. **Sanitize `run_id`** — reject values containing `..` or `/` in the `run_detail` handler.
2. **Add `error.html` template** — render 404 and 500 responses with Askama.
3. **Use `spawn_blocking`** — wrap `discover_runs()`, `discover_pending_gates()`, `PhaseLock::acquire()` in `tokio::task::spawn_blocking`.
4. **Extract shared manifest parsing** — create `src/ui/manifest.rs` with `parse_manifest()` shared by both `discovery.rs` and routes.
5. **Add `/health`** — simple 200 OK endpoint.

## Decision Recommendation

**PROCEED_WITH_FIXES**

The design is approved with two high-priority fixes (path sanitization, error template) to be implemented during the implementation phase. The medium-priority items (spawn_blocking, manifest refactoring) improve correctness and maintainability but are not blocking.
