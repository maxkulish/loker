//! TDD test contract for the canonical `design-doc-tdd` workflow file.
#![cfg(test)]

use loker::workflow::grammar::{Strategy, Workflow};
use std::collections::HashSet;

/// Canonical workflow TOML string.
fn canonical_toml() -> &'static str {
    include_str!("../.lok/workflows/design-doc-tdd.toml")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Split a backend string into scheme/fn parts for family detection.
fn backend_family(raw: &str) -> String {
    raw.split('/').next().unwrap_or(raw).to_string()
}

/// Apply family-of detection over raw backend strings.
/// Returns how many distinct families are present.
fn count_families(backends: &[String]) -> usize {
    let families: HashSet<_> = backends.iter().map(|s| backend_family(s)).collect();
    families.len()
}

// ---------------------------------------------------------------------------
// ST1: Author the canonical file (implicitly validated by every test below)
// ---------------------------------------------------------------------------

/// ST1-1: `Workflow::from_str` returns Ok; `lint()` produces no errors.
#[test]
fn roundtrip_parses_clean() {
    let wf: Workflow = canonical_toml()
        .parse()
        .expect("canonical design-doc-tdd.toml should parse cleanly — check grammar");
    assert_eq!(wf.name, "design-doc-tdd");
    let warnings = wf.lint();
    // Contract reservation warnings are expected (FR-31 forward-compat).
    // Only fail on unexpected warnings.
    for w in &warnings {
        assert!(
            w.contains("phase.contract reserved for post-v0"),
            "unexpected lint warning: {w}"
        );
    }
}

/// ST1-2: phase names and order match PRD §UC-1.
#[test]
fn phase_names_and_order_match_prd() {
    let wf: Workflow = canonical_toml().parse().unwrap();
    let names: Vec<_> = wf.phases.iter().map(|p| p.name.clone()).collect();
    assert_eq!(
        names,
        ["design", "review", "implement", "verify"],
        "phase names and order must match PRD §UC-1"
    );
}

/// ST1-3: strategy variants match PRD §UC-1.
#[test]
fn phase_strategies_match_prd() {
    let wf: Workflow = canonical_toml().parse().unwrap();
    assert_eq!(wf.phases.len(), 4, "all 4 phases must be present");

    assert_eq!(
        wf.phases[0].strategy,
        Strategy::Single,
        "design phase must use single strategy"
    );
    assert_eq!(
        wf.phases[1].strategy,
        Strategy::ParallelFanOut { min_responses: 2 },
        "review phase must use parallel(min_responses=2)"
    );
    assert_eq!(
        wf.phases[2].strategy,
        Strategy::EscalatingRetry {
            pass_failure_context: true
        },
        "implement phase must use escalating(pass_failure_context=true)"
    );
    assert_eq!(
        wf.phases[3].strategy,
        Strategy::ParallelFanOut { min_responses: 1 },
        "verify phase must use parallel(min_responses=1)"
    );
}

// ---------------------------------------------------------------------------
// ST2: Hook wiring (gated — grammar lacks `hooks` field)
// ---------------------------------------------------------------------------

/// ST2-1: `implement` phase has a forward-compat `[phase.contract]` block
/// with a `test_runner` hook comment.
/// SKIP: grammar lacks `hooks` field. Follow-up: extend Phase struct.
#[test]
#[ignore = "grammar Phase lacks hooks field — follow-up to add hooks support"]
fn implement_phase_has_test_runner_hook() {
    let wf: Workflow = canonical_toml().parse().unwrap();
    let implement = wf
        .phases
        .iter()
        .find(|p| p.name == "implement")
        .expect("implement phase must exist");

    let contract = implement
        .contract
        .as_ref()
        .expect("implement phase must have a contract block (FR-31 forward-compat)");
    assert!(
        contract.get("hooks").is_some(),
        "implement phase contract.hooks must be present"
    );
}

// ---------------------------------------------------------------------------
// ST3: Review phase spans ≥ 2 families (FR-13)
// ---------------------------------------------------------------------------

/// ST3-1: review phase backends span ≥ 2 distinct families.
#[test]
fn review_phase_spans_two_families() {
    let wf: Workflow = canonical_toml().parse().unwrap();
    let review = wf
        .phases
        .iter()
        .find(|p| p.name == "review")
        .expect("review phase must exist");

    let families = count_families(&review.backends);
    assert!(
        families >= 2,
        "review phase must span ≥ 2 families (FR-13), got {families}: {:?}",
        review.backends
    );
}

// ---------------------------------------------------------------------------
// ST4: Implement phase backend ordering (escalating cheap → strong)
// ---------------------------------------------------------------------------

/// ST4-1: implement phase has ≥ 2 backends for escalating retry.
#[test]
fn implement_phase_has_enough_backends() {
    let wf: Workflow = canonical_toml().parse().unwrap();
    let implement = wf
        .phases
        .iter()
        .find(|p| p.name == "implement")
        .expect("implement phase must exist");

    assert!(
        implement.backends.len() >= 2,
        "implement phase must have ≥ 2 backends for escalating retry, got {}",
        implement.backends.len()
    );
}

// ---------------------------------------------------------------------------
// ST5: Input topological validity
// ---------------------------------------------------------------------------

/// ST5-1: all `phase:X` refs name a phase that appears earlier in the list.
#[test]
fn inputs_are_topologically_valid() {
    let wf: Workflow = canonical_toml().parse().unwrap();
    let phase_indices: std::collections::HashMap<_, _> = wf
        .phases
        .iter()
        .enumerate()
        .map(|(i, p)| (p.name.as_str(), i))
        .collect();

    for phase in &wf.phases {
        for input in &phase.inputs {
            if input.starts_with("phase:") {
                let target = input.strip_prefix("phase:").unwrap();
                let target_idx = *phase_indices.get(target).unwrap_or(&usize::MAX);
                let phase_idx = *phase_indices.get(phase.name.as_str()).unwrap();
                assert!(
                    target_idx < phase_idx,
                    "phase '{}' references phase '{}' which is not earlier (forward ref)",
                    phase.name,
                    target
                );
            }
            // spec and var: prefixes are accepted (no validation needed)
            assert!(
                input == "spec" || input.starts_with("phase:") || input.starts_with("var:"),
                "input '{}' in phase '{}' has invalid prefix",
                input,
                phase.name
            );
        }
    }
}

// ---------------------------------------------------------------------------
// ST6: Forward-compat contract block (FR-31)
// ---------------------------------------------------------------------------

/// ST6-1: appending a `[phases.contract]` sub-table to an existing phase
/// must parse and `lint()` must return the FR-31 reservation warning.
#[test]
fn forward_compat_phase_contract_block_parses() {
    let base = canonical_toml();
    // Append a contract sub-table to the design phase (which exists in canonical_toml).
    // TOML lets you redefine a table header; the new fields are merged in.
    let with_contract = format!(
        "{}\n\n[phases.contract]\nrequired_capability = \"file_edit\"\ntimeout_ms = 30000",
        base
    );
    let wf: Workflow = with_contract
        .parse()
        .expect("workflow with phase.contract must parse — FR-31 forward-compat");
    let warnings = wf.lint();
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("phase.contract reserved for post-v0")),
        "lint must emit FR-31 reservation warning"
    );
}

// ---------------------------------------------------------------------------
// ST7: Verify phase inputs
// ---------------------------------------------------------------------------

/// ST7-1: verify phase inputs reference only prior phases.
#[test]
fn verify_phase_inputs_reference_implement() {
    let wf: Workflow = canonical_toml().parse().unwrap();
    let verify = wf
        .phases
        .iter()
        .find(|p| p.name == "verify")
        .expect("verify phase must exist");

    assert!(
        verify.inputs.iter().any(|i| i == "phase:implement"),
        "verify phase must have an input from phase:implement"
    );
}
