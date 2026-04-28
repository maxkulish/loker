//! `Family` enum and `family_of` / `enforce_cross_family` helpers.
//!
//! Single source of truth for mapping a backend identifier to a model family.
//! This module is intentionally thin — it contains no backend trait impls,
//! no HTTP logic, and no config parsing. It is a pure lookup table that
//! downstream code (strategies, aggregators, phase runners) can call
//! without constructing a backend instance.
//!
//! ## Source of truth decision (CLO-265 AC9)
//!
//! The D1 round-trip spike (`docs/spikes/2026-04-25-tensorzero-roundtrip.md` §3)
//! proved the TensorZero gateway response carries no provider metadata. The
//! only stable family signal is the **loker-side backend naming convention**:
//! either the raw `backend_id` string for non-gateway backends, or the
//! suffix after `tensorzero/` (or after the `loker_<purpose>_` prefix) for
//! gateway-routed backends. This keeps the lookup static, deterministic, and
//! testable without a live gateway.
//!
//! When adding a new known vendor, add a closed variant to `Family` and
//! update `family_of`. Do **not** use `Family::Other("anthropic")` — that
//! is what `Family::Anthropic` is for. `Other` is reserved for self-hosted
//! or future vendors that have not yet been promoted to a closed variant.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Known model families, plus an `Other` escape hatch for self-hosted or
/// future vendors.
///
/// `#[non_exhaustive]` so a new closed variant can be added without
/// breaking downstream `match` arms that handle `Other` as a fallback.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Anthropic,
    Google,
    OpenAI,
    Zhipu,
    Local,
    Other(String),
}

impl Family {
    /// Discriminant label used in schema fields (`Attempt.family`) and in
    /// error text. Returns `"other"` for the `Other` variant regardless of
    /// the inner payload — the payload is not part of the discriminant.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Anthropic => "anthropic",
            Self::Google => "google",
            Self::OpenAI => "openai",
            Self::Zhipu => "zhipu",
            Self::Local => "local",
            Self::Other(_) => "other",
        }
    }
}

impl fmt::Display for Family {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Resolve a backend identifier to its `Family`.
///
/// ## Lookup table
///
/// | Input | Output |
/// |-------|--------|
/// | `"claude"` | `Family::Anthropic` |
/// | `"gemini"` | `Family::Google` |
/// | `"openai"`, `"codex"` | `Family::OpenAI` |
/// | `"ollama"` | `Family::Local` |
/// | `"bedrock"` | `Family::Other("bedrock")` |
/// | `"tensorzero"` | `Family::Other("tensorzero")` |
/// | `"tensorzero/<anything>"` | `family_of("<anything>")` (suffix-only passthrough) |
/// | `"loker_<purpose>_<family>"` | parse suffix: `openai`→OpenAI, `anthropic`→Anthropic, `google`/`gemini`→Google, `local`/`ollama`→Local, otherwise `Other` |
/// | any other string | `Family::Other(string.into())` |
///
/// The `tensorzero/` prefix stripping lets the workflow loader (T-029) pass
/// `backend_id = "tensorzero/loker_review_anthropic"` and receive the
/// correct family without knowing whether the identifier is a raw backend
/// or a gateway-routed one.
pub fn family_of(backend_id: &str) -> Family {
    if let Some(suffix) = backend_id.strip_prefix("tensorzero/") {
        return family_of(suffix);
    }

    if let Some(suffix) = backend_id.strip_prefix("loker_") {
        // loker_<purpose>_<family> — the family is the last underscore-separated token
        if let Some(pos) = suffix.rfind('_') {
            let family_token = &suffix[pos + 1..];
            return family_of_suffix(family_token);
        }
    }

    match backend_id {
        "claude" => Family::Anthropic,
        "gemini" => Family::Google,
        "openai" | "codex" => Family::OpenAI,
        "ollama" => Family::Local,
        "bedrock" => Family::Other("bedrock".into()),
        "tensorzero" => Family::Other("tensorzero".into()),
        other => Family::Other(other.into()),
    }
}

fn family_of_suffix(token: &str) -> Family {
    match token {
        "openai" => Family::OpenAI,
        "anthropic" => Family::Anthropic,
        "google" | "gemini" => Family::Google,
        "local" | "ollama" => Family::Local,
        other => Family::Other(other.into()),
    }
}

/// Errors that can surface at phase-runner level, **above** individual
/// strategy execution. These are distinct from `StrategyError` because
/// they describe invariants the phase runner enforces (e.g. cross-family
/// diversity) rather than runtime backend or rendering failures inside a
/// single strategy.
///
/// `#[non_exhaustive]` so new phase-level invariants can be added without
/// breaking downstream consumers.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum PhaseError {
    #[error("family overlap: found {family} on {count} backends")]
    FamilyOverlap { family: Family, count: usize },
}

/// Verify that every backend in `targets` resolves to a *different*
/// family. If any family appears on more than one backend, return
/// `Err(PhaseError::FamilyOverlap)`. An empty slice is allowed.
///
/// Two backends with `Family::Other("a")` and `Family::Other("b")` are
/// considered different families; two backends with `Family::Other("x")`
/// and `Family::Other("x")` are considered the same family.
pub fn enforce_cross_family(targets: &[&str]) -> Result<(), PhaseError> {
    use std::collections::HashMap;

    let mut counts: HashMap<Family, usize> = HashMap::new();
    for t in targets {
        let family = family_of(t);
        *counts.entry(family).or_insert(0) += 1;
    }

    for (family, count) in counts {
        if count > 1 {
            return Err(PhaseError::FamilyOverlap { family, count });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_of_claude() {
        assert_eq!(family_of("claude"), Family::Anthropic);
    }

    #[test]
    fn family_of_gemini() {
        assert_eq!(family_of("gemini"), Family::Google);
    }

    #[test]
    fn family_of_openai() {
        assert_eq!(family_of("openai"), Family::OpenAI);
    }

    #[test]
    fn family_of_codex() {
        assert_eq!(family_of("codex"), Family::OpenAI);
    }

    #[test]
    fn family_of_ollama() {
        assert_eq!(family_of("ollama"), Family::Local);
    }

    #[test]
    fn family_of_bedrock() {
        assert_eq!(family_of("bedrock"), Family::Other("bedrock".into()));
    }

    #[test]
    fn family_of_tensorzero() {
        assert_eq!(family_of("tensorzero"), Family::Other("tensorzero".into()));
    }

    #[test]
    fn family_of_tensorzero_function_name() {
        // tensorzero/<anything> passes through suffix only
        assert_eq!(family_of("tensorzero/loker_d1_openai"), Family::OpenAI);
        assert_eq!(
            family_of("tensorzero/loker_review_anthropic"),
            Family::Anthropic
        );
    }

    #[test]
    fn family_of_tensorzero_unknown_suffix() {
        assert_eq!(
            family_of("tensorzero/deepseek_chat"),
            Family::Other("deepseek_chat".into())
        );
    }

    #[test]
    fn family_of_loker_prefix_openai() {
        assert_eq!(family_of("loker_d1_openai"), Family::OpenAI);
    }

    #[test]
    fn family_of_loker_prefix_anthropic() {
        assert_eq!(family_of("loker_review_anthropic"), Family::Anthropic);
    }

    #[test]
    fn family_of_loker_prefix_google() {
        assert_eq!(family_of("loker_judge_google"), Family::Google);
    }

    #[test]
    fn family_of_loker_prefix_gemini() {
        assert_eq!(family_of("loker_judge_gemini"), Family::Google);
    }

    #[test]
    fn family_of_loker_prefix_local() {
        assert_eq!(family_of("loker_verify_local"), Family::Local);
    }

    #[test]
    fn family_of_loker_prefix_ollama() {
        assert_eq!(family_of("loker_verify_ollama"), Family::Local);
    }

    #[test]
    fn family_of_unknown() {
        assert_eq!(family_of("deepseek"), Family::Other("deepseek".into()));
    }

    #[test]
    fn family_of_empty_string() {
        assert_eq!(family_of(""), Family::Other("".into()));
    }

    #[test]
    fn family_of_tensorzero_slash_only() {
        assert_eq!(family_of("tensorzero/"), Family::Other("".into()));
    }

    #[test]
    fn family_of_loker_no_suffix() {
        assert_eq!(family_of("loker_"), Family::Other("loker_".into()));
    }

    #[test]
    fn display_anthropic() {
        assert_eq!(Family::Anthropic.to_string(), "anthropic");
    }

    #[test]
    fn display_other() {
        assert_eq!(Family::Other("deepseek".into()).to_string(), "other");
    }

    #[test]
    fn as_str_openai() {
        assert_eq!(Family::OpenAI.as_str(), "openai");
    }

    #[test]
    fn as_str_other() {
        assert_eq!(Family::Other("bedrock".into()).as_str(), "other");
    }

    #[test]
    fn enforce_mixed_families_ok() {
        assert!(enforce_cross_family(&["claude", "gemini", "openai"]).is_ok());
    }

    #[test]
    fn enforce_all_anthropic_rejected() {
        let result = enforce_cross_family(&["claude", "loker_review_anthropic"]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(
            err.to_string(),
            "family overlap: found anthropic on 2 backends"
        );
    }

    #[test]
    fn enforce_three_same_family() {
        let result =
            enforce_cross_family(&["claude", "loker_d1_anthropic", "loker_review_anthropic"]);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(
            err.to_string(),
            "family overlap: found anthropic on 3 backends"
        );
    }

    #[test]
    fn enforce_empty_slice_ok() {
        assert!(enforce_cross_family(&[] as &[&str]).is_ok());
    }

    #[test]
    fn enforce_single_backend_ok() {
        assert!(enforce_cross_family(&["claude"]).is_ok());
    }

    #[test]
    fn enforce_distinct_other_ok() {
        assert!(enforce_cross_family(&["deepseek", "bedrock"]).is_ok());
    }

    #[test]
    fn enforce_same_other_rejected() {
        let result = enforce_cross_family(&["bedrock", "bedrock"]);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "family overlap: found other on 2 backends"
        );
    }

    #[test]
    fn enforce_two_distinct_others_ok() {
        assert!(enforce_cross_family(&["bedrock", "tensorzero"]).is_ok());
    }
}
