use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use jsonschema::Validator;
use serde_json::Value;

const SCHEMA_DIR: &str = "docs/schemas";
const FIXTURE_DIR: &str = "tests/fixtures/schemas";
const SCHEMA_SUFFIX: &str = ".schema.json";

#[test]
fn run_artefact_schemas_validate_their_fixtures() {
    let repo_root = repo_root();
    let schema_dir = repo_root.join(SCHEMA_DIR);
    let fixture_dir = repo_root.join(FIXTURE_DIR);

    let schemas = collect_schemas(&schema_dir);
    assert!(
        !schemas.is_empty(),
        "no schemas found under {}; the validator harness needs at least one *.schema.json to run",
        SCHEMA_DIR
    );

    let mut failures: Vec<String> = Vec::new();
    let mut covered_fixture_dirs: BTreeSet<PathBuf> = BTreeSet::new();

    for schema in &schemas {
        let validator = match build_validator(&schema.path) {
            Ok(v) => v,
            Err(e) => {
                failures.push(format!(
                    "{}: failed to compile schema: {}",
                    rel(&repo_root, &schema.path),
                    e
                ));
                continue;
            }
        };

        let positives = collect_fixtures(&fixture_dir, &schema.basename, "positive");
        let negatives = collect_fixtures(&fixture_dir, &schema.basename, "negative");
        covered_fixture_dirs.insert(fixture_dir.join(&schema.basename));

        if positives.is_empty() {
            failures.push(format!(
                "{}: missing positive fixtures (need at least one under {}/{}/positive/)",
                rel(&repo_root, &schema.path),
                FIXTURE_DIR,
                schema.basename
            ));
        }
        if negatives.is_empty() {
            failures.push(format!(
                "{}: missing negative fixtures (need at least one under {}/{}/negative/)",
                rel(&repo_root, &schema.path),
                FIXTURE_DIR,
                schema.basename
            ));
        }

        for fx in &positives {
            match load_json(fx) {
                Ok(value) => {
                    if !validator.is_valid(&value) {
                        let errs: Vec<String> = validator
                            .iter_errors(&value)
                            .map(|e| format!("{} at {}", e, e.instance_path()))
                            .collect();
                        failures.push(format!(
                            "{}: positive fixture must validate but did not: {}",
                            rel(&repo_root, fx),
                            errs.join("; ")
                        ));
                    }
                }
                Err(e) => failures.push(format!(
                    "{}: positive fixture is not valid JSON: {}",
                    rel(&repo_root, fx),
                    e
                )),
            }
        }

        for fx in &negatives {
            match load_json(fx) {
                Ok(value) => {
                    if validator.is_valid(&value) {
                        failures.push(format!(
                            "{}: negative fixture must fail validation but passed (encode exactly one violation, name the file after it)",
                            rel(&repo_root, fx)
                        ));
                    }
                }
                Err(e) => failures.push(format!(
                    "{}: negative fixture is not valid JSON: {}",
                    rel(&repo_root, fx),
                    e
                )),
            }
        }
    }

    if let Ok(entries) = fs::read_dir(&fixture_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if !covered_fixture_dirs.contains(&path) {
                failures.push(format!(
                    "{}: fixture directory has no matching schema at {}/{}{}",
                    rel(&repo_root, &path),
                    SCHEMA_DIR,
                    path.file_name().unwrap().to_string_lossy(),
                    SCHEMA_SUFFIX
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "schema validation failed:\n  - {}",
        failures.join("\n  - ")
    );
}

struct SchemaEntry {
    basename: String,
    path: PathBuf,
}

fn collect_schemas(dir: &Path) -> Vec<SchemaEntry> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if let Some(stem) = name.strip_suffix(SCHEMA_SUFFIX) {
            out.push(SchemaEntry {
                basename: stem.to_string(),
                path: path.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.basename.cmp(&b.basename));
    out
}

fn collect_fixtures(fixture_dir: &Path, basename: &str, kind: &str) -> Vec<PathBuf> {
    let dir = fixture_dir.join(basename).join(kind);
    let mut out = Vec::new();
    let entries = match fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn build_validator(schema_path: &Path) -> Result<Validator, String> {
    let schema = load_json(schema_path).map_err(|e| e.to_string())?;
    jsonschema::options()
        .with_draft(jsonschema::Draft::Draft202012)
        .build(&schema)
        .map_err(|e| e.to_string())
}

fn load_json(path: &Path) -> Result<Value, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    serde_json::from_str(&text).map_err(|e| format!("parse {}: {}", path.display(), e))
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
