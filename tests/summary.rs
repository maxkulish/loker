/// Integration tests for the summary.json pipeline.
///
/// Test plan (§5 of CLO-296):
/// 1. `single_phase_success` — 1 phase, 1 backend, token totals match
/// 2. `multi_backend_parallel` — 3 backends, separate rollups per backend/model
/// 3. `failed_phase_emitted` — phase shows `failed`, summary still emitted
/// 4. `price_table_miss_no_panic` — unknown backend, cost_usd=None, no panic
/// 5. `budget_exceeded_warning` — budget_warning=true, exit code unchanged
///
/// All tests create a temporary run directory with markers, trace.jsonl,
/// and optionally prices.toml, then invoke `SummaryWriter::write_summary`
/// and verify the output.
#[cfg(test)]
mod summary_integration_tests {
    use std::path::Path;

    use loker::manifest::{Kind, Manifest, ManifestEntry, Producer};
    use loker::SummaryWriter;

    // ------------------------------------------------------------------
    // Test 1: single phase, single backend, token totals match
    // ------------------------------------------------------------------
    #[test]
    fn single_phase_success() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path();

        // Create a completed phase marker
        let markers = run_dir.join("markers");
        std::fs::create_dir_all(&markers).unwrap();
        std::fs::write(markers.join("design.completed"), "{}").unwrap();

        // Create trace.jsonl with two backend calls using the same model
        let trace_path = run_dir.join("trace.jsonl");
        std::fs::write(
            &trace_path,
            r#"{"name":"backend.anthropic","gen_ai.system":"anthropic","gen_ai.request.model":"claude-opus-4-7","gen_ai.usage.input_tokens":100,"gen_ai.usage.output_tokens":200}
{"name":"backend.anthropic","gen_ai.system":"anthropic","gen_ai.request.model":"claude-opus-4-7","gen_ai.usage.input_tokens":50,"gen_ai.usage.output_tokens":75}
"#,
        )
        .unwrap();

        // Create manifest
        let manifest_path = run_dir.join("manifest.json");
        let mut manifest = Manifest::new("single-phase-test");
        manifest
            .append(
                ManifestEntry::from_payload(
                    "preexisting.md",
                    Kind::DesignMd,
                    1,
                    Producer::Single,
                    Some("design".into()),
                    Some(0),
                    b"hello",
                ),
                &manifest_path,
            )
            .unwrap();

        // Create prices.toml for cost computation
        create_prices_toml(run_dir);

        // Run the full pipeline
        let writer = SummaryWriter::new(false);
        let summary = writer
            .write_summary(run_dir, &mut manifest, &trace_path, None)
            .expect("write_summary should succeed for single_phase_success");

        // Verify summary.json was written
        assert!(run_dir.join("summary.json").exists());

        // Verify token totals: 100+50=150 input, 200+75=275 output
        assert_eq!(summary.totals.input_tokens, 150);
        assert_eq!(summary.totals.output_tokens, 275);

        // Verify backend usage
        assert_eq!(summary.backends.len(), 1);
        assert_eq!(summary.backends[0].name, "anthropic");
        assert_eq!(summary.backends[0].model, "claude-opus-4-7");
        assert_eq!(summary.backends[0].input_tokens, 150);
        assert_eq!(summary.backends[0].output_tokens, 275);

        // Verify cost: 150/1M * $15 + 275/1M * $75 = $0.00225 + $0.020625 = $0.022875
        let cost = summary.backends[0]
            .cost_usd
            .expect("cost_usd should be Some");
        assert!((cost - 0.022875).abs() < 0.001);

        // Verify phase
        assert_eq!(summary.phases.len(), 1);
        assert_eq!(summary.phases[0].phase, "design");

        // Verify status is success since all phases completed
        assert!(
            matches!(summary.status, loker::RunStatus::Success),
            "status should be Success"
        );

        // Verify manifest entry
        assert!(
            manifest.sha256_for("summary.json").is_some(),
            "manifest should have summary.json entry"
        );
    }

    // ------------------------------------------------------------------
    // Test 2: 3 backends, separate rollups per backend/model
    // ------------------------------------------------------------------
    #[test]
    fn multi_backend_parallel() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path();

        // Create markers for two phases
        let markers = run_dir.join("markers");
        std::fs::create_dir_all(&markers).unwrap();
        std::fs::write(markers.join("design.completed"), "{}").unwrap();
        std::fs::write(markers.join("implement.completed"), "{}").unwrap();

        // Create trace.jsonl with calls to 3 different backends
        let trace_path = run_dir.join("trace.jsonl");
        std::fs::write(
            &trace_path,
            r#"{"name":"backend.anthropic","gen_ai.system":"anthropic","gen_ai.request.model":"claude-opus-4-7","gen_ai.usage.input_tokens":100,"gen_ai.usage.output_tokens":200}
{"name":"backend.openai","gen_ai.system":"openai","gen_ai.request.model":"gpt-5","gen_ai.usage.input_tokens":50,"gen_ai.usage.output_tokens":80}
{"name":"backend.google","gen_ai.system":"google","gen_ai.request.model":"gemini-3.1-pro-preview","gen_ai.usage.input_tokens":200,"gen_ai.usage.output_tokens":400}
{"name":"backend.anthropic","gen_ai.system":"anthropic","gen_ai.request.model":"claude-opus-4-7","gen_ai.usage.input_tokens":30,"gen_ai.usage.output_tokens":60}
"#,
        )
        .unwrap();

        // Create manifest
        let manifest_path = run_dir.join("manifest.json");
        let mut manifest = Manifest::new("multi-backend-test");
        manifest
            .append(
                ManifestEntry::from_payload(
                    "preexisting.md",
                    Kind::DesignMd,
                    1,
                    Producer::Single,
                    None,
                    None,
                    b"data",
                ),
                &manifest_path,
            )
            .unwrap();

        // Create prices.toml
        create_prices_toml(run_dir);

        // Run pipeline
        let writer = SummaryWriter::new(false);
        let summary = writer
            .write_summary(run_dir, &mut manifest, &trace_path, None)
            .expect("write_summary should succeed for multi_backend_parallel");

        // Verify 3 distinct backends
        assert_eq!(summary.backends.len(), 3);

        // Anthropic: 100+30=130 input, 200+60=260 output
        let anthropic = summary
            .backends
            .iter()
            .find(|b| b.name == "anthropic")
            .expect("anthropic backend should exist");
        assert_eq!(anthropic.input_tokens, 130);
        assert_eq!(anthropic.output_tokens, 260);

        // OpenAI: 50 input, 80 output
        let openai = summary
            .backends
            .iter()
            .find(|b| b.name == "openai")
            .expect("openai backend should exist");
        assert_eq!(openai.input_tokens, 50);
        assert_eq!(openai.output_tokens, 80);

        // Google: 200 input, 400 output
        let google = summary
            .backends
            .iter()
            .find(|b| b.name == "google")
            .expect("google backend should exist");
        assert_eq!(google.input_tokens, 200);
        assert_eq!(google.output_tokens, 400);

        // Verify totals: 130+50+200 = 380 input, 260+80+400 = 740 output
        assert_eq!(summary.totals.input_tokens, 380);
        assert_eq!(summary.totals.output_tokens, 740);

        // Verify 2 phases
        assert_eq!(summary.phases.len(), 2);

        // Verify all costs are present (all backends are in prices.toml)
        for backend in &summary.backends {
            assert!(
                backend.cost_usd.is_some(),
                "cost_usd should be Some for {}:{}",
                backend.name,
                backend.model
            );
        }

        // Totals cost should also be Some
        assert!(
            summary.totals.cost_usd.is_some(),
            "totals.cost_usd should be Some"
        );
    }

    // ------------------------------------------------------------------
    // Test 3: phase shows 'failed', summary still emitted
    // ------------------------------------------------------------------
    #[test]
    fn failed_phase_emitted() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path();

        // Create a failed phase marker
        let markers = run_dir.join("markers");
        std::fs::create_dir_all(&markers).unwrap();
        std::fs::write(
            markers.join("implement.failed"),
            r#"{"error":"verify_failed"}"#,
        )
        .unwrap();

        // Create trace.jsonl with one backend call
        let trace_path = run_dir.join("trace.jsonl");
        std::fs::write(
            &trace_path,
            r#"{"name":"backend.anthropic","gen_ai.system":"anthropic","gen_ai.request.model":"claude-opus-4-7","gen_ai.usage.input_tokens":100,"gen_ai.usage.output_tokens":200}
"#,
        )
        .unwrap();

        // Create manifest
        let manifest_path = run_dir.join("manifest.json");
        let mut manifest = Manifest::new("failed-phase-test");
        manifest
            .append(
                ManifestEntry::from_payload(
                    "preexisting.md",
                    Kind::DesignMd,
                    1,
                    Producer::Single,
                    None,
                    None,
                    b"data",
                ),
                &manifest_path,
            )
            .unwrap();

        create_prices_toml(run_dir);

        // Run pipeline
        let writer = SummaryWriter::new(false);
        let summary = writer
            .write_summary(run_dir, &mut manifest, &trace_path, None)
            .expect("write_summary should succeed even with a failed phase");

        // Verify summary.json was written
        assert!(run_dir.join("summary.json").exists());

        // Verify the phase shows as failed
        assert_eq!(summary.phases.len(), 1);
        assert_eq!(summary.phases[0].phase, "implement");

        // PhaseStatus serializes as snake_case: "failed" should match
        let json = serde_json::to_value(&summary.phases[0]).unwrap();
        assert_eq!(json["status"], "failed");

        // Run status should be "partial" because at least one phase failed
        assert!(
            summary.phases[0].attempts >= 1,
            "failed phase should have at least 1 attempt"
        );

        // Backend data should still be present
        assert_eq!(summary.backends.len(), 1);
        assert_eq!(summary.backends[0].input_tokens, 100);
        assert_eq!(summary.backends[0].output_tokens, 200);

        // Run status should be Partial (not Success) since a phase failed
        assert!(
            matches!(summary.status, loker::RunStatus::Partial),
            "run status should be Partial when a phase failed"
        );
    }

    // ------------------------------------------------------------------
    // Test 4: unknown backend not in prices.toml, cost_usd=None
    // ------------------------------------------------------------------
    #[test]
    fn price_table_miss_no_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path();

        // Create a completed phase marker
        let markers = run_dir.join("markers");
        std::fs::create_dir_all(&markers).unwrap();
        std::fs::write(markers.join("design.completed"), "{}").unwrap();

        // Create trace.jsonl with an unknown backend not in prices.toml
        let trace_path = run_dir.join("trace.jsonl");
        std::fs::write(
            &trace_path,
            r#"{"name":"backend.unknown","gen_ai.system":"unknown-vendor","gen_ai.request.model":"experimental-v99","gen_ai.usage.input_tokens":1000,"gen_ai.usage.output_tokens":2000}
"#,
        )
        .unwrap();

        // Create manifest
        let manifest_path = run_dir.join("manifest.json");
        let mut manifest = Manifest::new("price-miss-test");
        manifest
            .append(
                ManifestEntry::from_payload(
                    "preexisting.md",
                    Kind::DesignMd,
                    1,
                    Producer::Single,
                    None,
                    None,
                    b"data",
                ),
                &manifest_path,
            )
            .unwrap();

        // Create prices.toml but WITHOUT the unknown backend's entry
        create_prices_toml(run_dir);

        // Run pipeline — should not panic
        let writer = SummaryWriter::new(false);
        let summary = writer
            .write_summary(run_dir, &mut manifest, &trace_path, None)
            .expect("write_summary should not panic on unknown backend");

        // Verify summary.json was written
        assert!(run_dir.join("summary.json").exists());

        // Backend should exist with tokens but cost_usd=None
        assert_eq!(summary.backends.len(), 1);
        assert_eq!(summary.backends[0].name, "unknown-vendor");
        assert_eq!(summary.backends[0].input_tokens, 1000);
        assert_eq!(summary.backends[0].output_tokens, 2000);
        assert!(
            summary.backends[0].cost_usd.is_none(),
            "cost_usd should be None for unknown backend"
        );

        // Totals cost_usd should also be None since no backend had cost
        assert!(
            summary.totals.cost_usd.is_none(),
            "totals.cost_usd should be None when all backends are unknown"
        );

        // The summary JSON should not have a cost_usd field
        let json = serde_json::to_value(&summary).unwrap();
        assert!(
            json["totals"].get("cost_usd").is_none(),
            "totals.cost_usd should be absent from JSON when None"
        );
        assert!(
            json["backends"][0].get("cost_usd").is_none(),
            "backend cost_usd should be absent from JSON when None"
        );
    }

    // ------------------------------------------------------------------
    // Test 5: budget_warning=true when cost exceeds cost_budget_usd
    // ------------------------------------------------------------------
    #[test]
    fn budget_exceeded_warning() {
        let tmp = tempfile::tempdir().unwrap();
        let run_dir = tmp.path();

        // Create a completed phase marker
        let markers = run_dir.join("markers");
        std::fs::create_dir_all(&markers).unwrap();
        std::fs::write(markers.join("design.completed"), "{}").unwrap();

        // Create trace.jsonl with enough tokens to exceed budget
        let trace_path = run_dir.join("trace.jsonl");
        std::fs::write(
            &trace_path,
            r#"{"name":"backend.anthropic","gen_ai.system":"anthropic","gen_ai.request.model":"claude-opus-4-7","gen_ai.usage.input_tokens":100000,"gen_ai.usage.output_tokens":200000}
"#,
        )
        .unwrap();

        // Create manifest
        let manifest_path = run_dir.join("manifest.json");
        let mut manifest = Manifest::new("budget-test");
        manifest
            .append(
                ManifestEntry::from_payload(
                    "preexisting.md",
                    Kind::DesignMd,
                    1,
                    Producer::Single,
                    None,
                    None,
                    b"data",
                ),
                &manifest_path,
            )
            .unwrap();

        // Create prices.toml
        create_prices_toml(run_dir);

        // Run pipeline with a small budget ($1.00)
        let writer = SummaryWriter::new(false);
        let summary = writer
            .write_summary(run_dir, &mut manifest, &trace_path, Some(1.00))
            .expect("write_summary should succeed");

        // Cost: 0.1M * $15 + 0.2M * $75 = $1.50 + $15.00 = $16.50 >> $1.00
        assert_eq!(
            summary.budget_warning,
            Some(true),
            "budget_warning should be Some(true) when cost exceeds budget"
        );

        // Run pipeline without a budget
        let writer2 = SummaryWriter::new(false);
        let summary2 = writer2
            .write_summary(run_dir, &mut manifest, &trace_path, None)
            .expect("write_summary should succeed without budget");

        // Without a budget, no warning
        assert!(
            summary2.budget_warning.is_none(),
            "budget_warning should be None when no budget is set"
        );

        // With a generous budget that's not exceeded
        let writer3 = SummaryWriter::new(false);
        let summary3 = writer3
            .write_summary(run_dir, &mut manifest, &trace_path, Some(100.00))
            .expect("write_summary should succeed with generous budget");

        // Budget not exceeded (16.50 < 100.00), so no warning
        assert!(
            summary3.budget_warning.is_none(),
            "budget_warning should be None when budget is not exceeded"
        );

        // Verify all variants produce valid JSON
        for (_, s) in [
            ("exceeded", &summary),
            ("no_budget", &summary2),
            ("generous", &summary3),
        ] {
            let json = serde_json::to_value(s).unwrap();
            assert!(json.get("loker.run_id").is_some());
            assert!(json.get("totals").is_some());
        }

        // Verify idempotent manifest registration: after 3 write_summary calls,
        // only 1 summary.json entry should exist
        let summary_entry_count = manifest
            .entries
            .iter()
            .filter(|e| e.name == "summary.json")
            .count();
        assert_eq!(
            summary_entry_count, 1,
            "manifest should have exactly 1 summary.json entry after 3 write_summary calls"
        );
    }

    // ------------------------------------------------------------------
    // Helpers
    // ------------------------------------------------------------------

    /// Create a prices.toml under `run_dir/docs/prices.toml` with entries
    /// for all standard backends used in these tests.
    fn create_prices_toml(run_dir: &Path) {
        let prices_dir = run_dir.join("docs");
        std::fs::create_dir_all(&prices_dir).unwrap();
        std::fs::write(
            prices_dir.join("prices.toml"),
            r#"["anthropic:claude-opus-4-7"]
input_per_mtok = 15.0
output_per_mtok = 75.0

["openai:gpt-5"]
input_per_mtok = 10.0
output_per_mtok = 40.0

["google:gemini-3.1-pro-preview"]
input_per_mtok = 1.25
output_per_mtok = 5.0
"#,
        )
        .unwrap();
    }
}
