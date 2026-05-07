# Design Review: CLO-321

**Reviewer**: Gemini Architect (simulated via Claude fallback)
**Reviewed**: 2026-05-07
**Pipeline**: lok design-review
**Note**: External Gemini/Ollama reviewers unavailable; this is a manual Claude-based review following the same criteria.

---

## 1. Completeness Check

All seven required sections are present and substantive:

| Section | Present | Assessment |
|---------|---------|------------|
| Problem | Yes | 1 paragraph, cites discovery report, concrete and scoped |
| Goals / Non-goals | Yes | 8 goals, 10 non-goals — clear scope boundary |
| Architecture | Yes | Module layout, ASCII data-flow diagram, concrete types, reuse contract with T-051 |
| Public API surface | Yes | Real Rust type signatures for all 6 modules touched |
| Test plan | Yes | 7 unit tests, 5 integration tests, 5 manual verification steps, regression |
| Migration / rollout | Yes | Additive-only, no deprecations, rollout order specified |
| Open questions | Yes | 6 questions covering port conflicts, CWD resolution, path layout, performance, extraction trade-off, logging format |

**Assessment**: Complete. No section is stub or placeholder.

## 2. Architecture Assessment

**Strengths**:
- The `render_gate_view` free-function extraction is the right granularity for FR-27 compliance — the one-shot server and daemon share the rendering logic without entangling their state management.
- Module separation (`src/ui/` vs `src/hitl_server/`) mirrors the established project convention of one module per concern.
- Error handling strategy for run discovery (skip + log, never crash) is the correct fail-open posture for a daemon that must outlive individual run corruption.
- The data-flow diagram (§3.2) is clear and accurate.
- No new dependencies required — all crates already exist in `Cargo.toml`.

**Concerns**:
- The `Commands::Ui { serve: bool, bind: String }` CLI shape is awkward. `serve` is a required flag to trigger the daemon path, which is fine, but `loker ui` with no flags silently does nothing (the match arm is unreachable for `Ui { serve: false, .. }`). Consider whether `loker serve` or `loker daemon` would be a flatter CLI shape, or gate `serve` as an optional flag where absence produces a usage error rather than a no-op.
- The `ui::routes::AppState` currently wraps only `project_root: PathBuf`. By T-053 this will grow to include a runs cache and possibly a notification channel; the design acknowledges this implicitly but does not reserve an extension point (e.g., a `DaemonState` struct with an obvious place for future fields). This is minor for v0.
- The path `GET /gates/:phase` in the daemon is mentioned but the design correctly flags (in Open Questions) that `:run_id` is needed in the path too. This is deferred to T-053.

## 3. Alignment with Handoff & Roadmap

- **Handoff alignment**: The design follows the handoff's "new primitives land as new modules" convention by adding `src/ui/` rather than mutating `src/hitl_server/`. The TDD-first approach is reflected in the concrete test plan with named test functions.
- **Roadmap alignment**: Direct match for Phase 12 / T-052. Correctly depends on T-051 and blocks T-053/T-054/T-055. The scope boundaries match the roadmap's "Slice C" description.
- **Active milestone note**: CLAUDE.md says M7/M8 (Slice B) is active, but T-052 is in Phase 12 (M11). This is expected — the roadmap's "parallel-OK" tasks allow working ahead. No contradiction.

## 4. Security Review

- **Binding**: `127.0.0.1` enforced by default; `--bind` flag allows override. This matches FR-28.
- **No auth in v0**: explicit in non-goals. Acceptable for localhost-only binding.
- **Path traversal**: Run directory names come from filesystem, not user input. `manifest.json` reads are from canonicalised paths.
- **CSRF**: `GET /` is a read-only JSON endpoint — no CSRF concern. POST endpoints not wired in daemon v0.
- **Missing**: The design does not state that the `phase_status` field in `RunSummary` is sanitised against injection. Phase names come from marker files, which are already validated by `PhaseLock::validate_phase_name`, but the design should note this dependency explicitly.

## 5. Implementation Concerns

- The rollout order (§6) is sensible: extract `render_gate_view` first (with regression tests), then add the `ui` module. Both can ship in one PR.
- The `Commands::Ui` match arm needs a concrete `else` branch — currently `serve: false` reaches the arm but does nothing. Consider `anyhow::bail!("expected --serve flag; see loker ui --help")`.
- Test fixtures must use the same `manifest.json` shape as `RunDir::create` but via direct file writes, not via `RunState::load` round-trips. The test plan documents this explicitly.
- Integration tests use port `0` to avoid CI collisions — this is the correct pattern.

## 6. Concurrency & Async

- The daemon's `axum::serve` runs on a single tokio runtime, matching the existing one-shot pattern.
- Graceful shutdown via `tokio::signal::ctrl_c()` + `SIGTERM` (Unix) is correct; the `cfg(unix)` gate is properly documented.
- Run discovery is synchronous (`fs::read_dir` on each `GET /`). For v0 with low run counts this is fine; the open questions section flags the re-scan cost explicitly.
- No shared mutable state between requests — each `GET /` call scans fresh. No locking needed. This is the simplest correct approach for v0.

## 7. Blind Spots

- **CLI UX for no-flag invocation**: `loker ui` (without `--serve`) currently has no defined behaviour. Should produce a usage error, not a silent no-op.
- **Stderr vs stdout for the "listening on" message**: The daemon prints to stderr (§4.4), which is correct for a daemon (stdout may be piped), but not explicitly documented as a design decision.
- **`project_root` resolution failure**: If `find_project_root()` returns `None` (no `lok.toml` found), the design says to "refuse to start" but the exact error message and exit code are not specified.
- **Manifest schema version evolution**: If a future loker version bumps the manifest schema, the daemon's `load_run_summary` will fail to parse old runs. This is acceptable for v0 but should be noted as a compatibility consideration.
- **Port 0 for integration tests requires port discovery**: The test helper must expose the actually-bound port. The design mentions this but does not specify the helper function signature.

## 8. Verdict

**APPROVE_WITH_SUGGESTIONS**

The design is thorough, well-scoped, and technically sound. The concerns identified are all minor refinements (CLI UX, documentation gaps) rather than architectural flaws. The chosen approach (separate `src/ui/` module + route extraction) is the correct one.

## 9. Actionable Feedback

1. **[Medium] Define `loker ui` no-flag behaviour**: Either emit a usage error or make `--serve` the default behaviour (removing the required flag). A silent no-op is poor UX.
2. **[Low] Document the stderr convention**: Add a sentence to §4.4 explaining why the "listening on" message goes to stderr (daemon convention, stdout may be piped).
3. **[Low] Specify `find_project_root()` failure exit**: Document the error message format and exit code when no `lok.toml` is found.
4. **[Low] Add security note about phase-name sanitisation**: In §4 (security), note that phase names in `RunSummary` come from marker files already validated by `PhaseLock::validate_phase_name`.
5. **[Low] Specify integration test port-discovery helper**: Add the function signature `async fn spawn_daemon(project_root: PathBuf) -> (JoinHandle<()>, SocketAddr)` for reuse in integration tests.
6. **[Minor] Reserve `DaemonState` extension point**: Add a comment in `routes.rs` noting that `AppState` will grow in T-053.
