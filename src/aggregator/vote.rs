//! `Aggregator::Vote` implementation.
//!
//! Pure, synchronous vote counting over parallel branch outcomes.
//! No secondary backend calls, no async, no I/O.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::family::{family_of, Family};

use super::{AggregatedArtifact, BranchOutcome};

/// How a ballot is normalised and interpreted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BallotSchema {
    /// Free text: each backend returns prose; normalise before bucketing.
    FreeText,
}

/// How to resolve a tie when no strict majority exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TieBreak {
    /// Pick the candidate whose backend family matches the given family.
    /// If multiple candidates match, first occurrence in arrival order wins.
    ClosestToFamily(Family),
    /// Deterministic shuffle from a fixed seed.
    Random { seed: u64 },
    /// Pick the candidate whose successful response arrived first.
    FirstResponder,
}

/// Config payload for the Vote aggregator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VoteConfig {
    pub ballot_schema: BallotSchema,
    pub tie_break: TieBreak,
    /// Number of abstentions (errors + malformed answers) that triggers
    /// `QuorumLost`. Fires when **strictly more** than `abstain_threshold`
    /// are abstentions.
    pub abstain_threshold: usize,
}

/// A single candidate vote after normalisation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoteCandidate {
    pub backend_id: String,
    pub family: String,
    pub normalised: String,
    pub original: String,
    pub arrival_order: usize,
}

/// Result of a vote aggregation, including metadata for traceability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoteResult {
    /// The winning text (normalised key).
    pub winner: String,
    /// Sorted descending by count for snapshot determinism.
    pub vote_counts: Vec<(String, usize)>,
    pub abstain_count: usize,
    pub total_branches: usize,
    pub tie_broken: bool,
    pub tie_break_rule: String,
}

/// Errors specific to Vote aggregation.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VoteError {
    #[error("quorum lost: {abstains} abstentions exceed threshold {threshold}")]
    QuorumLost { abstains: usize, threshold: usize },

    #[error("no candidates available")]
    NoCandidates,
}

/// Normalise a ballot text for comparison.
pub fn normalise_ballot(text: &str) -> String {
    text.trim().to_lowercase()
}

/// Aggregate vote outcomes from parallel branches.
///
/// Returns the aggregated artefact and structured result metadata.
/// Pure synchronous function — no async, no backend calls.
pub fn aggregate_vote(
    branches: &[BranchOutcome],
    config: &VoteConfig,
) -> Result<(AggregatedArtifact, VoteResult), VoteError> {
    let mut abstain_count = 0;
    let mut candidates: Vec<VoteCandidate> = Vec::new();
    let total = branches.len();

    for (arrival_order, branch) in branches.iter().enumerate() {
        match branch {
            BranchOutcome::Success(success) => {
                let normalised = normalise_ballot(&success.output);
                if normalised.is_empty() {
                    abstain_count += 1;
                } else {
                    candidates.push(VoteCandidate {
                        backend_id: success.backend_id.clone(),
                        family: success.family.clone(),
                        normalised,
                        original: success.output.clone(),
                        arrival_order,
                    });
                }
            }
            BranchOutcome::Failure(_) => {
                abstain_count += 1;
            }
        }
    }

    if abstain_count > config.abstain_threshold {
        return Err(VoteError::QuorumLost {
            abstains: abstain_count,
            threshold: config.abstain_threshold,
        });
    }

    if candidates.is_empty() {
        return Err(VoteError::NoCandidates);
    }

    // Count votes by normalised bucket.
    // BTreeMap ensures deterministic iteration order for tie-break determinism.
    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, c) in candidates.iter().enumerate() {
        buckets.entry(c.normalised.clone()).or_default().push(idx);
    }

    let max_votes = buckets.values().map(|v| v.len()).max().unwrap_or(0);
    let winners: Vec<&str> = buckets
        .iter()
        .filter(|(_, v)| v.len() == max_votes)
        .map(|(k, _)| k.as_str())
        .collect();

    let strict_majority = max_votes * 2 > candidates.len();
    let mut result = if strict_majority {
        let winner_text = winners[0];
        VoteResult {
            winner: winner_text.into(),
            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
            abstain_count,
            total_branches: total,
            tie_broken: false,
            tie_break_rule: "none (strict majority)".into(),
        }
    } else {
        let chosen_text = resolve_tie(&winners, &candidates, &buckets, &config.tie_break);
        VoteResult {
            winner: chosen_text,
            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
            abstain_count,
            total_branches: total,
            tie_broken: true,
            tie_break_rule: format_tie_break_rule(&config.tie_break),
        }
    };

    // Sort vote_counts descending for stable output
    result.vote_counts.sort_by_key(|b| std::cmp::Reverse(b.1));

    let text = build_aggregated_text(&result, &candidates, &buckets);
    let artifact = AggregatedArtifact {
        text,
        successful: candidates.len(),
        failed: abstain_count,
    };

    Ok((artifact, result))
}

fn resolve_tie(
    tied_buckets: &[&str],
    candidates: &[VoteCandidate],
    buckets: &BTreeMap<String, Vec<usize>>,
    tie_break: &TieBreak,
) -> String {
    match tie_break {
        TieBreak::FirstResponder => tied_buckets
            .iter()
            .min_by_key(|&&bucket| {
                buckets[bucket]
                    .iter()
                    .map(|&ci| candidates[ci].arrival_order)
                    .min()
                    .unwrap_or(usize::MAX)
            })
            .copied()
            .unwrap_or(tied_buckets[0])
            .to_string(),

        TieBreak::ClosestToFamily(target_family) => {
            let matching: Vec<&str> = tied_buckets
                .iter()
                .copied()
                .filter(|&bucket| {
                    buckets[bucket]
                        .iter()
                        .any(|&ci| family_of(&candidates[ci].backend_id) == *target_family)
                })
                .collect();

            if matching.is_empty() {
                resolve_tie(tied_buckets, candidates, buckets, &TieBreak::FirstResponder)
            } else if matching.len() == 1 {
                matching[0].to_string()
            } else {
                resolve_tie(&matching, candidates, buckets, &TieBreak::FirstResponder)
            }
        }

        TieBreak::Random { seed } => {
            use rand::rngs::StdRng;
            use rand::Rng;
            use rand::SeedableRng;

            let mut rng = StdRng::seed_from_u64(*seed);
            let idx = rng.random_range(0..tied_buckets.len());
            tied_buckets[idx].to_string()
        }
    }
}

fn format_tie_break_rule(tie_break: &TieBreak) -> String {
    match tie_break {
        TieBreak::ClosestToFamily(f) => format!("closest_to_family({})", f),
        TieBreak::Random { seed } => format!("random(seed={})", seed),
        TieBreak::FirstResponder => "first_responder".into(),
    }
}

fn build_aggregated_text(
    result: &VoteResult,
    candidates: &[VoteCandidate],
    buckets: &BTreeMap<String, Vec<usize>>,
) -> String {
    // Pick the winner's original text from the first candidate in the winning bucket.
    let winner_original = buckets
        .get(&result.winner)
        .and_then(|indices| {
            indices
                .first()
                .and_then(|&idx| candidates.get(idx).map(|c| c.original.as_str()))
        })
        .unwrap_or(&result.winner);

    let mut lines = Vec::new();
    lines.push(winner_original.to_string());
    lines.push(String::new());

    // Build deterministic metadata comment block
    lines.push("<!-- loker: Vote aggregator metadata".into());
    lines.push(format!("  winner: {}", sanitize_comment(&result.winner)));
    lines.push(format!("  total_branches: {}", result.total_branches));
    lines.push("  vote_counts:".into());
    for (text, count) in &result.vote_counts {
        lines.push(format!("    {}: {}", sanitize_comment(text), count));
    }
    lines.push(format!("  abstain_count: {}", result.abstain_count));
    lines.push(format!("  tie_broken: {}", result.tie_broken));
    lines.push(format!(
        "  tie_break_rule: {}",
        sanitize_comment(&result.tie_break_rule)
    ));
    lines.push("-->".into());
    lines.push(String::new());

    lines.join("\n")
}

/// Replace `-->` with `-- >` to prevent premature HTML comment closure.
fn sanitize_comment(text: &str) -> String {
    text.replace("-->", "-- >")
}

#[cfg(test)]
mod tests {
    use super::super::BranchSuccess;
    use super::*;

    fn success(backend_id: &str, family: &str, output: &str) -> BranchOutcome {
        BranchOutcome::Success(BranchSuccess {
            backend_id: backend_id.into(),
            family: family.into(),
            index: 1,
            output: output.into(),
        })
    }

    fn failure(backend_id: &str, family: &str, reason: &str) -> BranchOutcome {
        BranchOutcome::Failure(super::super::BranchFailure {
            backend_id: backend_id.into(),
            family: family.into(),
            index: 1,
            reason: reason.into(),
        })
    }

    fn make_config(tie_break: TieBreak) -> VoteConfig {
        VoteConfig {
            ballot_schema: BallotSchema::FreeText,
            tie_break,
            abstain_threshold: 99,
        }
    }

    #[test]
    fn free_text_clear_winner() {
        let branches = vec![
            success("claude", "anthropic", "yes"),
            success("codex", "openai", "yes"),
            success("gemini", "google", "no"),
        ];
        let config = make_config(TieBreak::FirstResponder);
        let (artifact, result) = aggregate_vote(&branches, &config).unwrap();
        assert_eq!(result.winner, "yes");
        assert!(!result.tie_broken);
        assert_eq!(
            result.vote_counts,
            vec![("yes".into(), 2), ("no".into(), 1)]
        );
        assert_eq!(result.abstain_count, 0);
        assert!(artifact.text.contains("yes"));
    }

    #[test]
    fn free_text_tie_first_responder() {
        let branches = vec![
            success("claude", "anthropic", "yes"),
            success("gemini", "google", "no"),
        ];
        let config = make_config(TieBreak::FirstResponder);
        let (_, result) = aggregate_vote(&branches, &config).unwrap();
        assert_eq!(result.winner, "yes");
        assert!(result.tie_broken);
        // "yes" arrives first (index 0), so FirstResponder picks it
    }

    #[test]
    fn free_text_tie_closest_family() {
        let branches = vec![
            success("claude", "anthropic", "a"),
            success("gemini", "google", "b"),
        ];
        let config = make_config(TieBreak::ClosestToFamily(Family::Anthropic));
        let (_, result) = aggregate_vote(&branches, &config).unwrap();
        assert_eq!(result.winner, "a");
        assert!(result.tie_broken);
        assert_eq!(result.tie_break_rule, "closest_to_family(anthropic)");
    }

    #[test]
    fn free_text_tie_random_deterministic() {
        let branches = vec![
            success("claude", "anthropic", "a"),
            success("gemini", "google", "b"),
        ];
        let config = make_config(TieBreak::Random { seed: 42 });
        let (_, result1) = aggregate_vote(&branches, &config).unwrap();
        let (_, result2) = aggregate_vote(&branches, &config).unwrap();
        assert_eq!(result1.winner, result2.winner);
        assert!(result1.tie_broken);
    }

    #[test]
    fn abstain_backend_error() {
        let branches = vec![
            success("claude", "anthropic", "yes"),
            success("gemini", "google", "yes"),
            failure("codex", "openai", "network timeout"),
        ];
        let config = make_config(TieBreak::FirstResponder);
        let (_, result) = aggregate_vote(&branches, &config).unwrap();
        assert_eq!(result.abstain_count, 1);
        assert_eq!(result.vote_counts, vec![("yes".into(), 2)]);
    }

    #[test]
    fn quorum_lost() {
        let branches = vec![
            success("claude", "anthropic", "yes"),
            failure("gemini", "google", "boom"),
            failure("codex", "openai", "network"),
        ];
        let config = VoteConfig {
            ballot_schema: BallotSchema::FreeText,
            tie_break: TieBreak::FirstResponder,
            abstain_threshold: 1,
        };
        let err = aggregate_vote(&branches, &config).unwrap_err();
        assert_eq!(
            err,
            VoteError::QuorumLost {
                abstains: 2,
                threshold: 1,
            }
        );
    }

    #[test]
    fn empty_input() {
        let branches: Vec<BranchOutcome> = vec![];
        let config = make_config(TieBreak::FirstResponder);
        let err = aggregate_vote(&branches, &config).unwrap_err();
        assert_eq!(err, VoteError::NoCandidates);
    }

    #[test]
    fn all_abstain() {
        let branches = vec![
            failure("claude", "anthropic", "boom"),
            failure("gemini", "google", "boom"),
            failure("codex", "openai", "boom"),
        ];
        let config = VoteConfig {
            ballot_schema: BallotSchema::FreeText,
            tie_break: TieBreak::FirstResponder,
            abstain_threshold: 1,
        };
        let err = aggregate_vote(&branches, &config).unwrap_err();
        assert_eq!(
            err,
            VoteError::QuorumLost {
                abstains: 3,
                threshold: 1,
            }
        );
    }

    #[test]
    fn normalise_case() {
        let branches = vec![
            success("a", "anthropic", "YES"),
            success("b", "openai", "yes"),
            success("c", "google", "Yes"),
        ];
        let config = make_config(TieBreak::FirstResponder);
        let (_, result) = aggregate_vote(&branches, &config).unwrap();
        assert_eq!(result.vote_counts.len(), 1);
        assert_eq!(result.vote_counts[0].1, 3);
    }

    #[test]
    fn normalise_whitespace() {
        let branches = vec![
            success("a", "anthropic", "  yes  "),
            success("b", "openai", "yes\n"),
        ];
        let config = make_config(TieBreak::FirstResponder);
        let (_, result) = aggregate_vote(&branches, &config).unwrap();
        assert_eq!(result.vote_counts.len(), 1);
        assert_eq!(result.vote_counts[0].1, 2);
    }

    #[test]
    fn closest_family_no_match_fallback() {
        let branches = vec![
            success("gemini", "google", "a"),
            success("openai", "openai", "b"),
        ];
        let config = make_config(TieBreak::ClosestToFamily(Family::Anthropic));
        let (_, result) = aggregate_vote(&branches, &config).unwrap();
        // No match for Anthropic: falls back to FirstResponder
        assert!(result.tie_broken);
    }

    #[test]
    fn closest_family_multiple_matching_buckets() {
        let branches = vec![
            success("claude", "anthropic", "a"),
            success("gemini", "google", "b"),
            success("loker_d1_anthropic", "anthropic", "a"),
        ];
        let config = make_config(TieBreak::ClosestToFamily(Family::Anthropic));
        let (_, result) = aggregate_vote(&branches, &config).unwrap();
        // "a" has two Anthropic candidates, "b" has zero.
        // ClosestToFamily matches "a" uniquely, so no need for fallback.
        assert_eq!(result.winner, "a");
    }

    #[test]
    fn closest_family_multiple_buckets_match() {
        // Tie between "a" (anthropic + google) and "b" (anthropic + openai)
        // When both tied buckets contain Anthropic, FirstResponder fallback
        // among the matching subset should pick the one arriving first.
        let branches = vec![
            success("claude", "anthropic", "a"),
            success("gemini", "google", "a"),
            success("loker_d1_anthropic", "anthropic", "b"),
            success("openai", "openai", "b"),
        ];
        let config = make_config(TieBreak::ClosestToFamily(Family::Anthropic));
        let (_, result) = aggregate_vote(&branches, &config).unwrap();
        // Both "a" and "b" have Anthropic candidates; tie -> FirstResponder
        // picks the bucket whose first candidate arrived earliest.
        // "a" arrives at 0, "b" arrives at 2.
        assert_eq!(result.winner, "a");
    }

    #[test]
    fn empty_ballot_counts_as_abstain() {
        let branches = vec![
            success("a", "anthropic", ""),
            success("b", "openai", "yes"),
            success("c", "google", "yes"),
        ];
        let config = make_config(TieBreak::FirstResponder);
        let (_, result) = aggregate_vote(&branches, &config).unwrap();
        assert_eq!(result.abstain_count, 1);
        assert_eq!(result.winner, "yes");
    }

    #[test]
    fn whitespace_only_ballot_counts_as_abstain() {
        let branches = vec![
            success("a", "anthropic", "   "),
            success("b", "openai", "yes"),
            success("c", "google", "yes"),
        ];
        let config = make_config(TieBreak::FirstResponder);
        let (_, result) = aggregate_vote(&branches, &config).unwrap();
        assert_eq!(result.abstain_count, 1);
        assert_eq!(result.winner, "yes");
    }

    #[test]
    fn vote_counts_sorted_descending() {
        let branches = vec![
            success("a", "anthropic", "yes"),
            success("b", "openai", "yes"),
            success("c", "google", "no"),
            success("d", "zhipu", "maybe"),
        ];
        let config = make_config(TieBreak::FirstResponder);
        let (_, result) = aggregate_vote(&branches, &config).unwrap();
        assert_eq!(
            result.vote_counts,
            vec![
                ("yes".into(), 2),
                // "maybe" and "no" tie at 1; BTreeMap iteration order is alphabetical,
                // and stable_sort preserves that relative order.
                ("maybe".into(), 1),
                ("no".into(), 1),
            ]
        );
    }

    #[test]
    fn sanitize_comment_in_metadata() {
        let branches = vec![success("a", "anthropic", "ok --> bad")];
        let config = make_config(TieBreak::FirstResponder);
        let (artifact, _) = aggregate_vote(&branches, &config).unwrap();
        // The metadata should have sanitized the `-->` in the winner text.
        assert!(artifact.text.contains("ok -- > bad"));
        // Ensure the comment block is intact.
        assert!(artifact.text.contains("-->\n"));
    }

    #[test]
    fn vote_snapshot() {
        let branches = vec![
            success("claude", "anthropic", " YES "),
            success("codex", "openai", "yes"),
            success("gemini", "google", "no"),
        ];
        let config = make_config(TieBreak::FirstResponder);
        let (artifact, _) = aggregate_vote(&branches, &config).unwrap();
        insta::assert_snapshot!(artifact.text);
    }

    #[test]
    fn normalise_ballot_basic() {
        assert_eq!(normalise_ballot("  YES  "), "yes");
        assert_eq!(normalise_ballot("Yes\n"), "yes");
        assert_eq!(normalise_ballot(""), "");
    }
}
