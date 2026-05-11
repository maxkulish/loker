//! Integration test: manifest kind matches artefact filename per phase.
//!
//! Covers the bug fixed in CLO-358 where every markdown artefact was
//! hardcoded to `kind: "design.md"` regardless of actual filename.
//!
//! These tests drive `PhaseConfig` through the public `build_phase_config`
//! pipeline (parsing a real grammar TOML), so a regression that hardcodes
//! `artefact_kind` would resurface here. Earlier versions of this file
//! constructed `PhaseConfig` via `PhaseConfig::single` and overwrote
//! `artefact_kind` after the fact, which masked exactly the bug being
//! guarded against.

use std::collections::HashMap;
use std::path::PathBuf;

use loker::manifest::{Kind, Manifest};
use loker::phase_runner::persist::commit_success;
use loker::workflow::grammar::Workflow;
use loker::workflow::phase_bridge::build_phase_config;
use uuid::Uuid;

fn build_cfg_for(phase_name: &str, output: &str) -> loker::phase_runner::PhaseConfig {
    let toml = format!(
        r#"
name = "kind-per-phase"
[[phases]]
name = "{phase_name}"
strategy = {{ single = {{}} }}
backends = ["claude/"]
prompt_template = "irrelevant"
inputs = ["spec"]
output = "{output}"
"#
    );
    let wf: Workflow = toml.parse().expect("workflow parses");
    let phase = wf.phases.into_iter().next().expect("one phase");
    let phase_outputs: HashMap<String, (String, PathBuf)> = HashMap::new();
    let vars: HashMap<String, String> = HashMap::new();
    build_phase_config(&phase, &phase_outputs, None, &vars)
        .expect("build_phase_config succeeds for valid grammar")
}

#[test]
fn manifest_kind_matches_filename_for_each_phase() {
    let tmp = tempfile::tempdir().unwrap();
    let run_id = Uuid::nil();

    let phases = vec![
        ("design", "design.md", Kind::DesignMd),
        ("review", "review.md", Kind::ReviewMd),
        ("plan", "plan.md", Kind::PlanMd),
        (
            "synthesis",
            "synthesis.md",
            Kind::OtherMd("synthesis.md".into()),
        ),
    ];

    for (phase, output, expected_kind) in &phases {
        let cfg = build_cfg_for(phase, output);
        // Sanity: build_phase_config itself must compute the right kind.
        assert_eq!(
            cfg.artefact_kind, *expected_kind,
            "build_phase_config produced wrong kind for {output}"
        );

        let (path, entry) = commit_success(tmp.path(), &cfg, b"hello", 0, run_id).unwrap();
        assert_eq!(entry.name, *output);
        assert_eq!(entry.kind, *expected_kind);
        assert!(path.exists());
    }

    let manifest = Manifest::load(&tmp.path().join("manifest.json")).unwrap();
    assert_eq!(manifest.entries.len(), phases.len());

    for (expected_name, expected_kind) in phases.iter().map(|(_, n, k)| (*n, k.clone())) {
        let entry = manifest
            .entries
            .iter()
            .find(|e| e.name == expected_name)
            .unwrap_or_else(|| panic!("missing manifest entry for {}", expected_name));
        assert_eq!(
            entry.kind, expected_kind,
            "kind mismatch for {}: got {:?}, expected {:?}",
            expected_name, entry.kind, expected_kind
        );
    }
}

#[test]
fn build_phase_config_kind_from_grammar_filename() {
    // Drives the full bridge: grammar TOML -> Phase -> PhaseConfig. A regression
    // that ignored `phase.output` and re-hardcoded `artefact_kind` would fail here.
    for (phase_name, output, expected_kind) in [
        ("design", "design.md", Kind::DesignMd),
        ("plan", "plan.md", Kind::PlanMd),
        ("review", "review.md", Kind::ReviewMd),
        (
            "analysis",
            "analysis.md",
            Kind::OtherMd("analysis.md".into()),
        ),
    ] {
        let cfg = build_cfg_for(phase_name, output);
        assert_eq!(cfg.artefact_name, output);
        assert_eq!(cfg.artefact_kind, expected_kind);
    }
}
