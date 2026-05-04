//! Price table for computing LLM call costs from static prices.toml.
//!
//! Provides `PriceTable` that loads `<backend>:<model>` keyed prices
//! from a TOML file and computes USD cost for given token counts.
//! Parse errors are non-fatal — the table logs a warning and returns
//! empty, so every backend gets `cost_unknown`.

use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Price per million tokens for a single model.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelPrice {
    /// USD per million input tokens.
    pub input_per_mtok: f64,
    /// USD per million output tokens.
    pub output_per_mtok: f64,
}

/// Price table loaded from `prices.toml`.
///
/// Keys are `"<backend>:<model>"` — for example `"anthropic:claude-opus-4-7"`.
#[derive(Debug, Clone, Default)]
pub struct PriceTable {
    prices: HashMap<String, ModelPrice>,
}

impl PriceTable {
    /// Load prices from a TOML file at `path`.
    ///
    /// If the file doesn't exist, the TOML is malformed, or deserialization
    /// fails, a warning is emitted to stderr and an empty table is returned.
    /// This ensures the summary is always emitted even when pricing data is
    /// unavailable — backends simply get `cost_usd: None`.
    pub fn load(path: &Path) -> Self {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("warn: prices.toml not readable: {e}; all costs will be unknown");
                return Self::default();
            }
        };

        let raw: HashMap<String, ModelPrice> = match toml::from_str(&content) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("warn: prices.toml parse error: {e}; all costs will be unknown");
                return Self::default();
            }
        };

        PriceTable { prices: raw }
    }

    /// Look up the price for a `(backend, model)` pair.
    pub fn lookup(&self, backend: &str, model: &str) -> Option<&ModelPrice> {
        let key = format!("{backend}:{model}");
        self.prices.get(&key)
    }

    /// Compute the cost in USD for a given number of tokens.
    ///
    /// Returns `None` if the (backend, model) pair is not in the table.
    pub fn compute_cost(
        &self,
        backend: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
    ) -> Option<f64> {
        let price = self.lookup(backend, model)?;
        let input_cost = (input_tokens as f64 / 1_000_000.0) * price.input_per_mtok;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * price.output_per_mtok;
        Some(input_cost + output_cost)
    }

    /// Returns the number of entries in the price table.
    pub fn len(&self) -> usize {
        self.prices.len()
    }

    /// Returns true if the price table is empty.
    pub fn is_empty(&self) -> bool {
        self.prices.is_empty()
    }

    /// Load the default price table compiled into the binary from `docs/prices.toml`.
    ///
    /// This is the primary source of pricing data for production use. It is always
    /// available (no filesystem dependency) and guaranteed to parse correctly at
    /// compile time. Unknown models still return `None` from [`lookup`]/[`compute_cost`].
    ///
    /// To override prices for testing, construct a [`PriceTable`] via file [`load`]
    /// and pass it to [`crate::SummaryWriter::write_summary`] via a custom wrapper
    /// (or create a test-specific prices.toml in the run dir).
    pub fn builtin() -> Self {
        let content = include_str!("../../docs/prices.toml");
        #[allow(clippy::unwrap_used)]
        let raw: HashMap<String, ModelPrice> = toml::from_str(content)
            .expect("docs/prices.toml must be valid TOML (checked at compile time)");
        PriceTable { prices: raw }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_table_returns_none() {
        let table = PriceTable::default();
        assert!(table.lookup("anthropic", "claude-opus-4-7").is_none());
        assert!(table
            .compute_cost("anthropic", "claude-opus-4-7", 100, 200)
            .is_none());
    }

    #[test]
    fn missing_file_returns_empty() {
        let table = PriceTable::load(Path::new("/nonexistent/prices.toml"));
        assert!(table.is_empty());
    }

    #[test]
    fn load_valid_prices() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("prices.toml");
        std::fs::write(
            &path,
            r#"
["anthropic:claude-opus-4-7"]
input_per_mtok = 15.0
output_per_mtok = 75.0

["openai:gpt-5"]
input_per_mtok = 10.0
output_per_mtok = 40.0
"#,
        )
        .unwrap();

        let table = PriceTable::load(&path);
        assert_eq!(table.len(), 2);
        let price = table.lookup("anthropic", "claude-opus-4-7").unwrap();
        assert!((price.input_per_mtok - 15.0).abs() < f64::EPSILON);
        assert!((price.output_per_mtok - 75.0).abs() < f64::EPSILON);
    }

    #[test]
    fn compute_cost_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("prices.toml");
        std::fs::write(
            &path,
            r#"
["mock:mock-model"]
input_per_mtok = 10.0
output_per_mtok = 30.0
"#,
        )
        .unwrap();

        let table = PriceTable::load(&path);
        // 1M input tokens @ $10 = $10, 2M output tokens @ $30 = $60 → $70
        let cost = table
            .compute_cost("mock", "mock-model", 1_000_000, 2_000_000)
            .unwrap();
        assert!((cost - 70.0).abs() < 0.001);
    }

    #[test]
    fn zero_tokens_zero_cost() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("prices.toml");
        std::fs::write(
            &path,
            r#"
["m:gpt-4"]
input_per_mtok = 30.0
output_per_mtok = 60.0
"#,
        )
        .unwrap();

        let table = PriceTable::load(&path);
        let cost = table.compute_cost("m", "gpt-4", 0, 0).unwrap();
        assert!((cost - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn malformed_toml_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bad.toml");
        std::fs::write(&path, "this is { not valid toml").unwrap();
        let table = PriceTable::load(&path);
        assert!(table.is_empty());
    }
}
