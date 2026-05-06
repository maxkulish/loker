# Pre-PR validation: clo-318

**Reviewer**: Claude (fallback)
**Reviewed**: 2026-05-06
**Pipeline**: lok implement-gate
**Note**: Both external reviewers failed; this is the fallback review
---

## Findings

### F1 [low] Negative pending fixture no longer matches its filename
**Where:** tests/fixtures/schemas/pending/negative/low_severity_null_timeout.json:1
**What:** The harness convention (tests/schema_validation.rs:90) is "encode exactly one violation, name the file after it." Since the schema now permits `timeout_at: null` for any severity, this fixture was repurposed to violate `decision_options: minItems: 1` (it sets `decision_options: []`), but kept its old name. Future readers will be misled about what the fixture proves.
**Suggested fix:** Rename to `empty_decision_options.json` (or similar) and update the `prompt_summary` to describe the actual violation. No code change needed.

### F2 [info] `#[allow(clippy::large_enum_variant)]` on `VerifyHookName`
**Where:** src/phase_runner.rs (VerifyHookName declaration site)
**What:** `HumanVerifierConfig` grew to include `HumanTimeoutPolicy` (three `HumanTimeoutRule`s), pushing `VerifyHookName::HumanVerifier(HumanVerifierConfig)` past clippy's large-variant threshold. The allow-attribute is defensible because the enum is small in practice, but the size delta will compound if another variant grows.
**Suggested fix:** Optional - box the payload (`HumanVerifier(Box<HumanVerifierConfig>)`) to drop the lint suppression. Not blocking.

### F3 [info] HITL hook is built twice per phase
**Where:** src/phase_runner.rs (HumanVerifier dispatch branch)
**What:** `dispatch::resolve_verify_hook` already constructs a `HumanVerifier` hook, but the HITL branch discards it and calls `HumanVerifier::new(human_cfg.clone())` again so it can use `verify_with_report`. Minor wasted work; not a correctness issue.
**Suggested fix:** Have `resolve_verify_hook` return an enum (or expose the concrete `HumanVerifier`) so the runner can downcast/match without rebuilding. Defer until a second hook needs the same treatment.

### F4 [info] Malformed-response path drops timeout context
**Where:** src/strategy/verify/human_verifier.rs (malformed-response branch in `verify_with_report`)
**What:** When the persisted response fails to parse, the verifier writes a fresh pending and returns Fail with `default_report()` (severity from config, `NotTimedOut`, no `timeout_at`). If the gate had already timed out at the moment of the malformed read, the trace/marker will not reflect that. Matches the design's "do not silently auto-approve on broken audit state" stance, but consumers lose the deadline signal.
**Suggested fix:** Optional - thread the rule's `timeout_at` and recompute `timed_out` in this branch too, so the report still carries the deadline even when outcome is `NotTimedOut` due to forced re-pending. Non-blocking.

## Verdict
approve_with_changes

The implementation matches `docs/designs/clo-318-severity-ladder.md` end-to-end: severity-driven timeout ladder with overrides, fake-clock seam, pending file as the deadline source of truth, marker `HitlMarkerContext`, four `loker.hitl.*` trace fields, and schema relaxations are all wired correctly. `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --tests` are all green; new integration tests cover the trace, marker, and timeout-decision paths called out in the plan's test matrix. The only change worth blocking on is F1 - the negative fixture's filename misrepresents what it proves and will mislead the next person debugging schema validation. F2-F4 are non-blocking observations the team can address opportunistically.
