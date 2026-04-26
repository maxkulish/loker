# loker - implementation roadmap

Linear task list derived from PRD `docs/prd/2026-04-25-loker.md`. Each task
is the next single-TDD-cycle unit of work: one task -> one TDD doc -> one
implementation pass -> one merge.

## How to use

1. Pick the lowest-numbered task whose dependencies are all green.
2. Open `docs/plans/<task-id>.md` (create if missing) and write the TDD
   doc: failing test contract first, then the implementation outline.
3. Implement, run `make check`, merge. Mark the task green here.
4. Tasks tagged **[parallel-ok]** can be picked up alongside the
   currently active one if context-switching is cheap.

Conventions for the table below:

- **Blocks** lists later task IDs that cannot start until this one is
  green.
- **After** lists prerequisites - all must be green before pickup.
- **PRD** points at the FR / NFR / open question this task closes.

## Phase 0 - Discovery foundations

These four spikes / written artefacts gate later milestones. T-001 must
finish before Phase 1 work hardens; T-002, T-003, T-004 can run alongside
Phase 1 - 4.

**Status (2026-04-26): all four shipped via PRs #3, #4, #5, #6.**

| ID | Linear | Task | After | Blocks | PRD | Notes |
|----|--------|------|-------|--------|-----|-------|
| ~~T-001~~ | CLO-243 done | D1 TensorZero round-trip spike. Stand up local Tier 2, run a `single_model` end-to-end via `genai::ServiceTargetResolver`, capture request/response shapes, family identity, token counts. Land verdict in `docs/spikes/2026-04-25-tensorzero-roundtrip.md`. | - | T-005 onward | §11 D1 | Unblocked T-005. |
| ~~T-002~~ | CLO-244 done | D2 JSON schemas. Author draft-2020-12 schemas for `trace.jsonl`, `manifest.json`, per-phase result files, HITL `pending/<phase>.json` + `responses/<phase>.json`, `summary.json`. Drop fixtures + CI validator. | - | T-024, T-029, T-050 | §11 D2 | [parallel-ok] |
| ~~T-003~~ | CLO-245 done | D3 atomic run-state rules. Pick between `.tmp + rename + status marker` vs. attempt-directory schemes; document the chosen write protocol + fault-injection plan in `docs/run-state.md`. | - | T-024, T-031 | §11 D3 | [parallel-ok] |
| ~~T-004~~ | CLO-246 done | D4 UI threat model. Concrete attacker model (cross-origin tabs, extensions, symlinks, traversal, stale locks) + mitigations + M11 test list in `docs/security/2026-04-25-ui-threat-model.md`. | - | T-050 onward | §11 D4 | [parallel-ok] |

## Phase 1 - HTTP-gateway backend (M1, in flight)

`src/backend/tensorzero.rs` is already scaffolded. T-005 reconciles it
with the spike findings; the rest hardens it to the FR-2 / FR-3 contract.

**Status (2026-04-26): T-005 (CLO-247) + T-007 (CLO-249) shipped via PRs #7 and #8 - T-006/T-008/T-010 unblocked; T-013 (CLO-258) gains its retryability dependency.**

| ID | Linear | Task | After | Blocks | PRD |
|----|--------|------|-------|--------|-----|
| ~~T-005~~ | CLO-247 done | Reconcile in-flight `tensorzero.rs` with D1 findings. Confirm `genai::ServiceTargetResolver` wiring, header set, model-name mapping. | T-001 | T-006 | FR-3 |
| T-006 | CLO-248 | Wiremock unit-test contract per M1 plan: 200, 429, 500, malformed JSON, timeout, auth fail. | T-005 | T-009, Phase 2 | FR-2 |
| ~~T-007~~ | CLO-249 done | Error mapping `genai::Error -> BackendError`. Single source of truth for retryability flags. | T-005 | T-013 | FR-2 |
| T-008 | CLO-250 | Config schema for TensorZero in `src/config.rs` (endpoint, default model, timeout, retry policy). | T-005 | T-034 | FR-2 |
| T-009 | CLO-252 | Opt-in integration test gated by `LOKER_TZ_INTEGRATION=1`. One end-to-end round-trip against local gateway. | T-006 | - | FR-2 |
| T-010 | CLO-251 | `BackendCapabilities` struct: tool-use, streaming, file-edit. Validation rejects backends missing required capability. | T-005 | T-029 | FR-4 |

## Phase 2 - Strategy primitives (M2)

Depends on Phase 1 (`Backend` trait + at least one impl). Parallel with
Phase 3 and 4. Internal order is sequential.

| ID | Linear | Task | After | Blocks | PRD |
|----|--------|------|-------|--------|-----|
| T-011 | CLO-257 | `Strategy::SingleModel`: one backend, one prompt, one response. Mock-backend unit test. | T-006 | T-029 | FR-5 |
| T-012 | CLO-259 | `Strategy::ParallelFanOut` with `min_responses` floor. Surplus failures tolerated. | T-011 | T-029 | FR-6 |
| T-013 | CLO-258 | `Strategy::EscalatingRetry` walker. Stops at first verify pass; exhausts list with structured error. | T-007, T-020 | T-014, T-029 | FR-7 |
| T-014 | CLO-260 | `pass_failure_context` flag on `EscalatingRetry`. Off in v0 default; on in design-doc-tdd. | T-013 | T-035 | FR-8 |

## Phase 3 - Aggregator vocabulary (M3)

Parallel with Phase 2 and 4. T-015 is the load-bearing one - cross-family
enforcement is the loker thesis.

| ID | Task | After | Blocks | PRD |
|----|------|-------|--------|-----|
| T-015 | `family_of(backend_id)` lookup + cross-family runtime check (FR-13). Refusal raises `PhaseError::FamilyOverlap`. Source of truth for family resolution decided here (open question §8). | T-001 | T-017, T-019 | FR-13 |
| T-016 | `Aggregator::Concat`: per-source headings, snapshot test. | - | T-029 | FR-9 |
| T-017 | `Aggregator::LLMJudge`: judge prompt construction, family-overlap test, opt-out via `require_judge_different_family = false`. | T-015 | T-029 | FR-10 |
| T-018 | `Aggregator::AnyFail`: first failure wins on JSON verdict fixtures. | - | T-029 | FR-11 |
| T-019 | `Aggregator::Vote` (Should). Ballot schema + abstentions + tie-breakers (`ClosestToFamily`, `Random`, `FirstResponder`) decided in TDD doc before code. Demote to post-v0 if no concrete first use case lands by M3 start. | T-015 | - | FR-12 |

## Phase 4 - Verify hooks (M4)

Parallel with Phase 2 and 3. T-020 (trait + enum) gates the rest.

| ID | Task | After | Blocks | PRD |
|----|------|-------|--------|-----|
| T-020 | `VerifyHook` trait + `VerifyResult` enum (Pass / Fail concrete; `Repair { suggestion }` and `Score(f32)` reserved variants compile but unused). | - | T-013, T-021, T-022, T-023, T-050 | FR-18 |
| T-021 | `RunCommand` hook with the full sandboxing NFR row (cwd, env allowlist default-deny, wall+cpu timeouts, stdout/stderr byte caps, signal cleanup, network policy, file-mutation expectations, secret redaction). | T-020 | T-029 | FR-14, §5 Security |
| T-022 | `LLMVerifier` hook: yes/no prompt template, mock-backend fixture. | T-020 | T-029 | FR-15 |
| T-023 | `TestRunner` hook: parses `cargo test --message-format=json` and `pytest` JSON output. | T-020, T-021 | T-029 | FR-16 |

## Phase 5 - Run state & artefact manifest

Parallel with Phase 2 - 4. T-024 onward is what FR-21 + FR-23b - FR-23e
depend on; T-002 and T-003 must be green first.

| ID | Task | After | Blocks | PRD |
|----|------|-------|--------|-----|
| T-024 | Manifest writer: append-only `runs/<id>/manifest.json` with name, kind, schema version, sha256, producer backend(s). Crash-safe rewrite. | T-002 | T-029 | FR-23b |
| T-025 | Phase status markers (`phase.started`, `phase.completed`, `phase.failed`) with atomic write/rename protocol from T-003. | T-003 | T-031 | FR-23c, FR-21 |
| T-026 | Manifest-driven artefact load for downstream phases. Schema version mismatch raises `PhaseError::ArtefactSchemaMismatch`. | T-024 | T-029 | FR-23d |
| T-027 | Attempt directories `runs/<id>/attempts/<phase>/<n>/` for failed retries. Canonical artefact path remains the latest successful one. | T-025 | T-031 | FR-23e |

## Phase 6 - Phase runner & trace (M5)

The integration point. Needs Phase 2, 3, 4, 5 all green. Internal order
is sequential.

| ID | Task | After | Blocks | PRD |
|----|------|-------|--------|-----|
| T-028 | `PhaseRunner::run_phase` composing Strategy + Aggregator + VerifyHook. Phase-shape unit test on mock backend. | T-010, T-012, T-014, T-016, T-017, T-018, T-021, T-022, T-023, T-024, T-026 | T-029, T-031 | FR-19 |
| T-029 | `trace.jsonl` writer following OpenTelemetry GenAI semantic conventions; loker custom fields under `loker.*`. Schema-validated against T-002 fixture. | T-028 | T-035, T-045 | FR-20 |
| T-030 | Run directory layout `runs/<workflow>-<timestamp>-<short-uuid>/` + declared phase outputs. Snapshot test on fixture. | T-028 | T-035 | FR-22 |
| T-031 | Resumability via status markers (FR-21). Crash-mid-write fixture triggers rerun; fully-marked run makes zero backend calls. Step-level rerun for `parallel`. | T-025, T-027, T-028 | T-042 | FR-21 |
| T-032 | `summary.json` with per-backend tokens; warns when actual exceeds `cost_budget_usd` from `lok.toml`. Cost data source (open question §8) settled here. | T-028 | T-038 | FR-23, FR-23a |

## Phase 7 - Reference workflow (M6)

Slice A's exit gate. End-to-end green run on the calculator spec is the
deliverable.

| ID | Task | After | Blocks | PRD |
|----|------|-------|--------|-----|
| T-033 | TOML workflow grammar parser. Phase block (`name`, `strategy`, `backend(s)`, `prompt_template`, `inputs`, `output`); backend name resolver (`tensorzero/`, `claude/`, `codex/`, `gemini/`, `ollama/`); `phase.contract` reserved no-op. | T-008 | T-034 | FR-29, FR-30, FR-31 |
| T-034 | `design-doc-tdd` workflow file under `.lok/workflows/design-doc-tdd.toml`. Four phases per UC-1. | T-033 | T-037 | UC-1 |
| T-035 | Prompt templates for each phase. Variable substitution from upstream artefacts. | T-029, T-030, T-034 | T-037 | UC-1 |
| T-036 | Tiny example spec at `examples/specs/calculator.md`. | - | T-037 | KPI table row 1 |
| T-037 | M6 end-to-end integration test on calculator spec. Asserts artefacts, trace shape, exit code. | T-014, T-019 (or its demotion), T-032, T-035, T-036 | Phase 8+ | KPI table row 1 |

## Phase 8 - Deployment recipe (M7)

Parallel with Phase 7 once T-009 is green.

| ID | Task | After | Blocks | PRD |
|----|------|-------|--------|-----|
| T-038 | `deploy/tensorzero/docker-compose.yml` for Tier 2 (gateway + ClickHouse + UI). README snippet. | T-009 | T-040 | M7 |
| T-039 | `loker doctor` extension: TensorZero reachability check (HEALTHY / UNREACHABLE). | T-008 | T-041 | FR-35 |

## Phase 9 - CLI surface (M8)

Sequential within phase.

| ID | Task | After | Blocks | PRD |
|----|------|-------|--------|-----|
| T-040 | `loker run <workflow> [--spec] [--var] [--rerun phase=]`. All flags exercised in integration. | T-037 | T-042, T-043 | FR-32 |
| T-041 | `loker resume <run_id>`. Pause / resume round-trip test. | T-031, T-040 | T-046 | FR-33 |
| T-042 | `loker explain <workflow>`: DAG + per-phase strategy summary. Snapshot test. | T-040 | - | FR-34 |
| T-043 | `loker trace <run_id>` pretty-printer. Snapshot test. | T-029 | - | FR-36 |
| T-044 | `loker ls --blocked` enumerating HITL-pending runs. (Should) | T-050 | - | FR-37 |

## Phase 10 - Documentation (M9)

End of Slice B. Marks v0 of the docs surface.

| ID | Task | After | Blocks | PRD |
|----|------|-------|--------|-----|
| T-045 | README rewrite: thesis, three primitives, install, one-page example. | Phase 9 except T-044 | - | M9 |
| T-046 | One-page tutorial: clone -> first run -> read trace. | T-045 | - | M9 |
| T-047 | `docs/migration-from-lok.md`: every lok knob mapped to its loker equivalent. Deprecation window note. | T-045 | - | M9 |

## Phase 11 - HITL hook (M10)

Slice C entry. Depends on Phase 4 trait + D4 threat model.

| ID | Task | After | Blocks | PRD |
|----|------|-------|--------|-----|
| T-048 | `HumanVerifier` hook scaffold: writes `runs/<id>/pending/<phase>.json`, blocks on `runs/<id>/responses/<phase>.json`, maps decision + comments to `VerifyResult`. | T-002, T-020 | T-049, T-051 | FR-17 |
| T-049 | Severity ladder: low (1h), medium (24h), high (infinite). Timeout-as-fail with fake-clock test. Interaction with `EscalatingRetry` resolved here (open question §8). | T-013, T-048 | T-053 | FR-17 |
| T-050 | First-write-wins per-phase advisory lock (`<phase>.json.lock`) with 60s heartbeat auto-release. | T-004, T-048 | T-051, T-053 | FR-26 |
| T-051 | Per-gate fallback axum server (one-shot, no daemon). Shared route module with M11. | T-049, T-050 | T-053 | FR-27 |

## Phase 12 - UI daemon (M11)

Slice C exit. Threat-model test suite is the gating artefact.

| ID | Task | After | Blocks | PRD |
|----|------|-------|--------|-----|
| T-052 | Daemon mode `loker ui --serve`. Localhost-only bind. Shares 100% of route handlers with T-051. | T-051 | T-053, T-054, T-055 | FR-24, FR-27, FR-28 |
| T-053 | Sessions list (left pane) + per-run trace + pending panel (right pane). Snapshot tests on active / blocked / complete fixtures. | T-052 | T-054 | FR-24 |
| T-054 | SSE tail-f of `trace.jsonl` driven by `notify`. Integration test asserts <200ms delivery. | T-053 | T-055 | FR-25 |
| T-055 | Threat-model test suite per T-004: traversal, symlink, CSRF, stale lock takeover. M11 close gate. | T-004, T-054 | - | §5 Security, §8 Risks |

## Critical-path summary

The minimum sequential chain to ship Slice A (vertical core) is:

```
T-001 -> T-005 -> T-006 -> T-011 -> T-012 -> T-013 -> T-014
                                       \                \
                                        \                T-029 (via T-028)
                                         T-020 -> T-021 -^
T-002, T-003 -> T-024, T-025 -> T-031 ----------^
                                                T-028 -> T-029 -> T-030 -> T-032 -> T-037
```

With T-002, T-003, T-004 picked up in parallel, the wall-clock floor for
Slice A is roughly the longest single chain (Phase 1 -> Phase 6 -> Phase
7), not the sum of all tasks.

## Out-of-roadmap

Anything in PRD §6 "Out of Scope" or "Future Phases" is post-v0 and does
not appear here.
