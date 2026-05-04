# Pre-PR validation: clo-296

**Reviewer**: Synthesis (Claude)
**Reviewed**: 2026-05-04
**Pipeline**: lok implement-gate
---

## Reviewer Status
| Reviewer | Status | Detail |
|----------|--------|--------|
| Codex | REVIEW_FAILED | Shell heredoc quoting bug in wrapper script (`unexpected EOF while looking for matching '`) — never invoked the model |
| Gemini | REVIEW_FAILED | Same shell heredoc quoting bug — never invoked the model |
| Claude (fallback) | OK | Full review against design + plan; nine findings (3 HIGH, 2 MEDIUM, 3 LOW, 1 INFO) |

Both external reviewers failed due to a tooling/scripting bug in `.pi/agents/codex-pre-pr` and `.pi/agents/gemini-architect` wrappers, not model failures. Synthesis below relies on the Claude fallback only — single-reviewer signal, so weight findings accordingly.

## Verdict
rework

## Must Fix Before PR
- **F1 [HIGH] Phase status enum mismatch.** `PhaseStatus::Completed` serializes to `"completed"` but `docs/schemas/summary.schema.json` enum is `["success","failed","skipped"]`. Writer output fails its own schema. Pick one (rename variant via `#[serde(rename = "success")]` OR update schema + fixtures to `"completed"`) and reconcile design text. **Plan ST6's required `jsonschema::validator_for` writer-output assertion is missing — add it now so this can't regress.**
- **F2 [HIGH] Manifest not idempotent.** `SummaryWriter::write_summary` calls `manifest.append`, so re-finalize/resume accumulates duplicate `Kind::SummaryJson` rows — directly violates plan ST4 ("idempotent on re-finalize"). The budget test calls write three times and asserts nothing about entry count, hiding the bug. Add `Manifest::replace_or_append` (or in-place sha256 update) and assert exactly one summary.json entry after N calls.
- **F3 [HIGH] `prices.toml` resolution broken outside source tree.** Path probe walks repo-relative locations only; in any `/usr/local/bin/loker` install all backends silently emit `cost_usd: null`. Choose one deterministic strategy (compile-time `include_str!`, or config-driven path with default) and add a test that runs `write_summary` from a foreign CWD and asserts cost is computed.
- **F4 [MEDIUM] `duration_ms` always 0; timestamps from dir mtime.** `compute_duration_ms` returns `0` and `get_run_start_time` reads parent `markers/` dir mtime; design §line 321 specifies trace.jsonl as the source. Derive started/finished from trace events (first/last `time_unix_nano`) and compute real duration. Currently every replay/resume reports duration_ms=0 + finished_at=now — useless for the cost-tracking story this task exists to support.
- **F5 [MEDIUM] Hardcoded `attempts = 3` for failed phases.** `parse_marker_name` fabricates the count instead of reading marker JSON. Misreports retry budget. Read `attempts` from marker file content; default 1 if absent.

These five fit a single bounded fix only if the F1 design↔schema reconciliation and the F3 path-resolution strategy are decided up front; both are real choices, not mechanical edits. Combined with the missing programmatic schema validation (ST6) and the test gaps that hid F2, the closest-correct path is **rework** rather than approve_with_changes.

## Out of Scope / Deferred
- **F6 [LOW] `cost_unknown` field.** Design mentions it; struct/schema lack it. Either drop from design or add to schema + struct in a follow-up — not a regression, schema is already silent on it.
- **F7 [LOW] Test fixtures use non-UUID `run_id`.** Schema says `format: uuid`; jsonschema crate default is lenient so it passes. Use `Uuid::new_v4()` in tests when convenient. No production code change.
- **F8 [LOW] Dead `fsync` field on `SummaryWriter`.** `atomic_write` already fsyncs. Remove the param next pass; doesn't affect correctness.
- **F9 [INFO] M4 vs M6 ordering.** CLO-296 ships ahead of open M4 work (CLO-271/CLO-273). Not a code issue — confirm with user that the out-of-order merge is intentional. No blocking.

## False Positives / Tooling Artifacts
- **Codex review failure.** `.pi/agents/` script has an unterminated heredoc — fix the wrapper (likely a `'` inside the persona body inside a single-quoted heredoc). Not a model verdict.
- **Gemini review failure.** Same scripting bug. Same fix.
- No findings were false positives in substance; F1–F5 are verifiable against the schema, manifest API, and source.

## Recommendation
**STOP_FOR_USER** — two reviewer pipelines are broken (heredoc bug in `.pi/agents/codex-pre-pr.md` and `.pi/agents/gemini-architect.md` invocations), so this synthesis is single-source. Independent of that, F1 and F3 require small design decisions (which side wins on the `success`/`completed` enum; how `prices.toml` is located in installed builds) before code fixes are mechanical. Recommend the user (a) repair the review wrappers and re-run, and (b) confirm the two design choices, then return to implement to address F1–F5 in one focused pass with the missing ST6 schema-validation test added.
