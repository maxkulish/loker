Ripgrep is not available. Falling back to GrepTool.
Error executing tool run_shell_command: Tool "run_shell_command" not found. Did you mean one of: "update_topic", "grep_search", "invoke_agent"?
# Gemini design / implementation review - CLO-285

## Context
- Branch: HEAD
- Design: `docs/designs/clo-285-manifest-load.md`
- Plan / Spec: Issue `CLO-285`

## Findings

### F1 [major] Silent swallow of IO/JSON errors in `.completed` marker parsing
**Where:** `src/run_state/load.rs`:149-152
**What:** The `.completed` file parsing uses nested `if let Ok(...)` to ignore `read_to_string` and `serde_json::from_str` failures.
**Why it matters:** If a `.completed` marker is unreadable (e.g., due to a corrupted file, invalid JSON, or temporary IO lock), `has_completed_markers` is still set to `true`, but the target SHA is silently dropped. This causes `orphan_sweep` to incorrectly sweep and permanently drop perfectly valid manifest entries, resulting in accidental data loss.
**Suggested fix:** Propagate the error using `?` (e.g., `let text = std::fs::read_to_string(&path)?;` and `let marker: CompletedMarker = serde_json::from_str(&text)?;`), mapping to `LoadError::Io` or `LoadError::Json` respectively.

### F2 [minor] `Heartbeat` struct is missing from public re-exports
**Where:** `src/run_state/mod.rs`
**What:** The module re-exports `HeartbeatStatus`, `LoadError`, `PhaseStatus`, and `RunState` but forgets to re-export `Heartbeat`.
**Why it matters:** `HeartbeatStatus::Live` wraps `Heartbeat`. While technically accessible via `loker::run_state::load::Heartbeat`, failing to flatten it alongside the enum breaks API ergonomics and the API surface defined in the design doc.
**Suggested fix:** Add `Heartbeat` to the `pub use` list in `src/run_state/mod.rs`.

### F3 [minor] Redundant `HeartbeatStatus::Missing` variant
**Where:** `src/run_state/load.rs`
**What:** `RunState` defines `pub heartbeat: Option<HeartbeatStatus>`, but `HeartbeatStatus` itself has a `Missing` variant. The loader code produces `None` when the heartbeat is missing and never populates `Some(HeartbeatStatus::Missing)`.
**Why it matters:** API consumers face an ambiguous choice between `None` and `Some(HeartbeatStatus::Missing)`, leaving the `Missing` variant as dead code.
**Suggested fix:** Either change the field to `pub heartbeat: HeartbeatStatus` (and return `Missing` instead of `None`), or remove the `Missing` variant from the enum entirely.

### F4 [minor] Unused error variants `StaleWriter` and `LiveWriter`
**Where:** `src/run_state/load.rs`:44-52
**What:** `LoadError` contains variants for `StaleWriter` and `LiveWriter` that are never instantiated.
**Why it matters:** While the design doc incorrectly requested *both* the error variants and the `RunState` struct field, the implementation correctly favored returning the state on the struct but left the unused error variants as confusing dead code.
**Suggested fix:** Remove `StaleWriter` and `LiveWriter` from the `LoadError` enum.

### F5 [nit] Inline logic duplicates `status_from_heartbeat`
**Where:** `src/run_state/load.rs`:85-92
**What:** The logic for calculating heartbeat TTL is written inline inside `RunState::load`, leaving the standalone `status_from_heartbeat` function as dead code.
**Why it matters:** Results in unnecessary WET code.
**Suggested fix:** Call `Self::status_from_heartbeat(&hb, heartbeat_ttl_seconds)` directly inside the `.map()` closure.

### F6 [nit] Unnecessary clone of `phase` string
**Where:** `src/run_state/load.rs`:155
**What:** `update_phase_status(&mut status, phase.clone(), PhaseStatus::Completed);` is immediately followed by a `continue;`.
**Why it matters:** Causes an avoidable allocation. The `phase` variable is not used after the clone.
**Suggested fix:** Pass `phase` directly without calling `.clone()`.

### F7 [nit] Inconsistent path formatting in `LoadError::ArtefactCorrupt`
**Where:** `src/run_state/load.rs`:183 and 199
**What:** When a `ChangesDir` validation fails, the error is populated with `entry.name.clone()`. When a standard file validation fails, it uses `path.display().to_string()`.
**Why it matters:** Creates inconsistent error string formatting for consumers logging or matching on the path.
**Suggested fix:** Consistently use `path.display().to_string()` for both cases.

### F8 [nit] Missing `WARN` prefix and `TODO` comment in orphan logging
**Where:** `src/run_state/load.rs`:222-225
**What:** The `eprintln!` does not include a `WARN:` prefix and lacks the required `TODO` comment for replacing it with a future logger.
**Why it matters:** Deviates from the strict logging requirement in the design doc.
**Suggested fix:** Prepend `WARN: ` to the logged string and add the inline `// TODO: replace with trace logger` comment.

## Strengths
- Excellent test coverage that accurately maps all cases specified in the design document.
- The marker rank precedence algorithm correctly handles mixed-state phase marker scenarios safely.
- Clean and consistent usage of `serde` mappings for the marker file reading logic.

## Verdict
approve_with_changes

The loader correctly matches the architecture and gracefully achieves the tricky requirement of skipping the orphan sweep when no completed markers are present. However, the silent swallowing of `.completed` marker read/parse errors is a major correctness flaw that could lead to data loss during a resume operation if a marker file is temporarily locked or corrupted. Resolving this, alongside cleaning up the API ergonomics around `HeartbeatStatus` and `LoadError`, will make this ready to merge.
