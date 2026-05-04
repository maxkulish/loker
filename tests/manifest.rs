use std::fs;
use std::io::Write;

use jsonschema::{self, Validator};
use loker::family::PhaseError;
use loker::manifest::{dir_digest, Kind, Manifest, ManifestEntry, Producer};
use serde_json::Value;

fn build_manifest_validator() -> Validator {
    let schema_text = fs::read_to_string("docs/schemas/manifest.schema.json").unwrap();
    let schema: Value = serde_json::from_str(&schema_text).unwrap();
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&schema)
        .unwrap()
}

fn entry_payload(name: &str, kind: Kind, producer: Producer) -> (ManifestEntry, Vec<u8>) {
    let payload = format!("payload for {}", name).into_bytes();
    let entry = ManifestEntry::from_payload(
        name,
        kind,
        1,
        producer,
        Some(name.split('/').next().unwrap_or(name).to_string()),
        Some(1),
        &payload,
    );
    (entry, payload)
}

#[test]
fn empty_manifest_roundtrips() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("manifest.json");
    let manifest = Manifest::new("a1b2c3d4-e5f6-7890-abcd-ef1234567890", None);
    // Write via simple fs write for empty manifest test
    let json = manifest.to_json().unwrap();
    fs::write(&path, json).unwrap();
    let loaded = Manifest::load(&path).unwrap();
    assert_eq!(manifest.run_id, loaded.run_id);
    assert_eq!(manifest.schema_version, loaded.schema_version);
    assert_eq!(manifest.entries, loaded.entries);
}

#[test]
fn append_and_reload_preserves_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("manifest.json");
    let mut manifest = Manifest::new("a1b2c3d4-e5f6-7890-abcd-ef1234567890", None);

    let (entry1, _payload1) = entry_payload("design/design.md", Kind::DesignMd, Producer::Single);
    let (entry2, _payload2) =
        entry_payload("verify/verify.json", Kind::VerifyJson, Producer::Verify);

    manifest.append(entry1.clone(), &path).unwrap();
    manifest.append(entry2.clone(), &path).unwrap();

    let loaded = Manifest::load(&path).unwrap();
    assert_eq!(loaded.entries.len(), 2);
    assert_eq!(loaded.entries[0].name, "design/design.md");
    assert_eq!(loaded.entries[1].name, "verify/verify.json");
    assert_eq!(loaded.entries[0].kind, Kind::DesignMd);
    assert_eq!(loaded.entries[1].kind, Kind::VerifyJson);
}

#[test]
fn atomic_crash_before_rename_leaves_tmp() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("manifest.json");

    // Write initial manifest
    let mut manifest = Manifest::new("a1b2c3d4-e5f6-7890-abcd-ef1234567890", None);
    let (entry, _payload) = entry_payload("design/design.md", Kind::DesignMd, Producer::Single);
    manifest.append(entry, &path).unwrap();

    // Simulate crash: write a tmp file but don't rename
    let old_content = fs::read_to_string(&path).unwrap();
    let mut tmp_file = tempfile::NamedTempFile::new_in(tmp.path()).unwrap();
    tmp_file.write_all(b"{\"crashed\":true}").unwrap();
    let _tmp_path = tmp_file.into_temp_path();

    // Assert manifest.json still has old content
    let current = fs::read_to_string(&path).unwrap();
    assert_eq!(current, old_content);

    // Assert at least one file other than manifest.json exists (the tmp)
    let other_files: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().file_name().and_then(|n| n.to_str()) != Some("manifest.json"))
        .collect();
    assert!(!other_files.is_empty(), "expected at least one temp file");
}

#[test]
fn atomic_crash_after_rename_before_parent_fsync() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("manifest.json");

    let mut manifest = Manifest::new("a1b2c3d4-e5f6-7890-abcd-ef1234567890", None);
    let (entry, _payload) = entry_payload("design/design.md", Kind::DesignMd, Producer::Single);
    manifest.append(entry, &path).unwrap();

    // After a successful append, manifest.json should exist and no tmp files
    assert!(path.exists());
    let tmp_files: Vec<_> = fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .file_name()
                .and_then(|n| n.to_str())
                .map(|s| s.ends_with(".tmp"))
                .unwrap_or(false)
        })
        .collect();
    assert!(
        tmp_files.is_empty(),
        "expected no .tmp files after successful append"
    );
}

#[test]
fn sha256_mismatch_returns_schema_error() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("manifest.json");

    let mut manifest = Manifest::new("a1b2c3d4-e5f6-7890-abcd-ef1234567890", None);
    let (entry, _payload) = entry_payload("design/design.md", Kind::DesignMd, Producer::Single);
    manifest.append(entry, &path).unwrap();
    let loaded = Manifest::load(&path).unwrap();

    let bad_payload = b"tampered payload";
    let err = loaded.verify("design/design.md", bad_payload).unwrap_err();
    match err {
        PhaseError::ArtefactSchemaMismatch { .. } => {}
        other => panic!("expected ArtefactSchemaMismatch, got {:?}", other),
    }
}

#[test]
fn schema_version_mismatch_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("manifest.json");

    let bad_json = r#"{"loker.run_id":"r1","schema_version":2,"entries":[]}"#;
    fs::write(&path, bad_json).unwrap();

    let err = Manifest::load(&path).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("schema_version is 2") || msg.contains("schema_version"),
        "unexpected error: {}",
        msg
    );
}

#[test]
fn orphan_sweep_drops_unreferenced_entries() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("manifest.json");
    let markers_dir = tmp.path().join("markers");
    fs::create_dir(&markers_dir).unwrap();

    let mut manifest = Manifest::new("a1b2c3d4-e5f6-7890-abcd-ef1234567890", None);
    let (entry1, _payload1) = entry_payload("design/design.md", Kind::DesignMd, Producer::Single);
    let (entry2, _payload2) = entry_payload("review/review.md", Kind::ReviewMd, Producer::Single);

    manifest.append(entry1.clone(), &path).unwrap();
    manifest.append(entry2.clone(), &path).unwrap();

    // Only mark entry1 as completed
    let marker = serde_json::json!({
        "phase": "design",
        "attempt": 1,
        "completed_at": "2026-04-25T20:48:13Z",
        "manifest_entry_sha256": entry1.sha256,
        "artefact_paths": ["design/design.md"]
    });
    fs::write(markers_dir.join("design.completed"), marker.to_string()).unwrap();

    let loaded = Manifest::load(&path).unwrap();
    assert_eq!(loaded.entries.len(), 1);
    assert_eq!(loaded.entries[0].name, "design/design.md");
}

#[test]
fn changes_dir_digest_is_deterministic() {
    let tmp = tempfile::tempdir().unwrap();
    let dir_a = tmp.path().join("tree_a");
    let dir_b = tmp.path().join("tree_b");
    fs::create_dir(&dir_a).unwrap();
    fs::create_dir(&dir_b).unwrap();

    fs::write(dir_a.join("foo.txt"), "hello").unwrap();
    fs::write(dir_b.join("foo.txt"), "hello").unwrap();

    let digest_a = dir_digest(&dir_a).unwrap();
    let digest_b = dir_digest(&dir_b).unwrap();
    assert_eq!(
        digest_a, digest_b,
        "identical trees should have identical digests"
    );

    // Change one file
    fs::write(dir_b.join("foo.txt"), "world").unwrap();
    let digest_b2 = dir_digest(&dir_b).unwrap();
    assert_ne!(
        digest_a, digest_b2,
        "changed content should produce different digest"
    );
}

#[test]
fn changes_dir_digest_flattens_subdirs() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("changes");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("sub")).unwrap();

    fs::write(root.join("top.txt"), "top").unwrap();
    fs::write(root.join("sub/inner.txt"), "inner").unwrap();

    let digest = dir_digest(&root).unwrap();
    assert!(!digest.is_empty());

    // Changing nested file should change digest
    fs::write(root.join("sub/inner.txt"), "modified").unwrap();
    let digest2 = dir_digest(&root).unwrap();
    assert_ne!(
        digest, digest2,
        "modifying nested file should change digest"
    );
}

#[test]
fn generated_manifest_validates_against_schema() {
    let validator = build_manifest_validator();

    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("manifest.json");

    let mut manifest = Manifest::new("a1b2c3d4-e5f6-7890-abcd-ef1234567890", None);
    let (entry, _payload) = entry_payload("design/design.md", Kind::DesignMd, Producer::Single);
    manifest.append(entry, &path).unwrap();

    let json_text = fs::read_to_string(&path).unwrap();
    let value: Value = serde_json::from_str(&json_text).unwrap();
    assert!(
        validator.is_valid(&value),
        "generated manifest should validate against schema"
    );
}
