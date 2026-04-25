//! CLO-243 round-trip spike runner.
//!
//! Drives the local Tier-2 TensorZero gateway end-to-end and dumps the
//! request/response shapes to `tests/fixtures/tensorzero/`. Run after
//! `cd tensorzero && docker compose up -d`.
//!
//! Usage:
//!   cargo run --example tensorzero_spike
//!
//! Env vars:
//!   TENSORZERO_GATEWAY_URL   default http://localhost:3000
//!   LOKER_TZ_FIXTURE_DIR     default tests/fixtures/tensorzero
//!
//! The runner exercises four scenarios and writes one request + one
//! response file per scenario, sanitised of any auth header value.

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const DEFAULT_GATEWAY_URL: &str = "http://localhost:3000";
const DEFAULT_FIXTURE_DIR: &str = "tests/fixtures/tensorzero";
const SANITISED: &str = "<REDACTED>";

struct Scenario {
    slug: &'static str,
    function: &'static str,
    user_message: &'static str,
    expect_success: bool,
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        slug: "anthropic_success",
        function: "loker_d1_anthropic",
        user_message: "Reply with the single word: pong.",
        expect_success: true,
    },
    Scenario {
        slug: "openai_success",
        function: "loker_d1_openai",
        user_message: "Reply with the single word: pong.",
        expect_success: true,
    },
    Scenario {
        slug: "unknown_function",
        function: "loker_d1_does_not_exist",
        user_message: "Reply with the single word: pong.",
        expect_success: false,
    },
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let gateway =
        std::env::var("TENSORZERO_GATEWAY_URL").unwrap_or_else(|_| DEFAULT_GATEWAY_URL.to_string());
    let fixture_dir = PathBuf::from(
        std::env::var("LOKER_TZ_FIXTURE_DIR").unwrap_or_else(|_| DEFAULT_FIXTURE_DIR.to_string()),
    );
    fs::create_dir_all(&fixture_dir)?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;

    println!("Gateway: {gateway}");
    println!("Fixture dir: {}", fixture_dir.display());

    // Health probe first so the operator notices a stack that isn't up.
    let health = client
        .get(format!("{gateway}/health"))
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("gateway /health unreachable: {e}"))?;
    println!("/health -> {}", health.status());

    let mut summary = serde_json::Map::new();

    for scenario in SCENARIOS {
        println!("\n--- {} ---", scenario.slug);
        let req_body = build_request(scenario.function, scenario.user_message);
        let started = Instant::now();
        let resp = client
            .post(format!("{gateway}/openai/v1/chat/completions"))
            .header("content-type", "application/json")
            .header("authorization", "Bearer not-used")
            .json(&req_body)
            .send()
            .await?;
        let status = resp.status();
        let elapsed = started.elapsed();
        let raw_body = resp.text().await?;
        let parsed: Value = serde_json::from_str(&raw_body).unwrap_or_else(|_| json!(raw_body));

        write_fixture(
            &fixture_dir,
            &format!("{}_request.json", scenario.slug),
            &sanitise_request(&req_body),
        )?;
        write_fixture(
            &fixture_dir,
            &format!("{}_response.json", scenario.slug),
            &parsed,
        )?;

        println!("HTTP {status} in {} ms", elapsed.as_millis());
        summary.insert(
            scenario.slug.to_string(),
            json!({
                "http_status": status.as_u16(),
                "expect_success": scenario.expect_success,
                "elapsed_ms": elapsed.as_millis() as u64,
                "model_in_response": parsed.get("model").cloned(),
                "object": parsed.get("object").cloned(),
                "has_episode_id": parsed.get("episode_id").is_some(),
                "usage_keys": parsed
                    .get("usage")
                    .and_then(|u| u.as_object())
                    .map(|o| o.keys().cloned().collect::<Vec<_>>())
                    .unwrap_or_default(),
                "error": parsed.get("error").cloned(),
            }),
        );
    }

    write_fixture(
        &fixture_dir,
        "summary.json",
        &Value::Object(summary.clone()),
    )?;
    println!(
        "\nSummary written. Inspect {}/summary.json",
        fixture_dir.display()
    );
    Ok(())
}

fn build_request(function: &str, content: &str) -> Value {
    json!({
        "model": format!("tensorzero::function_name::{function}"),
        "messages": [
            { "role": "user", "content": content }
        ],
        "max_tokens": 32,
        "temperature": 0.0,
    })
}

fn sanitise_request(req: &Value) -> Value {
    let mut clone = req.clone();
    if let Some(obj) = clone.as_object_mut() {
        obj.insert("_meta_authorization_header".to_string(), json!(SANITISED));
    }
    clone
}

fn write_fixture(dir: &PathBuf, name: &str, value: &Value) -> anyhow::Result<()> {
    let path = dir.join(name);
    let body = serde_json::to_string_pretty(value)?;
    fs::write(&path, body + "\n")?;
    println!("wrote {}", path.display());
    Ok(())
}
