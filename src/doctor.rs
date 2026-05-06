use crate::config::{Config, TensorZeroConfig};
use anyhow::Context;
use colored::Colorize;
use std::time::Duration;

/// A single diagnostic check result.
#[derive(Debug, Clone)]
pub struct CheckRow {
    pub name: &'static str,
    pub status: String,
    pub detail: Option<String>,
    pub is_ok: bool,
    pub is_critical: bool,
}

/// Run all diagnostic checks and return rows + overall failure flag.
pub async fn run(config: &Config) -> (Vec<CheckRow>, bool) {
    let mut rows = Vec::new();

    // Binary checks
    rows.extend(check_binaries());

    // API key checks
    rows.extend(check_api_keys());

    // TensorZero gateway reachability
    rows.push(probe_tensorzero(config).await);

    let failed = rows.iter().any(|r| r.is_critical && !r.is_ok);
    (rows, failed)
}

fn check_binaries() -> Vec<CheckRow> {
    let checks = vec![
        ("codex", "codex", "npm install -g @openai/codex"),
        ("gemini", "npx", "Install Node.js (npx comes with npm)"),
        (
            "claude",
            "claude",
            "Install Claude Code: https://claude.ai/claude-code",
        ),
    ];

    let mut rows = Vec::new();
    for (name, binary, install_hint) in &checks {
        let found = which::which(binary).is_ok();
        rows.push(CheckRow {
            name,
            status: if found {
                "ready".to_string()
            } else {
                "not found".to_string()
            },
            detail: if found {
                None
            } else {
                Some((*install_hint).to_string())
            },
            is_ok: found,
            is_critical: false,
        });
    }
    rows
}

fn check_api_keys() -> Vec<CheckRow> {
    let keys = vec![
        ("ANTHROPIC_API_KEY", "claude backend"),
        ("GOOGLE_API_KEY", "gemini backend"),
        ("AWS_PROFILE", "bedrock backend (or AWS_ACCESS_KEY_ID)"),
    ];

    let mut rows = Vec::new();
    for (name, desc) in &keys {
        let set = std::env::var(name).is_ok();
        rows.push(CheckRow {
            name,
            status: if set {
                "set".to_string()
            } else {
                "not set".to_string()
            },
            detail: Some((*desc).to_string()),
            is_ok: set,
            is_critical: false,
        });
    }
    rows
}

async fn probe_tensorzero(config: &Config) -> CheckRow {
    let tz = match config.tensorzero {
        None => {
            return CheckRow {
                name: "tensorzero",
                status: "not configured".to_string(),
                detail: None,
                is_ok: true,
                is_critical: false,
            };
        }
        Some(ref cfg) => cfg,
    };

    let (endpoint, api_key) = match resolve_tensorzero_opts(tz) {
        Ok((ep, key)) => (ep, key),
        Err(e) => {
            return CheckRow {
                name: "tensorzero",
                status: "UNREACHABLE (config)".to_string(),
                detail: Some(format!("{e:#}")),
                is_ok: false,
                is_critical: true,
            };
        }
    };

    let url = format!("{}/health", endpoint.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());

    let mut req = client.get(&url);
    if let Some(ref key) = api_key {
        req = req.header("Authorization", format!("Bearer {}", key));
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => {
            return CheckRow {
                name: "tensorzero",
                status: format!("UNREACHABLE ({})", classify_transport_error(&e)),
                detail: Some(e.to_string()),
                is_ok: false,
                is_critical: true,
            };
        }
    };

    match resp.status().as_u16() {
        200..=299 => CheckRow {
            name: "tensorzero",
            status: "HEALTHY".to_string(),
            detail: None,
            is_ok: true,
            is_critical: true,
        },
        401..=403 => CheckRow {
            name: "tensorzero",
            status: "UNREACHABLE (auth)".to_string(),
            detail: Some(format!("HTTP {}", resp.status())),
            is_ok: false,
            is_critical: true,
        },
        500..=599 => CheckRow {
            name: "tensorzero",
            status: "UNREACHABLE (server error)".to_string(),
            detail: Some(format!("HTTP {}", resp.status())),
            is_ok: false,
            is_critical: true,
        },
        status => CheckRow {
            name: "tensorzero",
            status: "UNREACHABLE (network)".to_string(),
            detail: Some(format!("HTTP {}", status)),
            is_ok: false,
            is_critical: true,
        },
    }
}

fn resolve_tensorzero_opts(tz: &TensorZeroConfig) -> anyhow::Result<(String, Option<String>)> {
    // Try to resolve via to_backend_opts(); if api_key_env is unset or empty, that's okay.
    let api_key = match tz.api_key_env.as_deref() {
        Some(var) if !var.is_empty() => {
            let val = std::env::var(var)
                .with_context(|| format!("Missing environment variable: {}", var))?;
            Some(val)
        }
        _ => None,
    };
    Ok((tz.endpoint.clone(), api_key))
}

fn classify_transport_error(e: &reqwest::Error) -> &'static str {
    if e.is_timeout() {
        return "timeout";
    }
    if e.is_connect() {
        let msg = e.to_string().to_lowercase();
        if msg.contains("dns") || msg.contains("resolve") {
            return "DNS";
        }
        if msg.contains("refused") {
            return "connection refused";
        }
        return "connection refused";
    }
    "network"
}

/// Pretty-print doctor rows with the original loker doctor styling.
pub fn print_rows(rows: &[CheckRow]) {
    println!("{}", "Lok Doctor".cyan().bold());
    println!("{}", "=".repeat(50).dimmed());
    println!();
    println!(
        "Lok is an orchestration layer for LLM backends. It's the brain\n\
        that coordinates the arms you already have installed.\n"
    );
    println!("{}", "Checking backends...".yellow());
    println!();

    let mut backends_ready = 0;
    for row in rows {
        if row.name == "codex" || row.name == "gemini" || row.name == "claude" {
            if row.is_ok {
                println!("  {} {} - {}", "✓".green(), row.name, row.status);
                backends_ready += 1;
            } else {
                println!("  {} {} - {}", "✗".red(), row.name, row.status);
                if let Some(ref d) = row.detail {
                    println!("    {}", d.dimmed());
                }
            }
        }
    }
    println!();
    println!("{}", "Checking API keys...".yellow());
    println!();

    for row in rows {
        if row.name == "ANTHROPIC_API_KEY"
            || row.name == "GOOGLE_API_KEY"
            || row.name == "AWS_PROFILE"
        {
            if row.is_ok {
                println!(
                    "  {} {} - set ({})",
                    "✓".green(),
                    row.name,
                    row.detail.as_ref().unwrap()
                );
            } else {
                println!(
                    "  {} {} - not set ({})",
                    "○".yellow(),
                    row.name,
                    row.detail.as_ref().unwrap()
                );
            }
        }
    }

    // TensorZero summary
    println!();
    println!("{}", "Checking TensorZero gateway...".yellow());
    println!();
    for row in rows {
        if row.name == "tensorzero" {
            if row.is_ok {
                println!("  {} {} - {}", "✓".green(), row.name, row.status.green());
            } else if row.status.contains("not configured") {
                println!(
                    "  {} {} - {}",
                    "○".dimmed(),
                    row.name.dimmed(),
                    row.status.dimmed()
                );
            } else {
                println!("  {} {} - {}", "✗".red(), row.name, row.status.red());
                if let Some(ref d) = row.detail {
                    println!("    {}", d.dimmed());
                }
            }
        }
    }

    println!();
    if backends_ready > 0 {
        println!(
            "{} {} backend(s) ready. Run {} to see them.",
            "✓".green(),
            backends_ready,
            "loker backends".cyan()
        );
    } else {
        println!(
            "{} No backends found. Install at least one LLM CLI to get started.",
            "!".red()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn tensorzero_not_configured_is_not_critical() {
        let config = Config::default();
        let row = probe_tensorzero(&config).await;
        assert_eq!(row.status, "not configured");
        assert!(row.is_ok);
        assert!(!row.is_critical);
    }

    #[tokio::test]
    async fn tensorzero_healthy_200() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"gateway": "ok"})),
            )
            .mount(&server)
            .await;

        let config = Config {
            tensorzero: Some(TensorZeroConfig {
                endpoint: format!("{}", server.uri()),
                default_model: "test".to_string(),
                api_key_env: None,
                timeout_secs: 60,
                retry_policy: crate::config::RetryPolicyConfig::default(),
            }),
            ..Config::default()
        };

        let row = probe_tensorzero(&config).await;
        assert_eq!(row.status, "HEALTHY");
        assert!(row.is_ok);
        assert!(row.is_critical);
    }

    #[tokio::test]
    async fn tensorzero_auth_failure_401() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let config = Config {
            tensorzero: Some(TensorZeroConfig {
                endpoint: format!("{}", server.uri()),
                default_model: "test".to_string(),
                api_key_env: None,
                timeout_secs: 60,
                retry_policy: crate::config::RetryPolicyConfig::default(),
            }),
            ..Config::default()
        };

        let row = probe_tensorzero(&config).await;
        assert_eq!(row.status, "UNREACHABLE (auth)");
        assert!(!row.is_ok);
        assert!(row.is_critical);
    }

    #[tokio::test]
    async fn tensorzero_server_error_503() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(503).set_body_string("maintenance"))
            .mount(&server)
            .await;

        let config = Config {
            tensorzero: Some(TensorZeroConfig {
                endpoint: format!("{}", server.uri()),
                default_model: "test".to_string(),
                api_key_env: None,
                timeout_secs: 60,
                retry_policy: crate::config::RetryPolicyConfig::default(),
            }),
            ..Config::default()
        };

        let row = probe_tensorzero(&config).await;
        assert_eq!(row.status, "UNREACHABLE (server error)");
        assert!(!row.is_ok);
        assert!(row.is_critical);
    }

    #[tokio::test]
    async fn tensorzero_timeout() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(30)))
            .mount(&server)
            .await;

        let config = Config {
            tensorzero: Some(TensorZeroConfig {
                endpoint: format!("{}", server.uri()),
                default_model: "test".to_string(),
                api_key_env: None,
                timeout_secs: 60,
                retry_policy: crate::config::RetryPolicyConfig::default(),
            }),
            ..Config::default()
        };

        let row = probe_tensorzero(&config).await;
        assert_eq!(row.status, "UNREACHABLE (timeout)");
        assert!(!row.is_ok);
        assert!(row.is_critical);
    }

    #[tokio::test]
    async fn tensorzero_connection_refused() {
        // Use a port that should be closed on localhost
        let config = Config {
            tensorzero: Some(TensorZeroConfig {
                endpoint: "http://127.0.0.1:1".to_string(),
                default_model: "test".to_string(),
                api_key_env: None,
                timeout_secs: 60,
                retry_policy: crate::config::RetryPolicyConfig::default(),
            }),
            ..Config::default()
        };

        let row = probe_tensorzero(&config).await;
        assert!(
            row.status.contains("UNREACHABLE"),
            "expected UNREACHABLE, got {:?}",
            row.status
        );
        assert!(!row.is_ok);
        assert!(row.is_critical);
    }

    #[tokio::test]
    async fn tensorzero_dns_failure() {
        // http://invalid.invalid is a reserved name guaranteed to NXDOMAIN.
        let config = Config {
            tensorzero: Some(TensorZeroConfig {
                endpoint: "http://invalid.invalid:3000".to_string(),
                default_model: "test".to_string(),
                api_key_env: None,
                timeout_secs: 60,
                retry_policy: crate::config::RetryPolicyConfig::default(),
            }),
            ..Config::default()
        };

        let row = probe_tensorzero(&config).await;
        assert!(
            row.status.contains("UNREACHABLE"),
            "expected UNREACHABLE, got {:?}",
            row.status
        );
        assert!(!row.is_ok);
        assert!(row.is_critical);
    }

    #[tokio::test]
    async fn tensorzero_bearer_auth_header_sent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .and(header("authorization", "Bearer test-secret"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"gateway": "ok"})),
            )
            .mount(&server)
            .await;

        std::env::set_var("LOK_DOCTOR_TZ_TEST_KEY", "test-secret");
        let config = Config {
            tensorzero: Some(TensorZeroConfig {
                endpoint: format!("{}", server.uri()),
                default_model: "test".to_string(),
                api_key_env: Some("LOK_DOCTOR_TZ_TEST_KEY".to_string()),
                timeout_secs: 60,
                retry_policy: crate::config::RetryPolicyConfig::default(),
            }),
            ..Config::default()
        };

        let row = probe_tensorzero(&config).await;
        assert_eq!(row.status, "HEALTHY");
        std::env::remove_var("LOK_DOCTOR_TZ_TEST_KEY");
    }

    #[tokio::test]
    async fn doctor_run_exit_code_not_configured() {
        let config = Config::default();
        let (_, failed) = run(&config).await;
        assert!(!failed, "should not fail when tensorzero is not configured");
    }

    #[tokio::test]
    async fn doctor_run_exit_code_fails_when_critical_check_fails() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let config = Config {
            tensorzero: Some(TensorZeroConfig {
                endpoint: format!("{}", server.uri()),
                default_model: "test".to_string(),
                api_key_env: None,
                timeout_secs: 60,
                retry_policy: crate::config::RetryPolicyConfig::default(),
            }),
            ..Config::default()
        };

        let (_, failed) = run(&config).await;
        assert!(failed, "should fail when critical check fails");
    }
}
