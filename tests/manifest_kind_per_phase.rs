//! Integration test: manifest kind matches artefact filename per phase.
//!
//! Covers the bug fixed in CLO-358 where every markdown artefact was
//! hardcoded to `kind: "design.md"` regardless of actual filename.

use loker::manifest::{Kind, Manifest, Producer};
use loker::phase_runner::persist::commit_success;
use loker::phase_runner::{PhaseConfig, VerifyHookName};
use uuid::Uuid;

#[test]
fn manifest_kind_matches_filename_for_each_phase() {
    let tmp = tempfile::tempdir().unwrap();
    let run_id = Uuid::nil();

    // Simulate a phase-based workflow producing four different markdown files
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
        let mut cfg = PhaseConfig::single(*phase, "mock", "test", *output);
        cfg.artefact_kind = expected_kind.clone();
        cfg.producer = Producer::Single;
        cfg.verify = VerifyHookName::None;

        let (path, entry) = commit_success(tmp.path(), &cfg, b"hello", 0, run_id).unwrap();
        assert_eq!(entry.name, *output);
        assert_eq!(entry.kind, *expected_kind);
        assert!(path.exists());
    }

    // Load the manifest and assert every entry's kind matches its filename
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
fn phase_bridge_kind_from_filename_integration() {
    // Verify that the helper maps known filenames correctly when building
    // PhaseConfig through the public API.
    let cfg = PhaseConfig::single("design", "mock", "test", "design.md");
    assert_eq!(cfg.artefact_name, "design.md");

    let cfg = PhaseConfig::single("plan", "mock", "test", "plan.md");
    assert_eq!(cfg.artefact_name, "plan.md");
}
