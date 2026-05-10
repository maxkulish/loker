use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub conductor: ConductorConfig,
    #[serde(default)]
    pub cache: crate::cache::CacheConfig,
    #[serde(default)]
    pub tensorzero: Option<TensorZeroConfig>,
    #[serde(default)]
    pub backends: HashMap<String, BackendConfig>,
    #[serde(default)]
    pub tasks: HashMap<String, TaskConfig>,
    #[serde(default)]
    pub roles: HashMap<String, crate::role::RoleConfig>,
    #[serde(default)]
    pub teams: HashMap<String, crate::role::TeamConfig>,
    #[serde(default)]
    pub run_state: RunStateConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AttemptRetention {
    #[default]
    Unbounded,
    Keep(usize),
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
#[serde(deny_unknown_fields)]
pub struct RunStateConfig {
    #[serde(default)]
    pub keep_attempts: AttemptRetention,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    #[serde(default = "default_parallel")]
    pub parallel: bool,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
    #[serde(default = "default_retries")]
    pub max_retries: usize,
    #[serde(default = "default_retry_delay_ms")]
    pub retry_delay_ms: u64,
    /// Optional wrapper for shell commands (e.g., "nix-shell --run '{cmd}'" or "docker exec dev sh -c '{cmd}'")
    /// The {cmd} placeholder will be replaced with the actual command
    #[serde(default)]
    pub command_wrapper: Option<String>,
    /// Default team for role resolution (can be overridden via CLI --team flag)
    #[serde(default)]
    pub team: Option<String>,
}

fn default_parallel() -> bool {
    true
}

fn default_timeout() -> u64 {
    300
}

fn default_retries() -> usize {
    0
}

fn default_retry_delay_ms() -> u64 {
    1000
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            parallel: default_parallel(),
            timeout: default_timeout(),
            max_retries: default_retries(),
            retry_delay_ms: default_retry_delay_ms(),
            command_wrapper: None,
            team: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct ConductorConfig {
    #[serde(default = "default_max_rounds")]
    pub max_rounds: usize,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
}

fn default_max_rounds() -> usize {
    5
}

fn default_max_tokens() -> usize {
    4096
}

impl Default for ConductorConfig {
    fn default() -> Self {
        Self {
            max_rounds: default_max_rounds(),
            max_tokens: default_max_tokens(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct BackendConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub skip_lines: usize,
    pub api_key_env: Option<String>,
    pub model: Option<String>,
    /// Per-backend timeout in seconds (overrides defaults.timeout)
    pub timeout: Option<u64>,
    /// Per-backend retry limit (overrides defaults.max_retries)
    pub max_retries: Option<usize>,
    /// Per-backend retry delay in milliseconds (overrides defaults.retry_delay_ms)
    pub retry_delay_ms: Option<u64>,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicyConfig {
    #[serde(default = "default_retries")]
    pub max_retries: usize,
    #[serde(default = "default_retry_delay_ms")]
    pub delay_ms: u64,
}

impl Default for RetryPolicyConfig {
    fn default() -> Self {
        Self {
            max_retries: default_retries(),
            delay_ms: default_retry_delay_ms(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct TensorZeroConfig {
    pub endpoint: String,
    pub default_model: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default = "default_tz_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub retry_policy: RetryPolicyConfig,
}

fn default_tz_timeout_secs() -> u64 {
    60
}

impl TensorZeroConfig {
    pub fn to_backend_opts(&self) -> Result<crate::backend::TensorZeroBackendOpts> {
        let api_key = match self.api_key_env.as_deref() {
            Some(var) if !var.is_empty() => Some(
                std::env::var(var)
                    .with_context(|| format!("Missing environment variable: {}", var))?,
            ),
            _ => None,
        };
        Ok(crate::backend::TensorZeroBackendOpts {
            endpoint: self.endpoint.clone(),
            model: self.default_model.clone(),
            api_key,
            timeout: std::time::Duration::from_secs(self.timeout_secs),
        })
    }

    pub fn validate(&self) -> Result<()> {
        if self.endpoint.is_empty() {
            anyhow::bail!("tensorzero.endpoint: must not be empty");
        }
        let url = reqwest::Url::parse(&self.endpoint).with_context(|| {
            format!("tensorzero.endpoint: not a valid URL: '{}'", self.endpoint)
        })?;
        let scheme = url.scheme();
        if scheme != "http" && scheme != "https" {
            anyhow::bail!(
                "tensorzero.endpoint: scheme must be http or https, got '{}' in '{}'",
                scheme,
                self.endpoint
            );
        }
        if self.default_model.is_empty() {
            anyhow::bail!("tensorzero.default_model: must not be empty");
        }
        if self.timeout_secs == 0 {
            anyhow::bail!("tensorzero.timeout_secs: must be greater than zero");
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct TaskConfig {
    pub description: Option<String>,
    #[serde(default)]
    pub backends: Vec<String>,
    #[serde(default)]
    pub prompts: Vec<TaskPrompt>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct TaskPrompt {
    pub name: String,
    pub prompt: String,
}

impl Default for Config {
    fn default() -> Self {
        let mut backends = HashMap::new();

        backends.insert(
            "codex".to_string(),
            BackendConfig {
                enabled: true,
                command: Some("codex".to_string()),
                args: vec![
                    "exec".to_string(),
                    "--json".to_string(),
                    "-s".to_string(),
                    "read-only".to_string(),
                ],
                skip_lines: 0,
                api_key_env: None,
                model: None,
                timeout: None,
                max_retries: None,
                retry_delay_ms: None,
            },
        );

        backends.insert(
            "gemini".to_string(),
            BackendConfig {
                enabled: true,
                command: Some("npx".to_string()),
                args: vec!["@google/gemini-cli".to_string()],
                skip_lines: 1,
                api_key_env: None,
                model: None,
                timeout: Some(600), // Gemini goes agentic, needs more time
                max_retries: None,
                retry_delay_ms: None,
            },
        );

        backends.insert(
            "claude".to_string(),
            BackendConfig {
                enabled: true,
                command: Some("claude".to_string()), // CLI mode by default (Claude Code)
                args: vec![],
                skip_lines: 0,
                api_key_env: None,
                model: None, // Uses Claude Code's default model
                timeout: None,
                max_retries: None,
                retry_delay_ms: None,
            },
        );

        backends.insert(
            "ollama".to_string(),
            BackendConfig {
                enabled: true,
                command: Some("http://localhost:11434".to_string()), // Base URL
                args: vec![],
                skip_lines: 0,
                api_key_env: None,
                model: Some("llama3.2".to_string()), // Default model
                timeout: None,
                max_retries: None,
                retry_delay_ms: None,
            },
        );

        let mut tasks = HashMap::new();

        tasks.insert(
            "hunt".to_string(),
            TaskConfig {
                description: Some("Find bugs and code issues".to_string()),
                backends: vec!["codex".to_string()],
                prompts: vec![
                    TaskPrompt {
                        name: "errors".to_string(),
                        prompt: "Find error handling problems in this codebase. Look for: unchecked errors, panics/crashes waiting to happen, missing input validation, swallowed exceptions. List up to 5 specific issues with file:line. Be concise.".to_string(),
                    },
                    TaskPrompt {
                        name: "perf".to_string(),
                        prompt: "Find performance issues in this codebase. Look for: inefficient loops, unnecessary allocations, blocking calls in async code, O(n^2) patterns, missing caching opportunities. List up to 5 specific issues with file:line. Be concise.".to_string(),
                    },
                    TaskPrompt {
                        name: "dead-code".to_string(),
                        prompt: "Find unused or dead code in this codebase. Look for: unreachable code, unused functions/methods, redundant conditions, commented-out code that should be removed. List up to 5 specific issues with file:line. Be concise.".to_string(),
                    },
                ],
            },
        );

        tasks.insert(
            "audit".to_string(),
            TaskConfig {
                description: Some("Security audit".to_string()),
                backends: vec!["gemini".to_string()],
                prompts: vec![
                    TaskPrompt {
                        name: "injection".to_string(),
                        prompt: "Search for injection vulnerabilities (SQL, command, code injection). Look for: unsanitized user input in queries/commands, string interpolation in SQL, shell command construction. List up to 5 specific issues with file:line. Be concise.".to_string(),
                    },
                    TaskPrompt {
                        name: "auth".to_string(),
                        prompt: "Search for authentication/authorization issues. Look for: missing auth checks, privilege escalation, insecure token handling, hardcoded credentials. List up to 5 specific issues with file:line. Be concise.".to_string(),
                    },
                    TaskPrompt {
                        name: "secrets".to_string(),
                        prompt: "Search for exposed secrets and sensitive data. Look for: hardcoded API keys, passwords in code, secrets in logs, sensitive data in error messages. List up to 5 specific issues with file:line. Be concise.".to_string(),
                    },
                ],
            },
        );

        Self {
            defaults: Defaults::default(),
            conductor: ConductorConfig::default(),
            cache: crate::cache::CacheConfig::default(),
            tensorzero: None,
            backends,
            tasks,
            roles: HashMap::new(),
            teams: HashMap::new(),
            run_state: RunStateConfig::default(),
        }
    }
}

/// Recursively deep-merge two TOML values. Tables recurse; scalars/arrays replace.
fn deep_merge(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_table), toml::Value::Table(overlay_table)) => {
            for (key, value) in overlay_table {
                match base_table.get_mut(&key) {
                    Some(existing) => deep_merge(existing, value),
                    None => {
                        base_table.insert(key, value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

/// Merge a TOML file into a base value. Returns Ok(()) if file doesn't exist.
/// Validates the file against Config struct first to catch unknown fields with
/// file-specific error context before merging.
fn merge_toml_file(base: &mut toml::Value, path: &Path) -> Result<()> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(anyhow::anyhow!("Error reading {}: {}", path.display(), e)),
    };

    // Validate against Config struct first to catch unknown fields with file context
    let _: Config =
        toml::from_str(&content).with_context(|| format!("Error parsing {}", path.display()))?;

    let overlay: toml::Value =
        toml::from_str(&content).with_context(|| format!("Error parsing {}", path.display()))?;

    deep_merge(base, overlay);
    Ok(())
}

/// Core config loading with injectable paths for testability.
pub fn load_config_from_paths(
    cwd: &Path,
    home_dir: Option<&Path>,
    explicit_path: Option<&Path>,
) -> Result<Config> {
    // Start with serialized defaults as base TOML value
    let default_config = Config::default();
    let mut base: toml::Value =
        toml::Value::try_from(&default_config).context("Failed to serialize default config")?;

    // Explicit path: merge with defaults (not hollow)
    if let Some(p) = explicit_path {
        merge_toml_file(&mut base, p)?;
        let cfg: Config = base
            .try_into::<Config>()
            .with_context(|| format!("Error parsing {}", p.display()))?;
        if let Some(tz) = cfg.tensorzero.as_ref() {
            tz.validate()?;
        }
        return Ok(cfg);
    }

    // Layer 2: user config. Prefer ~/.config/loker/loker.toml; fall back to
    // ~/.config/lok/lok.toml (legacy from lok-fork repos).
    if let Some(home) = home_dir {
        let user_loker = home.join(".config/loker/loker.toml");
        let user_lok = home.join(".config/lok/lok.toml");
        if user_loker.exists() {
            merge_toml_file(&mut base, &user_loker)?;
        } else {
            merge_toml_file(&mut base, &user_lok)?;
        }
    }

    // Layer 3: project config. Prefer ./loker.toml; fall back to ./lok.toml.
    let project_loker = cwd.join("loker.toml");
    let project_lok = cwd.join("lok.toml");
    if project_loker.exists() {
        merge_toml_file(&mut base, &project_loker)?;
    } else {
        merge_toml_file(&mut base, &project_lok)?;
    }

    // Deserialize merged TOML into Config (deny_unknown_fields applied here)
    let cfg: Config = base
        .try_into::<Config>()
        .context("Failed to deserialize merged config")?;
    if let Some(tz) = cfg.tensorzero.as_ref() {
        tz.validate()?;
    }
    Ok(cfg)
}

pub fn load_config(path: Option<&Path>) -> Result<Config> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let home = dirs::home_dir();
    load_config_from_paths(&cwd, home.as_deref(), path)
}

pub fn init_config() -> Result<()> {
    let config = Config::default();
    let content = toml::to_string_pretty(&config)?;

    if Path::new("loker.toml").exists() {
        anyhow::bail!("loker.toml already exists");
    }

    fs::write("loker.toml", content)?;
    println!("Created loker.toml");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();

        // Check defaults
        assert!(config.defaults.parallel);
        assert_eq!(config.defaults.timeout, 300);

        // Check default backends exist
        assert!(config.backends.contains_key("codex"));
        assert!(config.backends.contains_key("gemini"));
        assert!(config.backends.contains_key("claude"));

        // Check default tasks exist
        assert!(config.backends.contains_key("codex"));
        assert!(config.tasks.contains_key("hunt"));
        assert!(config.tasks.contains_key("audit"));
    }

    #[test]
    fn test_conductor_defaults() {
        let config = Config::default();

        assert_eq!(config.conductor.max_rounds, 5);
        assert_eq!(config.conductor.max_tokens, 4096);
    }

    #[test]
    fn test_conductor_custom_config() {
        let toml_str = r#"
[conductor]
max_rounds = 10
max_tokens = 8192
"#;
        let config: Config = toml::from_str(toml_str).unwrap();

        assert_eq!(config.conductor.max_rounds, 10);
        assert_eq!(config.conductor.max_tokens, 8192);
    }

    #[test]
    fn test_codex_backend_defaults() {
        let config = Config::default();
        let codex = config.backends.get("codex").unwrap();

        assert!(codex.enabled);
        assert_eq!(codex.command, Some("codex".to_string()));
        assert_eq!(codex.skip_lines, 0);
    }

    #[test]
    fn test_gemini_backend_defaults() {
        let config = Config::default();
        let gemini = config.backends.get("gemini").unwrap();

        assert!(gemini.enabled);
        assert_eq!(gemini.command, Some("npx".to_string()));
        assert_eq!(gemini.skip_lines, 1);
    }

    #[test]
    fn test_claude_backend_defaults() {
        let config = Config::default();
        let claude = config.backends.get("claude").unwrap();

        assert!(claude.enabled);
        assert_eq!(claude.command, Some("claude".to_string())); // CLI mode by default
        assert!(claude.api_key_env.is_none()); // No API key needed for CLI
        assert!(claude.model.is_none()); // Uses Claude Code's default
    }

    #[test]
    fn test_hunt_task_defaults() {
        let config = Config::default();
        let hunt = config.tasks.get("hunt").unwrap();

        assert_eq!(
            hunt.description,
            Some("Find bugs and code issues".to_string())
        );
        assert!(hunt.backends.contains(&"codex".to_string()));
        assert!(!hunt.prompts.is_empty());
    }

    #[test]
    fn test_parse_minimal_config() {
        let toml_str = r#"
[defaults]
parallel = false
timeout = 60
"#;
        let config: Config = toml::from_str(toml_str).unwrap();

        assert!(!config.defaults.parallel);
        assert_eq!(config.defaults.timeout, 60);
        assert!(config.backends.is_empty());
        assert!(config.tasks.is_empty());
    }

    #[test]
    fn test_parse_custom_backend() {
        let toml_str = r#"
[backends.custom]
enabled = true
command = "my-llm"
args = ["--flag", "value"]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let custom = config.backends.get("custom").unwrap();

        assert!(custom.enabled);
        assert_eq!(custom.command, Some("my-llm".to_string()));
        assert_eq!(custom.args, vec!["--flag", "value"]);
    }

    #[test]
    fn test_parse_custom_task() {
        let toml_str = r#"
[tasks.review]
description = "Code review"
backends = ["codex", "gemini"]

[[tasks.review.prompts]]
name = "style"
prompt = "Check code style"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let review = config.tasks.get("review").unwrap();

        assert_eq!(review.description, Some("Code review".to_string()));
        assert_eq!(review.backends, vec!["codex", "gemini"]);
        assert_eq!(review.prompts.len(), 1);
        assert_eq!(review.prompts[0].name, "style");
    }

    #[test]
    fn test_backend_config_defaults() {
        let toml_str = r#"
[backends.minimal]
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        let minimal = config.backends.get("minimal").unwrap();

        // Check default values are applied
        assert!(minimal.enabled); // default_enabled
        assert!(minimal.args.is_empty()); // default empty vec
        assert_eq!(minimal.skip_lines, 0); // default 0
    }

    #[test]
    fn test_config_serialization_roundtrip() {
        let original = Config::default();
        let serialized = toml::to_string_pretty(&original).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();

        // Check key fields survived roundtrip
        assert_eq!(original.defaults.parallel, deserialized.defaults.parallel);
        assert_eq!(original.defaults.timeout, deserialized.defaults.timeout);
        assert_eq!(original.backends.len(), deserialized.backends.len());
        assert_eq!(original.tasks.len(), deserialized.tasks.len());
    }

    #[test]
    fn test_command_wrapper_config() {
        let toml_str = r#"
[defaults]
command_wrapper = "nix-shell --run '{cmd}'"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();

        assert_eq!(
            config.defaults.command_wrapper,
            Some("nix-shell --run '{cmd}'".to_string())
        );
    }

    #[test]
    fn test_command_wrapper_default_none() {
        let config = Config::default();
        assert!(config.defaults.command_wrapper.is_none());
    }

    #[test]
    fn test_command_wrapper_docker_example() {
        let toml_str = r#"
[defaults]
command_wrapper = "docker exec dev sh -c '{cmd}'"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();

        assert_eq!(
            config.defaults.command_wrapper,
            Some("docker exec dev sh -c '{cmd}'".to_string())
        );
    }

    #[test]
    fn test_deny_unknown_fields() {
        let toml_str = r#"
[defaults]
parallel = true
typo_field = "oops"
"#;
        let result = toml::from_str::<Config>(toml_str);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("unknown field"),
            "Error should mention unknown field: {}",
            err
        );
    }

    #[test]
    fn test_deep_merge_scalar_override() {
        let mut base: toml::Value = toml::from_str(
            r#"
[defaults]
timeout = 300
parallel = true
"#,
        )
        .unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
[defaults]
timeout = 60
"#,
        )
        .unwrap();
        deep_merge(&mut base, overlay);
        let config: Config = base.try_into().unwrap();
        assert_eq!(config.defaults.timeout, 60);
        assert!(config.defaults.parallel); // not overridden, stays true
    }

    #[test]
    fn test_deep_merge_boolean_override() {
        let mut base: toml::Value = toml::from_str(
            r#"
[defaults]
parallel = true
timeout = 300
"#,
        )
        .unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
[defaults]
parallel = false
"#,
        )
        .unwrap();
        deep_merge(&mut base, overlay);
        let config: Config = base.try_into().unwrap();
        assert!(!config.defaults.parallel); // false overrides true
        assert_eq!(config.defaults.timeout, 300); // unchanged
    }

    #[test]
    fn test_deep_merge_hashmap_add() {
        let mut base: toml::Value = toml::Value::try_from(Config::default()).unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
[backends.custom]
enabled = true
command = "my-llm"
"#,
        )
        .unwrap();
        deep_merge(&mut base, overlay);
        let config: Config = base.try_into().unwrap();
        // New backend added
        assert!(config.backends.contains_key("custom"));
        // Existing backends preserved
        assert!(config.backends.contains_key("codex"));
        assert!(config.backends.contains_key("claude"));
        assert!(config.backends.contains_key("gemini"));
        assert!(config.backends.contains_key("ollama"));
    }

    #[test]
    fn test_deep_merge_hashmap_override() {
        let mut base: toml::Value = toml::Value::try_from(Config::default()).unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
[backends.ollama]
enabled = false
command = "http://remote:11434"
model = "mistral"
"#,
        )
        .unwrap();
        deep_merge(&mut base, overlay);
        let config: Config = base.try_into().unwrap();
        let ollama = config.backends.get("ollama").unwrap();
        assert!(!ollama.enabled);
        assert_eq!(ollama.command, Some("http://remote:11434".to_string()));
        assert_eq!(ollama.model, Some("mistral".to_string()));
        // Other backends untouched
        assert!(config.backends.get("codex").unwrap().enabled);
    }

    #[test]
    fn test_deep_merge_partial_config() {
        let mut base: toml::Value = toml::Value::try_from(Config::default()).unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
[defaults]
timeout = 60
"#,
        )
        .unwrap();
        deep_merge(&mut base, overlay);
        let config: Config = base.try_into().unwrap();
        assert_eq!(config.defaults.timeout, 60);
        // Everything else from defaults preserved
        assert!(config.defaults.parallel);
        assert!(!config.backends.is_empty());
        assert!(!config.tasks.is_empty());
    }

    #[test]
    fn test_deep_merge_vec_replace() {
        let mut base: toml::Value = toml::Value::try_from(Config::default()).unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
[backends.codex]
args = ["exec", "--json", "-s", "full-auto"]
"#,
        )
        .unwrap();
        deep_merge(&mut base, overlay);
        let config: Config = base.try_into().unwrap();
        let codex = config.backends.get("codex").unwrap();
        // Args replaced entirely, not appended
        assert_eq!(codex.args, vec!["exec", "--json", "-s", "full-auto"]);
    }

    #[test]
    fn test_deep_merge_empty_overlay() {
        let mut base: toml::Value = toml::Value::try_from(Config::default()).unwrap();
        let overlay: toml::Value = toml::from_str("").unwrap();
        deep_merge(&mut base, overlay);
        let config: Config = base.try_into().unwrap();
        // Nothing changes
        assert!(config.defaults.parallel);
        assert_eq!(config.defaults.timeout, 300);
        assert_eq!(config.backends.len(), 4);
    }

    #[test]
    fn test_load_config_from_paths_no_files() {
        let tmp = tempfile::tempdir().unwrap();
        let config = load_config_from_paths(tmp.path(), None, None).unwrap();
        // Should return defaults when no config files exist
        assert!(config.defaults.parallel);
        assert_eq!(config.defaults.timeout, 300);
        assert_eq!(config.backends.len(), 4);
    }

    #[test]
    fn test_load_config_from_paths_project_only() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("lok.toml"),
            r#"
[defaults]
timeout = 60
"#,
        )
        .unwrap();
        let config = load_config_from_paths(tmp.path(), None, None).unwrap();
        assert_eq!(config.defaults.timeout, 60);
        // Defaults for everything else
        assert!(config.defaults.parallel);
        assert_eq!(config.backends.len(), 4);
    }

    #[test]
    fn test_load_config_from_paths_three_layers() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();

        // User config: set timeout
        let user_dir = home.path().join(".config/lok");
        fs::create_dir_all(&user_dir).unwrap();
        fs::write(
            user_dir.join("lok.toml"),
            r#"
[defaults]
timeout = 120

[backends.custom]
enabled = true
command = "user-backend"
"#,
        )
        .unwrap();

        // Project config: override timeout, add parallel=false
        fs::write(
            cwd.path().join("lok.toml"),
            r#"
[defaults]
timeout = 30
parallel = false
"#,
        )
        .unwrap();

        let config = load_config_from_paths(cwd.path(), Some(home.path()), None).unwrap();

        // Project wins for timeout
        assert_eq!(config.defaults.timeout, 30);
        // Project wins for parallel
        assert!(!config.defaults.parallel);
        // User's custom backend preserved
        assert!(config.backends.contains_key("custom"));
        // Default backends still there
        assert!(config.backends.contains_key("codex"));
    }

    #[test]
    fn test_load_config_from_paths_explicit_bypasses() {
        let tmp = tempfile::tempdir().unwrap();
        let explicit = tmp.path().join("custom.toml");
        fs::write(
            &explicit,
            r#"
[defaults]
timeout = 999
"#,
        )
        .unwrap();

        let config = load_config_from_paths(tmp.path(), None, Some(&explicit)).unwrap();

        assert_eq!(config.defaults.timeout, 999);
        // Explicit path still merges with defaults - not hollow
        assert_eq!(config.backends.len(), 4);
        assert!(config.defaults.parallel);
    }

    #[test]
    fn test_load_config_from_paths_user_parse_error() {
        let home = tempfile::tempdir().unwrap();
        let cwd = tempfile::tempdir().unwrap();

        let user_dir = home.path().join(".config/lok");
        fs::create_dir_all(&user_dir).unwrap();
        fs::write(user_dir.join("lok.toml"), "invalid [[ toml {{").unwrap();

        let result = load_config_from_paths(cwd.path(), Some(home.path()), None);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Error parsing"),
            "Error should mention file: {}",
            err
        );
    }

    #[test]
    fn test_tensorzero_config_serialization_roundtrip() {
        let original = Config {
            tensorzero: Some(TensorZeroConfig {
                endpoint: "https://gateway.local:3000".to_string(),
                default_model: "tensorzero::function_name::loker_d1_openai".to_string(),
                api_key_env: Some("TENSORZERO_API_KEY".to_string()),
                timeout_secs: 90,
                retry_policy: RetryPolicyConfig {
                    max_retries: 3,
                    delay_ms: 500,
                },
            }),
            ..Config::default()
        };

        let serialized = toml::to_string_pretty(&original).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();

        let original_tz = original.tensorzero.as_ref().unwrap();
        let round_tz = deserialized.tensorzero.as_ref().unwrap();
        assert_eq!(original_tz.endpoint, round_tz.endpoint);
        assert_eq!(original_tz.default_model, round_tz.default_model);
        assert_eq!(original_tz.api_key_env, round_tz.api_key_env);
        assert_eq!(original_tz.timeout_secs, round_tz.timeout_secs);
        assert_eq!(
            original_tz.retry_policy.max_retries,
            round_tz.retry_policy.max_retries
        );
        assert_eq!(
            original_tz.retry_policy.delay_ms,
            round_tz.retry_policy.delay_ms
        );
    }

    #[test]
    fn test_tensorzero_missing_endpoint_fails() {
        let toml_str = r#"
[tensorzero]
default_model = "loker_d1_openai"
"#;
        let result = toml::from_str::<Config>(toml_str);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("endpoint"),
            "Error should mention endpoint: {}",
            err
        );
    }

    #[test]
    fn test_tensorzero_invalid_url_fails() {
        let toml_str = r#"
[tensorzero]
endpoint = "not a url"
default_model = "loker_d1_openai"
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        let result = cfg.tensorzero.as_ref().unwrap().validate();
        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(
            err.contains("endpoint"),
            "Error should mention endpoint: {}",
            err
        );
    }

    #[test]
    fn test_tensorzero_zero_timeout_fails() {
        let toml_str = r#"
[tensorzero]
endpoint = "https://gateway.local"
default_model = "loker_d1_openai"
timeout_secs = 0
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        let result = cfg.tensorzero.as_ref().unwrap().validate();
        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(
            err.contains("timeout"),
            "Error should mention timeout: {}",
            err
        );
    }

    #[test]
    fn test_tensorzero_to_backend_opts_resolves_env() {
        let var_name = "LOK_TEST_TENSORZERO_KEY_CLO250";
        std::env::set_var(var_name, "secret-token-xyz");

        let cfg = TensorZeroConfig {
            endpoint: "https://gateway.local".to_string(),
            default_model: "loker_d1_openai".to_string(),
            api_key_env: Some(var_name.to_string()),
            timeout_secs: 60,
            retry_policy: RetryPolicyConfig::default(),
        };

        let opts = cfg.to_backend_opts().expect("env var is set");
        assert_eq!(opts.endpoint, "https://gateway.local");
        assert_eq!(opts.model, "loker_d1_openai");
        assert_eq!(opts.api_key.as_deref(), Some("secret-token-xyz"));
        assert_eq!(opts.timeout, std::time::Duration::from_secs(60));

        std::env::remove_var(var_name);
    }
}
