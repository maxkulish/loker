# Pre-PR validation: clo-296

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-04
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

`make check` passes; tests green; clippy warnings are pre-existing. Now I have everything I need.

## Findings

### F1 [HIGH] Phase status serializes as `"completed"` but schema requires `"success"`
**Where:** `src/summary/mod.rs:29-35` and `docs/schemas/summary.schema.json:88`
**What:** `PhaseStatus` uses `#[serde(rename_all = "snake_case")]` and emits `"completed"`, but `summary.schema.json` enum is `["success", "failed", "skipped"]` (matching the existing positive fixtures `tests/fixtures/schemas/summary/positive/*.json`). Writer output will fail schema validation. Plan ST6 explicitly required programmatic `jsonschema` validation of writer output — that test is missing, so the mismatch slipped through. The design doc text says "completed/failed/skipped" but the canonical schema says otherwise; schema wins.
**Suggested fix:** Either rename the variant via `#[serde(rename = "success")]` on `Completed` (and update tests), or change the schema + fixtures to `"completed"`. Then add a writer-output `jsonschema::validator_for` assertion in `tests/summary.rs` so this can't regress.

### F2 [HIGH] Manifest is not idempotent on re-finalize — duplicate entries accumulate
**Where:** `src/summary/mod.rs:222-231` (`manifest.append(...)`)
**What:** Plan ST4 requires "idempotent on re-finalize (overwrites existing summary.json + updates manifest sha256 in-place)". The implementation calls `manifest.append`, so each re-run pushes a new `Kind::SummaryJson` entry. `tests/summary.rs::budget_exceeded_warning` actually calls `write_summary` three times against the same manifest — but never asserts entry count, so the bug is invisible. Real consumers reading the manifest will see N duplicate summary.json rows after every resume.
**Suggested fix:** Add a `Manifest::replace_or_append(kind, entry)` (or guard `append` with a "remove existing of same path" pass), call it instead of `append`, and add an assertion to the budget test (`assert_eq!(manifest entries with kind=SummaryJson, 1)` after the third call).

### F3 [HIGH] `prices.toml` lookup is fragile and unreliable in production
**Where:** `src/summary/mod.rs:139-148`
**What:** Path resolution probes `run_dir/../../docs/prices.toml`, `../docs/prices.toml`, `run_dir/docs/prices.toml`, then CWD `docs/prices.toml`. This conflates a repo-relative location with a runtime-relative one. In any deployed install (`/usr/local/bin/loker` per `make release`) none of those paths exist, so every production run silently emits `cost_usd: None` for every backend — defeating the entire feature when used outside the source tree.
**Suggested fix:** Pick a deterministic resolution: bake the table at compile time via `include_str!("../../docs/prices.toml")` parsed once, OR resolve from a config-driven path (`lok.toml prices_table = "..."`) with a documented default under `~/.config/loker/`. Add a test that calls `write_summary` from a different CWD (`std::env::set_current_dir`) and proves cost is still computed.

### F4 [MEDIUM] `duration_ms` and `started_at`/`finished_at` are placeholders
**Where:** `src/summary/mod.rs:174-177, 371-393`
**What:** `compute_duration_ms` always returns `0`. `get_run_start_time` reads the `markers/` directory mtime (not "earliest started marker" as design specifies, line 321 of design). `finished_at` is set to `Utc::now()`. So summaries for replays/resumes will always show `duration_ms: 0` and a "now" finished_at — both misleading for downstream cost-tracking. Schema only requires `>= 0`, so it validates, but the data is wrong.
**Suggested fix:** Pull timestamps from trace.jsonl (first/last event `time_unix_nano` or equivalent) which is the design's recommended source, and compute `duration_ms = finished - started`. If trace is empty, fall back to marker mtimes — but read individual `*.started`/`*.completed` files, not the parent dir.

### F5 [MEDIUM] Phase `attempts=3` for `.failed` is a hardcoded lie
**Where:** `src/summary/mod.rs:361`
**What:** `parse_marker_name` returns `attempts = 3` for any `.failed` marker, regardless of the real retry count recorded in the marker JSON. This will mis-report budget overruns and post-mortem stats. The comment admits it ("Placeholder: actual count from marker content").
**Suggested fix:** Open the marker file and parse `attempts` (or equivalent field) from its JSON content; default to 1 if absent, not 3. Add a test fixture with `{"attempts": 5}` and assert it round-trips.

### F6 [LOW] Design's `cost_unknown` flag is missing
**Where:** `src/summary/reader.rs:13-21`, `src/summary/mod.rs:68-84`
**What:** Design §Goals says "missing entries produce `cost_unknown: true` per-backend". The struct only sets `cost_usd: None` and relies on `skip_serializing_if`. Downstream consumers can't distinguish "no price entry" from "price entry of 0" or "field omitted in older schema version". The schema also lacks the field.
**Suggested fix:** Either drop the requirement from the design (and note it as deferred), or add `#[serde(skip_serializing_if = "is_false")] pub cost_unknown: bool` to `BackendUsage` and update the schema. Pick one and reconcile design ↔ schema ↔ code.

### F7 [LOW] `loker.run_id` is not validated as UUID
**Where:** `src/summary/mod.rs:198`, tests use `"single-phase-test"` etc.
**What:** Schema declares `"format": "uuid"` for `loker.run_id`. The writer copies whatever string is in `manifest.run_id`. Tests use non-UUID identifiers like `"budget-test"`. With strict format validation enabled this fails; with lenient validation (jsonschema crate default) it passes. Still: the tests should use real UUIDs to exercise the production happy path.
**Suggested fix:** In tests, use `Manifest::new(&uuid::Uuid::new_v4().to_string())`. No code change needed.

### F8 [LOW] `SummaryWriter.fsync` field is dead
**Where:** `src/summary/mod.rs:111, 242-244`
**What:** `fsync` is stored but never read; a comment acknowledges `atomic_write` already does fsync. Dead state on a public-ish API.
**Suggested fix:** Drop the field and the `new(fsync: bool)` parameter, or actually use it to gate the fsync. Removing is simpler and matches the comment.

### F9 [INFO] Scope vs. active milestone
**Where:** `CLAUDE.md:3`, this branch
**What:** Active milestone is M4 (Verify hooks; CLO-271/CLO-273 open). CLO-296 is M6-adjacent ("hard dependency for the M6 end-to-end reference workflow" per design §Problem). Not wrong — but worth confirming with the user that M4 isn't being neglected by an out-of-order ship.
**Suggested fix:** None on the code; just confirm with the user that this PR's order is intentional before merging.

## Verdict
**rework**

The five integration tests pass and `make check` is clean, so the surface looks good — but two production bugs (F1 schema mismatch on phase status, F2 non-idempotent manifest) directly violate stated acceptance criteria from the plan, and F3 makes the cost feature silently degrade in any deployed install. F4/F5 ship factually wrong data (zero durations, fake attempt counts). The first three are blockers; F4–F8 are smaller but cluster into "v0 stub left in critical paths". Add the missing programmatic schema validation that ST6 required and most of these would have been caught before merge. Recommend rework: fix F1-F3, address F4/F5 even if minimally (read attempts from marker JSON, derive timestamps from trace), then re-review.
