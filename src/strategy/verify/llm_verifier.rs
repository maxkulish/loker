//! LLM-based verify hook.
//!
//! Delegates the verification decision to a backend (LLM) and parses a
//! deterministic yes/no verdict from the response.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;

use crate::backend::Backend;
use crate::strategy::verify::{FailureReason, VerifyContext, VerifyError, VerifyHook, VerifyResult};

/// Concrete verify hook that delegates to a backend and parses a
/// deterministic yes/no verdict from the backend response.
pub struct LLMVerifier {
    /// Identifier used for observability/debugging.
    pub backend: String,
    backend_client: Arc<dyn Backend>,
    /// Optional model override passed to the backend.
    pub model: Option<String>,
    /// Prompt template used for verification. `{candidate}` is replaced with
    /// the candidate text under test; any `{key}` present in `params`
    /// is also substituted.
    pub prompt_template: String,
    /// Optional system-level context prepended to the candidate prompt.
    pub system_prompt: Option<String>,
    /// Temperature hint used when available. `0.0` is deterministic default.
    pub temperature: f32,
    params: HashMap<String, String>,
}

impl LLMVerifier {
    pub const DEFAULT_TEMPERATURE: f32 = 0.0;

    /// Construct a verifier bound to a backend object.
    pub fn new(
        backend: impl Into<String>,
        backend_client: Arc<dyn Backend>,
        prompt_template: impl Into<String>,
    ) -> Self {
        Self {
            backend: backend.into(),
            backend_client,
            model: None,
            prompt_template: prompt_template.into(),
            system_prompt: None,
            temperature: Self::DEFAULT_TEMPERATURE,
            params: HashMap::new(),
        }
    }

    /// Set deterministic temperature hint (used where backend support exists).
    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = temperature;
        self
    }

    /// Set a model override.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set system prompt prepended to candidate prompt.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Add one user-supplied template parameter.
    pub fn with_param(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }

    fn rendered_prompt(&self, candidate: &str) -> String {
        let mut prompt = self.prompt_template.clone();

        // Sort params by key length descending so that longer, more specific
        // keys (e.g. {env_name}) are replaced before shorter substrings (e.g. {env}).
        // HashMap iteration order is non-deterministic, so we must sort explicitly
        // for reproducible prompt rendering.
        let mut sorted_params: Vec<(&String, &String)> = self.params.iter().collect();
        sorted_params.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
        for (key, value) in sorted_params {
            let needle = format!("{{{key}}}");
            prompt = prompt.replace(&needle, value);
        }

        let prompt = prompt.replace("{candidate}", candidate);

        match &self.system_prompt {
            Some(system) => format!("{system}\n\n{prompt}"),
            None => prompt,
        }
    }

    fn parse_response(raw: &str) -> VerifyResult {
        let first = raw
            .split_whitespace()
            .next()
            .map(|token| token.trim().trim_matches(|c: char| !c.is_alphanumeric()))
            .map(|token| token.to_ascii_lowercase())
            .unwrap_or_default();

        if first == "yes" {
            return VerifyResult::pass();
        }

        if first == "no" {
            return VerifyResult::fail_with(FailureReason::new("no").with_stdout(raw.to_string()));
        }

        VerifyResult::fail_with(
            FailureReason::new("unparseable verifier response").with_stdout(raw.to_string()),
        )
    }
}

#[async_trait]
impl VerifyHook for LLMVerifier {
    fn name(&self) -> &str {
        "LLMVerifier"
    }

    async fn verify(&self, ctx: &VerifyContext) -> Result<VerifyResult, VerifyError> {
        let prompt = self.rendered_prompt(&ctx.stdout);

        match self
            .backend_client
            .query(&prompt, Path::new("."), self.model.as_deref())
            .await
        {
            Ok(query) => Ok(Self::parse_response(&query.stdout)),
            Err(err) => Err(VerifyError::new(format!("backend error: {err}"))),
        }
    }
}
