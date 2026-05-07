# Pre-PR validation: clo-321

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-07
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [HIGH] Manifest field name mismatch — `run_id` always returns null in production
**Where:** src/ui/discovery.rs:111-114
**What:** `discovery.rs` reads `manifest.get("run_id")`, but `src/manifest.rs:86-87` declares `#[serde(rename = "loker.run_id")] pub run_id: String`. Real `manifest.json` files on disk store the key as `"loker.run_id"` (verified against `runs/.../manifest.json`). Every production run will report `run_id: null` from `GET /`. Tests pass only because fixtures hand-write the wrong key.
**Suggested fix:** Read `manifest.get("loker.run_id")` (and update all fixtures: discovery.rs:223, routes.rs:57, serve.rs:118, tests/ui_daemon.rs:56). Better: deserialize into the real `crate::manifest::Manifest` struct so the schema is authoritative.

### F2 [HIGH] `created_at` is not in the manifest schema — always null
**Where:** src/ui/discovery.rs:117-122
**What:** Discovery walks `created_at` → `created` → `timestamp`, none of which exist on `Manifest` (src/manifest.rs:83-93 has only `run_id`, `schema_version`, `workflow_name`, `entries`). A real manifest.json has no top-level timestamp at all (confirmed against on-disk runs). The summary's `created_at` will always be `None`. Tests pass because fixtures inject a fictitious `created_at` field.
**Suggested fix:** Either drop `created_at` from `RunSummary` for v0 (and from the design), or derive it from filesystem mtime of the run directory / from a `ManifestEntry` with `phase: None`. Update fixtures to stop pretending the field exists.

### F3 [MED] Stray insta pending snapshot file committed
**Where:** tests/.explain_cli.rs.pending-snap
**What:** A `.pending-snap` file (insta's transient diff log) was committed. These are normally gitignored. Inspecting it shows it captured an old build warning ("unused import: std::net::SocketAddr") generated during ST development.
**Suggested fix:** `git rm tests/.explain_cli.rs.pending-snap`; ensure `*.pending-snap` is in `.gitignore`.

### F4 [MED] Test fixtures encode a fictional manifest schema, hiding F1/F2
**Where:** src/ui/discovery.rs:222-231, src/ui/routes.rs:54-65, src/ui/serve.rs:117-128, tests/ui_daemon.rs:55-66
**What:** All four test sites build manifest JSON by hand with `"run_id"` and `"created_at"` keys that contradict the real schema. The Manifest struct uses `#[serde(deny_unknown_fields)]`, so attempting to round-trip these fixtures through `Manifest::from_json` would fail. Tests therefore validate the wrong contract.
**Suggested fix:** Use `Manifest::new(...)` + `serde_json::to_string` (or a shared `make_manifest` helper) so fixtures track schema changes. This will surface F1/F2 immediately.

### F5 [MED] Dead branch in `classify_marker`
**Where:** src/ui/discovery.rs:191-195
**What:** After `name.strip_suffix(".started")` is checked at line 188, the same call at line 191 is unreachable; its comment ("handle .started.<n>") is misleading because that case is actually handled by the `find(".started.")` branch at line 197.
**Suggested fix:** Delete lines 191-195 and the misleading comment.

### F6 [MED] Design–implementation drift: `render_gate_view` extracted but daemon doesn't use it
**Where:** docs/designs/clo-321-ui.md §3.4 / §4.4 vs src/ui/routes.rs:24-28
**What:** The design states the daemon "composes a separate gate router that mounts at `/gates/:phase`" and lists this as how FR-27 is satisfied. The implemented `ui_routes` mounts only `GET /`. The `render_gate_view` extraction lands but is consumed only by the unchanged one-shot. Plan ST3 acknowledges this defer ("documented extension point only"), but the design doc still claims composition. This is scope drift that future readers/T-053 will trip on.
**Suggested fix:** Update the design doc §3.4 / §4.4 to mark the `/gates/:phase` route as deferred to T-053, or wire a thin gate route now using `render_gate_view`. Don't ship the design and code disagreeing.

### F7 [LOW] `runs_list` masks serialization failures with empty `null`
**Where:** src/ui/routes.rs:31-34
**What:** `serde_json::to_value(runs).unwrap_or_default()` returns `Value::Null` on failure, not `[]`. Clients reading `Vec<RunSummary>` would error on the wrong shape. PathBuf serialization can fail on non-UTF-8 paths (Linux).
**Suggested fix:** Return `Result<Json<Value>, StatusCode>` and emit 500 + log on serialization error, or fall back to `Json(serde_json::Value::Array(vec![]))`.

### F8 [LOW] `daemon_shuts_down_gracefully` test doesn't exercise the shutdown path
**Where:** tests/ui_daemon.rs:167-184
**What:** Test calls `JoinHandle::abort()` instead of sending SIGINT/SIGTERM or completing the shutdown future. It validates only that abort works, not that `shutdown_signal()` is wired correctly. Plan ST6 explicitly required "trigger shutdown signal".
**Suggested fix:** Use a shutdown channel (`tokio::sync::oneshot`) that the test passes into `axum::serve(...).with_graceful_shutdown(...)`, or send a signal via `nix::sys::signal::kill` on Unix.

### F9 [LOW] `spawn_test_daemon` is unused; integration tests duplicate it
**Where:** src/ui/serve.rs:62-87 vs tests/ui_daemon.rs:22-46
**What:** `spawn_test_daemon` is declared `#[cfg(test)]`, which makes it invisible from integration tests (separate crate). The `DaemonFixture` reimplements the same logic. Either the helper should be `pub` (or behind `#[cfg(any(test, feature = "test-helpers"))]`), or it should be deleted.
**Suggested fix:** Delete `spawn_test_daemon` from `serve.rs` (the in-module test next to it can call axum directly), and let `DaemonFixture` be the single source.

### F10 [LOW] No warning when `--bind` is non-localhost
**Where:** src/main.rs:1180-1188 / src/ui/serve.rs:16-29
**What:** The runs JSON includes absolute filesystem paths and may include workflow paths that leak user home directories. There is no auth or TLS. Binding to `0.0.0.0` is allowed silently. Design lists this as a non-goal, but a stderr warning is cheap insurance.
**Suggested fix:** On startup, if the parsed bind address is not loopback, print a `WARN: --bind <addr> exposes run metadata over the network; no auth/TLS in v0`.

## Verdict
**rework**

F1 and F2 are correctness bugs in the daemon's API contract: every real run will return `run_id: null` and `created_at: null` because `discovery.rs` reads keys (`run_id`, `created_at`) that don't match the Manifest schema (`loker.run_id`, no top-level timestamp). The unit and integration tests don't catch this because the fixtures (F4) encode the same fictional schema. Combined with the stray `.pending-snap` file (F3) and the dead `classify_marker` branch (F5), this PR should not merge until fixtures are switched to construct manifests via the real `Manifest` type and the field reads are corrected. The rest (F6–F10) are smaller cleanups that can land in the same fix pass. Pre-existing clippy errors in `src/strategy/verify/human_verifier.rs` are present on `main` and not introduced here.
