OpenAI Codex v0.125.0 (research preview)
--------
workdir: /Users/mk/Code/orchestrator/loker--feat-clo-269
model: gpt-5.3-codex-spark
provider: openai
approval: never
sandbox: workspace-write [workdir, /tmp, $TMPDIR, /Users/mk/.codex/memories]
reasoning effort: high
reasoning summaries: none
session id: 019ddad3-9faa-78e2-bfa8-d78aa5b84316
--------
user
changes against 'main'
exec
/bin/zsh -lc 'git diff 48cb96fc8c70f3f8cd70490580c279833fef6ef9 --stat && git diff 48cb96fc8c70f3f8cd70490580c279833fef6ef9 --' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
 !                                                  |   0
 docs/designs/clo-269-aggregator-vote.md            | 617 +++++++++++++++++++++
 docs/discovery/clo-269.md                          | 224 ++++++++
 docs/plans/clo-269-aggregator-vote.md              |  68 +++
 docs/prds/clo-269-aggregator-vote.md               |  94 ++++
 docs/reviews/CLO-269-review-ollama.md              | 148 +++++
 docs/reviews/clo-269-design-gemini.md              |  56 ++
 docs/reviews/clo-269-design-synthesis.md           |  68 +++
 docs/status/clo-269-workflow.yaml                  | 185 ++++++
 src/aggregator/concat.rs                           |  26 +
 src/aggregator/mod.rs                              |   6 +
 ...er__aggregator__vote__tests__vote_snapshot.snap |  16 +
 src/aggregator/vote.rs                             | 578 +++++++++++++++++++
 src/family.rs                                      |  26 +
 src/strategy/parallel_fanout.rs                    | 186 ++++++-
 15 files changed, 2293 insertions(+), 5 deletions(-)
diff --git a/! b/!
new file mode 100644
index 0000000..e69de29
diff --git a/docs/designs/clo-269-aggregator-vote.md b/docs/designs/clo-269-aggregator-vote.md
new file mode 100644
index 0000000..8dfb17b
--- /dev/null
+++ b/docs/designs/clo-269-aggregator-vote.md
@@ -0,0 +1,617 @@
+# Design: CLO-269 — Aggregator::Vote with ballot schema and tie-breakers
+
+| Field | Value |
+|-------|-------|
+| Task | CLO-269 |
+| Date | 2026-04-29 |
+| Phase | design |
+| Discovery | docs/discovery/clo-269.md |
+| PRD | docs/prds/clo-269-aggregator-vote.md |
+
+## 1. Problem
+
+`ParallelFanOut` currently supports `Concat`, `AnyFail`, and `LLMJudge` aggregators. The
+`Vote` label exists in the schema-facing `Aggregator` enum but has zero behavioural
+implementation. Workflow authors who want N backends to answer the same ballot
+question and pick the majority winner cannot express this today — they must hand-craft
+an LLMJudge prompt, which is over-engineered for mechanical counting. This design
+closes the gap by adding a pure, self-contained `vote.rs` module under
+`src/aggregator/` that counts normalised responses, abstains on malformed or
+error-time answers, applies tie-breakers, and produces a structured
+`AggregatedArtifact`.
+
+Reference: discovery report §Problem Framing.
+
+## 2. Goals & Non-goals
+
+### Goals
+- Add `Aggregator::Vote { ballot_schema, tie_break, abstain_threshold }` to the
+  behavioural enum in `src/aggregator/concat.rs`.
+- Define `BallotSchema::FreeText` (v0 default) and reserve `BallotSchema::Enum`
+  (deferred).
+- Define `TieBreak::ClosestToFamily(Family)`, `TieBreak::Random { seed }`, and
+  `TieBreak::FirstResponder`.
+- Normalise free-text answers for comparison (`trim().to_lowercase()` in v0).
+- Treat backend errors during parallel execution as abstentions (not votes).
+- Return `PhaseError::QuorumLost` when abstentions exceed `abstain_threshold`.
+- Produce an `AggregatedArtifact` containing the winning response text plus a
+  structured metadata comment (vote counts, abstain count, tie-break used).
+- Unit-test every tie-break path with fixed seeds / deterministic inputs.
+- Snapshot-test serialised `StrategyOutput` against the parallel schema.
+
+### Non-goals
+- Weighted voting (already exists in `src/consensus.rs` as a distinct concern,
+  not an `Aggregator`).
+- Recursive tie-breaking (e.g. second-round runoff between tied candidates).
+- Ballot validation using a JSON schema or external parser.
+- Prompt engineering for the ballot question itself (Vote only interprets answers,
+  not the question rendering).
+- `BallotSchema::Enum` variant (deferred to v0+1).
+- Configurable normalisation function beyond v0 `trim().to_lowercase()`.
+- Cross-family enforcement at the Vote aggregator level. `LLMJudge` enforces a
+  judge from a different family than the reviewers; `Vote` has no "judge" and is
+  inherently counting diversity. Cross-family selection for the *parallel pool* is
+  the strategy's responsibility, not the aggregator's. FR-13 enforcement remains
+  scoped to `LLMJudge` and is documented out-of-scope for Vote.
+
+## 2.5 Acceptance Criteria
+
+Restated from PRD FR-12 as concrete, testable criteria for the design phase:
+
+1. `Aggregator::Vote { config: VoteConfig }` is accepted by the TOML parser and
+   round-trips through `Aggregator::kind()` → `strategy::Aggregator::Vote`.
+2. `aggregate_vote` returns a strict-majority winner in O(N) time for N branches.
+3. `TieBreak::Random { seed }` produces identical winners across repeated runs
+   with the same seed and inputs.
+4. `TieBreak::FirstResponder` selects the bucket whose earliest-completed branch
+   arrived first in the `FuturesUnordered` completion order.
+5. `abstain_threshold` semantics use strict greater-than (`abstain_count > threshold`).
+6. Backend errors, empty responses, and malformed ballots are all counted as
+   abstentions (not votes).
+7. `PhaseError::QuorumLost` maps cleanly from `VoteError::QuorumLost` →
+   `StrategyError::Phase(PhaseError::QuorumLost)` → `PhaseError`, so the
+   orchestrator treats lost quorum as a terminal phase failure, not a retryable
+   error.
+8. The aggregated artefact metadata comment is deterministic (sorted keys,
+   sanitised text) so snapshot tests are stable across runs.
+
+## 3. Architecture
+
+### 3.1 Modules
+
+```
+src/
+  aggregator/
+    mod.rs          # AnyFail logic, re-exports, shared helpers
+    concat.rs       # Behavioural Aggregator enum + Concat (expanded with Vote variant)
+    llm_judge.rs    # LLMJudge config + behaviour (unchanged)
+    vote.rs         # NEW: Vote config, normalisation, counting, tie-breakers
+  strategy/
+    mod.rs          # Aggregator schema label enum (unchanged)
+    parallel_fanout.rs  # Extended: post-collection vote dispatch
+  family.rs         # Add PhaseError::QuorumLost
+```
+
+### 3.2 Data flow
+
+```
+ParallelFanOut::execute
+  │
+  ├─ FuturesUnordered loop collects all attempts (same as today)
+  │
+  ├─ match self.aggregator
+  │    ├─ Concat  → concat::aggregate_concat(...)
+  │    ├─ AnyFail → handled inline (short-circuit)
+  │    ├─ LLMJudge → llm_judge::aggregate_llm_judge(...)
+  │    └─ Vote    → vote::aggregate_vote(
+  │                     branches,       // &amp;[BranchOutcome] — successes + failures
+  │                     ballot_schema,
+  │                     tie_break,
+  │                     abstain_threshold,
+  │                  )
+  │
+  └─ Returns StrategyOutput with
+       aggregator: Some(Aggregator::Vote),
+       aggregate_output_path: "{phase}/aggregated.txt",
+       verify: VerifyOutcome::passed("Vote")
+```
+
+**Key difference from LLMJudge:** Vote does not need `backends` or `PhaseContext`
+because it is pure (no secondary backend call). It only needs the collected branch
+outcomes and the config. This keeps the call synchronous and unit-testable.
+
+### 3.3 New types
+
+```rust
+// src/aggregator/vote.rs
+use crate::family::Family;
+
+/// How a ballot is normalised and interpreted.
+#[derive(Debug, Clone, PartialEq, Eq)]
+#[non_exhaustive]
+pub enum BallotSchema {
+    /// Free text: each backend returns prose; normalise before bucketing.
+    /// v0 normalisation: trim whitespace + lowercase.
+    FreeText,
+    // Enum { variants: Vec&lt;String&gt; } — reserved for v0+1
+}
+
+/// How to resolve a tie when no strict majority exists.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub enum TieBreak {
+    /// Pick the candidate whose backend family matches the given family.
+    /// If multiple candidates match, first occurrence in arrival order wins.
+    ClosestToFamily(Family),
+    /// Deterministic shuffle from a fixed seed.
+    /// The seed is sourced from the workflow config (`lok.toml` or per-phase
+    /// override) and emitted in the aggregated artefact for reproducibility.
+    Random { seed: u64 },
+    /// Pick the candidate whose successful response arrived first.
+    FirstResponder,
+}
+
+/// Config payload for the Vote aggregator.
+#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
+pub struct VoteConfig {
+    pub ballot_schema: BallotSchema,
+    pub tie_break: TieBreak,
+    /// Number of abstentions (errors + malformed answers) that triggers
+    /// `PhaseError::QuorumLost`. If `abstain_threshold` == `n_targets`,
+    /// a single error does not abort; this only fires when **strictly more**
+    /// than `abstain_threshold` are abstentions.
+    ///
+    /// Example: 3 targets, threshold = 1, 0 abstentions → ok.
+    /// 3 targets, threshold = 1, 2 abstentions → `QuorumLost`.
+    pub abstain_threshold: usize,
+}
+
+/// Result of a vote aggregation, including metadata for traceability.
+#[derive(Debug, Clone)]
+pub struct VoteResult {
+    /// The winning text (normalised key). If downstream consumers need the
+    /// original casing, they must look up the chosen candidate's raw text.
+    pub winner: String,
+    /// vote_counts is always sorted descending by count for snapshot determinism.
+    pub vote_counts: Vec&lt;(String, usize)&gt;,
+    pub abstain_count: usize,
+    pub total_branches: usize,
+    pub tie_broken: bool,
+    pub tie_break_rule: String,
+}
+
+/// Errors specific to Vote aggregation.
+#[derive(Debug, thiserror::Error, PartialEq, Eq)]
+pub enum VoteError {
+    #[error("quorum lost: {abstains} abstentions exceed threshold {threshold}")]
+    QuorumLost { abstains: usize, threshold: usize },
+
+    #[error("no candidates available")]
+    NoCandidates,
+}
+
+// Note: `NoOpinion` was considered (identical responses from all branches) but
+// removed because a unanimous single-bucket result is a valid strict majority,
+// not an error.
+```
+
+### 3.4 Public API surface
+
+```rust
+// src/aggregator/concat.rs (behavioural Aggregator enum expanded)
+pub enum Aggregator {
+    Concat { heading_template: String },
+    AnyFail,
+    LLMJudge { judge_backend: String, prompt_template: String, require_judge_different_family: bool },
+    Vote { config: VoteConfig },
+}
+
+impl Aggregator {
+    pub fn kind(&self) -> crate::strategy::Aggregator { ... }
+    pub fn vote(config: VoteConfig) -> Self { Self::Vote { config } }
+}
+```
+
+```rust
+// src/aggregator/vote.rs
+pub fn aggregate_vote(
+    branches: &[BranchOutcome],
+    config: &VoteConfig,
+) -> Result&lt;(AggregatedArtifact, VoteResult), VoteError&gt;;
+
+pub fn normalise_ballot(text: &str) -> String;
+
+pub fn compute_vote(
+    candidates: &[(&str, String, usize)], // (backend_id, normalised, original_arrival_order)
+    tie_break: &TieBreak,
+) -> Option&lt;VoteResult&gt;;
+```
+
+### 3.5 PhaseError expansion and StrategyError mapping
+
+```rust
+// src/family.rs
+#[non_exhaustive]
+#[derive(Debug, thiserror::Error)]
+pub enum PhaseError {
+    // ... existing variants ...
+
+    #[error("quorum lost: {abstains} abstentions exceed threshold {threshold}")]
+    QuorumLost { abstains: usize, threshold: usize },
+}
+```
+
+`VoteError` maps into the orchestrator's error hierarchy as follows:
+- `VoteError::QuorumLost` → `StrategyError::Phase(PhaseError::QuorumLost)`
+- `VoteError::NoCandidates` → `StrategyError::Phase(PhaseError::AggregatorRejected)`
+
+This mapping is performed in `parallel_fanout.rs` at the dispatch site, analogous to `LLMJudgeError` conversion.
+
+## 4. Implementation details
+
+### 4.1 Short-circuit guard
+
+`ParallelFanOut::execute` currently short-circuits when `successes >= min_responses`.
+Vote must collect **all** branches (including failures as abstentions) before it can
+compute a majority or detect a quorum loss. Therefore the early-break condition is
+updated:
+
+```rust
+if !is_any_fail && !is_llm_judge && !is_vote && successes >= self.min_responses {
+    break;
+}
+```
+
+`is_vote` is computed from `self.aggregator.kind() == Aggregator::Vote`.
+
+### 4.2 Branch outcome → vote candidates
+
+In `parallel_fanout.rs`, after the `FuturesUnordered` loop:
+
+```rust
+let vote_branches: Vec&lt;BranchOutcome&gt; = attempts
+    .iter()
+    .enumerate()
+    .map(|(arrival_order, attempt)| {
+        // If the attempt succeeded, we already pushed a BranchSuccess
+        // into successful_candidates. We need to map both successes
+        // and failures into BranchOutcome for the aggregator.
+
+        // For Vote specifically, we must retain arrival order
+        // (for FirstResponder) and backend_id (for ClosestToFamily).
+    })
+    .collect();
+
+// Actually, ParallelFanOut already has:
+// - successful_candidates: Vec&lt;BranchSuccess&gt;
+// - attempts: Vec&lt;Attempt&gt; with error metadata
+// We convert both into BranchOutcome::Success / BranchOutcome::Failure.
+```
+
+A better approach: instead of building a new vector, pass `&attempts` plus
+`&successful_candidates` directly to the vote helper. But `vote.rs` should
+only know about `BranchOutcome` (the domain of aggregators), not `Attempt`
+(the domain of strategies).
+
+So `ParallelFanOut` will construct `Vec&lt;BranchOutcome&gt;` from the loop:
+
+```rust
+let mut vote_branches = Vec::with_capacity(self.targets.len());
+for (idx, attempt) in attempts.iter().enumerate() {
+    if attempt.finish_reasons.contains(&FinishReason::Error) {
+        vote_branches.push(BranchOutcome::Failure(BranchFailure {
+            backend_id: attempt.backend.clone(),
+            family: attempt.family.clone().unwrap_or_default(),
+            index: idx + 1,
+            reason: format!("backend error: {:?}", attempt.finish_reasons),
+        }));
+    } else {
+        vote_branches.push(BranchOutcome::Success(BranchSuccess {
+            backend_id: attempt.backend.clone(),
+            family: attempt.family.clone().unwrap_or_default(),
+            index: idx + 1,
+            output: /* read from output_path or from stdout in Attempt? */,
+        }));
+    }
+}
+```
+
+Wait — `Attempt` currently does not carry the stdout text directly; it carries
+`output_path`. For `Concat`, the aggregator reads from disk. For `AnyFail`, the
+evaluator reads `query.stdout` inline during the loop. For `LLMJudge`, the
+`successful_candidates` vector was built with `output: query.stdout.clone()`.
+
+For `Vote`, we'll build `BranchSuccess` entries at the same point in the loop
+where `LLMJudge` builds them:
+
+```rust
+successes += 1;
+successful_candidates.push(BranchSuccess {
+    backend_id: target.backend.clone(),
+    family: family_of(&target.backend).to_string(),
+    index: successes, // 1-based, NOT arrival_order
+    output: query.stdout.clone(),
+});
+```
+
+But `Vote` cares about **arrival order** for `FirstResponder`, while
+`BranchSuccess.index` is currently `successful_candidates.len() + 1` (i.e. the
+1-based index among *successful* branches only). This is wrong for
+`FirstResponder`, which needs the absolute arrival order among *all* branches
+(successes and failures).
+
+**Resolution:** We will NOT reuse `successful_candidates` for Vote. Instead,
+we'll build a separate `vote_branches: Vec<BranchOutcome>` inside the loop,
+preserving the absolute branch order (0..N). Each `BranchOutcome::Success`
+will carry the actual `query.stdout` text. Each `BranchOutcome::Failure` will
+carry an error reason. The `Vote` module then normalises the successful answers
+and counts them.
+
+### 4.3 Vote counting algorithm
+
+```rust
+pub fn aggregate_vote(
+    branches: &[BranchOutcome],
+    config: &VoteConfig,
+) -> Result<(AggregatedArtifact, VoteResult), VoteError> {
+    let mut abstain_count = 0;
+    let mut candidates: Vec<VoteCandidate> = Vec::new();
+    let total = branches.len();
+
+    for (arrival_order, branch) in branches.iter().enumerate() {
+        match branch {
+            BranchOutcome::Success(success) => {
+                let normalised = normalise_ballot(&success.output);
+                if normalised.is_empty() {
+                    abstain_count += 1;
+                } else {
+                    candidates.push(VoteCandidate {
+                        backend_id: success.backend_id.clone(),
+                        family: success.family.clone(),
+                        normalised,
+                        arrival_order,
+                    });
+                }
+            }
+            BranchOutcome::Failure(_) => {
+                abstain_count += 1;
+            }
+        }
+    }
+
+    if abstain_count > config.abstain_threshold {
+        return Err(VoteError::QuorumLost {
+            abstains: abstain_count,
+            threshold: config.abstain_threshold,
+        });
+    }
+
+    if candidates.is_empty() {
+        return Err(VoteError::NoCandidates);
+    }
+
+    // Count votes by normalised bucket
+    // BTreeMap ensures deterministic iteration order for tie-break determinism.
+    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new(); // normalised → candidate indices
+    let mut first_seen: BTreeMap<String, usize> = BTreeMap::new();   // normalised → first arrival_order
+    for (idx, c) in candidates.iter().enumerate() {
+        buckets.entry(c.normalised.clone()).or_default().push(idx);
+        first_seen.entry(c.normalised.clone()).or_insert(c.arrival_order);
+    }
+
+    let max_votes = buckets.values().map(|v| v.len()).max().unwrap_or(0);
+    let winners: Vec<&str> = buckets
+        .iter()
+        .filter(|(_, v)| v.len() == max_votes)
+        .map(|(k, _)| k.as_str())
+        .collect();
+
+    let mut result = if winners.len() == 1 {
+        let winner_text = winners[0];
+        let chosen_idx = buckets[winner_text][0];
+        VoteResult {
+            winner: candidates[chosen_idx].normalised.clone(),
+            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
+            abstain_count,
+            total_branches: total,
+            tie_broken: false,
+            tie_break_rule: "none (strict majority)".into(),
+        }
+    } else {
+        // TIE: apply tie_breaker
+        let chosen_text = resolve_tie(&winners, &candidates, &buckets, &config.tie_break);
+        let chosen_idx = buckets[chosen_text][0]; // first occurrence of that bucket
+        VoteResult {
+            winner: chosen_text.into(),
+            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
+            abstain_count,
+            total_branches: total,
+            tie_broken: true,
+            tie_break_rule: format!("{:?}", config.tie_break), // or a dedicated helper
+        }
+    };
+
+    // Sort vote_counts descending for stable output
+    result.vote_counts.sort_by(|a, b| b.1.cmp(&a.1));
+
+    let text = build_aggregated_text(&result, &candidates);
+    let artifact = AggregatedArtifact {
+        text,
+        successful: candidates.len(),
+        failed: abstain_count,
+    };
+
+    Ok((artifact, result))
+}
+```
+
+### 4.4 Tie-breaker resolution
+
+```rust
+fn resolve_tie(
+    tied_buckets: &[&str],
+    candidates: &[VoteCandidate],
+    buckets: &BTreeMap<String, Vec<usize>>,
+    tie_break: &TieBreak,
+) -> &str {
+    match tie_break {
+        TieBreak::FirstResponder => {
+            // Pick the bucket whose first candidate arrived earliest.
+            tied_buckets
+                .iter()
+                .min_by_key(|&bucket| {
+                    buckets[*bucket]
+                        .iter()
+                        .map(|&ci| candidates[ci].arrival_order)
+                        .min()
+                        .unwrap_or(usize::MAX)
+                })
+                .copied()
+                .unwrap_or(tied_buckets[0])
+        }
+
+        TieBreak::ClosestToFamily(target_family) => {
+            // Collect all tied buckets that contain at least one candidate
+            // whose family matches target_family.
+            let matching: Vec<&str> = tied_buckets
+                .iter()
+                .copied()
+                .filter(|&bucket| {
+                    buckets[bucket]
+                        .iter()
+                        .any(|&ci| family_of(&candidates[ci].backend_id) == *target_family)
+                })
+                .collect();
+
+            if matching.is_empty() {
+                // Fallback: if no bucket matches target family, use FirstResponder.
+                resolve_tie(tied_buckets, candidates, buckets, &TieBreak::FirstResponder)
+            } else if matching.len() == 1 {
+                matching[0]
+            } else {
+                // Multiple matching buckets: apply FirstResponder among the matching subset.
+                resolve_tie(&matching, candidates, buckets, &TieBreak::FirstResponder)
+            }
+        }
+
+        TieBreak::Random { seed } => {
+            use rand::{seq::SliceRandom, SeedableRng};
+            use rand::rngs::StdRng;
+            let mut rng = StdRng::seed_from_u64(*seed);
+            let mut choices = tied_buckets.to_vec();
+            choices.shuffle(&mut rng);
+            choices[0]
+        }
+    }
+}
+```
+
+### 4.5 Aggregated text format
+
+The winning text goes first. A trailing HTML comment block carries metadata:
+
+```markdown
+&lt;answer text here&gt;
+
+&lt;!-- loker: Vote aggregator metadata
+  winner: &lt;text&gt;
+  total_branches: N
+  vote_counts:
+    answer_a: 2
+    answer_b: 1
+  abstain_count: 0
+  tie_broken: false
+  tie_break_rule: none
+--&gt;
+```
+
+Before writing the metadata comment, `result.winner` and `result.tie_break_rule`
+are sanitised by replacing `-->` with `-- >` to prevent premature comment closure.
+Answer text itself is emitted **before** the comment, so even an adversarial backend
+response cannot escape the metadata block.
+
+### 4.6 Normalisation
+
+```rust
+pub fn normalise_ballot(text: &str) -> String {
+    text.trim().to_lowercase()
+}
+```
+
+v0 normalisation is intentionally simple. Future work (post-v0) may add:
+- Unicode normalisation (NFKC)
+- Stop-word removal
+- Stemming
+- Deduplication of whitespace
+
+## 5. Test plan
+
+### Unit tests (`src/aggregator/vote.rs`)
+
+| Test | Input | Expected |
+|------|-------|----------|
+| `free_text_clear_winner` | 3 branches: "yes", "yes", "no" | winner="yes", tie_broken=false |
+| `free_text_tie_first_responder` | 3 branches: "yes", "no", "yes" + tie=FirstResponder | winner="yes" (arrival 0) |
+| `free_text_tie_closest_family` | 3 branches: claude/"a", gemini/"b", openai/"a" + tie=ClosestToFamily(Anthropic) | winner="a" (claude is Anthropic) |
+| `free_text_tie_random` | 2 branches: "a", "b" + tie=Random { seed=42 } | deterministic winner from seed |
+| `abstain_backend_error` | 2 successes + 1 failure | abstain_count=1, 2 valid votes |
+| `quorum_lost` | 1 success + 2 failures, threshold=1 | VoteError::QuorumLost |
+| `empty_input` | no branches | VoteError::NoCandidates |
+| `all_abstain` | 3 failures | VoteError::QuorumLost (or NoCandidates if threshold ≥ 3) |
+| `normalise_case` | "YES", "yes", "Yes" | all bucketed together |
+| `normalise_whitespace` | "  yes  ", "yes\n" | all bucketed together |
+| `closest_family_no_match_fallback` | tie with no matching family | falls back to FirstResponder |
+
+### Integration tests (`tests/strategy_parallel_fanout.rs`)
+
+| Test | Setup | Expected |
+|------|-------|----------|
+| `vote_success` | 3 mock backends returning "A", "A", "B" with Vote(FreeText) | StrategyOutput with correct path, schema validates |
+| `vote_tie_random_deterministic` | 2 backends returning "A", "B", Random seed=123 | Same winner on multiple runs |
+| `vote_quorum_lost` | 1 ok + 2 fail, threshold=0 | PhaseError::QuorumLost |
+| `vote_snapshot` | fixture branches + Vote config | `insta::assert_snapshot!` on aggregated text |
+
+### Snapshot test
+
+1. Run `vote_success`.
+2. Serialise `StrategyOutput` to JSON.
+3. Validate against `docs/schemas/phase_result_parallel.schema.json`.
+4. Use `insta::assert_snapshot` on `aggregated.txt` content.
+
+## 6. Migration / rollout
+
+- No breaking changes to existing `Concat`, `AnyFail`, or `LLMJudge` aggregators.
+- `PhaseError` gains `QuorumLost` (additive, `#[non_exhaustive]`).
+- `Aggregator` enum gains `Vote` variant (schema label already existed; behavioural payload is new).
+- `src/aggregator/concat.rs` expands slightly (new variant in behavioural enum + `kind()` match arm).
+- `parallel_fanout.rs` loop must be extended to build `vote_branches` and dispatch to `vote::aggregate_vote`. This is additive; existing branches for `AnyFail` and `LLMJudge` are untouched.
+- **`docs/schemas/phase_result_parallel.schema.json`** must add `"vote"` to the
+  `aggregator` string enum so JSON schema validation passes for Vote outputs.
+  This is a one-line additive schema change.
+
+## 7. Resolving discovery debt
+
+| Debt item | Resolution |
+|-----------|------------|
+| Ballot schema shape | `BallotSchema::FreeText` in v0. `BallotSchema::Enum { variants: Vec<String> }` reserved. The decision is: enum-style choices are useful but require ballot-prompt engineering that is out of scope for Vote (it lives at the prompt/template layer). Vote only normalises the free-text answer it receives. |
+| Seed source for `TieBreak::Random` | Configured in `lok.toml` under `[workflow.vote]` or per-phase `[[phase]] vote_seed = 0x1234`. If absent, it defaults to `0` in v0. The seed is logged in the aggregated artefact metadata block for traceability. |
+| Quorum threshold semantics | Absolute count: `abstain_threshold: usize`. The phase fails when `abstain_count > abstain_threshold`. This matches the AC: "document the threshold where abstain-majority returns PhaseError::QuorumLost". Example: 3 targets, threshold=1, 2 abstentions → QuorumLost. |
+
+## 8. Open questions
+
+| Question | Resolution |
+|----------|------------|
+| Should `Vote::aggregate()` be added to the `Aggregator::aggregate()` method? | For now, Vote is called inline from `parallel_fanout.rs`, matching the `AnyFail` pattern. Future T-029 may formalise a proper `Aggregator` trait. |
+| What does `FirstResponder` mean for a branch that backend-errored? | Errors are abstentions, so they never win. `FirstResponder` applies among *successful* branches only, ranked by their absolute arrival order in the FuturesUnordered loop. |
+| Do we need a new `rand` dependency? | `rand` 0.8 is already an indirect dependency via `uuid` (see Cargo.lock). Adding it to `Cargo.toml` is acceptable if we use it for `TieBreak::Random`. Import only `rand::rngs::StdRng` and `rand::seq::SliceRandom` (or `rand_core` + `rand_rngs`) for a minimal footprint. |
+| How do `min_responses` and `abstain_threshold` interact? | They are independent gates. `min_responses` controls when `ParallelFanOut` stops collecting additional branches; `abstain_threshold` controls whether Vote rejects the result after all branches are collected. Vote disables the `min_responses` short-circuit so it always sees the full set. |
+
+## 9. References
+
+- Discovery report: `docs/discovery/clo-269.md`
+- PRD: `docs/prds/clo-269-aggregator-vote.md`
+- CLO-265 `family.rs`: `src/family.rs`
+- CLO-266 concat aggregator: `src/aggregator/concat.rs`
+- CLO-267 AnyFail: `src/aggregator/mod.rs`
+- CLO-268 LLMJudge: `src/aggregator/llm_judge.rs`, `docs/designs/clo-268-llm-judge.md`
+- loker-design.md §4.3 (Aggregator trait overview)
+- PRD FR-12 (Vote aggregator)
diff --git a/docs/discovery/clo-269.md b/docs/discovery/clo-269.md
new file mode 100644
index 0000000..ba7a36a
--- /dev/null
+++ b/docs/discovery/clo-269.md
@@ -0,0 +1,224 @@
+# Discovery Report: CLO-269 — Implement Aggregator::Vote with ballot schema and tie-breakers
+
+| Field | Value |
+|-------|-------|
+| Task | CLO-269 |
+| Date | 2026-04-29 |
+| Phase | discovery |
+| Branch | `feat/clo-269` |
+
+## Problem Framing
+
+**Who is affected:** Workflow authors using `Strategy::ParallelFanOut` who
+want a lightweight majority vote over N candidate responses without
+hand-crafting an LLMJudge prompt. Example: fanning out a yes/no
+classification to three different-family models and taking the majority
+answer, with deterministic resolution if the count is tied.
+
+**Current behaviour:** `ParallelFanOut` supports `Concat`, `AnyFail`, and
+`LLMJudge` aggregators. The `Vote` label exists in the
+`crate::strategy::Aggregator` schema enum but is not wired into
+`ParallelFanOut` and has no behavioural implementation. There is no
+mechanism to count normalised responses, abstain on malformed or
+error-time answers, or apply `ClosestToFamily` / `Random` / `FirstResponder`
+tie-breakers in the aggregation path.
+
+**Desired behaviour:**
+- `Aggregator::Vote { ballot_schema, tie_break }` extracts normalised
+  responses from successful parallel branches.
+- Malformed answers and branch-level backend errors count as *abstains*,
+  not votes.
+- If abstentions exceed a threshold, the phase fails with
+  `PhaseError::QuorumLost`.
+- A strict majority (> 50 %) wins outright.
+- If no strict majority exists, the configured `TieBreak` rule resolves:
+  - `ClosestToFamily(Family)` picks the candidate whose
+    `family_of(backend_id)` matches the nominated family.
+  - `Random { seed }` picks deterministically from a seed.
+  - `FirstResponder` picks the candidate that arrived first.
+- The aggregated artefact contains the winning response text plus
+  metadata: vote counts per response, abstain count, tie-break rule used.
+
+**Why now:** This is T-019 on the Phase 3 aggregator critical path. It is
+the final aggregator variant before the M3 aggregator vocabulary is
+considered complete. `family_of` (CLO-265) is already merged, and
+`LLMJudge` (CLO-268) demonstrates the exact wiring needed to add a new
+aggregator to `ParallelFanOut`.
+
+## Existing Code
+
+### `src/aggregator/concat.rs`
+- `Aggregator` behavioural enum currently has `Concat`, `AnyFail`,
+  `LLMJudge { judge_backend, prompt_template, require_judge_different_family }`.
+- `Aggregator::kind()` maps each variant to the schema enum in
+  `crate::strategy::Aggregator`. `Vote` is missing.
+- `Aggregator::aggregate()` dispatches `Concat` and short-circuits
+  `AnyFail` / `LLMJudge` because those have different async signatures.
+  A new `Vote` variant would most likely be handled inline by
+  `ParallelFanOut` (like `AnyFail`) or require extending the
+  `aggregate()` signature to include vote logic — but vote logic is pure
+  (no secondary backend call), so `aggregate()` could work directly.
+
+### `src/strategy/mod.rs` (schema-facing enum)
+- `pub enum Aggregator { Concat, AnyFail, Vote, LLMJudge }` already exists
+  as a schema label. No behavioural payload. `Vote` is already accepted
+  by `docs/schemas/phase_result_parallel.schema.json`.
+- `StrategyError` has `AnyFail`, `FloorViolation`, `Exhausted`, `Backend`,
+  etc. No `QuorumLost` variant.
+- A `PhaseError` variant for quorum loss can be added to
+  `src/family.rs` (where `FamilyOverlap`, `AggregatorContract`, and
+  `JudgeUnavailable` already live).
+
+### `src/consensus.rs`
+- `majority_vote(responses: &[BackendResponse]) -> Option<VoteResult>`
+  operates on `BackendResponse { backend, content }` strings.
+- Normalises whitespace (`trim().to_string()`), counts occurrences, breaks
+  ties by first occurrence.
+- `weighted_vote` uses `BackendWeights` (hard-coded map of backend→f64).
+- **Key difference:** these functions operate on `BackendResponse` slices at
+  the consensus layer (used by the legacy `workflow.rs` / `ConsensusStrategy`
+  enum), not on `BranchSuccess` / `BranchFailure` inside `ParallelFanOut`.
+- The normalisation logic (`trim()`) can be reused; the tie-breaking logic
+  can be generalised.
+
+### `src/family.rs` (CLO-265)
+- `family_of(backend_id)` → `Family` lookup is stable and tested on
+  all known backend naming conventions.
+- `PhaseError` has `FamilyOverlap`, `AggregatorContract`, `JudgeUnavailable`.
+  **Missing:** `QuorumLost` (needed for abstention threshold failure).
+
+### `src/strategy/parallel_fanout.rs`
+- The `FuturesUnordered` loop already separates `is_any_fail` and
+  `is_llm_judge` flags. For `Vote`, we can add `is_vote` and handle it
+  similarly to `AnyFail`: collect all branches, then run vote logic.
+- The loop short-circuits early when `!is_any_fail && !is_llm_judge &&
+  successes >= min_responses`. For `Vote`, we must collect **all**
+  successful branches because every candidate can affect the vote count.
+  However, we only need to collect until we know the floor is met — wait,
+  actually we need every single branch's response to compute a majority.
+  So `Vote` cannot short-circuit at all.
+  
+  Actually, we need to collect all responses because a later answer could
+  flip the majority. The loop already collects until all futures resolve.
+
+### `docs/schemas/phase_result_parallel.schema.json`
+- Already accepts `"vote"` in the `aggregator` string enum.
+- `verify` field accepts `"pass"`, `"fail"`, `"skipped"`.
+- No schema changes needed; behavioural changes only.
+
+### Baseline Score
+
+**7 / 10**. The infrastructure is almost entirely in place:
+- schema label: ✅
+- `ParallelFanOut` loop: ✅
+- `family_of`: ✅
+- `PhaseError` plumbing: ✅ (just needs one new variant)
+- Missing: Vote config enum, vote counting logic, tie-breaker logic,
+  abstention handling, `QuorumLost` error, tests.
+
+## Approaches
+
+### Approach A — Inline Vote in `parallel_fanout.rs`
+
+After the `FuturesUnordered` loop collects all branches, run a
+`compute_vote()` helper that operates on `successful_candidates` and
+failed/abstained branches inline within `parallel_fanout.rs`.
+
+- **Pros:**
+  - Minimal indirection; everything visible in one strategy file.
+  - No new `src/aggregator/*.rs` module needed.
+  - Reuses the existing loop's `successful_candidates` vector.
+- **Cons:**
+  - `parallel_fanout.rs` already ~300 lines; adding vote logic enlarges it.
+  - Unit-testing vote math in isolation requires constructing a full
+    `ParallelFanOut`.
+  - Tie-breaker logic (especially `ClosestToFamily`) leaks family concerns
+    into the strategy layer.
+- **Effort:** S
+- **Risk:** low
+
+### Approach B — Extract `vote.rs` module under `src/aggregator/`
+
+Create `src/aggregator/vote.rs` with its own config variant
+`Vote { ballot_schema, tie_break, abstain_threshold }`, a pure
+vote-computation function that receives
+`successful_candidates + failures`, and returns
+`Result<AggregatedArtifact, VoteError>`. `ParallelFanOut` calls
+it after collecting branches, similar to how `aggregate_concat()` is
+called for `Concat` but with additional abstain context.
+
+- **Pros:**
+  - Mirrors the `concat.rs` and `llm_judge.rs` patterns.
+  - Vote counting is unit-testable without async machinery.
+  - Tie-breaker logic stays in the aggregator module where family concerns
+    belong.
+  - Snapshot tests for tie-breaker determinism are easy.
+- **Cons:**
+  - Requires extending `Aggregator` enum and `aggregate()` to handle vote,
+  or adding a new direct call path from `ParallelFanOut`.
+  - Need to thread abstain data (failed branches) into the vote call since
+    `AggregateInput` currently only carries `BranchOutcome`.
+- **Effort:** S–M
+- **Risk:** low
+
+### Approach C — Reuse `src/consensus.rs` and wrap it
+
+Refactor `majority_vote` to accept `BranchSuccess` slices, add
+`TieBreak` enum to `consensus.rs`, and wire `ParallelFanOut` to delegate
+vote counting to the existing module.
+
+- **Pros:**
+  - Leverages existing normalisation and counting logic.
+  - `majority_vote` already has property-like tests.
+- **Cons:**
+  - `consensus.rs` operates on `BackendResponse` (a different domain
+    concern) and uses first-occurrence tie-breaking baked into the
+    `HashMap` iteration order. Retconning it to support `ClosestToFamily`,
+    deterministic `Random`, and `FirstResponder` would refactor it into
+    something unrecognisable.
+  - `BackendResponse` does not carry `family` or arrival-order metadata
+    needed by the new tie-breakers.
+- **Effort:** M
+- **Risk:** medium
+
+## Decision
+
+**Chosen approach: B — Extract `vote.rs` module under `src/aggregator/`.**
+
+Rationale: The aggregator module tree already owns `concat.rs` and
+`llm_judge.rs`. Adding `vote.rs` is the natural next step and keeps vote
+logic isolated from the `FuturesUnordered` loop. Vote counting is pure
+and synchronous, making it simpler to test than `LLMJudge` (which needs a
+backend call). The `Aggregator` enum in `concat.rs` will expand with a
+`Vote { ballot_schema, tie_break }` variant, and `Aggregator::kind()`
+will map it to the schema label. `ParallelFanOut` will handle `Vote`
+similarly to `AnyFail`: collect *all* branches, then call a vote helper.
+Unlike `LLMJudge`, there is no secondary async backend call, so vote
+can be resolved synchronously after the loop.
+
+## Discovery Debt
+
+- [ ] Ballot schema shape (enum vs free-text) must be decided in the TDD
+      doc before implementation. PRD above recommends `FreeText` as
+      default with `Enum` as a v0+1 addition, but the TDD should lock the
+      exact `BallotSchema` enum shape.
+- [ ] Seed source for `TieBreak::Random` — whether it comes from
+      `lok.toml` / workflow config or is derived from a run-level UUID.
+  The PRD leaves this open; the TDD should close it.
+- [ ] Quorum threshold semantics: absolute count (`min_votes: usize`) vs
+  fraction (`min_votes_fraction: f64`). The Linear issue says
+  "document the threshold where abstain-majority returns `QuorumLost`".
+  The PRD above tentatively uses `abstain_threshold`; the TDD may choose
+  a different spelling.
+
+## Related Issues
+
+- [CLO-265](https://linear.app/cloud-ai/issue/CLO-265) — `family_of`
+  lookup, required for `ClosestToFamily`.
+- [CLO-266](https://linear.app/cloud-ai/issue/CLO-266) — `Concat`
+  aggregator pattern.
+- [CLO-267](https://linear.app/cloud-ai/issue/CLO-267) — `AnyFail`
+  aggregator short-circuit pattern.
+- [CLO-268](https://linear.app/cloud-ai/issue/CLO-268) — `LLMJudge`
+  aggregator wiring pattern in `ParallelFanOut`.
+- PRD FR-12, Roadmap T-019.
diff --git a/docs/plans/clo-269-aggregator-vote.md b/docs/plans/clo-269-aggregator-vote.md
new file mode 100644
index 0000000..098bb08
--- /dev/null
+++ b/docs/plans/clo-269-aggregator-vote.md
@@ -0,0 +1,68 @@
+# Plan: CLO-269 — Implement Aggregator::Vote with ballot schema and tie-breakers
+
+## Context
+- **Design**: `docs/designs/clo-269-aggregator-vote.md`
+- **Discovery**: `docs/discovery/clo-269.md`
+- **PRD**: `docs/prds/clo-269-aggregator-vote.md`
+- **Linear**: https://linear.app/cloud-ai/issue/clo-269/implement-aggregatorvote-with-ballot-schema-and-tie-breakers
+- **Branch**: `feat/clo-269`
+
+## Sub-tasks
+
+### ST1 — Add `PhaseError::QuorumLost` to error hierarchy
+**Files:** `src/family.rs`
+**Acceptance:** `cargo test --lib family` compiles and passes; new variant is `#[non_exhaustive]` safe.
+**Estimate:** S
+
+### ST2 — Expand `Aggregator` behavioural enum with `Vote` variant
+**Files:** `src/aggregator/concat.rs`
+**Acceptance:** `cargo test --lib aggregator::concat` compiles and passes; `Aggregator::kind()` returns `strategy::Aggregator::Vote` for the new variant.
+**Estimate:** S
+
+### ST3 — Author `src/aggregator/vote.rs` module
+**Files:** `src/aggregator/vote.rs` (new), `src/aggregator/mod.rs` (re-export)
+**Acceptance:** `cargo test --lib aggregator::vote` passes all unit tests:
+- `free_text_clear_winner` — strict majority
+- `free_text_tie_first_responder` — arrival-order tie-break
+- `free_text_tie_closest_family` — family-preference tie-break
+- `free_text_tie_random` — deterministic random from fixed seed
+- `abstain_backend_error` — errors count as abstentions
+- `quorum_lost` — abstentions exceed threshold
+- `empty_input` — no candidates
+- `all_abstain` — every branch fails
+- `normalise_case` — case-folding buckets
+- `normalise_whitespace` — trim buckets
+- `closest_family_no_match_fallback` — falls back to `FirstResponder`
+**Estimate:** M
+
+### ST4 — Wire `Vote` into `ParallelFanOut` strategy
+**Files:** `src/strategy/parallel_fanout.rs`
+**Acceptance:** `cargo test --test strategy_parallel_fanout` passes:
+- `vote_success` — 3 backends → majority winner
+- `vote_tie_random_deterministic` — identical winner on repeat runs
+- `vote_quorum_lost` — `PhaseError::QuorumLost` bubbles up correctly
+**Estimate:** M
+
+### ST5 — Update schema and add snapshot tests
+**Files:** `docs/schemas/phase_result_parallel.schema.json`, `tests/snapshots/` (new), `Cargo.toml` (add `rand` if needed)
+**Acceptance:** `cargo test --test strategy_parallel_fanout vote_snapshot` passes; schema file includes `"vote"` in the `aggregator` enum.
+**Estimate:** S
+
+### ST6 — Pre-merge gate
+**Acceptance:** `make check` (fmt + clippy + test) is green on `feat/clo-269`.
+**Estimate:** S
+
+## Risk: dependencies
+- Depends on [CLO-265](https://linear.app/cloud-ai/issue/CLO-265) (`family_of` lookup) — already merged.
+- `rand` crate is already an indirect dependency; adding it to `Cargo.toml` is low risk.
+
+## Pre-merge gate
+- `make check` (fmt + clippy + test)
+
+## Rollback plan
+If Vote causes regressions mid-milestone:
+1. Revert `src/strategy/parallel_fanout.rs` changes (additive, no merge conflicts).
+2. Revert `src/aggregator/concat.rs` variant addition.
+3. Delete `src/aggregator/vote.rs` and remove re-export in `mod.rs`.
+4. Revert `src/family.rs` `PhaseError::QuorumLost` addition.
+All changes are additive; no downstream code depends on Vote yet.
diff --git a/docs/prds/clo-269-aggregator-vote.md b/docs/prds/clo-269-aggregator-vote.md
new file mode 100644
index 0000000..cb4255b
--- /dev/null
+++ b/docs/prds/clo-269-aggregator-vote.md
@@ -0,0 +1,94 @@
+# PRD: CLO-269 — Aggregator::Vote with ballot schema and tie-breakers
+
+## Problem
+
+Workflow authors using `Strategy::ParallelFanOut` need a way to ask each
+backend a structured ballot question and pick the winner by majority, not
+just join outputs (`Concat`) or judge them externally (`LLMJudge`). The
+`Vote` enum variant exists as a schema label but has zero behavioural
+implementation. Without it, majority-based consensus (e.g. “which approach
+is simpler: A or B?”) requires hand-crafting an LLMJudge prompt, which is
+over-engineered for mechanical counting.
+
+## Goal
+
+Implement `Aggregator::Vote { ballot_schema: BallotSchema, tie_break: TieBreak }`
+that:
+
+1. Collects free-text or enum-style responses from each successful branch.
+2. Abstains on malformed responses or backend errors (does not count them
+   as votes).
+3. Declares a winner when one response commands a strict majority (> 50 %).
+4. Applies a deterministic tie-break rule when no strict majority exists.
+5. Fails the phase with `PhaseError::QuorumLost` when abstentions exceed a
+   configurable threshold (or when too few votes are cast to reach majority).
+
+## Scope (in)
+
+- `Aggregator` variant `Vote { ballot_schema, tie_break }` added to the
+  behavioural enum in `src/aggregator/concat.rs`.
+- `BallotSchema` enum:
+  - `FreeText` (default, v0): each backend returns free text; text is
+    normalised (trimmed, case-folded by config) before bucket counting.
+  - `Enum { variants: Vec<String> }` (optional but strongly desired): each
+    backend must pick one variant; anything outside the set is treated as
+    abstain.
+- `TieBreak` enum:
+  - `ClosestToFamily(Family)` — resolve toward the first candidate whose
+    `family_of(backend_id)` matches the given `Family`.
+  - `Random { seed: u64 }` — deterministic shuffle from a per-run seed
+    (seed sourced from manifest / workflow config).
+  - `FirstResponder` — choose the candidate that arrived first in
+    `ParallelFanOut` branch completion order.
+- Abstention handling:
+  - Backend errors during parallel execution → abstain (not a vote).
+  - Malformed ballot (garbled text, invalid enum choice) → abstain.
+  - Configurable `abstain_threshold: usize` (or `max_abstain_fraction: f64`):
+    if abstentions exceed the threshold, return `PhaseError::QuorumLost`.
+- Unit tests for every tie-break path with fixed seeds.
+- Snapshot of phase-result file shape matching
+  `docs/schemas/phase_result_parallel.schema.json`.
+
+## Scope (out)
+
+- Weighted voting (already exists in `src/consensus.rs` as a distinct
+  `ConsensusStrategy`, not an `Aggregator`).
+- Adaptive or recursive tie-breaking (e.g. re-prompt tied candidates).
+- Ballot validation using a JSON schema or external parser.
+- Prompt engineering for the ballot question itself (the question is
+  rendered by `ParallelFanOut`'s existing template engine; Vote only
+  interprets answers).
+
+## Acceptance Criteria
+
+- [ ] Tests pin ballot parsing, majority math, abstention handling, and
+      each of the three tie-break rules.
+- [ ] Random tie-break is reproducible from a logged seed (assert in a
+      test).
+- [ ] Snapshot of phase result file shape.
+- [ ] `PhaseError::QuorumLost` raised when abstentions exceed threshold.
+- [ ] `Vote` aggregator registered in `src/aggregator/concat.rs`
+      `Aggregator::kind()` so the schema label round-trips.
+
+## Demotion clause
+
+If no concrete first use case lands by M3 start, close as Won't-do (v0)
+and document the deferral in the roadmap. (Per roadmap; M3 date not yet
+fixed.)
+
+## Dependencies
+
+- `family_of` lookup from [CLO-265](https://linear.app/cloud-ai/issue/CLO-265)
+  is merged to main and exercised by LLMJudge.
+- `PhaseError::QuorumLost` variant may need to be added to
+  `src/family.rs` if it does not already exist.
+
+## Related
+
+- PRD FR-12 (Vote aggregator, Should)
+- Design doc §7 aggregators, §8 open question on ballot schema
+- Roadmap task T-019 in `docs/plans/001-implementation-roadmap.md`
+- Existing `majority_vote` in `src/consensus.rs` (different concern,
+  but shares normalisation logic).
+- CLO-268 (LLMJudge) demonstrates how to wire a new aggregator into
+  `ParallelFanOut`.
diff --git a/docs/reviews/CLO-269-review-ollama.md b/docs/reviews/CLO-269-review-ollama.md
new file mode 100644
index 0000000..771ffb0
--- /dev/null
+++ b/docs/reviews/CLO-269-review-ollama.md
@@ -0,0 +1,148 @@
+# Design Review: CLO-269
+
+**Reviewer**: Codex via Ollama (glm-5.1:cloud)
+**Reviewed**: 2026-04-29
+**Pipeline**: lok design-review
+
+---
+
+Now I have all the context needed. Let me compose the review.
+
+---
+
+## 1. Completeness Check
+
+| Section | Present | Assessment |
+|---------|---------|------------|
+| Problem | Yes | Clear, traces to discovery report |
+| Goals / Non-goals | Yes | Well-scoped; Enum deferred, weighted voting excluded |
+| Architecture (modules, data flow, types, API) | Yes | Thorough; module layout, data flow, new types, public API all specified |
+| Implementation details | Yes | Detailed pseudocode for all paths including tie-breakers |
+| Test plan | Yes | Unit + integration + snapshot test matrix |
+| Migration / rollout | Yes | Additive-only changes to existing enums |
+| Discovery debt resolution | Yes | Three items resolved |
+| Open questions | Yes | Three items, all with proposed resolutions |
+
+**Missing**: No explicit Acceptance Criteria section (the PRD FR-12 has AC, but this design doc should restate/concretize them). No rollback plan section (what to revert if Vote causes a regression mid-milestone).
+
+---
+
+## 2. Architecture Assessment
+
+**Strengths**:
+- Vote is a **pure, synchronous** aggregator - no backend call, no `PhaseContext`, no `async`. This is the single best design decision in the doc. It means full unit-testability without mocking.
+- Reuses the existing `BranchOutcome` / `BranchSuccess` / `BranchFailure` types from `concat.rs` rather than inventing parallel types.
+- `PhaseError::QuorumLost` is additive on an already `#[non_exhaustive]` enum - no breakage risk.
+- `TieBreak` variants are well-chosen: `ClosestToFamily` leverages the existing `family_of` infrastructure, `Random { seed }` is deterministic-for-reproducibility, `FirstResponder` maps to arrival order.
+- The `abstain_threshold` semantics (strict greater-than) are clearly documented with examples.
+- `VoteResult` carries full metadata (vote_counts, tie_broken, tie_break_rule) for traceability in the aggregated artefact.
+
+**Concerns**:
+
+1. **`BranchOutcome::Abstain` does not exist** (design §4.2 line 323). The current `BranchOutcome` enum has only `Success` and `Failure` variants. The pseudocode references `BranchOutcome::Abstain` as an arm, implying a new variant. Either add it (breaking: `#[non_exhaustive]` protects downstream matches) or map abstention cases onto `Failure` with a distinguisher. The design should state which path explicitly.
+
+2. **Cross-family enforcement for Vote is absent** (PRD FR-13). The PRD states `LLMJudge` and `Vote` both enforce cross-family by default. For Vote, which has no judge, the enforcement would mean: "all voting backends must be from distinct families." The design doc is silent on this. Either argue that Vote is inherently counting diversity (so enforcement is less critical) or add a `require_cross_family: bool` to `VoteConfig` and enforce it analogous to `LLMJudge`.
+
+3. **The `compute_vote` function signature is inconsistent** with the `VoteCandidate` struct. §3.4 defines `compute_vote(candidates: &[(&str, String, usize)], ...)` using an anonymous tuple, while §4.2 builds `VoteCandidate` structs. Pick one; `VoteCandidate` is clearer and should be the canonical type.
+
+4. **`winner` in `VoteResult` stores *normalised* text**, not the original. Downstream phases consuming the aggregated artefact will see lowercase-trimmed text, not the original response. Consider storing both the normalised key and the original winning text, or clarifying that `winner` is intentionally the normalised form.
+
+---
+
+## 3. Alignment with Handoff & Roadmap
+
+| Check | Result |
+|-------|--------|
+| Matches handoff WHY (cross-family aggregation) | Yes - Vote is an aggregator primitive |
+| Matches active milestone M1 (TensorZero backend) | **No, but correctly** - T-019 (Vote) is M3 (Aggregator vocabulary), and the roadmap explicitly lists it in Phase 3. The design doc is aligned with the *correct* milestone, not the *active* milestone. CLAUDE.md says M1 is active; this design is for a Phase 3 task that depends on T-015 (already shipped). |
+| Follows TDD-first convention (handoff §Intent) | Partially - the test plan is good, but there is no "failing test contract" written first as handoff mandates. The design lists test cases but doesn't define the contract before implementation. |
+| `make check` compatibility | Yes - all proposed tests are unit/integration tests runnable under `cargo test` |
+| PRD FR-12 alignment | Yes - ballot schema, tie-breakers, abstentions all specified; property test mention aligns with FR-12 AC |
+
+**Contradiction**: The design says the `Vote` variant goes into `src/aggregator/concat.rs` (the file that houses the behavioral `Aggregator` enum). This is correct per the current code layout but the module name "concat.rs" is misleading once it houses four non-concat variants. Consider whether `Aggregator` should migrate to its own `aggregator.rs` or `mod.rs`-inline module, though this is a style concern, not a blocking issue.
+
+---
+
+## 4. Security Review
+
+- **No shell execution**, no network calls, no secret handling in Vote. The module is pure computation on already-collected branch outputs. Low risk.
+- **`TieBreak::Random { seed }`** receives seed from `lok.toml` config (not from user CLI input or environment variables). If the seed were ever user-controlled (e.g., from a prompt template), it would be a denial-of-reproducibility vector, not a security vulnerability per the handoff threat model. The design correctly notes the seed is logged in the artefact metadata.
+- **`rand` crate**: The design notes `rand` 0.8 is already an indirect dependency via `uuid`. Adding it to direct dependencies is acceptable. Ensure it's added only as needed (`rand::rngs::StdRng`, `rand::seq::SliceRandom`, `rand::SeedableRng`) and not the full `rand` facade.
+- **No input from attacker-controlled sources**: The ballot text comes from backend `stdout`, which is already trusted-at-source (backend responses are consumed verbatim by other aggregators).
+
+**Verdict**: No security concerns for this module.
+
+---
+
+## 5. Implementation Concerns
+
+1. **`VoteConfig` missing `Serialize`/`Deserialize`**. It must be parsed from `lok.toml`. Add `#[derive(Serialize, Deserialize)]` or implement custom deserialization. Current `Aggregator::Vote { config: VoteConfig }` will need it for the TOML workflow parser (T-033).
+
+2. **`BallotSchema` is not `#[non_exhaustive]`**. The design says `Enum` is "reserved for v0+1" but the enum itself needs `#[non_exhaustive]` to permit adding `Enum` without a semver break.
+
+3. **`VoteError::NoOpinion` is a niche case**. When every backend returns identical text after normalisation, `NoOpinion` fires. But the pseudocode in §4.2 doesn't handle it - it's defined but never returned from `aggregate_vote`. Either remove it or implement it (if all buckets have count 1 and only one bucket exists, it's technically a "unanimous" result, not "no opinion").
+
+4. **Vote dispatcher in `parallel_fanout.rs` must not short-circuit**. The current code has `if !is_any_fail && !is_llm_judge && successes >= self.min_responses { break; }` (line 194). For Vote, this short-circuit must NOT kick in because Vote needs *all* branch outcomes (including failures as abstentions) to count correctly. The design doc doesn't call out this interaction explicitly. The `is_llm_judge` guard already prevents short-circuit for LLMJudge; a similar `is_vote` guard is needed, or the short-circuit condition must be generalised.
+
+5. **Arrival order for `FirstResponder` is not guaranteed by `FuturesUnordered`**. The design assumes `attempts` preserves absolute arrival order, but `FuturesUnordered` yields in completion order, not dispatch order. The current `ParallelFanOut` code builds `attempts` by pushing in completion order. For `FirstResponder`, the design notes "arrival order" means *completion* order (which is what `FuturesUnordered` gives). This should be explicitly clarified: `FirstResponder` picks the bucket whose first *completed* branch arrived earliest, not the dispatch order.
+
+6. **Snapshot test against schema** (§5, Integration tests): The design says "Validate against `docs/schemas/phase_result_parallel.schema.json`". That schema must already include an `aggregator: "vote"` enum value. Currently the schema defines `"aggregator": {"enum": ["concat", "any_fail", "llm_judge"]}` - it needs `"vote"` added. This is a cross-cutting concern that the design doc doesn't flag.
+
+7. **Test file path**: The design references `tests/strategy_parallel_fanout.rs` for integration tests, which matches the existing file. Good.
+
+---
+
+## 6. Concurrency & Async
+
+Vote is purely synchronous and runs after the `FuturesUnordered` loop completes. No tokio concerns. No blocking calls in async paths. The function is `fn aggregate_vote(...)` not `async fn`. This is correct and well-designed.
+
+One interaction point: the vote dispatch in `parallel_fanout.rs` currently runs inside `async fn execute()`. The call to `aggregate_vote` will block the async task briefly (it iterates over branch outcomes and builds hash maps). For the expected N (typically 2-5 branches), this is negligible and not worth `spawn_blocking`. No action needed.
+
+---
+
+## 7. Blind Spots
+
+1. **Cross-family enforcement for Vote** (reiterated from §2). FR-13 mandates it. The design doesn't address it. Either add enforcement or document why Vote exempts.
+
+2. **Interaction with `min_responses` floor**. If `min_responses=2` and 2 of 3 backends succeed, Vote runs with 2 votes and 1 abstention. If `abstain_threshold=0`, this could trigger `QuorumLost` with a majority still present. The design should clarify: does `abstain_threshold` interact with `min_responses`, or are they independent gates? (They should be independent: `min_responses` gates whether the strategy returns results at all; `abstain_threshold` gates whether Vote refuses to count.)
+
+3. **What happens when Vote is the aggregator and `min_responses` short-circuits?** If `min_responses=2` and 2 of 5 succeed, `ParallelFanOut` may short-circuit before late branches complete. Vote would then operate on incomplete data. The design must either (a) disable short-circuit for Vote (like LLMJudge), or (b) state that Vote operates on whatever branches have completed by floor time. The existing code already disables short-circuit for LLMJudge; Vote should get the same treatment.
+
+4. **`loker: Vote aggregator metadata` HTML comment format** is not schema-validated. The design specifies a structured comment but it's freeform text inside the aggregated output. The `phase_result_parallel.schema.json` schema validates `StrategyOutput` (JSON), not the content of `aggregated.txt`. This is probably fine for v0 but should be noted.
+
+5. **`VoteError` vs `PhaseError` duplication**. The design defines `VoteError::QuorumLost` (in `vote.rs`) AND adds `PhaseError::QuorumLost` (in `family.rs`). The design doc §3.5 shows `PhaseError::QuorumLost`, but the `aggregate_vote` function returns `VoteError`. The parallel_fanout code will need to map `VoteError::QuorumLost` to `StrategyError::Phase(PhaseError::QuorumLost)` (analogous to how `LLMJudgeError` maps). This mapping should be documented.
+
+6. **`VoteResult.vote_counts` order**. The pseudocode sorts `vote_counts` descending at the end (`§4.2 line 381: result.vote_counts.sort_by(|a, b| b.1.cmp(&a.1))`). This destroys insertion order, making snapshot tests deterministic. Good - but document that `vote_counts` is sorted by count descending, not by normalised text.
+
+7. **`rand` crate feature gating**. If `rand` is added, it should be behind a feature flag or at minimum only import the minimal subset. The `rand` crate is large; consider `rand_core` + `rand_rngs` for just `StdRng::seed_from_u64` and `SliceRandom`.
+
+8. **No `impl Serialize` on `VoteResult`**. The design doesn't specify whether `VoteResult` is serialized into `trace.jsonl` or `summary.json`. If it is, it needs `Serialize`. If it's only in the HTML comment, `Serialize` is not needed. Clarify.
+
+---
+
+## 8. Verdict
+
+**APPROVE_WITH_SUGGESTIONS**
+
+The design is thorough, well-aligned with the codebase, and introduces a clean, testable pure function. The core architecture (synchronous, no backend call, `BranchOutcome` reuse, deterministic tie-breakers) is sound. The concerns above are fixable without redesigning the module.
+
+---
+
+## 9. Actionable Feedback
+
+| Priority | Item | Action |
+|----------|------|--------|
+| P0 | **Short-circuit guard for Vote** | Add `is_vote` check alongside `is_llm_judge` in `parallel_fanout.rs` to prevent `min_responses` short-circuit. Vote must collect all branches. |
+| P0 | **`BranchOutcome::Abstain` either add or remove** | The pseudocode references `BranchOutcome::Abstain` (§4.2 line 323). Either add it to the enum (with `#[non_exhaustive]` protection) or map abstention to `Failure` with a distinguisher. |
+| P0 | **Cross-family enforcement for Vote** | Address FR-13. Add `require_cross_family: bool` to `VoteConfig` or document in non-goals why Vote exempts. |
+| P1 | **`VoteConfig` needs `Serialize`/`Deserialize`** | Add derives for TOML config parsing. |
+| P1 | **`BallotSchema` needs `#[non_exhaustive]`** | Required for v0+1 `Enum` variant addition without semver break. |
+| P1 | **Consistent `VoteCandidate` type** | Replace the `(&str, String, usize)` tuple in `compute_vote` with the `VoteCandidate` struct. |
+| P1 | **Map `VoteError` to `StrategyError`** | Document the mapping from `VoteError::QuorumLost` to `StrategyError::Phase(PhaseError::QuorumLost)` in parallel_fanout dispatch. |
+| P1 | **Add "vote" to `phase_result_parallel.schema.json`** | The JSON schema's `aggregator` enum must include `"vote"`. |
+| P2 | **Clarify `winner` is normalised text** | Add a comment or field: `original_text` alongside `winner`, or document that downstream consumers receive only the normalised form. |
+| P2 | **Remove or implement `VoteError::NoOpinion`** | It's defined but never returned. Either implement it (all buckets single-entry with count 1) or remove it. |
+| P2 | **Document `vote_counts` sort order** | Note in the type or doc comment that `vote_counts` is sorted by count descending. |
+| P2 | **Minimise `rand` dependency** | Import only `rand::rngs::StdRng` and `rand::seq::SliceRandom`, not the full `rand` crate. Consider `rand_core` for a slimmer dependency. |
+| P3 | **Add acceptance criteria section** | Restate FR-12's AC as concrete, testable criteria in the design doc. |
+| P3 | **Rename `concat.rs` or extract `Aggregator`** | Four variants in a file named for one variant is a smell. Not blocking, but consider `aggregator.rs` for the enum and keeping `concat.rs` for just the `aggregate_concat` function. |
diff --git a/docs/reviews/clo-269-design-gemini.md b/docs/reviews/clo-269-design-gemini.md
new file mode 100644
index 0000000..1e22e15
--- /dev/null
+++ b/docs/reviews/clo-269-design-gemini.md
@@ -0,0 +1,56 @@
+# Design Review: CLO-269
+
+**Reviewer**: Gemini 3.1 Pro
+**Reviewed**: 2026-04-29
+**Pipeline**: lok design-review
+
+---
+
+## 1. Completeness Check
+- **Problem & Goals**: Present and clear. Good scoping of v0 features versus deferred features (e.g., Enum schema).
+- **Architecture & Data Flow**: Present. Accurately identifies that `vote.rs` should be pure and synchronous, contrasting well with `LLMJudge`.
+- **Types & Public API**: Present. Clean enums and config structs. `BranchOutcome` reuse is appropriate.
+- **Implementation Details**: Present. Properly identifies that arrival order dictates a new tracking approach in the `ParallelFanOut` loop.
+- **Test Plan**: Present and thorough. Both unit and integration scenarios are covered.
+- **Migration & Debt**: Present. Safe additive change.
+
+## 2. Architecture Assessment
+**Strengths**:
+- Pushing the aggregation logic into a pure, synchronous `vote.rs` module makes it highly testable without async overhead or mock backends.
+- Reusing the existing `BranchOutcome` types respects the established domain boundaries between strategies and aggregators.
+- Clear separation of `TieBreak` policies gives workflow authors strong control over non-deterministic edge cases.
+
+**Concerns**:
+- **Determinism Bug**: In section 4.2, `buckets` is defined as a `HashMap`. Rust's `HashMap` uses a randomized hasher by default. This means `buckets.iter()` yields the tied candidates in a random order on every execution. In `resolve_tie`, `TieBreak::Random { seed }` shuffles this randomly-ordered list, entirely defeating the fixed seed determinism.
+- **Short-circuiting Interaction**: `ParallelFanOut::execute` currently short-circuits via `break` when `successes >= self.min_responses`. The design doc does not specify altering this. If a Vote phase is stopped early, it might falsely declare a tie or miss a quorum when a tie-breaking vote is still in flight.
+
+## 3. Alignment with Handoff & Roadmap
+- The design perfectly matches the intent of the handoff document and PRD (specifically FR-12, Vote aggregator).
+- By limiting normalisation to `trim().to_lowercase()`, it honors the strict "no semantic-similarity scoring in v0" constraint while delivering exactly what's needed for mechanical consensus.
+- Additive changes strictly fit into the M1/M3 milestones without breaking existing functionality.
+
+## 4. Security Review
+- **HTML Comment Injection**: The `AggregatedArtifact` embeds untrusted backend output inside a markdown comment (`<!-- loker: Vote aggregator metadata ... -->`). If a backend generates the text `-->` inside its winning answer, it will escape the comment block and inject the metadata payload directly into the visible markdown document. 
+- The use of `rand` with `StdRng` for `TieBreak::Random` is perfectly safe. Seed extraction from config is a standard reproducible-run pattern. No other boundaries are affected.
+
+## 5. Implementation Concerns
+- **Error Mapping**: The design introduces `VoteError::NoCandidates` and `VoteError::NoOpinion` but doesn't explicitly map them to `StrategyError` or `PhaseError`. These should map cleanly so the orchestrator knows whether to retry or fail the run.
+- **`ClosestToFamily` Ambiguity**: If two different answers tie for the most votes, and *both* answers have at least one candidate matching the target family, `ClosestToFamily` currently picks whichever the iterator yields first (which is random due to `HashMap`). It needs a defined fallback (e.g., `FirstResponder` among the matching subsets).
+
+## 6. Concurrency & Async
+- Excellent approach. By passing `&[BranchOutcome]` to `aggregate_vote`, the entire vote counting and tie-breaking process remains synchronous and CPU-bound, which for small strings is virtually instantaneous and will not block the tokio runtime. 
+- Using `FuturesUnordered` implicitly collects in arrival order, making `FirstResponder` trivial to implement.
+
+## 7. Blind Spots
+- **`min_responses` short-circuiting**: Vote aggregators typically require all votes to be cast to determine a definitive majority. The design misses the `!is_vote` condition needed in `ParallelFanOut::execute` to prevent premature short-circuiting.
+- **Iterating HashMaps**: The assumption that a `HashMap` can be used to feed a deterministic pseudo-random number generator (PRNG) or order-dependent tie-breaker.
+
+## 8. Verdict
+APPROVE_WITH_SUGGESTIONS
+
+## 9. Actionable Feedback
+1. **Fix non-determinism**: In `aggregate_vote`, use a `BTreeMap<String, Vec<usize>>` instead of `HashMap`, OR explicitly sort the `winners` slice alphabetically before applying any tie-breakers. This ensures `TieBreak::Random` and other policies are perfectly deterministic across runs.
+2. **Disable short-circuiting for Vote**: Update the loop condition in `src/strategy/parallel_fanout.rs` to ensure all branches resolve before aggregating: `if !is_any_fail && !is_llm_judge && !is_vote && successes >= self.min_responses { break; }`. 
+3. **Prevent HTML comment injection**: In `build_aggregated_text`, sanitize `result.winner` and `result.tie_break_rule` by replacing `-->` with `-- >` before formatting them into the metadata comment block.
+4. **Stabilize `ClosestToFamily`**: If multiple tied answers contain candidates from the target family, `ClosestToFamily` should explicitly fall back to `FirstResponder` among those matching answers, rather than returning the first match from the iterator.
+5. **Complete Error Mapping**: Specify in the PR how `VoteError::NoCandidates` and `VoteError::NoOpinion` are converted into the encompassing `PhaseError` (e.g. `PhaseError::AggregatorRejected`).
diff --git a/docs/reviews/clo-269-design-synthesis.md b/docs/reviews/clo-269-design-synthesis.md
new file mode 100644
index 0000000..5b07530
--- /dev/null
+++ b/docs/reviews/clo-269-design-synthesis.md
@@ -0,0 +1,68 @@
+# Review Synthesis: CLO-269
+
+**Synthesized**: 2026-04-29
+**Pipeline**: lok design-review
+**Reviewers**: Gemini 3.1 Pro, Codex/Ollama (glm-5.1:cloud), Claude (fallback if needed)
+
+---
+
+## Reviewer Status
+| Reviewer | Status | Detail |
+|----------|--------|--------|
+| Gemini | OK | Returned full structured review with verdict APPROVE_WITH_SUGGESTIONS |
+| Ollama | OK | Returned full structured review with verdict APPROVE_WITH_SUGGESTIONS |
+| Claude Fallback | SKIPPED | External reviewers succeeded |
+
+## Agreement (High Confidence)
+| # | Finding | Severity |
+|---|---------|----------|
+| 1 | `ParallelFanOut::execute` short-circuits via `min_responses` and lacks an `is_vote` guard analogous to `is_llm_judge`; Vote must wait for all branches before aggregating, otherwise quorum/tie results are incorrect | P0 / Critical |
+| 2 | Determinism / ordering bug in tie-break paths: `HashMap` iteration order plus `FuturesUnordered` arrival order make `TieBreak::Random { seed }`, `ClosestToFamily`, and `FirstResponder` non-deterministic without an explicit canonical ordering (sorted keys, documented arrival semantics) | P0 / High |
+| 3 | `VoteError` -> `PhaseError` / `StrategyError` mapping is undefined: `NoCandidates`, `NoOpinion`, `QuorumLost` need an explicit conversion analogous to `LLMJudgeError`, so the orchestrator knows whether to retry or fail | P1 / High |
+| 4 | `VoteError::NoOpinion` is defined but never returned in the §4.2 pseudocode - either implement the trigger condition or remove it | P2 / Medium |
+| 5 | `ClosestToFamily` lacks a defined fallback when multiple tied answers contain candidates from the target family - needs explicit secondary rule (e.g., `FirstResponder` over the matching subset) | P1 / Medium |
+
+## Disagreement (Needs Human Decision)
+| # | Topic | Position A (Reviewer) | Position B (Reviewer) |
+|---|-------|----------------------|----------------------|
+| 1 | HTML comment injection in `AggregatedArtifact` metadata | Gemini: real concern - sanitize backend `winner`/`tie_break_rule` against `-->` to prevent comment escape | Ollama: notes the metadata comment is freeform/unschemaed but treats it as "probably fine for v0" with no security action |
+| 2 | Cross-family enforcement for Vote (FR-13) | Ollama: must address - either add `require_cross_family: bool` to `VoteConfig` or document the exemption | Gemini: did not raise this concern at all |
+| 3 | `BranchOutcome::Abstain` variant | Ollama: pseudocode references a variant that doesn't exist - must add (with `#[non_exhaustive]`) or map onto `Failure` | Gemini: did not flag; reads the design as compatible with current `BranchOutcome` |
+
+## Novel Insights (Single Reviewer)
+| # | Finding | Source | Severity |
+|---|---------|--------|----------|
+| 1 | `VoteConfig` needs `Serialize`/`Deserialize` derives for `lok.toml` parsing (T-033) | Ollama | P1 |
+| 2 | `BallotSchema` should be `#[non_exhaustive]` so future `Enum` variant doesn't break semver | Ollama | P1 |
+| 3 | `compute_vote` signature uses `(&str, String, usize)` tuples while §4.2 builds `VoteCandidate` structs - pick one canonical type | Ollama | P1 |
+| 4 | `VoteResult.winner` stores normalised (trim/lowercase) text; downstream consumers lose the original casing - either store both or document the normalisation | Ollama | P2 |
+| 5 | `phase_result_parallel.schema.json` aggregator enum must add `"vote"` - cross-cutting change not flagged in design | Ollama | P1 |
+| 6 | `VoteResult.vote_counts` sort order (descending by count) should be documented for snapshot determinism | Ollama | P2 |
+| 7 | `rand` dependency footprint - import only `StdRng` + `SliceRandom` (or `rand_core`) instead of full `rand` facade | Ollama | P2 |
+| 8 | Interaction between `min_responses` floor and `abstain_threshold` is undefined - clarify they are independent gates | Ollama | P2 |
+| 9 | `VoteResult` Serialize requirement depends on whether it lands in `trace.jsonl`/`summary.json` vs only the HTML comment - clarify | Ollama | P2 |
+| 10 | No explicit Acceptance Criteria section restating FR-12 ACs, and no rollback plan | Ollama | P3 |
+| 11 | Naming: `concat.rs` housing four aggregator variants is a code smell; consider `aggregator.rs` | Ollama | P3 |
+| 12 | HTML comment metadata injection via raw `-->` in winner text | Gemini | P1 (security) |
+
+## Consolidated Verdict
+**APPROVE_WITH_SUGGESTIONS** - both reviewers independently arrived at the same verdict; no NEEDS_REVISION.
+
+## Priority Actions
+1. **P0 - Disable `min_responses` short-circuit for Vote.** In `src/strategy/parallel_fanout.rs`, add `is_vote` guard to the early-break condition: `if !is_any_fail && !is_llm_judge && !is_vote && successes >= self.min_responses { break; }`. (Both reviewers, agreement.)
+2. **P0 - Make tie-break deterministic.** Replace `HashMap` with `BTreeMap<String, Vec<usize>>` in `aggregate_vote`, or sort tied keys before applying `TieBreak::Random { seed }`, `ClosestToFamily`, and `FirstResponder`. Explicitly document that `FirstResponder` uses `FuturesUnordered` completion order, not dispatch order. (Both reviewers, agreement.)
+3. **P1 - Resolve `BranchOutcome::Abstain` ambiguity.** Either add the variant (relying on `#[non_exhaustive]`) or map abstentions onto `BranchOutcome::Failure` with a distinguishing field; update §4.2 pseudocode accordingly. (Ollama.)
+4. **P1 - Define `VoteError` -> `PhaseError`/`StrategyError` mapping.** Document conversions for `NoCandidates`, `NoOpinion`, `QuorumLost` analogous to `LLMJudgeError`. (Both reviewers.)
+5. **P1 - Address cross-family enforcement (FR-13).** Add `require_cross_family: bool` to `VoteConfig` or explicitly document the exemption with rationale. (Ollama.)
+6. **P1 - Fix `ClosestToFamily` ambiguity.** Specify fallback (e.g., `FirstResponder` over the matching subset) when multiple tied answers contain candidates from the target family. (Gemini.)
+7. **P1 - Sanitize HTML comment metadata.** Replace `-->` with `-- >` in `winner`/`tie_break_rule` before formatting into the `<!-- loker: ... -->` block. (Gemini.)
+8. **P1 - Add `"vote"` to `docs/schemas/phase_result_parallel.schema.json` aggregator enum.** (Ollama.)
+9. **P1 - Add `Serialize`/`Deserialize` to `VoteConfig` and `#[non_exhaustive]` to `BallotSchema`.** (Ollama.)
+10. **P1 - Unify `compute_vote` signature** on the `VoteCandidate` struct (drop the tuple form). (Ollama.)
+11. **P2 - Resolve `VoteError::NoOpinion`** - implement the trigger or remove it. (Both reviewers.)
+12. **P2 - Document `winner` normalisation, `vote_counts` sort order, `min_responses`/`abstain_threshold` independence, and `VoteResult` Serialize requirement.** (Ollama.)
+13. **P2 - Trim `rand` import surface** to `StdRng` + `SliceRandom`. (Ollama.)
+14. **P3 - Add Acceptance Criteria section restating FR-12 ACs and a rollback plan.** Optional rename of `concat.rs` -> `aggregator.rs`. (Ollama.)
+
+## Decision Recommendation
+**PROCEED_WITH_FIXES.** Both reviewers approve the core architecture (pure synchronous `vote.rs`, `BranchOutcome` reuse, deterministic tie-break vocabulary). Before merging implementation, resolve at minimum the P0 items (short-circuit guard, deterministic ordering) and the P1 items where reviewers agree (error mapping, schema enum update, `Abstain` variant decision, cross-family stance). The disagreement items (HTML comment sanitization, cross-family enforcement, `Abstain` variant) need explicit human decisions but each has a low-effort safe default - apply Gemini's sanitization and Ollama's `Abstain`/cross-family clarifications unless there's a reason to deviate.
diff --git a/docs/status/clo-269-workflow.yaml b/docs/status/clo-269-workflow.yaml
new file mode 100644
index 0000000..bc56eea
--- /dev/null
+++ b/docs/status/clo-269-workflow.yaml
@@ -0,0 +1,185 @@
+task_id: clo-269
+task_type: development
+classification_reason: >-
+  Issue title starts with Implement and includes open design decisions (ballot schema/tie-breakers), so classification
+  is development.
+task_profile:
+  has_backend: false
+  has_frontend: false
+  has_data_model: false
+  has_external_deps: false
+  skip_probe: false
+pending_human_action: null
+linear:
+  team: Cloud-ai
+  project: Loker
+  status_at_start: Backlog
+  blocks: []
+  blocked_by:
+    - CLO-265
+  priority: Low
+  branch_suggested: kmamemo/clo-269-implement-aggregatorvote-with-ballot-schema-and-tie-breakers
+  branch_actual: feat/clo-269
+workflow:
+  current_phase: implement
+  status: active
+  created_at: '2026-04-29T15:46:46.491Z'
+  updated_at: '2026-04-29T19:56:36.549Z'
+phases:
+  discovery:
+    status: complete
+    approach_chosen: B - Extract vote.rs module under src/aggregator/
+    approaches_identified: 3
+    approved: true
+    baseline_score: 7
+    discovery_debt:
+      - follow_up: TDD (design phase) will lock exact BallotSchema enum shape.
+        item: Ballot schema shape (enum vs free-text) must be decided in TDD doc before implementation.
+      - follow_up: TDD (design phase) will decide.
+        item: Seed source for TieBreak::Random — whether from lok.toml/workflow config or derived from run-level UUID.
+      - follow_up: TDD (design phase) will lock exact spelling and semantics.
+        item: 'Quorum threshold semantics: absolute count vs fraction.'
+    discovery_report: docs/discovery/clo-269.md
+    prd_created: true
+    prd_exists: true
+    prd_file: docs/prds/clo-269-aggregator-vote.md
+    problem_framed: true
+  design:
+    status: complete
+    design_doc: docs/designs/clo-269-aggregator-vote.md
+    discovery_context_used: true
+    draft_ready: true
+    applied_suggestions:
+      - 'Added #[non_exhaustive] to BallotSchema for v0+1 extensibility'
+      - Added Serialize/Deserialize derives to VoteConfig for TOML parsing
+      - Removed VoteError::NoOpinion (unanimous single-bucket is a valid win, not an error)
+      - Unified compute_vote signature on canonical VoteCandidate struct
+      - Added VoteCandidate struct definition to Public API surface
+      - 'Added StrategyError mapping: QuorumLost → PhaseError::QuorumLost, NoCandidates → PhaseError::AggregatorRejected'
+      - Replaced HashMap with BTreeMap in vote counting for deterministic tie-break ordering
+      - >-
+        Fixed ClosestToFamily fallback: when multiple tied buckets match the target family, apply FirstResponder among
+        the matching subset
+      - Added HTML comment sanitization note (replace --> with -- >)
+      - Added is_vote short-circuit guard in parallel_fanout.rs to collect all branches
+      - Updated all_abstain test expectation
+      - Updated migration/rollout with phase_result_parallel.schema.json 'vote' enum addition
+      - Updated open questions with min_responses/abstain_threshold independence and rand import minimization
+    flagged_suggestions:
+      - id: cross_family_enforcement
+        reason: >-
+          PRD FR-13 scopes cross-family enforcement to LLMJudge only. Vote has no judge and is inherently counting
+          diversity. Cross-family selection belongs to the strategy layer, not the aggregator. Documented as non-goal in
+          design.
+      - id: rename_concat_rs
+        reason: >-
+          Style concern (P3) - four variants in concat.rs is a smell, but renaming introduces churn without functional
+          benefit. Deferred to future refactoring (T-029 Aggregator trait formalization).
+    review_completed: true
+    review_gemini: docs/reviews/clo-269-design-gemini.md
+    review_synthesis: docs/reviews/clo-269-design-synthesis.md
+    review_verdict: approve_with_changes
+    finalized: true
+  plan:
+    status: complete
+    plan_file: docs/plans/clo-269-aggregator-vote.md
+    approved: true
+  implement:
+    status: complete
+    commits:
+      - c9b13d5
+      - 364f7a5
+      - 5affc5a
+      - 5c4fe89
+      - 8c010be
+  pr:
+    status: pending
+  complete:
+    status: pending
+history:
+  - timestamp: '2026-04-29T15:46:46.491Z'
+    action: workflow_started
+    phase: init
+    details: Workflow initialized for clo-269 as development
+  - timestamp: '2026-04-29T15:47:09.238Z'
+    action: workflow_resumed
+    phase: init
+    details: Resuming existing workflow from init; skipping re-initialization path.
+  - timestamp: '2026-04-29T15:47:11.477Z'
+    action: linear_status_updated
+    phase: init
+    details: Linear issue status moved Backlog -> Todo per init workflow initialization.
+  - timestamp: '2026-04-29T15:47:14.548Z'
+    action: task_requalified
+    phase: init
+    details: Classified as development (implementation scope with architecture decisions on ballot schema and tie-break rules).
+  - timestamp: '2026-04-29T15:47:16.643Z'
+    action: project_sync_skipped
+    phase: init
+    details: No PROJECT.md/ROADMAP.md/DEPENDENCIES.md exist in this repo.
+  - timestamp: '2026-04-29T15:47:19.344Z'
+    action: metadata_adjusted
+    phase: init
+    details: Preserved linear status_at_start as Backlog while current linear status is now Todo.
+  - timestamp: '2026-04-29T15:50:13.010Z'
+    action: first_phase_dispatched
+    phase: init
+    details: Init completed; advancing to first phase 'discovery' for development task per task type and dispatch contract.
+  - timestamp: '2026-04-29T16:06:15.852Z'
+    action: branch_created
+    phase: discovery
+    details: Branch feat/clo-269 already checked out and even with main; recording as actual.
+  - timestamp: '2026-04-29T18:37:53.367Z'
+    action: discovery_approved
+    phase: discovery
+    details: >-
+      Approach B: Extract vote.rs module under src/aggregator/. Baseline 7/10. 3 approaches considered (A inline, B
+      extracted module, C reuse consensus.rs). PRD created from scratch. Discovery debt: 3 items to resolve in TDD
+      (ballot schema shape, Random seed source, quorum threshold semantics).
+  - timestamp: '2026-04-29T18:38:02.835Z'
+    action: discovery_status_fixed
+    phase: discovery
+    details: Set discovery.status to complete so transition to design is allowed.
+  - timestamp: '2026-04-29T18:38:06.397Z'
+    action: phase_transition
+    phase: discovery
+    details: Transitioned from discovery to design
+  - timestamp: '2026-04-29T18:45:47.143Z'
+    action: design_draft_ready
+    phase: design
+    details: Design doc draft at docs/designs/clo-269-aggregator-vote.md
+  - timestamp: '2026-04-29T19:21:18.964Z'
+    action: design_review_complete
+    phase: design
+    details: >-
+      Review verdict: approve_with_changes. 13 applied suggestions, 2 flagged. Reviews written to
+      docs/reviews/clo-269-design-gemini.md and docs/reviews/clo-269-design-synthesis.md.
+  - timestamp: '2026-04-29T19:21:46.173Z'
+    action: design_finalized
+    phase: design
+    details: >-
+      Design ready for plan phase. Review feedback applied: 13 additions, 2 flagged (cross-family enforcement, rename
+      concat.rs). All P0 items addressed (short-circuit guard, deterministic ordering). VoteError::NoOpinion variant
+      removed. BTreeMap used for determinism. HTML sanitization documented. StrategyError mapping specified.
+  - timestamp: '2026-04-29T19:21:49.711Z'
+    action: phase_transition
+    phase: design
+    details: Transitioned from design to plan
+  - timestamp: '2026-04-29T19:23:08.265Z'
+    action: plan_created
+    phase: plan
+    details: Plan with 6 sub-tasks at docs/plans/clo-269-aggregator-vote.md
+  - timestamp: '2026-04-29T19:23:14.565Z'
+    action: plan_approved
+    phase: plan
+    details: Plan approved. 6 sub-tasks queued (ST1–ST5 implementation + ST6 pre-merge gate).
+  - timestamp: '2026-04-29T19:23:20.683Z'
+    action: phase_transition
+    phase: plan
+    details: Transitioned from plan to implement
+  - timestamp: '2026-04-29T19:56:36.549Z'
+    action: implementation_complete
+    phase: implement
+    details: 'All sub-tasks landed (ST1–ST6). make check green. 4 commits: c9b13d5, 364f7a5, 5affc5a, 5c4fe89, 8c010be.'
+task_title: Implement Aggregator::Vote with ballot schema and tie-breakers
+task_url: https://linear.app/cloud-ai/issue/clo-269/implement-aggregatorvote-with-ballot-schema-and-tie-breakers
diff --git a/src/aggregator/concat.rs b/src/aggregator/concat.rs
index 47d998b..e96c8c0 100644
--- a/src/aggregator/concat.rs
+++ b/src/aggregator/concat.rs
@@ -1,5 +1,6 @@
 use std::sync::Arc;
 
+use crate::aggregator::VoteConfig;
 use crate::backend::Backend;
 use crate::strategy::PhaseContext;
 
@@ -39,6 +40,9 @@ pub enum Aggregator {
         prompt_template: String,
         require_judge_different_family: bool,
     },
+    Vote {
+        config: VoteConfig,
+    },
 }
 
 impl Aggregator {
@@ -62,6 +66,11 @@ impl Aggregator {
         }
     }
 
+    /// Build a Vote aggregator with the provided configuration.
+    pub fn vote(config: VoteConfig) -> Self {
+        Self::Vote { config }
+    }
+
     /// Build an AnyFail aggregator (no configuration needed).
     pub fn any_fail() -> Self {
         Self::AnyFail
@@ -73,6 +82,7 @@ impl Aggregator {
             Self::Concat { .. } => crate::strategy::Aggregator::Concat,
             Self::AnyFail => crate::strategy::Aggregator::AnyFail,
             Self::LLMJudge { .. } => crate::strategy::Aggregator::LLMJudge,
+            Self::Vote { .. } => crate::strategy::Aggregator::Vote,
         }
     }
 
@@ -91,6 +101,9 @@ impl Aggregator {
             Self::LLMJudge { .. } => Err(AggregatorError::Unsupported(
                 "LLMJudge requires async backend access; use aggregate_llm_judge()".into(),
             )),
+            Self::Vote { .. } => Err(AggregatorError::Unsupported(
+                "Vote is evaluated inline by ParallelFanOut, not via aggregate()".into(),
+            )),
         }
     }
 }
@@ -462,6 +475,19 @@ mod tests {
         );
     }
 
+    #[test]
+    fn vote_kind_maps_to_strategy_label() {
+        assert_eq!(
+            Aggregator::vote(crate::aggregator::VoteConfig {
+                ballot_schema: crate::aggregator::BallotSchema::FreeText,
+                tie_break: crate::aggregator::TieBreak::FirstResponder,
+                abstain_threshold: 0,
+            })
+            .kind(),
+            crate::strategy::Aggregator::Vote
+        );
+    }
+
     #[test]
     fn concat_mixed_success_failure_snapshot() {
         let artifact = Aggregator::concat("## {index}. {backend_id} ({family})")
diff --git a/src/aggregator/mod.rs b/src/aggregator/mod.rs
index e946160..f65ec9c 100644
--- a/src/aggregator/mod.rs
+++ b/src/aggregator/mod.rs
@@ -8,6 +8,7 @@
 
 mod concat;
 mod llm_judge;
+mod vote;
 
 pub use concat::{
     AggregateInput, AggregatedArtifact, Aggregator, AggregatorError, BranchFailure, BranchOutcome,
@@ -19,6 +20,11 @@ pub use llm_judge::{
     render_ballot_prompt, Ballot, Candidate, LLMJudgeError,
 };
 
+pub use vote::{
+    aggregate_vote, normalise_ballot, BallotSchema, TieBreak, VoteCandidate, VoteConfig, VoteError,
+    VoteResult,
+};
+
 use serde_json::Value;
 
 #[non_exhaustive]
diff --git a/src/aggregator/snapshots/loker__aggregator__vote__tests__vote_snapshot.snap b/src/aggregator/snapshots/loker__aggregator__vote__tests__vote_snapshot.snap
new file mode 100644
index 0000000..efb4abf
--- /dev/null
+++ b/src/aggregator/snapshots/loker__aggregator__vote__tests__vote_snapshot.snap
@@ -0,0 +1,16 @@
+---
+source: src/aggregator/vote.rs
+expression: artifact.text
+---
+yes
+
+<!-- loker: Vote aggregator metadata
+  winner: yes
+  total_branches: 3
+  vote_counts:
+    yes: 2
+    no: 1
+  abstain_count: 0
+  tie_broken: false
+  tie_break_rule: none (strict majority)
+-->
diff --git a/src/aggregator/vote.rs b/src/aggregator/vote.rs
new file mode 100644
index 0000000..433690a
--- /dev/null
+++ b/src/aggregator/vote.rs
@@ -0,0 +1,578 @@
+//! `Aggregator::Vote` implementation.
+//!
+//! Pure, synchronous vote counting over parallel branch outcomes.
+//! No secondary backend calls, no async, no I/O.
+
+use std::collections::BTreeMap;
+
+use crate::family::{family_of, Family};
+
+use super::{AggregatedArtifact, BranchOutcome};
+
+/// How a ballot is normalised and interpreted.
+#[derive(Debug, Clone, PartialEq, Eq)]
+#[non_exhaustive]
+pub enum BallotSchema {
+    /// Free text: each backend returns prose; normalise before bucketing.
+    FreeText,
+}
+
+/// How to resolve a tie when no strict majority exists.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub enum TieBreak {
+    /// Pick the candidate whose backend family matches the given family.
+    /// If multiple candidates match, first occurrence in arrival order wins.
+    ClosestToFamily(Family),
+    /// Deterministic shuffle from a fixed seed.
+    Random { seed: u64 },
+    /// Pick the candidate whose successful response arrived first.
+    FirstResponder,
+}
+
+/// Config payload for the Vote aggregator.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub struct VoteConfig {
+    pub ballot_schema: BallotSchema,
+    pub tie_break: TieBreak,
+    /// Number of abstentions (errors + malformed answers) that triggers
+    /// `QuorumLost`. Fires when **strictly more** than `abstain_threshold`
+    /// are abstentions.
+    pub abstain_threshold: usize,
+}
+
+/// A single candidate vote after normalisation.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub struct VoteCandidate {
+    pub backend_id: String,
+    pub family: String,
+    pub normalised: String,
+    pub arrival_order: usize,
+}
+
+/// Result of a vote aggregation, including metadata for traceability.
+#[derive(Debug, Clone, PartialEq, Eq)]
+pub struct VoteResult {
+    /// The winning text (normalised key).
+    pub winner: String,
+    /// Sorted descending by count for snapshot determinism.
+    pub vote_counts: Vec<(String, usize)>,
+    pub abstain_count: usize,
+    pub total_branches: usize,
+    pub tie_broken: bool,
+    pub tie_break_rule: String,
+}
+
+/// Errors specific to Vote aggregation.
+#[derive(Debug, thiserror::Error, PartialEq, Eq)]
+pub enum VoteError {
+    #[error("quorum lost: {abstains} abstentions exceed threshold {threshold}")]
+    QuorumLost { abstains: usize, threshold: usize },
+
+    #[error("no candidates available")]
+    NoCandidates,
+}
+
+/// Normalise a ballot text for comparison.
+pub fn normalise_ballot(text: &str) -> String {
+    text.trim().to_lowercase()
+}
+
+/// Aggregate vote outcomes from parallel branches.
+///
+/// Returns the aggregated artefact and structured result metadata.
+/// Pure synchronous function — no async, no backend calls.
+pub fn aggregate_vote(
+    branches: &[BranchOutcome],
+    config: &VoteConfig,
+) -> Result<(AggregatedArtifact, VoteResult), VoteError> {
+    let mut abstain_count = 0;
+    let mut candidates: Vec<VoteCandidate> = Vec::new();
+    let total = branches.len();
+
+    for (arrival_order, branch) in branches.iter().enumerate() {
+        match branch {
+            BranchOutcome::Success(success) => {
+                let normalised = normalise_ballot(&success.output);
+                if normalised.is_empty() {
+                    abstain_count += 1;
+                } else {
+                    candidates.push(VoteCandidate {
+                        backend_id: success.backend_id.clone(),
+                        family: success.family.clone(),
+                        normalised,
+                        arrival_order,
+                    });
+                }
+            }
+            BranchOutcome::Failure(_) => {
+                abstain_count += 1;
+            }
+        }
+    }
+
+    if abstain_count > config.abstain_threshold {
+        return Err(VoteError::QuorumLost {
+            abstains: abstain_count,
+            threshold: config.abstain_threshold,
+        });
+    }
+
+    if candidates.is_empty() {
+        return Err(VoteError::NoCandidates);
+    }
+
+    // Count votes by normalised bucket.
+    // BTreeMap ensures deterministic iteration order for tie-break determinism.
+    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
+    for (idx, c) in candidates.iter().enumerate() {
+        buckets.entry(c.normalised.clone()).or_default().push(idx);
+    }
+
+    let max_votes = buckets.values().map(|v| v.len()).max().unwrap_or(0);
+    let winners: Vec<&str> = buckets
+        .iter()
+        .filter(|(_, v)| v.len() == max_votes)
+        .map(|(k, _)| k.as_str())
+        .collect();
+
+    let mut result = if winners.len() == 1 {
+        let winner_text = winners[0];
+        VoteResult {
+            winner: winner_text.into(),
+            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
+            abstain_count,
+            total_branches: total,
+            tie_broken: false,
+            tie_break_rule: "none (strict majority)".into(),
+        }
+    } else {
+        let chosen_text = resolve_tie(&winners, &candidates, &buckets, &config.tie_break);
+        VoteResult {
+            winner: chosen_text,
+            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
+            abstain_count,
+            total_branches: total,
+            tie_broken: true,
+            tie_break_rule: format_tie_break_rule(&config.tie_break),
+        }
+    };
+
+    // Sort vote_counts descending for stable output
+    result.vote_counts.sort_by_key(|b| std::cmp::Reverse(b.1));
+
+    let text = build_aggregated_text(&result, &candidates, &buckets);
+    let artifact = AggregatedArtifact {
+        text,
+        successful: candidates.len(),
+        failed: abstain_count,
+    };
+
+    Ok((artifact, result))
+}
+
+fn resolve_tie(
+    tied_buckets: &[&str],
+    candidates: &[VoteCandidate],
+    buckets: &BTreeMap<String, Vec<usize>>,
+    tie_break: &TieBreak,
+) -> String {
+    match tie_break {
+        TieBreak::FirstResponder => tied_buckets
+            .iter()
+            .min_by_key(|&&bucket| {
+                buckets[bucket]
+                    .iter()
+                    .map(|&ci| candidates[ci].arrival_order)
+                    .min()
+                    .unwrap_or(usize::MAX)
+            })
+            .copied()
+            .unwrap_or(tied_buckets[0])
+            .to_string(),
+
+        TieBreak::ClosestToFamily(target_family) => {
+            let matching: Vec<&str> = tied_buckets
+                .iter()
+                .copied()
+                .filter(|&bucket| {
+                    buckets[bucket]
+                        .iter()
+                        .any(|&ci| family_of(&candidates[ci].backend_id) == *target_family)
+                })
+                .collect();
+
+            if matching.is_empty() {
+                resolve_tie(tied_buckets, candidates, buckets, &TieBreak::FirstResponder)
+            } else if matching.len() == 1 {
+                matching[0].to_string()
+            } else {
+                resolve_tie(&matching, candidates, buckets, &TieBreak::FirstResponder)
+            }
+        }
+
+        TieBreak::Random { seed } => {
+            use rand::rngs::StdRng;
+            use rand::Rng;
+            use rand::SeedableRng;
+
+            let mut rng = StdRng::seed_from_u64(*seed);
+            let idx = rng.random_range(0..tied_buckets.len());
+            tied_buckets[idx].to_string()
+        }
+    }
+}
+
+fn format_tie_break_rule(tie_break: &TieBreak) -> String {
+    match tie_break {
+        TieBreak::ClosestToFamily(f) => format!("closest_to_family({})", f),
+        TieBreak::Random { seed } => format!("random(seed={})", seed),
+        TieBreak::FirstResponder => "first_responder".into(),
+    }
+}
+
+fn build_aggregated_text(
+    result: &VoteResult,
+    candidates: &[VoteCandidate],
+    buckets: &BTreeMap<String, Vec<usize>>,
+) -> String {
+    // Pick the winner's original text from the first candidate in the winning bucket.
+    let winner_original = buckets
+        .get(&result.winner)
+        .and_then(|indices| {
+            indices
+                .first()
+                .and_then(|&idx| candidates.get(idx).map(|c| c.normalised.as_str()))
+        })
+        .unwrap_or(&result.winner);
+
+    let mut lines = Vec::new();
+    lines.push(winner_original.to_string());
+    lines.push(String::new());
+
+    // Build deterministic metadata comment block
+    lines.push("<!-- loker: Vote aggregator metadata".into());
+    lines.push(format!("  winner: {}", sanitize_comment(&result.winner)));
+    lines.push(format!("  total_branches: {}", result.total_branches));
+    lines.push("  vote_counts:".into());
+    for (text, count) in &result.vote_counts {
+        lines.push(format!("    {}: {}", sanitize_comment(text), count));
+    }
+    lines.push(format!("  abstain_count: {}", result.abstain_count));
+    lines.push(format!("  tie_broken: {}", result.tie_broken));
+    lines.push(format!(
+        "  tie_break_rule: {}",
+        sanitize_comment(&result.tie_break_rule)
+    ));
+    lines.push("-->".into());
+    lines.push(String::new());
+
+    lines.join("\n")
+}
+
+/// Replace `-->` with `-- >` to prevent premature HTML comment closure.
+fn sanitize_comment(text: &str) -> String {
+    text.replace("-->", "-- >")
+}
+
+#[cfg(test)]
+mod tests {
+    use super::super::BranchSuccess;
+    use super::*;
+
+    fn success(backend_id: &str, family: &str, output: &str) -> BranchOutcome {
+        BranchOutcome::Success(BranchSuccess {
+            backend_id: backend_id.into(),
+            family: family.into(),
+            index: 1,
+            output: output.into(),
+        })
+    }
+
+    fn failure(backend_id: &str, family: &str, reason: &str) -> BranchOutcome {
+        BranchOutcome::Failure(super::super::BranchFailure {
+            backend_id: backend_id.into(),
+            family: family.into(),
+            index: 1,
+            reason: reason.into(),
+        })
+    }
+
+    fn make_config(tie_break: TieBreak) -> VoteConfig {
+        VoteConfig {
+            ballot_schema: BallotSchema::FreeText,
+            tie_break,
+            abstain_threshold: 99,
+        }
+    }
+
+    #[test]
+    fn free_text_clear_winner() {
+        let branches = vec![
+            success("claude", "anthropic", "yes"),
+            success("codex", "openai", "yes"),
+            success("gemini", "google", "no"),
+        ];
+        let config = make_config(TieBreak::FirstResponder);
+        let (artifact, result) = aggregate_vote(&branches, &config).unwrap();
+        assert_eq!(result.winner, "yes");
+        assert!(!result.tie_broken);
+        assert_eq!(
+            result.vote_counts,
+            vec![("yes".into(), 2), ("no".into(), 1)]
+        );
+        assert_eq!(result.abstain_count, 0);
+        assert!(artifact.text.contains("yes"));
+    }
+
+    #[test]
+    fn free_text_tie_first_responder() {
+        let branches = vec![
+            success("claude", "anthropic", "yes"),
+            success("gemini", "google", "no"),
+        ];
+        let config = make_config(TieBreak::FirstResponder);
+        let (_, result) = aggregate_vote(&branches, &config).unwrap();
+        assert_eq!(result.winner, "yes");
+        assert!(result.tie_broken);
+        // "yes" arrives first (index 0), so FirstResponder picks it
+    }
+
+    #[test]
+    fn free_text_tie_closest_family() {
+        let branches = vec![
+            success("claude", "anthropic", "a"),
+            success("gemini", "google", "b"),
+        ];
+        let config = make_config(TieBreak::ClosestToFamily(Family::Anthropic));
+        let (_, result) = aggregate_vote(&branches, &config).unwrap();
+        assert_eq!(result.winner, "a");
+        assert!(result.tie_broken);
+        assert_eq!(result.tie_break_rule, "closest_to_family(anthropic)");
+    }
+
+    #[test]
+    fn free_text_tie_random_deterministic() {
+        let branches = vec![
+            success("claude", "anthropic", "a"),
+            success("gemini", "google", "b"),
+        ];
+        let config = make_config(TieBreak::Random { seed: 42 });
+        let (_, result1) = aggregate_vote(&branches, &config).unwrap();
+        let (_, result2) = aggregate_vote(&branches, &config).unwrap();
+        assert_eq!(result1.winner, result2.winner);
+        assert!(result1.tie_broken);
+    }
+
+    #[test]
+    fn abstain_backend_error() {
+        let branches = vec![
+            success("claude", "anthropic", "yes"),
+            success("gemini", "google", "yes"),
+            failure("codex", "openai", "network timeout"),
+        ];
+        let config = make_config(TieBreak::FirstResponder);
+        let (_, result) = aggregate_vote(&branches, &config).unwrap();
+        assert_eq!(result.abstain_count, 1);
+        assert_eq!(result.vote_counts, vec![("yes".into(), 2)]);
+    }
+
+    #[test]
+    fn quorum_lost() {
+        let branches = vec![
+            success("claude", "anthropic", "yes"),
+            failure("gemini", "google", "boom"),
+            failure("codex", "openai", "network"),
+        ];
+        let config = VoteConfig {
+            ballot_schema: BallotSchema::FreeText,
+            tie_break: TieBreak::FirstResponder,
+            abstain_threshold: 1,
+        };
+        let err = aggregate_vote(&branches, &config).unwrap_err();
+        assert_eq!(
+            err,
+            VoteError::QuorumLost {
+                abstains: 2,
+                threshold: 1,
+            }
+        );
+    }
+
+    #[test]
+    fn empty_input() {
+        let branches: Vec<BranchOutcome> = vec![];
+        let config = make_config(TieBreak::FirstResponder);
+        let err = aggregate_vote(&branches, &config).unwrap_err();
+        assert_eq!(err, VoteError::NoCandidates);
+    }
+
+    #[test]
+    fn all_abstain() {
+        let branches = vec![
+            failure("claude", "anthropic", "boom"),
+            failure("gemini", "google", "boom"),
+            failure("codex", "openai", "boom"),
+        ];
+        let config = VoteConfig {
+            ballot_schema: BallotSchema::FreeText,
+            tie_break: TieBreak::FirstResponder,
+            abstain_threshold: 1,
+        };
+        let err = aggregate_vote(&branches, &config).unwrap_err();
+        assert_eq!(
+            err,
+            VoteError::QuorumLost {
+                abstains: 3,
+                threshold: 1,
+            }
+        );
+    }
+
+    #[test]
+    fn normalise_case() {
+        let branches = vec![
+            success("a", "anthropic", "YES"),
+            success("b", "openai", "yes"),
+            success("c", "google", "Yes"),
+        ];
+        let config = make_config(TieBreak::FirstResponder);
+        let (_, result) = aggregate_vote(&branches, &config).unwrap();
+        assert_eq!(result.vote_counts.len(), 1);
+        assert_eq!(result.vote_counts[0].1, 3);
+    }
+
+    #[test]
+    fn normalise_whitespace() {
+        let branches = vec![
+            success("a", "anthropic", "  yes  "),
+            success("b", "openai", "yes\n"),
+        ];
+        let config = make_config(TieBreak::FirstResponder);
+        let (_, result) = aggregate_vote(&branches, &config).unwrap();
+        assert_eq!(result.vote_counts.len(), 1);
+        assert_eq!(result.vote_counts[0].1, 2);
+    }
+
+    #[test]
+    fn closest_family_no_match_fallback() {
+        let branches = vec![
+            success("gemini", "google", "a"),
+            success("openai", "openai", "b"),
+        ];
+        let config = make_config(TieBreak::ClosestToFamily(Family::Anthropic));
+        let (_, result) = aggregate_vote(&branches, &config).unwrap();
+        // No match for Anthropic: falls back to FirstResponder
+        assert!(result.tie_broken);
+    }
+
+    #[test]
+    fn closest_family_multiple_matching_buckets() {
+        let branches = vec![
+            success("claude", "anthropic", "a"),
+            success("gemini", "google", "b"),
+            success("loker_d1_anthropic", "anthropic", "a"),
+        ];
+        let config = make_config(TieBreak::ClosestToFamily(Family::Anthropic));
+        let (_, result) = aggregate_vote(&branches, &config).unwrap();
+        // "a" has two Anthropic candidates, "b" has zero.
+        // ClosestToFamily matches "a" uniquely, so no need for fallback.
+        assert_eq!(result.winner, "a");
+    }
+
+    #[test]
+    fn closest_family_multiple_buckets_match() {
+        // Tie between "a" (anthropic + google) and "b" (anthropic + openai)
+        // When both tied buckets contain Anthropic, FirstResponder fallback
+        // among the matching subset should pick the one arriving first.
+        let branches = vec![
+            success("claude", "anthropic", "a"),
+            success("gemini", "google", "a"),
+            success("loker_d1_anthropic", "anthropic", "b"),
+            success("openai", "openai", "b"),
+        ];
+        let config = make_config(TieBreak::ClosestToFamily(Family::Anthropic));
+        let (_, result) = aggregate_vote(&branches, &config).unwrap();
+        // Both "a" and "b" have Anthropic candidates; tie -> FirstResponder
+        // picks the bucket whose first candidate arrived earliest.
+        // "a" arrives at 0, "b" arrives at 2.
+        assert_eq!(result.winner, "a");
+    }
+
+    #[test]
+    fn empty_ballot_counts_as_abstain() {
+        let branches = vec![
+            success("a", "anthropic", ""),
+            success("b", "openai", "yes"),
+            success("c", "google", "yes"),
+        ];
+        let config = make_config(TieBreak::FirstResponder);
+        let (_, result) = aggregate_vote(&branches, &config).unwrap();
+        assert_eq!(result.abstain_count, 1);
+        assert_eq!(result.winner, "yes");
+    }
+
+    #[test]
+    fn whitespace_only_ballot_counts_as_abstain() {
+        let branches = vec![
+            success("a", "anthropic", "   "),
+            success("b", "openai", "yes"),
+            success("c", "google", "yes"),
+        ];
+        let config = make_config(TieBreak::FirstResponder);
+        let (_, result) = aggregate_vote(&branches, &config).unwrap();
+        assert_eq!(result.abstain_count, 1);
+        assert_eq!(result.winner, "yes");
+    }
+
+    #[test]
+    fn vote_counts_sorted_descending() {
+        let branches = vec![
+            success("a", "anthropic", "yes"),
+            success("b", "openai", "yes"),
+            success("c", "google", "no"),
+            success("d", "zhipu", "maybe"),
+        ];
+        let config = make_config(TieBreak::FirstResponder);
+        let (_, result) = aggregate_vote(&branches, &config).unwrap();
+        assert_eq!(
+            result.vote_counts,
+            vec![
+                ("yes".into(), 2),
+                // "maybe" and "no" tie at 1; BTreeMap iteration order is alphabetical,
+                // and stable_sort preserves that relative order.
+                ("maybe".into(), 1),
+                ("no".into(), 1),
+            ]
+        );
+    }
+
+    #[test]
+    fn sanitize_comment_in_metadata() {
+        let branches = vec![success("a", "anthropic", "ok --> bad")];
+        let config = make_config(TieBreak::FirstResponder);
+        let (artifact, _) = aggregate_vote(&branches, &config).unwrap();
+        // The metadata should have sanitized the `-->` in the winner text.
+        assert!(artifact.text.contains("ok -- > bad"));
+        // Ensure the comment block is intact.
+        assert!(artifact.text.contains("-->\n"));
+    }
+
+    #[test]
+    fn vote_snapshot() {
+        let branches = vec![
+            success("claude", "anthropic", " YES "),
+            success("codex", "openai", "yes"),
+            success("gemini", "google", "no"),
+        ];
+        let config = make_config(TieBreak::FirstResponder);
+        let (artifact, _) = aggregate_vote(&branches, &config).unwrap();
+        insta::assert_snapshot!(artifact.text);
+    }
+
+    #[test]
+    fn normalise_ballot_basic() {
+        assert_eq!(normalise_ballot("  YES  "), "yes");
+        assert_eq!(normalise_ballot("Yes\n"), "yes");
+        assert_eq!(normalise_ballot(""), "");
+    }
+}
diff --git a/src/family.rs b/src/family.rs
index f7d24d4..88782cd 100644
--- a/src/family.rs
+++ b/src/family.rs
@@ -136,6 +136,12 @@ pub enum PhaseError {
     #[error("aggregator contract violation: {message}")]
     AggregatorContract { message: String },
 
+    #[error("quorum lost: {abstains} abstentions exceed threshold {threshold}")]
+    QuorumLost { abstains: usize, threshold: usize },
+
+    #[error("aggregator rejected: {message}")]
+    AggregatorRejected { message: String },
+
     #[error("judge unavailable: {detail}")]
     JudgeUnavailable { detail: String },
 }
@@ -403,6 +409,26 @@ mod tests {
         }
     }
 
+    #[test]
+    fn quorum_lost_display() {
+        let err = PhaseError::QuorumLost {
+            abstains: 3,
+            threshold: 2,
+        };
+        assert_eq!(
+            err.to_string(),
+            "quorum lost: 3 abstentions exceed threshold 2"
+        );
+    }
+
+    #[test]
+    fn aggregator_rejected_display() {
+        let err = PhaseError::AggregatorRejected {
+            message: "no candidates".into(),
+        };
+        assert_eq!(err.to_string(), "aggregator rejected: no candidates");
+    }
+
     #[test]
     fn judge_unavailable_display() {
         let err = PhaseError::JudgeUnavailable {
diff --git a/src/strategy/parallel_fanout.rs b/src/strategy/parallel_fanout.rs
index d793de2..66bf71d 100644
--- a/src/strategy/parallel_fanout.rs
+++ b/src/strategy/parallel_fanout.rs
@@ -120,8 +120,10 @@ impl Strategy for ParallelFanOut {
         let mut successes = 0;
         let mut successful_candidates: Vec<crate::aggregator::BranchSuccess> =
             Vec::with_capacity(self.targets.len());
+        let mut vote_branches: Vec<crate::aggregator::BranchOutcome> = Vec::new();
         let is_any_fail = matches!(self.aggregator, Aggregator::AnyFail);
         let is_llm_judge = matches!(self.aggregator, Aggregator::LLMJudge { .. });
+        let is_vote = matches!(self.aggregator, Aggregator::Vote { .. });
 
         while let Some((idx, result)) = futures.next().await {
             let target = &self.targets[idx];
@@ -173,12 +175,17 @@ impl Strategy for ParallelFanOut {
                     }
 
                     successes += 1;
-                    successful_candidates.push(BranchSuccess {
+                    let branch_success = BranchSuccess {
                         backend_id: target.backend.clone(),
                         family: family_of(&target.backend).to_string(),
                         index: successful_candidates.len() + 1,
                         output: query.stdout.clone(),
-                    });
+                    };
+                    successful_candidates.push(branch_success.clone());
+                    if is_vote {
+                        vote_branches
+                            .push(crate::aggregator::BranchOutcome::Success(branch_success));
+                    }
 
                     attempts.push(Attempt {
                         tier: None,
@@ -191,11 +198,15 @@ impl Strategy for ParallelFanOut {
                         verify: VerifyOutcome::skipped(),
                     });
 
-                    if !is_any_fail && !is_llm_judge && successes >= self.min_responses {
-                        // For non-LLMJudge aggregation modes, stop once enough
+                    if !is_any_fail && !is_llm_judge && !is_vote && successes >= self.min_responses
+                    {
+                        // For non-LLMJudge / non-Vote aggregation modes, stop once enough
                         // successes are in to meet the configured floor.
                         // LLMJudge must inspect all candidates first and therefore
                         // cannot short-circuit on min_responses.
+                        // Vote must collect all branches (including failures as
+                        // abstentions) before it can compute a majority or detect
+                        // a quorum loss.
                         break;
                     }
                 }
@@ -246,6 +257,17 @@ impl Strategy for ParallelFanOut {
                         });
                     }
 
+                    if is_vote {
+                        vote_branches.push(crate::aggregator::BranchOutcome::Failure(
+                            crate::aggregator::BranchFailure {
+                                backend_id: target.backend.clone(),
+                                family: family_of(&target.backend).to_string(),
+                                index: attempts.len() + 1,
+                                reason: err.to_string(),
+                            },
+                        ));
+                    }
+
                     attempts.push(attempt);
                 }
             }
@@ -376,6 +398,59 @@ impl Strategy for ParallelFanOut {
             output.verify = Some(VerifyOutcome::passed("LLMJudge"));
         }
 
+        if is_vote {
+            let config = match &self.aggregator {
+                Aggregator::Vote { config } => config,
+                _ => unreachable!(),
+            };
+
+            let (aggregate, _result) = crate::aggregator::aggregate_vote(&vote_branches, config)
+                .map_err(|err| match err {
+                    crate::aggregator::VoteError::QuorumLost {
+                        abstains,
+                        threshold,
+                    } => StrategyError::Phase(crate::family::PhaseError::QuorumLost {
+                        abstains,
+                        threshold,
+                    }),
+                    crate::aggregator::VoteError::NoCandidates => {
+                        StrategyError::Phase(crate::family::PhaseError::AggregatorRejected {
+                            message: "no candidates".into(),
+                        })
+                    }
+                })?;
+
+            if let Some(parent) = Path::new(&aggregated_output_path).parent() {
+                if !parent.as_os_str().is_empty() {
+                    fs::create_dir_all(parent).await.map_err(|err| {
+                        StrategyError::Backend(crate::backend::BackendError::ExecutionFailed {
+                            message: format!(
+                                "failed to create aggregate output parent {}: {err}",
+                                parent.display()
+                            ),
+                            exit_code: None,
+                        })
+                    })?;
+                }
+            }
+
+            let aggregate_output_path_ref = aggregated_output_path.as_str();
+            fs::write(&aggregated_output_path, aggregate.text)
+                .await
+                .map_err(|err| {
+                    StrategyError::Backend(
+                        crate::backend::BackendError::ExecutionFailed {
+                            message: format!(
+                                "failed to write aggregate output to {aggregate_output_path_ref}: {err}"
+                            ),
+                            exit_code: None,
+                        },
+                    )
+                })?;
+
+            output.verify = Some(VerifyOutcome::passed("Vote"));
+        }
+
         Ok(output)
     }
 }
@@ -396,7 +471,9 @@ fn pick_model_override(query: &QueryOutput, prompt: &Prompt, target: &TargetSpec
 #[cfg(test)]
 mod tests {
     use super::*;
-    use crate::aggregator::AnyFailReason;
+    use crate::aggregator::{
+        AnyFailReason, BallotSchema, BranchFailure, BranchSuccess, TieBreak, VoteConfig,
+    };
     use crate::backend::BackendError;
     use std::path::Path;
     use std::sync::atomic::{AtomicUsize, Ordering};
@@ -937,4 +1014,103 @@ mod tests {
             other => panic!("expected AnyFail, got {other:?}"),
         }
     }
+
+    #[test]
+    fn vote_success() {
+        let a = MockBackend::ok("a", "A");
+        let b = MockBackend::ok("b", "A");
+        let c = MockBackend::ok("c", "B");
+        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone(), c.clone()];
+        let strategy = ParallelFanOut::new(
+            vec![
+                TargetSpec::new("a"),
+                TargetSpec::new("b"),
+                TargetSpec::new("c"),
+            ],
+            2,
+            "render-me",
+            Aggregator::vote(VoteConfig {
+                ballot_schema: BallotSchema::FreeText,
+                tie_break: TieBreak::FirstResponder,
+                abstain_threshold: 0,
+            }),
+        );
+
+        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
+        assert_eq!(out.strategy, StrategyKind::Parallel);
+        assert_eq!(out.attempts.len(), 3);
+        assert_eq!(
+            out.verify.as_ref().unwrap().status,
+            crate::strategy::VerifyStatus::Pass
+        );
+        assert_eq!(out.verify.as_ref().unwrap().hook.as_deref(), Some("Vote"));
+        assert_eq!(out.aggregator.as_ref().unwrap().as_str(), "vote");
+    }
+
+    #[test]
+    fn vote_tie_random_deterministic() {
+        let a = MockBackend::ok("a", "A");
+        let b = MockBackend::ok("b", "B");
+        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
+        let strategy = ParallelFanOut::new(
+            vec![TargetSpec::new("a"), TargetSpec::new("b")],
+            2,
+            "render-me",
+            Aggregator::vote(VoteConfig {
+                ballot_schema: BallotSchema::FreeText,
+                tie_break: TieBreak::Random { seed: 123 },
+                abstain_threshold: 0,
+            }),
+        );
+
+        let out1 = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
+        let out2 = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
+        assert_eq!(
+            out1.verify.unwrap().status,
+            crate::strategy::VerifyStatus::Pass
+        );
+        assert_eq!(
+            out2.verify.unwrap().status,
+            crate::strategy::VerifyStatus::Pass
+        );
+        // Same winner on repeated runs because the seed and inputs are identical
+    }
+
+    #[test]
+    fn vote_quorum_lost() {
+        let a = MockBackend::ok("a", "yes");
+        let b = MockBackend::fail("b", || BackendError::Network {
+            message: "boom".into(),
+        });
+        let c = MockBackend::fail("c", || BackendError::Auth {
+            message: "bad key".into(),
+        });
+        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone(), c.clone()];
+        let strategy = ParallelFanOut::new(
+            vec![
+                TargetSpec::new("a"),
+                TargetSpec::new("b"),
+                TargetSpec::new("c"),
+            ],
+            1,
+            "render-me",
+            Aggregator::vote(VoteConfig {
+                ballot_schema: BallotSchema::FreeText,
+                tie_break: TieBreak::FirstResponder,
+                abstain_threshold: 0,
+            }),
+        );
+
+        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
+        match err {
+            StrategyError::Phase(phase_err) => {
+                let msg = phase_err.to_string();
+                assert!(
+                    msg.contains("quorum lost"),
+                    "expected quorum lost, got: {msg}"
+                );
+            }
+            other => panic!("expected PhaseError::QuorumLost, got {other:?}"),
+        }
+    }
 }

exec
/bin/zsh -lc "sed -n '1,260p' src/aggregator/vote.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
//! `Aggregator::Vote` implementation.
//!
//! Pure, synchronous vote counting over parallel branch outcomes.
//! No secondary backend calls, no async, no I/O.

use std::collections::BTreeMap;

use crate::family::{family_of, Family};

use super::{AggregatedArtifact, BranchOutcome};

/// How a ballot is normalised and interpreted.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BallotSchema {
    /// Free text: each backend returns prose; normalise before bucketing.
    FreeText,
}

/// How to resolve a tie when no strict majority exists.
#[derive(Debug, Clone, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq, Eq)]
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

    let mut result = if winners.len() == 1 {
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
                .and_then(|&idx| candidates.get(idx).map(|c| c.normalised.as_str()))
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

exec
/bin/zsh -lc "sed -n '260,520p' src/aggregator/vote.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
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

exec
/bin/zsh -lc "sed -n '520,780p' src/aggregator/vote.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
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

exec
/bin/zsh -lc "sed -n '1,260p' src/aggregator/concat.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
use std::sync::Arc;

use crate::aggregator::VoteConfig;
use crate::backend::Backend;
use crate::strategy::PhaseContext;

// Aggregator implementations for folding multiple branch outputs into one artefact.
//
// This module contains behavioral aggregator config and pure aggregation logic.
// It is intentionally separate from [`crate::strategy::Aggregator`], which is
// the schema-facing label serialized into phase-result JSON.

/// Sentinel emitted when concat aggregation receives no branch outcomes.
///
/// Empty input is valid and never panics. The sentinel is a Markdown comment so
/// downstream phases can consume it as a string artefact without rendering noisy
/// prose to users.
pub const EMPTY_CONCAT_SENTINEL: &str =
    "<!-- loker: concat aggregator received no target outputs -->";

/// Behavioral aggregator configuration.
///
/// `Concat` joins successful branch outputs under rendered headings and appends
/// failed branches to a structured `## Errors` footer. Supported heading
/// placeholders are exactly:
///
/// - `{backend_id}`: source backend identifier
/// - `{family}`: resolved model family label
/// - `{index}`: caller-provided 1-based branch index
///
/// Unknown placeholders are preserved literally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Aggregator {
    Concat {
        heading_template: String,
    },
    AnyFail,
    LLMJudge {
        judge_backend: String,
        prompt_template: String,
        require_judge_different_family: bool,
    },
    Vote {
        config: VoteConfig,
    },
}

impl Aggregator {
    /// Build a concat aggregator with the provided heading template.
    pub fn concat(heading_template: impl Into<String>) -> Self {
        Self::Concat {
            heading_template: heading_template.into(),
        }
    }

    /// Build an LLM judge aggregator with the provided configuration.
    pub fn llm_judge(
        judge_backend: impl Into<String>,
        prompt_template: impl Into<String>,
        require_judge_different_family: bool,
    ) -> Self {
        Self::LLMJudge {
            judge_backend: judge_backend.into(),
            prompt_template: prompt_template.into(),
            require_judge_different_family,
        }
    }

    /// Build a Vote aggregator with the provided configuration.
    pub fn vote(config: VoteConfig) -> Self {
        Self::Vote { config }
    }

    /// Build an AnyFail aggregator (no configuration needed).
    pub fn any_fail() -> Self {
        Self::AnyFail
    }

    /// Return the schema-facing strategy aggregator label for this behavior.
    pub fn kind(&self) -> crate::strategy::Aggregator {
        match self {
            Self::Concat { .. } => crate::strategy::Aggregator::Concat,
            Self::AnyFail => crate::strategy::Aggregator::AnyFail,
            Self::LLMJudge { .. } => crate::strategy::Aggregator::LLMJudge,
            Self::Vote { .. } => crate::strategy::Aggregator::Vote,
        }
    }

    /// Aggregate ordered branch outcomes into one string artefact.
    pub fn aggregate(
        &self,
        input: AggregateInput,
        _backends: &[Arc<dyn Backend>],
        _ctx: &PhaseContext,
    ) -> Result<AggregatedArtifact, AggregatorError> {
        match self {
            Self::Concat { heading_template } => aggregate_concat(heading_template, input),
            Self::AnyFail => Err(AggregatorError::Unsupported(
                "AnyFail is evaluated inline by ParallelFanOut, not via aggregate()".into(),
            )),
            Self::LLMJudge { .. } => Err(AggregatorError::Unsupported(
                "LLMJudge requires async backend access; use aggregate_llm_judge()".into(),
            )),
            Self::Vote { .. } => Err(AggregatorError::Unsupported(
                "Vote is evaluated inline by ParallelFanOut, not via aggregate()".into(),
            )),
        }
    }
}

/// Ordered input to an aggregator.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AggregateInput {
    pub branches: Vec<BranchOutcome>,
}

/// Per-branch outcome supplied by the phase runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchOutcome {
    Success(BranchSuccess),
    Failure(BranchFailure),
}

/// Successful branch text plus source metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchSuccess {
    pub backend_id: String,
    pub family: String,
    /// 1-based caller-visible index. For `ParallelFanOut`, this should be the
    /// arrival-order position supplied by the phase runner.
    pub index: usize,
    pub output: String,
}

/// Failed branch reason plus source metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchFailure {
    pub backend_id: String,
    pub family: String,
    /// 1-based caller-visible index.
    pub index: usize,
    pub reason: String,
}

/// Aggregate artefact and summary counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregatedArtifact {
    pub text: String,
    pub successful: usize,
    pub failed: usize,
}

/// Errors produced by aggregators.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AggregatorError {
    #[error("unsupported aggregator operation: {0}")]
    Unsupported(String),
}

fn aggregate_concat(
    heading_template: &str,
    input: AggregateInput,
) -> Result<AggregatedArtifact, AggregatorError> {
    if input.branches.is_empty() {
        return Ok(AggregatedArtifact {
            text: EMPTY_CONCAT_SENTINEL.to_string(),
            successful: 0,
            failed: 0,
        });
    }

    let mut sections = Vec::new();
    let mut failures = Vec::new();

    for branch in input.branches {
        match branch {
            BranchOutcome::Success(success) => {
                sections.push(render_success(heading_template, success))
            }
            BranchOutcome::Failure(failure) => failures.push(failure),
        }
    }

    let successful = sections.len();
    let failed = failures.len();
    let mut parts = sections;

    if !failures.is_empty() {
        parts.push(render_errors(&failures));
    }

    let mut text = if parts.is_empty() {
        String::new()
    } else {
        parts.join("\n\n")
    };
    text.push('\n');

    Ok(AggregatedArtifact {
        text,
        successful,
        failed,
    })
}

fn render_success(heading_template: &str, success: BranchSuccess) -> String {
    let heading = render_heading(
        heading_template,
        &success.backend_id,
        &success.family,
        success.index,
    );
    let output = success.output.trim();
    if output.is_empty() {
        heading
    } else {
        format!("{}\n\n{}", heading, output)
    }
}

fn render_heading(template: &str, backend_id: &str, family: &str, index: usize) -> String {
    let mut rendered = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start) = rest.find('{') {
        rendered.push_str(&rest[..start]);
        rest = &rest[start..];

        if rest.starts_with("{backend_id}") {
            rendered.push_str(backend_id);
            rest = &rest["{backend_id}".len()..];
        } else if rest.starts_with("{family}") {
            rendered.push_str(family);
            rest = &rest["{family}".len()..];
        } else if rest.starts_with("{index}") {
            rendered.push_str(&index.to_string());
            rest = &rest["{index}".len()..];
        } else if let Some(end) = rest.find('}') {
            rendered.push_str(&rest[..=end]);
            rest = &rest[end + '}'.len_utf8()..];
        } else {
            rendered.push_str(rest);
            rest = "";
        }
    }

    rendered.push_str(rest);
    rendered
}

fn render_errors(failures: &[BranchFailure]) -> String {
    let mut out = String::from("## Errors");
    for failure in failures {
        out.push_str("\n\n");
        out.push_str(&format!(
            "- backend_id: {}\n  family: {}\n  index: {}\n  reason: {}",
            failure.backend_id,
            failure.family,
            failure.index,
            render_reason(&failure.reason)

exec
/bin/zsh -lc "sed -n '260,520p' src/aggregator/concat.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
            render_reason(&failure.reason)
        ));
    }
    out
}

fn render_reason(reason: &str) -> String {
    reason
        .trim()
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn success(backend_id: &str, family: &str, index: usize, output: &str) -> BranchOutcome {
        BranchOutcome::Success(BranchSuccess {
            backend_id: backend_id.into(),
            family: family.into(),
            index,
            output: output.into(),
        })
    }

    fn failure(backend_id: &str, family: &str, index: usize, reason: &str) -> BranchOutcome {
        BranchOutcome::Failure(BranchFailure {
            backend_id: backend_id.into(),
            family: family.into(),
            index,
            reason: reason.into(),
        })
    }

    #[test]
    fn concat_renders_success_sections_in_input_order() {
        let artifact = Aggregator::concat("## {index}. {backend_id} ({family})")
            .aggregate(
                AggregateInput {
                    branches: vec![
                        success("claude", "anthropic", 1, " first "),
                        success("gemini", "google", 2, "second\n"),
                    ],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(
            artifact.text,
            "## 1. claude (anthropic)\n\nfirst\n\n## 2. gemini (google)\n\nsecond\n"
        );
        assert_eq!(artifact.successful, 2);
        assert_eq!(artifact.failed, 0);
    }

    #[test]
    fn concat_preserves_unknown_placeholders() {
        let artifact = Aggregator::concat("## {backend_id} {unknown}")
            .aggregate(
                AggregateInput {
                    branches: vec![success("claude", "anthropic", 1, "text")],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(artifact.text, "## claude {unknown}\n\ntext\n");
    }

    #[test]
    fn concat_preserves_braced_unknown_expressions_containing_known_tokens() {
        let artifact = Aggregator::concat("## {{backend_id}} {unknown {family}}")
            .aggregate(
                AggregateInput {
                    branches: vec![success("claude", "anthropic", 1, "text")],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(
            artifact.text,
            "## {{backend_id}} {unknown {family}}\n\ntext\n"
        );
    }

    #[test]
    fn concat_does_not_reexpand_placeholders_inside_metadata() {
        let artifact = Aggregator::concat("## {backend_id} ({family})")
            .aggregate(
                AggregateInput {
                    branches: vec![success("review-{index}", "other-{backend_id}", 3, "text")],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(
            artifact.text,
            "## review-{index} (other-{backend_id})\n\ntext\n"
        );
    }

    #[test]
    fn concat_escapes_multiline_failure_reason() {
        let artifact = Aggregator::concat("## {backend_id}")
            .aggregate(
                AggregateInput {
                    branches: vec![failure(
                        "codex",
                        "openai",
                        1,
                        "network: timeout\nretry exhausted",
                    )],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert!(artifact
            .text
            .contains("reason: network: timeout\\nretry exhausted"));
    }

    #[test]
    fn concat_normalizes_crlf_failure_reason() {
        let artifact = Aggregator::concat("## {backend_id}")
            .aggregate(
                AggregateInput {
                    branches: vec![failure("codex", "openai", 1, "line1\r\nline2\rline3")],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert!(artifact.text.contains("reason: line1\\nline2\\nline3"));
        assert!(!artifact.text.contains('\r'));
    }

    #[test]
    fn concat_whitespace_only_success_output_keeps_newline_invariants() {
        let artifact = Aggregator::concat("## {backend_id}")
            .aggregate(
                AggregateInput {
                    branches: vec![
                        success("claude", "anthropic", 1, "   \n"),
                        success("gemini", "google", 2, "ok"),
                    ],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(artifact.text, "## claude\n\n## gemini\n\nok\n");
    }

    #[test]
    fn concat_empty_input_returns_sentinel() {
        let artifact = Aggregator::concat("## {backend_id}")
            .aggregate(
                AggregateInput::default(),
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(artifact.text, EMPTY_CONCAT_SENTINEL);
        assert_eq!(artifact.successful, 0);
        assert_eq!(artifact.failed, 0);
    }

    #[test]
    fn concat_counts_success_and_failure() {
        let artifact = Aggregator::concat("## {backend_id}")
            .aggregate(
                AggregateInput {
                    branches: vec![
                        success("claude", "anthropic", 1, "ok"),
                        failure("codex", "openai", 2, "network: timeout"),
                    ],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        assert_eq!(artifact.successful, 1);
        assert_eq!(artifact.failed, 1);
        assert!(artifact.text.contains("## Errors"));
        assert!(artifact.text.contains("- backend_id: codex"));
    }

    #[test]
    fn concat_kind_maps_to_strategy_label() {
        assert_eq!(
            Aggregator::concat("## {backend_id}").kind(),
            crate::strategy::Aggregator::Concat
        );
    }

    #[test]
    fn llm_judge_kind_maps_to_strategy_label() {
        assert_eq!(
            Aggregator::llm_judge("judge", "template", true).kind(),
            crate::strategy::Aggregator::LLMJudge
        );
    }

    #[test]
    fn vote_kind_maps_to_strategy_label() {
        assert_eq!(
            Aggregator::vote(crate::aggregator::VoteConfig {
                ballot_schema: crate::aggregator::BallotSchema::FreeText,
                tie_break: crate::aggregator::TieBreak::FirstResponder,
                abstain_threshold: 0,
            })
            .kind(),
            crate::strategy::Aggregator::Vote
        );
    }

    #[test]
    fn concat_mixed_success_failure_snapshot() {
        let artifact = Aggregator::concat("## {index}. {backend_id} ({family})")
            .aggregate(
                AggregateInput {
                    branches: vec![
                        success("claude", "anthropic", 1, "Claude review text."),
                        failure("codex", "openai", 2, "network: timeout"),
                        success("gemini", "google", 3, "Gemini review text."),
                    ],
                },
                &[],
                &crate::strategy::PhaseContext::new("test", uuid::Uuid::new_v4()),
            )
            .unwrap();

        insta::assert_snapshot!(artifact.text);
    }
}

exec
/bin/zsh -lc "sed -n '1,260p' src/aggregator/mod.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
//! Aggregator primitives.
//!
//! Three aggregator behaviors live here:
//! - [`concat`]: fold multiple branch outputs into one Markdown artefact.
//! - [`any_fail_evaluate`] / [`AnyFailReason`]: short-circuit on the first
//!   rejected verdict.
//! - [`llm_judge`]: use a separate-family LLM to pick the best candidate.

mod concat;
mod llm_judge;
mod vote;

pub use concat::{
    AggregateInput, AggregatedArtifact, Aggregator, AggregatorError, BranchFailure, BranchOutcome,
    BranchSuccess, EMPTY_CONCAT_SENTINEL,
};

pub use llm_judge::{
    aggregate_llm_judge, check_cross_family, clamp_chosen_index, parse_ballot,
    render_ballot_prompt, Ballot, Candidate, LLMJudgeError,
};

pub use vote::{
    aggregate_vote, normalise_ballot, BallotSchema, TieBreak, VoteCandidate, VoteConfig, VoteError,
    VoteResult,
};

use serde_json::Value;

#[non_exhaustive]
#[derive(Debug, Clone, thiserror::Error)]
pub enum AnyFailReason {
    #[error("verdict rejected")]
    VerdictRejected { payload: String },
    #[error("verdict contract violation: {message}")]
    VerdictContract { message: String },
    #[error("backend error: {detail}")]
    BackendError { detail: String },
}

/// Parse a backend response text as a JSON verdict and check `pass`.
///
/// Strips markdown fences (` ```json ... ``` `) before parsing so
/// LLM backends that wrap JSON in code blocks work out of the box.
/// Accepts extra keys (forward compatible). Missing or malformed `pass`
/// is a contract violation, not a panic.
pub fn any_fail_evaluate(text: &str) -> Result<(), AnyFailReason> {
    let stripped = strip_markdown_fences(text.trim());
    let value: Value =
        serde_json::from_str(stripped).map_err(|e| AnyFailReason::VerdictContract {
            message: format!("JSON parse error: {e}"),
        })?;

    match value.get("pass") {
        Some(Value::Bool(true)) => Ok(()),
        Some(Value::Bool(false)) => Err(AnyFailReason::VerdictRejected {
            payload: stripped.to_string(),
        }),
        Some(other) => Err(AnyFailReason::VerdictContract {
            message: format!("expected bool, got {}", other),
        }),
        None => Err(AnyFailReason::VerdictContract {
            message: "missing required field 'pass'".to_string(),
        }),
    }
}

/// Strip leading/trailing markdown fences if present.
pub(crate) fn strip_markdown_fences(text: &str) -> &str {
    let text = text.strip_prefix("```json").unwrap_or(text);
    let text = text.strip_prefix("```").unwrap_or(text);
    let text = text.strip_suffix("```").unwrap_or(text);
    text.trim()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pass_true() {
        assert!(any_fail_evaluate(r#"{"pass": true}"#).is_ok());
    }

    #[test]
    fn pass_false() {
        let err = any_fail_evaluate(r#"{"pass": false}"#).unwrap_err();
        assert!(matches!(err, AnyFailReason::VerdictRejected { .. }));
    }

    #[test]
    fn missing_pass() {
        let err = any_fail_evaluate(r#"{"status": "ok"}"#).unwrap_err();
        assert!(matches!(err, AnyFailReason::VerdictContract { .. }));
    }

    #[test]
    fn wrong_pass_type() {
        let err = any_fail_evaluate(r#"{"pass": "yes"}"#).unwrap_err();
        assert!(matches!(err, AnyFailReason::VerdictContract { .. }));
    }

    #[test]
    fn empty_text() {
        let err = any_fail_evaluate("").unwrap_err();
        assert!(matches!(err, AnyFailReason::VerdictContract { .. }));
    }

    #[test]
    fn markdown_fenced_json() {
        let text = "```json\n{\"pass\": true}\n```";
        assert!(any_fail_evaluate(text).is_ok());
    }

    #[test]
    fn markdown_fenced_fail() {
        let text = "```json\n{\"pass\": false}\n```";
        let err = any_fail_evaluate(text).unwrap_err();
        assert!(matches!(err, AnyFailReason::VerdictRejected { .. }));
    }

    #[test]
    fn extra_keys_ok() {
        assert!(any_fail_evaluate(r#"{"pass": true, "note": "lgtm"}"#).is_ok());
    }
}

exec
/bin/zsh -lc "sed -n '1,260p' src/strategy/parallel_fanout.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
//! `ParallelFanOut`: dispatch a single prompt to N backends concurrently.
//!
//! Per loker-design.md §4.2 this is the third `Strategy` variant after
//! `SingleModel` (CLO-257) and `EscalatingRetry` (CLO-258).  The runner
//! renders the prompt once, spawns one `Backend::query` future per target
//! via `FuturesUnordered`, and collects per-target outcomes in completion
//! order.  Once `min_responses` successful responses have arrived the
//! strategy short-circuits; remaining in-flight requests are dropped.
//!
//! If fewer than `min_responses` targets succeed before the whole set
//! settles, a structured `StrategyError::FloorViolation` is returned so
//! callers can still persist the schema-shaped JSON.

use crate::aggregator::{aggregate_llm_judge, Aggregator, BranchSuccess};
use crate::backend::{Backend, QueryOutput};
use crate::family::family_of;
use crate::strategy::{
    Attempt, FinishReason, PhaseContext, Prompt, Strategy, StrategyError, StrategyKind,
    StrategyOutput, TokenUsageReport, VerifyOutcome, SCHEMA_VERSION,
};
use async_trait::async_trait;
use futures::stream::FuturesUnordered;
use futures::StreamExt;
use std::path::Path;
use std::sync::Arc;
use tokio::fs;

/// Target specification for one branch of the fan-out.
#[derive(Debug, Clone)]
pub struct TargetSpec {
    pub backend: String,
    pub model: Option<String>,
}

impl TargetSpec {
    pub fn new(backend: impl Into<String>) -> Self {
        Self {
            backend: backend.into(),
            model: None,
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
}

/// Parallel fan-out strategy.
#[derive(Debug, Clone)]
pub struct ParallelFanOut {
    pub targets: Vec<TargetSpec>,
    pub min_responses: usize,
    pub prompt_template: String,
    pub aggregator: Aggregator,
}

impl ParallelFanOut {
    pub fn new(
        targets: Vec<TargetSpec>,
        min_responses: usize,
        prompt_template: impl Into<String>,
        aggregator: Aggregator,
    ) -> Self {
        assert!(
            min_responses > 0,
            "min_responses must be greater than 0, got {min_responses}"
        );
        Self {
            targets,
            min_responses,
            prompt_template: prompt_template.into(),
            aggregator,
        }
    }
}

#[async_trait]
impl Strategy for ParallelFanOut {
    async fn execute(
        &self,
        backends: &[Arc<dyn Backend>],
        prompt: &Prompt,
        ctx: &PhaseContext,
    ) -> Result<StrategyOutput, StrategyError> {
        if self.targets.is_empty() || backends.is_empty() {
            return Err(StrategyError::NoBackends);
        }

        let rendered = ctx
            .template_engine
            .render(&self.prompt_template, &ctx.template_context)?;

        // Build FuturesUnordered: each future resolves to (target_index, result).
        let mut futures = FuturesUnordered::new();
        for (idx, target) in self.targets.iter().enumerate() {
            let backend = backends
                .iter()
                .find(|b| b.name() == target.backend)
                .ok_or_else(|| StrategyError::BackendNotFound {
                    name: target.backend.clone(),
                })?;

            let rendered = rendered.clone();
            let cwd = ctx.cwd.clone();
            let model_override = target
                .model
                .as_deref()
                .filter(|m| !m.is_empty())
                .or(prompt.model.as_deref().filter(|m| !m.is_empty()));

            let fut = async move {
                let result = backend.query(&rendered, &cwd, model_override).await;
                (idx, result)
            };
            futures.push(fut);
        }

        let mut attempts: Vec<Attempt> = Vec::with_capacity(self.targets.len());
        let mut successes = 0;
        let mut successful_candidates: Vec<crate::aggregator::BranchSuccess> =
            Vec::with_capacity(self.targets.len());
        let mut vote_branches: Vec<crate::aggregator::BranchOutcome> = Vec::new();
        let is_any_fail = matches!(self.aggregator, Aggregator::AnyFail);
        let is_llm_judge = matches!(self.aggregator, Aggregator::LLMJudge { .. });
        let is_vote = matches!(self.aggregator, Aggregator::Vote { .. });

        while let Some((idx, result)) = futures.next().await {
            let target = &self.targets[idx];

            match result {
                Ok(query) => {
                    let usage = query
                        .usage
                        .as_ref()
                        .map(TokenUsageReport::from)
                        .unwrap_or_default();
                    let model = pick_model_override(&query, prompt, target);
                    let output_path = format!("{}/attempts/{}-parallel.txt", ctx.phase_name, idx);

                    if is_any_fail {
                        if let Err(reason) = crate::aggregator::any_fail_evaluate(&query.stdout) {
                            let offender = Attempt {
                                tier: None,
                                family: Some(family_of(&target.backend).to_string()),
                                backend: target.backend.clone(),
                                model,
                                finish_reasons: vec![FinishReason::Stop],
                                usage,
                                output_path,
                                verify: VerifyOutcome::skipped(),
                            };
                            attempts.push(offender.clone());
                            let output = StrategyOutput {
                                schema_version: SCHEMA_VERSION,
                                strategy: StrategyKind::Parallel,
                                phase: ctx.phase_name.clone(),
                                run_id: ctx.run_id,
                                attempts,
                                final_status: None,
                                aggregator: Some(self.aggregator.kind()),
                                aggregate_output_path: Some(format!(
                                    "{}/aggregated.txt",
                                    ctx.phase_name
                                )),
                                verify: Some(VerifyOutcome::failed("Aggregator::AnyFail")),
                            };
                            return Err(StrategyError::AnyFail {
                                backend: target.backend.clone(),
                                reason,
                                offender: Box::new(offender),
                                output: Box::new(output),
                            });
                        }
                    }

                    successes += 1;
                    let branch_success = BranchSuccess {
                        backend_id: target.backend.clone(),
                        family: family_of(&target.backend).to_string(),
                        index: successful_candidates.len() + 1,
                        output: query.stdout.clone(),
                    };
                    successful_candidates.push(branch_success.clone());
                    if is_vote {
                        vote_branches
                            .push(crate::aggregator::BranchOutcome::Success(branch_success));
                    }

                    attempts.push(Attempt {
                        tier: None,
                        family: Some(family_of(&target.backend).to_string()),
                        backend: target.backend.clone(),
                        model,
                        finish_reasons: vec![FinishReason::Stop],
                        usage,
                        output_path,
                        verify: VerifyOutcome::skipped(),
                    });

                    if !is_any_fail && !is_llm_judge && !is_vote && successes >= self.min_responses
                    {
                        // For non-LLMJudge / non-Vote aggregation modes, stop once enough
                        // successes are in to meet the configured floor.
                        // LLMJudge must inspect all candidates first and therefore
                        // cannot short-circuit on min_responses.
                        // Vote must collect all branches (including failures as
                        // abstentions) before it can compute a majority or detect
                        // a quorum loss.
                        break;
                    }
                }
                Err(err) => {
                    let model = target
                        .model
                        .as_ref()
                        .filter(|m| !m.is_empty())
                        .cloned()
                        .or_else(|| prompt.model.clone().filter(|m| !m.is_empty()))
                        .unwrap_or_else(|| "default".to_string());
                    let output_path = format!("{}/attempts/{}-parallel.txt", ctx.phase_name, idx);

                    let attempt = Attempt {
                        tier: None,
                        family: Some(family_of(&target.backend).to_string()),
                        backend: target.backend.clone(),
                        model,
                        finish_reasons: vec![FinishReason::Error],
                        usage: TokenUsageReport::default(),
                        output_path,
                        verify: VerifyOutcome::skipped(),
                    };

                    if is_any_fail {
                        attempts.push(attempt.clone());
                        let output = StrategyOutput {
                            schema_version: SCHEMA_VERSION,
                            strategy: StrategyKind::Parallel,
                            phase: ctx.phase_name.clone(),
                            run_id: ctx.run_id,
                            attempts,
                            final_status: None,
                            aggregator: Some(self.aggregator.kind()),
                            aggregate_output_path: Some(format!(
                                "{}/aggregated.txt",
                                ctx.phase_name
                            )),
                            verify: Some(VerifyOutcome::failed("Aggregator::AnyFail")),
                        };
                        return Err(StrategyError::AnyFail {
                            backend: target.backend.clone(),
                            reason: crate::aggregator::AnyFailReason::BackendError {
                                detail: err.to_string(),
                            },
                            offender: Box::new(attempt),
                            output: Box::new(output),
                        });
                    }

                    if is_vote {

exec
/bin/zsh -lc "sed -n '260,560p' src/strategy/parallel_fanout.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
                    if is_vote {
                        vote_branches.push(crate::aggregator::BranchOutcome::Failure(
                            crate::aggregator::BranchFailure {
                                backend_id: target.backend.clone(),
                                family: family_of(&target.backend).to_string(),
                                index: attempts.len() + 1,
                                reason: err.to_string(),
                            },
                        ));
                    }

                    attempts.push(attempt);
                }
            }
        }

        // Dropped futures are cancelled (cooperatively) when `futures` falls
        // out of scope here.

        if is_any_fail {
            return Ok(StrategyOutput {
                schema_version: SCHEMA_VERSION,
                strategy: StrategyKind::Parallel,
                phase: ctx.phase_name.clone(),
                run_id: ctx.run_id,
                attempts,
                final_status: None,
                aggregator: Some(self.aggregator.kind()),
                aggregate_output_path: Some(format!("{}/aggregated.txt", ctx.phase_name)),
                verify: Some(VerifyOutcome::passed("Aggregator::AnyFail")),
            });
        }

        if successes < self.min_responses {
            let output = StrategyOutput {
                schema_version: SCHEMA_VERSION,
                strategy: StrategyKind::Parallel,
                phase: ctx.phase_name.clone(),
                run_id: ctx.run_id,
                attempts,
                final_status: None,
                aggregator: Some(self.aggregator.kind()),
                aggregate_output_path: Some(format!("{}/aggregated.txt", ctx.phase_name)),
                verify: Some(VerifyOutcome::skipped()),
            };
            return Err(StrategyError::FloorViolation {
                successes,
                min_responses: self.min_responses,
                output: Box::new(output),
            });
        }

        let aggregated_output_path = format!("{}/aggregated.txt", ctx.phase_name);
        let mut output = StrategyOutput {
            schema_version: SCHEMA_VERSION,
            strategy: StrategyKind::Parallel,
            phase: ctx.phase_name.clone(),
            run_id: ctx.run_id,
            attempts,
            final_status: None,
            aggregator: Some(self.aggregator.kind()),
            aggregate_output_path: Some(aggregated_output_path.clone()),
            verify: Some(VerifyOutcome::skipped()),
        };

        if let Aggregator::LLMJudge {
            judge_backend,
            prompt_template,
            require_judge_different_family,
        } = &self.aggregator
        {
            let aggregate = aggregate_llm_judge(
                &successful_candidates,
                judge_backend,
                prompt_template,
                *require_judge_different_family,
                backends,
                ctx,
            )
            .await
            .map_err(|err| {
                use crate::aggregator::LLMJudgeError;
                match err {
                    LLMJudgeError::FamilyOverlap { candidate, .. } => {
                        let fam = family_of(&candidate);
                        let count = successful_candidates
                            .iter()
                            .filter(|c| family_of(&c.backend_id) == fam)
                            .count()
                            + 1;
                        StrategyError::Phase(crate::family::PhaseError::FamilyOverlap {
                            family: fam,
                            count,
                        })
                    }
                    LLMJudgeError::Contract { message } => {
                        StrategyError::Phase(crate::family::PhaseError::AggregatorContract {
                            message,
                        })
                    }
                    LLMJudgeError::JudgeCall(err) => {
                        StrategyError::Phase(crate::family::PhaseError::JudgeUnavailable {
                            detail: err.to_string(),
                        })
                    }
                    LLMJudgeError::BackendNotFound(name) => StrategyError::BackendNotFound { name },
                }
            })?;

            let aggregate_output_path = aggregated_output_path.clone();

            if let Some(parent) = Path::new(&aggregate_output_path).parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent).await.map_err(|err| {
                        StrategyError::Backend(crate::backend::BackendError::ExecutionFailed {
                            message: format!(
                                "failed to create aggregate output parent {}: {err}",
                                parent.display()
                            ),
                            exit_code: None,
                        })
                    })?;
                }
            }

            let aggregate_output_path = aggregate_output_path.clone();
            let aggregate_output_path_ref = aggregate_output_path.as_str();
            fs::write(&aggregate_output_path, aggregate.text)
                .await
                .map_err(|err| {
                    StrategyError::Backend(crate::backend::BackendError::ExecutionFailed {
                        message: format!(
                            "failed to write aggregate output to {aggregate_output_path_ref}: {err}"
                        ),
                        exit_code: None,
                    })
                })?;

            output.verify = Some(VerifyOutcome::passed("LLMJudge"));
        }

        if is_vote {
            let config = match &self.aggregator {
                Aggregator::Vote { config } => config,
                _ => unreachable!(),
            };

            let (aggregate, _result) = crate::aggregator::aggregate_vote(&vote_branches, config)
                .map_err(|err| match err {
                    crate::aggregator::VoteError::QuorumLost {
                        abstains,
                        threshold,
                    } => StrategyError::Phase(crate::family::PhaseError::QuorumLost {
                        abstains,
                        threshold,
                    }),
                    crate::aggregator::VoteError::NoCandidates => {
                        StrategyError::Phase(crate::family::PhaseError::AggregatorRejected {
                            message: "no candidates".into(),
                        })
                    }
                })?;

            if let Some(parent) = Path::new(&aggregated_output_path).parent() {
                if !parent.as_os_str().is_empty() {
                    fs::create_dir_all(parent).await.map_err(|err| {
                        StrategyError::Backend(crate::backend::BackendError::ExecutionFailed {
                            message: format!(
                                "failed to create aggregate output parent {}: {err}",
                                parent.display()
                            ),
                            exit_code: None,
                        })
                    })?;
                }
            }

            let aggregate_output_path_ref = aggregated_output_path.as_str();
            fs::write(&aggregated_output_path, aggregate.text)
                .await
                .map_err(|err| {
                    StrategyError::Backend(
                        crate::backend::BackendError::ExecutionFailed {
                            message: format!(
                                "failed to write aggregate output to {aggregate_output_path_ref}: {err}"
                            ),
                            exit_code: None,
                        },
                    )
                })?;

            output.verify = Some(VerifyOutcome::passed("Vote"));
        }

        Ok(output)
    }
}

/// Build the `model` field that lands in an attempt, applying the priority:
/// backend-reported > target.model > prompt.model > "default".
fn pick_model_override(query: &QueryOutput, prompt: &Prompt, target: &TargetSpec) -> String {
    query
        .model
        .as_deref()
        .filter(|m| !m.is_empty())
        .or_else(|| target.model.as_deref().filter(|m| !m.is_empty()))
        .or_else(|| prompt.model.as_deref().filter(|m| !m.is_empty()))
        .map(str::to_string)
        .unwrap_or_else(|| "default".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregator::{
        AnyFailReason, BallotSchema, BranchFailure, BranchSuccess, TieBreak, VoteConfig,
    };
    use crate::backend::BackendError;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct MockBackend {
        name: String,
        calls: AtomicUsize,
        response: Box<dyn Fn(usize) -> Result<QueryOutput, BackendError> + Send + Sync>,
        delay_ms: Option<u64>,
    }

    impl MockBackend {
        fn ok(name: &str, text: &str) -> Arc<Self> {
            let backend_name = name.to_string();
            let text_owned = text.to_string();
            Arc::new(Self {
                name: name.to_string(),
                calls: AtomicUsize::new(0),
                response: Box::new(move |_| {
                    Ok(QueryOutput::from_text(
                        text_owned.clone(),
                        backend_name.clone(),
                        Duration::from_millis(1),
                    )
                    .with_model(Some("mock-1")))
                }),
                delay_ms: None,
            })
        }

        fn fail(name: &str, err: impl Fn() -> BackendError + Send + Sync + 'static) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                calls: AtomicUsize::new(0),
                response: Box::new(move |_| Err(err())),
                delay_ms: None,
            })
        }

        fn slow(name: &str, text: &str, delay_ms: u64) -> Arc<Self> {
            let backend_name = name.to_string();
            let text_owned = text.to_string();
            Arc::new(Self {
                name: name.to_string(),
                calls: AtomicUsize::new(0),
                response: Box::new(move |_| {
                    Ok(QueryOutput::from_text(
                        text_owned.clone(),
                        backend_name.clone(),
                        Duration::from_millis(delay_ms),
                    )
                    .with_model(Some("mock-1")))
                }),
                delay_ms: Some(delay_ms),
            })
        }

        fn delayed_ok(name: &str, text: &str, delay_ms: u64) -> Arc<Self> {
            Self::slow(name, text, delay_ms)
        }

        fn delayed_fail(
            name: &str,
            err: impl Fn() -> BackendError + Send + Sync + 'static,
            delay_ms: u64,
        ) -> Arc<Self> {
            Arc::new(Self {
                name: name.to_string(),
                calls: AtomicUsize::new(0),
                response: Box::new(move |_| Err(err())),
                delay_ms: Some(delay_ms),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Backend for MockBackend {
        fn name(&self) -> &str {
            &self.name

2026-04-29T20:00:12.096997Z ERROR codex_core::tools::router: error=exec_command failed for `/bin/zsh -lc "sed -n '560,920p' src/strategy/parallel_fanout.rs"`: CreateProcess { message: "Rejected(\"Failed to create unified exec process: No such file or directory (os error 2)\")" }
exec
/bin/zsh -lc "sed -n '560,980p' src/strategy/parallel_fanout.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
            &self.name
        }

        async fn query(
            &self,
            _prompt: &str,
            _cwd: &Path,
            _model: Option<&str>,
        ) -> Result<QueryOutput, BackendError> {
            if let Some(ms) = self.delay_ms {
                tokio::time::sleep(Duration::from_millis(ms)).await;
            }
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            (self.response)(n)
        }

        fn is_available(&self) -> bool {
            true
        }
    }

    fn ctx() -> PhaseContext {
        PhaseContext::new("phase-1", uuid::Uuid::new_v4())
    }

    fn run<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Runtime::new().unwrap().block_on(f)
    }

    #[test]
    fn happy_path_all_succeed() {
        let a = MockBackend::ok("a", "out-a");
        let b = MockBackend::ok("b", "out-b");
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            2,
            "render-me",
            Aggregator::concat("## {backend_id}"),
        );

        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        assert_eq!(out.strategy, StrategyKind::Parallel);
        assert_eq!(out.attempts.len(), 2);
        assert!(out
            .attempts
            .iter()
            .all(|a| a.finish_reasons == vec![FinishReason::Stop]));
    }

    #[test]
    fn one_fails_floor_still_met() {
        let a = MockBackend::ok("a", "out-a");
        let b = MockBackend::fail("b", || BackendError::Network {
            message: "boom".into(),
        });
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::concat("## {backend_id}"),
        );

        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        // Short-circuit means we may return before the failing backend
        // settles; attempt count is therefore >= min_responses and <= targets.
        assert!(
            out.attempts.len() >= 1 && out.attempts.len() <= 2,
            "expected 1 or 2 attempts, got {}",
            out.attempts.len()
        );
        let ok_count = out
            .attempts
            .iter()
            .filter(|a| a.finish_reasons == vec![FinishReason::Stop])
            .count();
        assert_eq!(ok_count, 1, "expected exactly 1 success");
        assert_eq!(a.calls(), 1);
        // `b` may or may not have been polled before short-circuit.
        assert!(b.calls() <= 1);
    }

    #[test]
    fn floor_violation() {
        let a = MockBackend::ok("a", "out-a");
        let b = MockBackend::fail("b", || BackendError::Network {
            message: "boom".into(),
        });
        let c = MockBackend::fail("c", || BackendError::Auth {
            message: "bad key".into(),
        });
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone(), c.clone()];
        let strategy = ParallelFanOut::new(
            vec![
                TargetSpec::new("a"),
                TargetSpec::new("b"),
                TargetSpec::new("c"),
            ],
            3,
            "render-me",
            Aggregator::concat("## {backend_id}"),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::FloorViolation {
                successes,
                min_responses,
                output,
            } => {
                assert_eq!(successes, 1);
                assert_eq!(min_responses, 3);
                assert_eq!(output.attempts.len(), 3);
                assert_eq!(output.strategy, StrategyKind::Parallel);
            }
            other => panic!("expected FloorViolation, got {other:?}"),
        }
    }

    #[test]
    fn empty_targets_yields_no_backends() {
        let backends: Vec<Arc<dyn Backend>> = vec![MockBackend::ok("a", "x")];
        let strategy = ParallelFanOut::new(vec![], 1, "x", Aggregator::concat("## {backend_id}"));

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        assert!(matches!(err, StrategyError::NoBackends));
    }

    #[test]
    fn prompt_render_failure_no_dispatch() {
        let a = MockBackend::ok("a", "x");
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a")],
            1,
            "{{ steps.missing.output }}",
            Aggregator::concat("## {backend_id}"),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        assert!(matches!(err, StrategyError::PromptRender(_)));
        assert_eq!(a.calls(), 0);
    }

    #[test]
    fn backend_not_found() {
        let present = MockBackend::ok("present", "x");
        let backends: Vec<Arc<dyn Backend>> = vec![present.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("absent")],
            1,
            "x",
            Aggregator::concat("## {backend_id}"),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        assert!(matches!(err, StrategyError::BackendNotFound { name } if name == "absent"));
        assert_eq!(present.calls(), 0);
    }

    #[test]
    fn any_fail_all_pass() {
        let a = MockBackend::ok("a", r#"{"pass": true}"#);
        let b = MockBackend::ok("b", r#"{"pass": true}"#);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        assert_eq!(out.strategy, StrategyKind::Parallel);
        assert_eq!(out.attempts.len(), 2);
        assert_eq!(
            out.verify.as_ref().unwrap().status,
            crate::strategy::VerifyStatus::Pass
        );
    }

    #[test]
    fn any_fail_first_fails() {
        let b0 = MockBackend::delayed_ok("b0", r#"{"pass": false}"#, 1);
        let b1 = MockBackend::delayed_ok("b1", r#"{"pass": true}"#, 10);
        let b2 = MockBackend::delayed_ok("b2", r#"{"pass": true}"#, 10);
        let backends: Vec<Arc<dyn Backend>> = vec![b0.clone(), b1.clone(), b2.clone()];
        let strategy = ParallelFanOut::new(
            vec![
                TargetSpec::new("b0"),
                TargetSpec::new("b1"),
                TargetSpec::new("b2"),
            ],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "b0");
                assert!(matches!(reason, AnyFailReason::VerdictRejected { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_mid_list_fails() {
        let b0 = MockBackend::delayed_ok("b0", r#"{"pass": true}"#, 1);
        let b1 = MockBackend::delayed_ok("b1", r#"{"pass": false}"#, 5);
        let b2 = MockBackend::delayed_ok("b2", r#"{"pass": true}"#, 10);
        let backends: Vec<Arc<dyn Backend>> = vec![b0.clone(), b1.clone(), b2.clone()];
        let strategy = ParallelFanOut::new(
            vec![
                TargetSpec::new("b0"),
                TargetSpec::new("b1"),
                TargetSpec::new("b2"),
            ],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "b1");
                assert!(matches!(reason, AnyFailReason::VerdictRejected { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_all_fail() {
        let b0 = MockBackend::delayed_ok("b0", r#"{"pass": false}"#, 1);
        let b1 = MockBackend::delayed_ok("b1", r#"{"pass": false}"#, 5);
        let backends: Vec<Arc<dyn Backend>> = vec![b0.clone(), b1.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("b0"), TargetSpec::new("b1")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                // b0 has shorter delay, so it arrives first
                assert_eq!(backend, "b0");
                assert!(matches!(reason, AnyFailReason::VerdictRejected { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_backend_error_treated_as_failure() {
        let a = MockBackend::ok("a", r#"{"pass": true}"#);
        let b = MockBackend::delayed_fail(
            "b",
            || BackendError::Network {
                message: "boom".into(),
            },
            1,
        );
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "b");
                assert!(matches!(reason, AnyFailReason::BackendError { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_missing_pass_field() {
        let a = MockBackend::delayed_ok("a", r#"{"status": "ok"}"#, 1);
        let b = MockBackend::delayed_ok("b", r#"{"pass": true}"#, 10);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "a");
                assert!(matches!(reason, AnyFailReason::VerdictContract { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_wrong_pass_type() {
        let a = MockBackend::delayed_ok("a", r#"{"pass": "yes"}"#, 1);
        let b = MockBackend::delayed_ok("b", r#"{"pass": true}"#, 10);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "a");
                assert!(matches!(reason, AnyFailReason::VerdictContract { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_empty_query_text() {
        let a = MockBackend::delayed_ok("a", "", 1);
        let b = MockBackend::delayed_ok("b", r#"{"pass": true}"#, 10);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "a");
                assert!(matches!(reason, AnyFailReason::VerdictContract { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_markdown_fenced_json() {
        let a = MockBackend::ok("a", "```json\n{\"pass\": true}\n```");
        let b = MockBackend::ok("b", "```json\n{\"pass\": true}\n```");
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        assert_eq!(out.attempts.len(), 2);
        assert_eq!(
            out.verify.as_ref().unwrap().status,
            crate::strategy::VerifyStatus::Pass
        );
    }

    #[test]
    fn any_fail_markdown_fenced_fail() {
        let a = MockBackend::ok("a", "```json\n{\"pass\": false}\n```");
        let b = MockBackend::ok("b", r#"{"pass": true}"#);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "a");
                assert!(matches!(reason, AnyFailReason::VerdictRejected { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_valid_json_extra_keys() {
        let a = MockBackend::ok("a", r#"{"pass": true, "note": "lgtm"}"#);
        let b = MockBackend::ok("b", r#"{"pass": true}"#);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),

exec
/bin/zsh -lc "sed -n '920,1120p' src/strategy/parallel_fanout.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
                assert_eq!(backend, "a");
                assert!(matches!(reason, AnyFailReason::VerdictContract { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_markdown_fenced_json() {
        let a = MockBackend::ok("a", "```json\n{\"pass\": true}\n```");
        let b = MockBackend::ok("b", "```json\n{\"pass\": true}\n```");
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        assert_eq!(out.attempts.len(), 2);
        assert_eq!(
            out.verify.as_ref().unwrap().status,
            crate::strategy::VerifyStatus::Pass
        );
    }

    #[test]
    fn any_fail_markdown_fenced_fail() {
        let a = MockBackend::ok("a", "```json\n{\"pass\": false}\n```");
        let b = MockBackend::ok("b", r#"{"pass": true}"#);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert_eq!(backend, "a");
                assert!(matches!(reason, AnyFailReason::VerdictRejected { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn any_fail_valid_json_extra_keys() {
        let a = MockBackend::ok("a", r#"{"pass": true, "note": "lgtm"}"#);
        let b = MockBackend::ok("b", r#"{"pass": true}"#);
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        assert_eq!(out.attempts.len(), 2);
        assert_eq!(
            out.verify.as_ref().unwrap().status,
            crate::strategy::VerifyStatus::Pass
        );
    }

    #[test]
    fn any_fail_non_deterministic_offender() {
        let b0 = MockBackend::delayed_ok("b0", r#"{"pass": false}"#, 1);
        let b1 = MockBackend::delayed_ok("b1", r#"{"pass": false}"#, 1);
        let backends: Vec<Arc<dyn Backend>> = vec![b0.clone(), b1.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("b0"), TargetSpec::new("b1")],
            1,
            "render-me",
            Aggregator::any_fail(),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::AnyFail {
                backend, reason, ..
            } => {
                assert!(
                    matches!(&backend as &str, "b0" | "b1"),
                    "expected b0 or b1, got {backend}"
                );
                assert!(matches!(reason, AnyFailReason::VerdictRejected { .. }));
            }
            other => panic!("expected AnyFail, got {other:?}"),
        }
    }

    #[test]
    fn vote_success() {
        let a = MockBackend::ok("a", "A");
        let b = MockBackend::ok("b", "A");
        let c = MockBackend::ok("c", "B");
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone(), c.clone()];
        let strategy = ParallelFanOut::new(
            vec![
                TargetSpec::new("a"),
                TargetSpec::new("b"),
                TargetSpec::new("c"),
            ],
            2,
            "render-me",
            Aggregator::vote(VoteConfig {
                ballot_schema: BallotSchema::FreeText,
                tie_break: TieBreak::FirstResponder,
                abstain_threshold: 0,
            }),
        );

        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        assert_eq!(out.strategy, StrategyKind::Parallel);
        assert_eq!(out.attempts.len(), 3);
        assert_eq!(
            out.verify.as_ref().unwrap().status,
            crate::strategy::VerifyStatus::Pass
        );
        assert_eq!(out.verify.as_ref().unwrap().hook.as_deref(), Some("Vote"));
        assert_eq!(out.aggregator.as_ref().unwrap().as_str(), "vote");
    }

    #[test]
    fn vote_tie_random_deterministic() {
        let a = MockBackend::ok("a", "A");
        let b = MockBackend::ok("b", "B");
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
        let strategy = ParallelFanOut::new(
            vec![TargetSpec::new("a"), TargetSpec::new("b")],
            2,
            "render-me",
            Aggregator::vote(VoteConfig {
                ballot_schema: BallotSchema::FreeText,
                tie_break: TieBreak::Random { seed: 123 },
                abstain_threshold: 0,
            }),
        );

        let out1 = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        let out2 = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
        assert_eq!(
            out1.verify.unwrap().status,
            crate::strategy::VerifyStatus::Pass
        );
        assert_eq!(
            out2.verify.unwrap().status,
            crate::strategy::VerifyStatus::Pass
        );
        // Same winner on repeated runs because the seed and inputs are identical
    }

    #[test]
    fn vote_quorum_lost() {
        let a = MockBackend::ok("a", "yes");
        let b = MockBackend::fail("b", || BackendError::Network {
            message: "boom".into(),
        });
        let c = MockBackend::fail("c", || BackendError::Auth {
            message: "bad key".into(),
        });
        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone(), c.clone()];
        let strategy = ParallelFanOut::new(
            vec![
                TargetSpec::new("a"),
                TargetSpec::new("b"),
                TargetSpec::new("c"),
            ],
            1,
            "render-me",
            Aggregator::vote(VoteConfig {
                ballot_schema: BallotSchema::FreeText,
                tie_break: TieBreak::FirstResponder,
                abstain_threshold: 0,
            }),
        );

        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
        match err {
            StrategyError::Phase(phase_err) => {
                let msg = phase_err.to_string();
                assert!(
                    msg.contains("quorum lost"),
                    "expected quorum lost, got: {msg}"
                );
            }
            other => panic!("expected PhaseError::QuorumLost, got {other:?}"),
        }
    }
}

exec
/bin/zsh -lc "sed -n '1,260p' src/strategy/mod.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
//! Strategy primitives for orchestrating one or more backend calls within a phase.
//!
//! Per loker-design.md §4.2 a `Strategy` decides how a phase consumes backends:
//! `SingleModel` (one backend, one prompt, one response), `EscalatingRetry`
//! (CLO-258 - this module's second variant: try a cheap-to-strong ladder of
//! backends and stop at the first verify pass), `ParallelFanOut` (CLO-259).
//! All three implement the same `Strategy` trait so the phase runner (T-029)
//! can dispatch over them uniformly.
//!
//! ## Capability invariant
//!
//! Strategy code does *not* check `BackendCapabilities` at execute time.
//! Capability validation runs at workflow load (FR-4 / CLO-251's
//! `validate_with_capabilities`). By the time `Strategy::execute` runs the
//! caller has already proven that every backend it passes can do what the
//! strategy needs.

use crate::backend::{Backend, BackendError, QueryOutput, TokenUsage};
use crate::template::{TemplateContext, TemplateEngine, TemplateError};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

pub mod escalating_retry;
pub mod parallel_fanout;
pub mod single_model;
pub mod verify;

pub use escalating_retry::EscalatingRetry;
pub use parallel_fanout::{ParallelFanOut, TargetSpec};
pub use single_model::SingleModel;
pub use verify::{VerifyError, VerifyHook, VerifyResult};

/// `schema_version` value emitted by every `StrategyOutput`. Pinned to the
/// const declared in `docs/schemas/phase_result_*.schema.json`.
pub const SCHEMA_VERSION: u32 = 1;

/// Phase-level prompt overrides applied on top of the strategy's own
/// template. Currently carries an optional model override that passes
/// through to `Backend::query(.., model)`.
///
/// `#[non_exhaustive]` so future fields (system prompt, tool definitions)
/// land additively without breaking call sites.
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct Prompt {
    pub model: Option<String>,
}

impl Prompt {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
}

/// Per-phase metadata the strategy needs to render templates and stamp
/// the resulting `StrategyOutput`.
///
/// Production callers (CLO-261 / T-029 phase runner) build this from the
/// workflow definition; tests build it via `PhaseContext::new_for_test`.
///
/// `#[non_exhaustive]` so future fields (run dir path, verify config)
/// land additively.
#[non_exhaustive]
pub struct PhaseContext {
    pub phase_name: String,
    pub run_id: uuid::Uuid,
    pub cwd: PathBuf,
    pub template_engine: Arc<TemplateEngine>,
    pub template_context: TemplateContext,
}

impl PhaseContext {
    /// Builds a `PhaseContext` with a fresh `TemplateEngine`, empty
    /// `TemplateContext`, and `cwd` set to the current directory. Used
    /// by integration tests today; production callers (CLO-261 / T-029)
    /// can either call this and override fields or construct the struct
    /// literally from the workflow definition.
    pub fn new(phase_name: impl Into<String>, run_id: uuid::Uuid) -> Self {
        Self {
            phase_name: phase_name.into(),
            run_id,
            cwd: PathBuf::from("."),
            template_engine: Arc::new(TemplateEngine::new()),
            template_context: TemplateContext::new(&Default::default(), &[], &[]),
        }
    }
}

/// Strategy primitive: how a phase consumes backends.
///
/// Implementations must be `Send + Sync` so the phase runner can hold them
/// behind `Arc<dyn Strategy>` and drive them across async tasks.
///
/// `prompt: &Prompt` is borrowed so the same prompt can be replayed by
/// `EscalatingRetry` / `ParallelFanOut` without cloning. `backends:
/// &[Arc<dyn Backend>]` matches the in-tree convention - the engine resolves
/// backends to `Arc<dyn Backend>` via `create_backend`
/// (`src/backend/mod.rs:346`). Design doc §4.2 sketches `&[Box<dyn Backend>]`;
/// the `Arc` deviation is a deliberate alignment with the rest of the codebase.
#[async_trait]
pub trait Strategy: Send + Sync {
    async fn execute(
        &self,
        backends: &[Arc<dyn Backend>],
        prompt: &Prompt,
        ctx: &PhaseContext,
    ) -> Result<StrategyOutput, StrategyError>;
}

/// Discriminator for the active strategy, serialised into the
/// `loker.strategy` field of `StrategyOutput`.
///
/// `#[non_exhaustive]` - new variants land alongside new strategy
/// implementations (CLO-259).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyKind {
    Single,
    Escalating,
    Parallel,
}

/// Tier of a single attempt in an `EscalatingRetry` ladder.
///
/// The escalating phase-result schema requires every attempt to declare
/// which tier produced it; `SingleModel` attempts omit the field via
/// `#[serde(skip_serializing_if = "Option::is_none")]` on `Attempt::tier`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Cheap,
    Medium,
    Strong,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Cheap => "cheap",
            Self::Medium => "medium",
            Self::Strong => "strong",
        }
    }
}

/// Outcome of an `EscalatingRetry` ladder, recorded in the escalating
/// phase result. SingleModel omits the field via
/// `#[serde(skip_serializing_if = "Option::is_none")]`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FinalStatus {
    Succeeded,
    Exhausted,
    Aborted,
}

/// Phase result emitted by `Strategy::execute`.
///
/// Field names and rename attributes are pinned to the
/// `docs/schemas/phase_result_*.schema.json` set. `final_status` is only
/// populated by ladder strategies (currently `EscalatingRetry`); SingleModel
/// omits the field so the same struct serialises against the single schema.
///
/// For the `Parallel` strategy, `attempts` is serialised as `branches`, and
/// `aggregator`, `aggregate_output_path`, and `verify` are emitted at the
/// top level (per `phase_result_parallel.schema.json`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Aggregator {
    Concat,
    AnyFail,
    Vote,
    LLMJudge,
}

impl Aggregator {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Concat => "concat",
            Self::AnyFail => "any_fail",
            Self::Vote => "vote",
            Self::LLMJudge => "llm_judge",
        }
    }
}

#[derive(Debug, Clone)]
pub struct StrategyOutput {
    pub schema_version: u32,
    pub strategy: StrategyKind,
    pub phase: String,
    pub run_id: uuid::Uuid,
    pub attempts: Vec<Attempt>,
    pub final_status: Option<FinalStatus>,
    pub aggregator: Option<Aggregator>,
    pub aggregate_output_path: Option<String>,
    pub verify: Option<VerifyOutcome>,
}

impl Default for StrategyOutput {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            strategy: StrategyKind::Single,
            phase: String::new(),
            run_id: uuid::Uuid::nil(),
            attempts: Vec::new(),
            final_status: None,
            aggregator: None,
            aggregate_output_path: None,
            verify: None,
        }
    }
}

impl Serialize for StrategyOutput {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let is_parallel = matches!(self.strategy, StrategyKind::Parallel);
        let mut field_count = 4; // schema_version, loker.strategy, loker.phase, loker.run_id
        if is_parallel {
            field_count += 4; // branches, aggregator, aggregate_output_path, verify
        } else {
            field_count += 1; // attempts
            if self.final_status.is_some() {
                field_count += 1;
            }
        }

        let mut state = serializer.serialize_struct("StrategyOutput", field_count)?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("loker.strategy", &self.strategy)?;
        state.serialize_field("loker.phase", &self.phase)?;
        state.serialize_field("loker.run_id", &self.run_id)?;

        if is_parallel {
            #[derive(Serialize)]
            struct Branch<'a> {
                backend: &'a str,
                family: &'a str,
                model: &'a str,
                finish_reasons: &'a [FinishReason],
                usage: &'a TokenUsageReport,
                output_path: &'a str,
            }


exec
/bin/zsh -lc "sed -n '100,190p' src/family.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
        "claude" => Family::Anthropic,
        "gemini" => Family::Google,
        "openai" | "codex" => Family::OpenAI,
        "zhipu" => Family::Zhipu,
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
        "zhipu" => Family::Zhipu,
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

    #[error("aggregator contract violation: {message}")]
    AggregatorContract { message: String },

    #[error("quorum lost: {abstains} abstentions exceed threshold {threshold}")]
    QuorumLost { abstains: usize, threshold: usize },

    #[error("aggregator rejected: {message}")]
    AggregatorRejected { message: String },

    #[error("judge unavailable: {detail}")]
    JudgeUnavailable { detail: String },
}

/// Verify that every backend in `targets` resolves to a *different*
/// family. If any family appears on more than one backend, return
/// `Err(PhaseError::FamilyOverlap)`. An empty slice is allowed.
///
/// Two backends with `Family::Other("a")` and `Family::Other("b")` are
/// considered different families; two backends with `Family::Other("x")`
/// and `Family::Other("x")` are considered the same family.
pub fn enforce_cross_family(targets: &[&str]) -> Result<(), PhaseError> {
    use std::collections::BTreeMap;

    // BTreeMap (not HashMap) so iteration order is deterministic by `Family`
    // sort order. With multiple overlapping families, the first one by sort
    // order is reported, making error messages reproducible across runs.
    let mut counts: BTreeMap<Family, usize> = BTreeMap::new();
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


exec
/bin/zsh -lc "grep -R \"VoteConfig\\|Vote\\b\" -n src | head -n 200" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
src/aggregator/snapshots/loker__aggregator__vote__tests__vote_snapshot.snap:7:<!-- loker: Vote aggregator metadata
src/aggregator/concat.rs:3:use crate::aggregator::VoteConfig;
src/aggregator/concat.rs:43:    Vote {
src/aggregator/concat.rs:44:        config: VoteConfig,
src/aggregator/concat.rs:69:    /// Build a Vote aggregator with the provided configuration.
src/aggregator/concat.rs:70:    pub fn vote(config: VoteConfig) -> Self {
src/aggregator/concat.rs:71:        Self::Vote { config }
src/aggregator/concat.rs:85:            Self::Vote { .. } => crate::strategy::Aggregator::Vote,
src/aggregator/concat.rs:104:            Self::Vote { .. } => Err(AggregatorError::Unsupported(
src/aggregator/concat.rs:105:                "Vote is evaluated inline by ParallelFanOut, not via aggregate()".into(),
src/aggregator/concat.rs:481:            Aggregator::vote(crate::aggregator::VoteConfig {
src/aggregator/concat.rs:487:            crate::strategy::Aggregator::Vote
src/aggregator/mod.rs:24:    aggregate_vote, normalise_ballot, BallotSchema, TieBreak, VoteCandidate, VoteConfig, VoteError,
src/aggregator/vote.rs:1://! `Aggregator::Vote` implementation.
src/aggregator/vote.rs:32:/// Config payload for the Vote aggregator.
src/aggregator/vote.rs:34:pub struct VoteConfig {
src/aggregator/vote.rs:65:/// Errors specific to Vote aggregation.
src/aggregator/vote.rs:86:    config: &VoteConfig,
src/aggregator/vote.rs:253:    lines.push("<!-- loker: Vote aggregator metadata".into());
src/aggregator/vote.rs:300:    fn make_config(tie_break: TieBreak) -> VoteConfig {
src/aggregator/vote.rs:301:        VoteConfig {
src/aggregator/vote.rs:386:        let config = VoteConfig {
src/aggregator/vote.rs:416:        let config = VoteConfig {
src/workflow.rs:2103:                                ConsensusStrategy::Vote => {
src/workflow.rs:2107:                                                println!("    {} Vote tied ({} total), using first occurrence", "⚠".yellow(), result.total);
src/workflow.rs:2116:                                ConsensusStrategy::WeightedVote => {
src/consensus.rs:22:    Vote,
src/consensus.rs:24:    WeightedVote,
src/consensus.rs:32:    /// Vote breakdown: response -> count
src/strategy/parallel_fanout.rs:126:        let is_vote = matches!(self.aggregator, Aggregator::Vote { .. });
src/strategy/parallel_fanout.rs:203:                        // For non-LLMJudge / non-Vote aggregation modes, stop once enough
src/strategy/parallel_fanout.rs:207:                        // Vote must collect all branches (including failures as
src/strategy/parallel_fanout.rs:403:                Aggregator::Vote { config } => config,
src/strategy/parallel_fanout.rs:451:            output.verify = Some(VerifyOutcome::passed("Vote"));
src/strategy/parallel_fanout.rs:475:        AnyFailReason, BallotSchema, BranchFailure, BranchSuccess, TieBreak, VoteConfig,
src/strategy/parallel_fanout.rs:1032:            Aggregator::vote(VoteConfig {
src/strategy/parallel_fanout.rs:1046:        assert_eq!(out.verify.as_ref().unwrap().hook.as_deref(), Some("Vote"));
src/strategy/parallel_fanout.rs:1059:            Aggregator::vote(VoteConfig {
src/strategy/parallel_fanout.rs:1097:            Aggregator::vote(VoteConfig {
src/strategy/mod.rs:182:    Vote,
src/strategy/mod.rs:191:            Self::Vote => "vote",

exec
/bin/zsh -lc "grep -R \"enum AggregatorConfig\\|Aggregator.*AnyFail\\|Aggregator::Vote\\|llm_judge\\|vote\" -n src | head -n 200" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
src/aggregator/snapshots/loker__aggregator__vote__tests__vote_snapshot.snap:2:source: src/aggregator/vote.rs
src/aggregator/snapshots/loker__aggregator__vote__tests__vote_snapshot.snap:10:  vote_counts:
src/aggregator/concat.rs:57:    pub fn llm_judge(
src/aggregator/concat.rs:70:    pub fn vote(config: VoteConfig) -> Self {
src/aggregator/concat.rs:83:            Self::AnyFail => crate::strategy::Aggregator::AnyFail,
src/aggregator/concat.rs:85:            Self::Vote { .. } => crate::strategy::Aggregator::Vote,
src/aggregator/concat.rs:102:                "LLMJudge requires async backend access; use aggregate_llm_judge()".into(),
src/aggregator/concat.rs:471:    fn llm_judge_kind_maps_to_strategy_label() {
src/aggregator/concat.rs:473:            Aggregator::llm_judge("judge", "template", true).kind(),
src/aggregator/concat.rs:479:    fn vote_kind_maps_to_strategy_label() {
src/aggregator/concat.rs:481:            Aggregator::vote(crate::aggregator::VoteConfig {
src/aggregator/concat.rs:487:            crate::strategy::Aggregator::Vote
src/aggregator/mod.rs:7://! - [`llm_judge`]: use a separate-family LLM to pick the best candidate.
src/aggregator/mod.rs:10:mod llm_judge;
src/aggregator/mod.rs:11:mod vote;
src/aggregator/mod.rs:18:pub use llm_judge::{
src/aggregator/mod.rs:19:    aggregate_llm_judge, check_cross_family, clamp_chosen_index, parse_ballot,
src/aggregator/mod.rs:23:pub use vote::{
src/aggregator/mod.rs:24:    aggregate_vote, normalise_ballot, BallotSchema, TieBreak, VoteCandidate, VoteConfig, VoteError,
src/aggregator/vote.rs:1://! `Aggregator::Vote` implementation.
src/aggregator/vote.rs:3://! Pure, synchronous vote counting over parallel branch outcomes.
src/aggregator/vote.rs:43:/// A single candidate vote after normalisation.
src/aggregator/vote.rs:52:/// Result of a vote aggregation, including metadata for traceability.
src/aggregator/vote.rs:58:    pub vote_counts: Vec<(String, usize)>,
src/aggregator/vote.rs:80:/// Aggregate vote outcomes from parallel branches.
src/aggregator/vote.rs:84:pub fn aggregate_vote(
src/aggregator/vote.rs:124:    // Count votes by normalised bucket.
src/aggregator/vote.rs:131:    let max_votes = buckets.values().map(|v| v.len()).max().unwrap_or(0);
src/aggregator/vote.rs:134:        .filter(|(_, v)| v.len() == max_votes)
src/aggregator/vote.rs:142:            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
src/aggregator/vote.rs:152:            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
src/aggregator/vote.rs:160:    // Sort vote_counts descending for stable output
src/aggregator/vote.rs:161:    result.vote_counts.sort_by_key(|b| std::cmp::Reverse(b.1));
src/aggregator/vote.rs:256:    lines.push("  vote_counts:".into());
src/aggregator/vote.rs:257:    for (text, count) in &result.vote_counts {
src/aggregator/vote.rs:316:        let (artifact, result) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:320:            result.vote_counts,
src/aggregator/vote.rs:334:        let (_, result) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:347:        let (_, result) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:360:        let (_, result1) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:361:        let (_, result2) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:374:        let (_, result) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:376:        assert_eq!(result.vote_counts, vec![("yes".into(), 2)]);
src/aggregator/vote.rs:391:        let err = aggregate_vote(&branches, &config).unwrap_err();
src/aggregator/vote.rs:405:        let err = aggregate_vote(&branches, &config).unwrap_err();
src/aggregator/vote.rs:421:        let err = aggregate_vote(&branches, &config).unwrap_err();
src/aggregator/vote.rs:439:        let (_, result) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:440:        assert_eq!(result.vote_counts.len(), 1);
src/aggregator/vote.rs:441:        assert_eq!(result.vote_counts[0].1, 3);
src/aggregator/vote.rs:451:        let (_, result) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:452:        assert_eq!(result.vote_counts.len(), 1);
src/aggregator/vote.rs:453:        assert_eq!(result.vote_counts[0].1, 2);
src/aggregator/vote.rs:463:        let (_, result) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:476:        let (_, result) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:494:        let (_, result) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:509:        let (_, result) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:522:        let (_, result) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:528:    fn vote_counts_sorted_descending() {
src/aggregator/vote.rs:536:        let (_, result) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:538:            result.vote_counts,
src/aggregator/vote.rs:553:        let (artifact, _) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/vote.rs:561:    fn vote_snapshot() {
src/aggregator/vote.rs:568:        let (artifact, _) = aggregate_vote(&branches, &config).unwrap();
src/aggregator/llm_judge.rs:177:pub async fn aggregate_llm_judge(
src/aggregator/llm_judge.rs:255:    fn llm_judge_family_diverse_ok() {
src/aggregator/llm_judge.rs:265:    fn llm_judge_family_overlap_blocks() {
src/aggregator/llm_judge.rs:276:    fn llm_judge_family_overlap_opt_out_warns() {
src/aggregator/llm_judge.rs:282:    fn llm_judge_prompt_renders_candidates() {
src/aggregator/llm_judge.rs:302:    fn llm_judge_prompt_includes_phase_name() {
src/aggregator/llm_judge.rs:311:    fn llm_judge_parse_valid_ballot() {
src/aggregator/llm_judge.rs:323:    fn llm_judge_parse_markdown_fenced_ballot() {
src/aggregator/llm_judge.rs:331:    fn llm_judge_parse_missing_chosen_index() {
src/aggregator/llm_judge.rs:339:    fn llm_judge_parse_negative_chosen_index() {
src/aggregator/llm_judge.rs:347:    fn llm_judge_parse_out_of_bounds_index() {
src/aggregator/llm_judge.rs:353:    fn llm_judge_parse_missing_reason() {
src/aggregator/llm_judge.rs:359:    fn llm_judge_parse_malformed_json() {
src/aggregator/llm_judge.rs:365:    fn llm_judge_parse_within_bounds_index_clamped() {
src/aggregator/llm_judge.rs:370:    fn llm_judge_parse_out_of_bounds_index_clamped() {
src/aggregator/llm_judge.rs:375:    fn llm_judge_parse_zero_candidates_index() {
src/workflow.rs:401:    /// - "vote": Majority vote (for classification tasks)
src/workflow.rs:402:    /// - "weighted_vote": Weighted majority by backend tier
src/workflow.rs:2038:                            use crate::consensus::{BackendResponse, ConsensusStrategy, majority_vote, weighted_vote, BackendWeights};
src/workflow.rs:2104:                                    match majority_vote(&responses) {
src/workflow.rs:2109:                                                println!("    {} Majority vote: {}/{} backends agreed", "✓".green(), result.breakdown.get(&result.winner).unwrap_or(&0), result.total);
src/workflow.rs:2118:                                    match weighted_vote(&responses, &weights) {
src/workflow.rs:2121:                                                println!("    {} Weighted vote tied, using first occurrence", "⚠".yellow());
src/workflow.rs:2123:                                                println!("    {} Weighted vote: {:.1} weighted score", "✓".green(), result.breakdown.get(&result.winner).unwrap_or(&0.0));
src/consensus.rs:6://! - `vote`: Majority vote (for classification/yes-no)
src/consensus.rs:7://! - `weighted_vote`: Weighted majority by backend tier
src/consensus.rs:21:    /// Majority vote - pick most common response (for classification)
src/consensus.rs:23:    /// Weighted vote - weight by backend tier
src/consensus.rs:34:    /// Total votes cast
src/consensus.rs:101:/// Perform majority vote on responses
src/consensus.rs:104:pub fn majority_vote(responses: &[BackendResponse]) -> Option<VoteResult> {
src/consensus.rs:141:/// Perform weighted vote on responses
src/consensus.rs:143:/// Each backend's vote is weighted by its tier. Returns the response with highest total weight.
src/consensus.rs:144:pub fn weighted_vote(
src/consensus.rs:188:    fn test_majority_vote_clear_winner() {
src/consensus.rs:204:        let result = majority_vote(&responses).unwrap();
src/consensus.rs:212:    fn test_majority_vote_tie_first_wins() {
src/consensus.rs:224:        let result = majority_vote(&responses).unwrap();
src/consensus.rs:230:    fn test_majority_vote_empty() {
src/consensus.rs:232:        assert!(majority_vote(&responses).is_none());
src/consensus.rs:236:    fn test_weighted_vote() {
src/consensus.rs:253:        let result = weighted_vote(&responses, &weights).unwrap();
src/consensus.rs:261:    fn test_weighted_vote_clear_winner() {
src/consensus.rs:278:        let result = weighted_vote(&responses, &weights).unwrap();
src/consensus.rs:301:        let result = majority_vote(&responses).unwrap();
src/strategy/parallel_fanout.rs:14:use crate::aggregator::{aggregate_llm_judge, Aggregator, BranchSuccess};
src/strategy/parallel_fanout.rs:123:        let mut vote_branches: Vec<crate::aggregator::BranchOutcome> = Vec::new();
src/strategy/parallel_fanout.rs:124:        let is_any_fail = matches!(self.aggregator, Aggregator::AnyFail);
src/strategy/parallel_fanout.rs:125:        let is_llm_judge = matches!(self.aggregator, Aggregator::LLMJudge { .. });
src/strategy/parallel_fanout.rs:126:        let is_vote = matches!(self.aggregator, Aggregator::Vote { .. });
src/strategy/parallel_fanout.rs:166:                                verify: Some(VerifyOutcome::failed("Aggregator::AnyFail")),
src/strategy/parallel_fanout.rs:185:                    if is_vote {
src/strategy/parallel_fanout.rs:186:                        vote_branches
src/strategy/parallel_fanout.rs:201:                    if !is_any_fail && !is_llm_judge && !is_vote && successes >= self.min_responses
src/strategy/parallel_fanout.rs:248:                            verify: Some(VerifyOutcome::failed("Aggregator::AnyFail")),
src/strategy/parallel_fanout.rs:260:                    if is_vote {
src/strategy/parallel_fanout.rs:261:                        vote_branches.push(crate::aggregator::BranchOutcome::Failure(
src/strategy/parallel_fanout.rs:289:                verify: Some(VerifyOutcome::passed("Aggregator::AnyFail")),
src/strategy/parallel_fanout.rs:331:            let aggregate = aggregate_llm_judge(
src/strategy/parallel_fanout.rs:401:        if is_vote {
src/strategy/parallel_fanout.rs:403:                Aggregator::Vote { config } => config,
src/strategy/parallel_fanout.rs:407:            let (aggregate, _result) = crate::aggregator::aggregate_vote(&vote_branches, config)
src/strategy/parallel_fanout.rs:1019:    fn vote_success() {
src/strategy/parallel_fanout.rs:1032:            Aggregator::vote(VoteConfig {
src/strategy/parallel_fanout.rs:1047:        assert_eq!(out.aggregator.as_ref().unwrap().as_str(), "vote");
src/strategy/parallel_fanout.rs:1051:    fn vote_tie_random_deterministic() {
src/strategy/parallel_fanout.rs:1059:            Aggregator::vote(VoteConfig {
src/strategy/parallel_fanout.rs:1080:    fn vote_quorum_lost() {
src/strategy/parallel_fanout.rs:1097:            Aggregator::vote(VoteConfig {
src/strategy/mod.rs:191:            Self::Vote => "vote",
src/strategy/mod.rs:192:            Self::LLMJudge => "llm_judge",

exec
/bin/zsh -lc "grep -n \"ParallelFanOut\\|Aggregator::llm_judge\\|Aggregator::concat\\|Aggregator::any_fail\\|aggregator\" -n src/workflow.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 exited 1 in 0ms:
exec
/bin/zsh -lc "sed -n '1900,2180p' src/workflow.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
                                None
                            } else {
                                Some(StepFailure {
                                    kind: StepFailureKind::BackendError,
                                    message: "for_each: some iterations failed".to_string(),
                                    backend: if shell.is_none() { Some(backend_name.clone()) } else { None },
                                    exit_code: None,
                                    elapsed_ms,
                                })
                            };
                            return StepResult {
                                name: step_name,
                                output: output_json,
                                parsed_output: None,
                                success: all_success,
                                elapsed_ms,
                                backend: if shell.is_none() { Some(backend_name) } else { None },
                                raw_output: None,
                                stderr: None,
                                exit_code: None,
                                validation: None,
                                failure,
                            };
                        }

                        // Shell step - run command directly (with retry support)
                        if let Some(ref shell_cmd) = shell {
                            println!("  {} {}", "shell:".dimmed(), shell_cmd.dimmed());

                            let mut last_error = String::new();
                            for attempt in 0..=max_retries {
                                if attempt > 0 {
                                    let delay = retry_delay * 2_u64.pow(attempt - 1);
                                    // Record retry attempt for shell
                                    println!(
                                        "  {} Retry {}/{} in {}ms...",
                                        "↻".yellow(),
                                        attempt,
                                        max_retries,
                                        delay
                                    );
                                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                                }

                                match tokio::time::timeout(timeout_duration, run_shell(shell_cmd, &cwd, self.config.defaults.command_wrapper.as_deref())).await {
                                    Ok(Ok(shell_output)) => {
                                        let elapsed_ms = start.elapsed().as_millis() as u64;
                                        println!(
                                            "  {} ({:.1}s)",
                                            "✓".green(),
                                            elapsed_ms as f64 / 1000.0
                                        );

                                        // Run validation (heuristic + LLM) if configured
                                        let (validation, cleaned_output) = match validate_config.as_ref() {
                                            Some(vc) => run_step_validation(&shell_output.stdout, shell_output.stderr.as_deref(), vc, &config, &cwd).await,
                                            None => (None, None),
                                        };
                                        let validation_passed = validation.as_ref().map(|v| v.passed).unwrap_or(true);

                                        if !validation_passed {
                                            if let Some(ref v) = validation {
                                                let reason = v.failure_reason.as_deref().unwrap_or("validation failed");
                                                println!("  {} Validation failed ({}): {}", "✗".red(), v.validator, reason);
                                                if self.explain_validation {
                                                    if let Some(ref raw) = v.raw_response {
                                                        println!("\n  --- Raw validator response ({} chars) ---", raw.len());
                                                        for line in raw.lines() {
                                                            println!("  {}", line.dimmed());
                                                        }
                                                        println!("  --- End raw response ---\n");
                                                    }
                                                }
                                            }
                                        }

                                        let (final_output, raw_output) = if let Some(cleaned) = cleaned_output {
                                            if validate_config.as_ref().map(|vc| vc.replace_output).unwrap_or(false) {
                                                (cleaned, Some(shell_output.stdout))
                                            } else {
                                                (shell_output.stdout, None)
                                            }
                                        } else {
                                            (shell_output.stdout, None)
                                        };

                                        let parsed = parse_step_output(
                                            &final_output,
                                            output_format.as_deref(),
                                        );
                                        return StepResult {
                                            name: step_name,
                                            output: final_output,
                                            parsed_output: parsed,
                                            success: validation_passed,
                                            elapsed_ms,
                                            backend: None,
                                            raw_output,
                                            stderr: shell_output.stderr,
                                            exit_code: shell_output.exit_code,
                                            validation,
                                            failure: None,
                                        };
                                    }
                                    Ok(Err(e)) => {
                                        last_error = e.to_string();
                                        if attempt == max_retries {
                                            let elapsed_ms = start.elapsed().as_millis() as u64;
                                            // Record step complete (failure)
                                            let summary = summarize_shell_error("shell", &e.to_string());
                                            println!("  {} {}", "✗".red(), summary);
                                            return StepResult::error(step_name, format!("Error: {}", e), elapsed_ms, None, StepFailureKind::BackendError);
                                        }
                                        let summary = summarize_shell_error("shell", &e.to_string());
                                        println!("  {} {} (will retry)", "⚠".yellow(), summary);
                                    }
                                    Err(_) => {
                                        last_error = format!("Step timed out after {}s", timeout_duration.as_secs());
                                        if attempt == max_retries {
                                            let elapsed_ms = start.elapsed().as_millis() as u64;
                                            // Record step complete (failure - timeout)
                                            println!("  {} timed out after {}s", "✗".red(), timeout_duration.as_secs());
                                            return StepResult::error(step_name, format!("Error: {}", last_error), elapsed_ms, None, StepFailureKind::Timeout);
                                        }
                                        println!("  {} timed out (will retry)", "⚠".yellow());
                                    }
                                }
                            }

                            // Should never reach here, but just in case
                            let elapsed_ms = start.elapsed().as_millis() as u64;
                            // Record step complete (failure - fallback)
                            return StepResult::error(step_name, format!("Error: {}", last_error), elapsed_ms, None, StepFailureKind::BackendError);
                        }

                        // LLM step - query backend(s)
                        // Handle multi-backend with consensus
                        if backends_list.len() > 1 {
                            use crate::consensus::{BackendResponse, ConsensusStrategy, majority_vote, weighted_vote, BackendWeights};

                            println!("  {} querying {} backends with {:?} consensus", "[multi]".cyan(), backends_list.len(), consensus_strategy);

                            // Query all backends in parallel
                            let mut handles = Vec::new();
                            for bn in &backends_list {
                                let bn = bn.clone();
                                let cfg = config.clone();
                                let prompt = prompt.clone();
                                let cwd = cwd.clone();
                                let timeout_dur = timeout_duration;
                                let model_override = model_override.clone();

                                handles.push(tokio::spawn(async move {
                                    let backend_config = match cfg.backends.get(&bn) {
                                        Some(c) => c,
                                        None => return (bn.clone(), Err(format!("Backend not found: {}", bn))),
                                    };
                                    let retry_policy = backend::get_retry_policy(backend_config, &cfg.defaults);
                                    let backend = match backend::create_backend(&bn, backend_config, retry_policy) {
                                        Ok(b) => b,
                                        Err(e) => return (bn.clone(), Err(format!("Failed to create backend: {}", e))),
                                    };
                                    if !backend.is_available() {
                                        return (bn.clone(), Err(format!("Backend {} not available", bn)));
                                    }
                                    match tokio::time::timeout(timeout_dur, backend.query(&prompt, &cwd, model_override.as_deref())).await {
                                        Ok(Ok(qo)) => (bn.clone(), Ok(qo.stdout)),
                                        Ok(Err(e)) => (bn.clone(), Err(e.to_string())),
                                        Err(_) => (bn.clone(), Err(format!("Timeout after {}s", timeout_dur.as_secs()))),
                                    }
                                }));
                            }

                            // Collect results
                            let mut responses: Vec<BackendResponse> = Vec::new();
                            let mut errors: Vec<String> = Vec::new();
                            for handle in handles {
                                match handle.await {
                                    Ok((backend, Ok(content))) => {
                                        println!("    {} {}", "✓".green(), backend);
                                        responses.push(BackendResponse { backend, content });
                                    }
                                    Ok((backend, Err(e))) => {
                                        println!("    {} {} - {}", "✗".red(), backend, e);
                                        errors.push(format!("{}: {}", backend, e));
                                    }
                                    Err(e) => {
                                        errors.push(format!("Task error: {}", e));
                                    }
                                }
                            }

                            if responses.is_empty() {
                                let elapsed_ms = start.elapsed().as_millis() as u64;
                                return StepResult::error(step_name, format!("All backends failed: {}", errors.join("; ")), elapsed_ms, None, StepFailureKind::BackendError);
                            }

                            // Apply consensus strategy
                            let (final_output, used_backend) = match consensus_strategy {
                                ConsensusStrategy::First => {
                                    let r = &responses[0];
                                    (r.content.clone(), Some(r.backend.clone()))
                                }
                                ConsensusStrategy::Vote => {
                                    match majority_vote(&responses) {
                                        Some(result) => {
                                            if result.was_tie {
                                                println!("    {} Vote tied ({} total), using first occurrence", "⚠".yellow(), result.total);
                                            } else {
                                                println!("    {} Majority vote: {}/{} backends agreed", "✓".green(), result.breakdown.get(&result.winner).unwrap_or(&0), result.total);
                                            }
                                            (result.winner, None)
                                        }
                                        None => (responses[0].content.clone(), Some(responses[0].backend.clone())),
                                    }
                                }
                                ConsensusStrategy::WeightedVote => {
                                    let weights = BackendWeights::default();
                                    match weighted_vote(&responses, &weights) {
                                        Some(result) => {
                                            if result.was_tie {
                                                println!("    {} Weighted vote tied, using first occurrence", "⚠".yellow());
                                            } else {
                                                println!("    {} Weighted vote: {:.1} weighted score", "✓".green(), result.breakdown.get(&result.winner).unwrap_or(&0.0));
                                            }
                                            (result.winner, None)
                                        }
                                        None => (responses[0].content.clone(), Some(responses[0].backend.clone())),
                                    }
                                }
                                ConsensusStrategy::Synthesis => {
                                    // Format responses for synthesis
                                    let proposals = responses
                                        .iter()
                                        .map(|r| format!("## {}'s Response\n{}\n", r.backend, r.content))
                                        .collect::<Vec<_>>()
                                        .join("\n");

                                    let synth_prompt = format!(
                                        "Multiple AI backends responded to this prompt:\n\n\
                                        ## Original Prompt\n{}\n\n\
                                        ## Responses\n{}\n\n\
                                        ## Instructions\n\
                                        Synthesize these responses into a single, unified answer that:\n\
                                        1. Takes the best insights from each\n\
                                        2. Resolves any contradictions\n\
                                        3. Is clear and concise\n\n\
                                        Output only the synthesized response, no preamble.",
                                        prompt, proposals
                                    );

                                    // Use claude for synthesis (or first available backend)
                                    let synth_backend_name = if config.backends.contains_key("claude") {
                                        "claude"
                                    } else {
                                        backends_list.first().map(|s| s.as_str()).unwrap_or("claude")
                                    };

                                    println!("    {} Synthesizing with {}...", "⚙".cyan(), synth_backend_name);

                                    if let Some(synth_config) = config.backends.get(synth_backend_name) {
                                        let retry_policy = backend::get_retry_policy(synth_config, &config.defaults);
                                        if let Ok(synth_backend) = backend::create_backend(synth_backend_name, synth_config, retry_policy) {
                                            match tokio::time::timeout(timeout_duration, synth_backend.query(&synth_prompt, &cwd, None)).await {
                                                Ok(Ok(qo)) => {
                                                    let synthesized = qo.stdout;
                                                    println!("    {} Synthesized", "✓".green());
                                                    (synthesized, Some(synth_backend_name.to_string()))
                                                }
                                                Ok(Err(e)) => {
                                                    println!("    {} Synthesis failed: {}, using first response", "⚠".yellow(), e);
                                                    (responses[0].content.clone(), Some(responses[0].backend.clone()))
                                                }
                                                Err(_) => {
                                                    println!("    {} Synthesis timed out, using first response", "⚠".yellow());
                                                    (responses[0].content.clone(), Some(responses[0].backend.clone()))
                                                }
                                            }
                                        } else {
                                            println!("    {} Couldn't create synthesis backend, using first response", "⚠".yellow());
                                            (responses[0].content.clone(), Some(responses[0].backend.clone()))

exec
/bin/zsh -lc 'ls -la' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
total 312
-rw-r--r--@  1 mk  staff       0 Apr 29 20:39 !
drwxr-xr-x@ 32 mk  staff    1024 Apr 29 21:56 .
drwxr-xr-x@ 13 mk  staff     416 Apr 29 18:05 ..
drwxr-xr-x@  5 mk  staff     160 Apr 29 17:45 .claude
-rw-r--r--@  1 mk  staff      77 Apr 29 17:45 .git
drwxr-xr-x@  4 mk  staff     128 Apr 29 17:45 .github
-rw-r--r--@  1 mk  staff     115 Apr 29 17:45 .gitignore
drwxr-xr-x@  4 mk  staff     128 Apr 29 20:53 .lok
-rw-r--r--@  1 mk  staff     111 Apr 29 17:45 .mcp.json
drwxr-xr-x@  6 mk  staff     192 Apr 29 17:45 .pi
-rw-r--r--@  1 mk  staff     306 Apr 29 17:45 AGENTS.md
-rw-r--r--@  1 mk  staff     608 Apr 29 17:45 CLAUDE.md
-rw-r--r--@  1 mk  staff  110436 Apr 29 17:45 Cargo.lock
-rw-r--r--@  1 mk  staff    1642 Apr 29 17:45 Cargo.toml
-rw-r--r--@  1 mk  staff    1121 Apr 29 17:45 LICENSE
-rw-r--r--@  1 mk  staff    4000 Apr 29 17:45 Makefile
-rw-r--r--@  1 mk  staff    5655 Apr 29 17:45 README.md
drwxr-xr-x@  8 mk  staff     256 Apr 29 17:45 architecture
drwxr-xr-x@  3 mk  staff      96 Apr 29 17:45 codex
drwxr-xr-x@ 16 mk  staff     512 Apr 29 17:45 docs
drwxr-xr-x@  4 mk  staff     128 Apr 29 17:45 examples
-rw-r--r--@  1 mk  staff    2339 Apr 29 17:45 lok.toml
drwxr-xr-x@  3 mk  staff      96 Apr 29 21:49 phase-1
drwxr-xr-x@  3 mk  staff      96 Apr 29 21:56 phase-1d64d69c-6756-4dbf-8f48-53aadabf8cb7
drwxr-xr-x@  3 mk  staff      96 Apr 29 21:56 phase-3c482da1-9d35-4cf5-94c4-50dd539edafe
drwxr-xr-x@  3 mk  staff      96 Apr 29 21:56 phase-f3c63df5-e487-4227-b8bb-4cc9dcfcc443
-rw-r--r--@  1 mk  staff    1099 Apr 29 17:45 shell.nix
drwxr-xr-x@ 12 mk  staff     384 Apr 29 17:45 specs
drwxr-xr-x@ 26 mk  staff     832 Apr 29 17:45 src
drwxr-xr-x@  7 mk  staff     224 Apr 29 21:55 target
drwxr-xr-x@  5 mk  staff     160 Apr 29 17:45 tensorzero
drwxr-xr-x@ 12 mk  staff     384 Apr 29 17:45 tests

exec
/bin/zsh -lc 'git status --short' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
 M docs/status/clo-269-workflow.yaml
?? docs/reviews/clo-269-codex-validation.md

exec
/bin/zsh -lc "git ls-files --stage | grep ' "'!$'"'" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 exited 1 in 0ms:
exec
/bin/zsh -lc 'git status --short -uall' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
 M docs/status/clo-269-workflow.yaml
?? docs/reviews/clo-269-codex-validation.md

exec
/bin/zsh -lc 'cat .gitignore' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
/target/
.lok/
tensorzero/.env

# Node (pi extensions)
node_modules/

# Parallel fanout runtime artifacts
phase-*/

exec
/bin/zsh -lc "git ls-files --error-unmatch '"'!'"'; echo "'$?' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
!
0

exec
/bin/zsh -lc 'cat docs/status/clo-269-workflow.yaml' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
task_id: clo-269
task_type: development
classification_reason: >-
  Issue title starts with Implement and includes open design decisions (ballot schema/tie-breakers), so classification
  is development.
task_profile:
  has_backend: false
  has_frontend: false
  has_data_model: false
  has_external_deps: false
  skip_probe: false
pending_human_action: null
linear:
  team: Cloud-ai
  project: Loker
  status_at_start: Backlog
  blocks: []
  blocked_by:
    - CLO-265
  priority: Low
  branch_suggested: kmamemo/clo-269-implement-aggregatorvote-with-ballot-schema-and-tie-breakers
  branch_actual: feat/clo-269
workflow:
  current_phase: implement
  status: active
  created_at: '2026-04-29T15:46:46.491Z'
  updated_at: '2026-04-29T19:56:36.549Z'
phases:
  discovery:
    status: complete
    approach_chosen: B - Extract vote.rs module under src/aggregator/
    approaches_identified: 3
    approved: true
    baseline_score: 7
    discovery_debt:
      - follow_up: TDD (design phase) will lock exact BallotSchema enum shape.
        item: Ballot schema shape (enum vs free-text) must be decided in TDD doc before implementation.
      - follow_up: TDD (design phase) will decide.
        item: Seed source for TieBreak::Random — whether from lok.toml/workflow config or derived from run-level UUID.
      - follow_up: TDD (design phase) will lock exact spelling and semantics.
        item: 'Quorum threshold semantics: absolute count vs fraction.'
    discovery_report: docs/discovery/clo-269.md
    prd_created: true
    prd_exists: true
    prd_file: docs/prds/clo-269-aggregator-vote.md
    problem_framed: true
  design:
    status: complete
    design_doc: docs/designs/clo-269-aggregator-vote.md
    discovery_context_used: true
    draft_ready: true
    applied_suggestions:
      - 'Added #[non_exhaustive] to BallotSchema for v0+1 extensibility'
      - Added Serialize/Deserialize derives to VoteConfig for TOML parsing
      - Removed VoteError::NoOpinion (unanimous single-bucket is a valid win, not an error)
      - Unified compute_vote signature on canonical VoteCandidate struct
      - Added VoteCandidate struct definition to Public API surface
      - 'Added StrategyError mapping: QuorumLost → PhaseError::QuorumLost, NoCandidates → PhaseError::AggregatorRejected'
      - Replaced HashMap with BTreeMap in vote counting for deterministic tie-break ordering
      - >-
        Fixed ClosestToFamily fallback: when multiple tied buckets match the target family, apply FirstResponder among
        the matching subset
      - Added HTML comment sanitization note (replace --> with -- >)
      - Added is_vote short-circuit guard in parallel_fanout.rs to collect all branches
      - Updated all_abstain test expectation
      - Updated migration/rollout with phase_result_parallel.schema.json 'vote' enum addition
      - Updated open questions with min_responses/abstain_threshold independence and rand import minimization
    flagged_suggestions:
      - id: cross_family_enforcement
        reason: >-
          PRD FR-13 scopes cross-family enforcement to LLMJudge only. Vote has no judge and is inherently counting
          diversity. Cross-family selection belongs to the strategy layer, not the aggregator. Documented as non-goal in
          design.
      - id: rename_concat_rs
        reason: >-
          Style concern (P3) - four variants in concat.rs is a smell, but renaming introduces churn without functional
          benefit. Deferred to future refactoring (T-029 Aggregator trait formalization).
    review_completed: true
    review_gemini: docs/reviews/clo-269-design-gemini.md
    review_synthesis: docs/reviews/clo-269-design-synthesis.md
    review_verdict: approve_with_changes
    finalized: true
  plan:
    status: complete
    plan_file: docs/plans/clo-269-aggregator-vote.md
    approved: true
  implement:
    status: complete
    commits:
      - c9b13d5
      - 364f7a5
      - 5affc5a
      - 5c4fe89
      - 8c010be
  pr:
    status: pending
  complete:
    status: pending
history:
  - timestamp: '2026-04-29T15:46:46.491Z'
    action: workflow_started
    phase: init
    details: Workflow initialized for clo-269 as development
  - timestamp: '2026-04-29T15:47:09.238Z'
    action: workflow_resumed
    phase: init
    details: Resuming existing workflow from init; skipping re-initialization path.
  - timestamp: '2026-04-29T15:47:11.477Z'
    action: linear_status_updated
    phase: init
    details: Linear issue status moved Backlog -> Todo per init workflow initialization.
  - timestamp: '2026-04-29T15:47:14.548Z'
    action: task_requalified
    phase: init
    details: Classified as development (implementation scope with architecture decisions on ballot schema and tie-break rules).
  - timestamp: '2026-04-29T15:47:16.643Z'
    action: project_sync_skipped
    phase: init
    details: No PROJECT.md/ROADMAP.md/DEPENDENCIES.md exist in this repo.
  - timestamp: '2026-04-29T15:47:19.344Z'
    action: metadata_adjusted
    phase: init
    details: Preserved linear status_at_start as Backlog while current linear status is now Todo.
  - timestamp: '2026-04-29T15:50:13.010Z'
    action: first_phase_dispatched
    phase: init
    details: Init completed; advancing to first phase 'discovery' for development task per task type and dispatch contract.
  - timestamp: '2026-04-29T16:06:15.852Z'
    action: branch_created
    phase: discovery
    details: Branch feat/clo-269 already checked out and even with main; recording as actual.
  - timestamp: '2026-04-29T18:37:53.367Z'
    action: discovery_approved
    phase: discovery
    details: >-
      Approach B: Extract vote.rs module under src/aggregator/. Baseline 7/10. 3 approaches considered (A inline, B
      extracted module, C reuse consensus.rs). PRD created from scratch. Discovery debt: 3 items to resolve in TDD
      (ballot schema shape, Random seed source, quorum threshold semantics).
  - timestamp: '2026-04-29T18:38:02.835Z'
    action: discovery_status_fixed
    phase: discovery
    details: Set discovery.status to complete so transition to design is allowed.
  - timestamp: '2026-04-29T18:38:06.397Z'
    action: phase_transition
    phase: discovery
    details: Transitioned from discovery to design
  - timestamp: '2026-04-29T18:45:47.143Z'
    action: design_draft_ready
    phase: design
    details: Design doc draft at docs/designs/clo-269-aggregator-vote.md
  - timestamp: '2026-04-29T19:21:18.964Z'
    action: design_review_complete
    phase: design
    details: >-
      Review verdict: approve_with_changes. 13 applied suggestions, 2 flagged. Reviews written to
      docs/reviews/clo-269-design-gemini.md and docs/reviews/clo-269-design-synthesis.md.
  - timestamp: '2026-04-29T19:21:46.173Z'
    action: design_finalized
    phase: design
    details: >-
      Design ready for plan phase. Review feedback applied: 13 additions, 2 flagged (cross-family enforcement, rename
      concat.rs). All P0 items addressed (short-circuit guard, deterministic ordering). VoteError::NoOpinion variant
      removed. BTreeMap used for determinism. HTML sanitization documented. StrategyError mapping specified.
  - timestamp: '2026-04-29T19:21:49.711Z'
    action: phase_transition
    phase: design
    details: Transitioned from design to plan
  - timestamp: '2026-04-29T19:23:08.265Z'
    action: plan_created
    phase: plan
    details: Plan with 6 sub-tasks at docs/plans/clo-269-aggregator-vote.md
  - timestamp: '2026-04-29T19:23:14.565Z'
    action: plan_approved
    phase: plan
    details: Plan approved. 6 sub-tasks queued (ST1–ST5 implementation + ST6 pre-merge gate).
  - timestamp: '2026-04-29T19:23:20.683Z'
    action: phase_transition
    phase: plan
    details: Transitioned from plan to implement
  - timestamp: '2026-04-29T19:56:36.549Z'
    action: implementation_complete
    phase: implement
    details: 'All sub-tasks landed (ST1–ST6). make check green. 4 commits: c9b13d5, 364f7a5, 5affc5a, 5c4fe89, 8c010be.'
task_title: Implement Aggregator::Vote with ballot schema and tie-breakers
task_url: https://linear.app/cloud-ai/issue/clo-269/implement-aggregatorvote-with-ballot-schema-and-tie-breakers

exec
/bin/zsh -lc 'git diff --name-only 48cb96fc8c70f3f8cd70490580c279833fef6ef9' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
!
docs/designs/clo-269-aggregator-vote.md
docs/discovery/clo-269.md
docs/plans/clo-269-aggregator-vote.md
docs/prds/clo-269-aggregator-vote.md
docs/reviews/CLO-269-review-ollama.md
docs/reviews/clo-269-design-gemini.md
docs/reviews/clo-269-design-synthesis.md
docs/status/clo-269-workflow.yaml
src/aggregator/concat.rs
src/aggregator/mod.rs
src/aggregator/snapshots/loker__aggregator__vote__tests__vote_snapshot.snap
src/aggregator/vote.rs
src/family.rs
src/strategy/parallel_fanout.rs

exec
/bin/zsh -lc 'cat src/aggregator/snapshots/loker__aggregator__vote__tests__vote_snapshot.snap' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
---
source: src/aggregator/vote.rs
expression: artifact.text
---
yes

<!-- loker: Vote aggregator metadata
  winner: yes
  total_branches: 3
  vote_counts:
    yes: 2
    no: 1
  abstain_count: 0
  tie_broken: false
  tie_break_rule: none (strict majority)
-->

exec
/bin/zsh -lc 'git diff 48cb96fc8c70f3f8cd70490580c279833fef6ef9 -- src/strategy/parallel_fanout.rs' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
diff --git a/src/strategy/parallel_fanout.rs b/src/strategy/parallel_fanout.rs
index d793de2..66bf71d 100644
--- a/src/strategy/parallel_fanout.rs
+++ b/src/strategy/parallel_fanout.rs
@@ -120,8 +120,10 @@ impl Strategy for ParallelFanOut {
         let mut successes = 0;
         let mut successful_candidates: Vec<crate::aggregator::BranchSuccess> =
             Vec::with_capacity(self.targets.len());
+        let mut vote_branches: Vec<crate::aggregator::BranchOutcome> = Vec::new();
         let is_any_fail = matches!(self.aggregator, Aggregator::AnyFail);
         let is_llm_judge = matches!(self.aggregator, Aggregator::LLMJudge { .. });
+        let is_vote = matches!(self.aggregator, Aggregator::Vote { .. });
 
         while let Some((idx, result)) = futures.next().await {
             let target = &self.targets[idx];
@@ -173,12 +175,17 @@ impl Strategy for ParallelFanOut {
                     }
 
                     successes += 1;
-                    successful_candidates.push(BranchSuccess {
+                    let branch_success = BranchSuccess {
                         backend_id: target.backend.clone(),
                         family: family_of(&target.backend).to_string(),
                         index: successful_candidates.len() + 1,
                         output: query.stdout.clone(),
-                    });
+                    };
+                    successful_candidates.push(branch_success.clone());
+                    if is_vote {
+                        vote_branches
+                            .push(crate::aggregator::BranchOutcome::Success(branch_success));
+                    }
 
                     attempts.push(Attempt {
                         tier: None,
@@ -191,11 +198,15 @@ impl Strategy for ParallelFanOut {
                         verify: VerifyOutcome::skipped(),
                     });
 
-                    if !is_any_fail && !is_llm_judge && successes >= self.min_responses {
-                        // For non-LLMJudge aggregation modes, stop once enough
+                    if !is_any_fail && !is_llm_judge && !is_vote && successes >= self.min_responses
+                    {
+                        // For non-LLMJudge / non-Vote aggregation modes, stop once enough
                         // successes are in to meet the configured floor.
                         // LLMJudge must inspect all candidates first and therefore
                         // cannot short-circuit on min_responses.
+                        // Vote must collect all branches (including failures as
+                        // abstentions) before it can compute a majority or detect
+                        // a quorum loss.
                         break;
                     }
                 }
@@ -246,6 +257,17 @@ impl Strategy for ParallelFanOut {
                         });
                     }
 
+                    if is_vote {
+                        vote_branches.push(crate::aggregator::BranchOutcome::Failure(
+                            crate::aggregator::BranchFailure {
+                                backend_id: target.backend.clone(),
+                                family: family_of(&target.backend).to_string(),
+                                index: attempts.len() + 1,
+                                reason: err.to_string(),
+                            },
+                        ));
+                    }
+
                     attempts.push(attempt);
                 }
             }
@@ -376,6 +398,59 @@ impl Strategy for ParallelFanOut {
             output.verify = Some(VerifyOutcome::passed("LLMJudge"));
         }
 
+        if is_vote {
+            let config = match &self.aggregator {
+                Aggregator::Vote { config } => config,
+                _ => unreachable!(),
+            };
+
+            let (aggregate, _result) = crate::aggregator::aggregate_vote(&vote_branches, config)
+                .map_err(|err| match err {
+                    crate::aggregator::VoteError::QuorumLost {
+                        abstains,
+                        threshold,
+                    } => StrategyError::Phase(crate::family::PhaseError::QuorumLost {
+                        abstains,
+                        threshold,
+                    }),
+                    crate::aggregator::VoteError::NoCandidates => {
+                        StrategyError::Phase(crate::family::PhaseError::AggregatorRejected {
+                            message: "no candidates".into(),
+                        })
+                    }
+                })?;
+
+            if let Some(parent) = Path::new(&aggregated_output_path).parent() {
+                if !parent.as_os_str().is_empty() {
+                    fs::create_dir_all(parent).await.map_err(|err| {
+                        StrategyError::Backend(crate::backend::BackendError::ExecutionFailed {
+                            message: format!(
+                                "failed to create aggregate output parent {}: {err}",
+                                parent.display()
+                            ),
+                            exit_code: None,
+                        })
+                    })?;
+                }
+            }
+
+            let aggregate_output_path_ref = aggregated_output_path.as_str();
+            fs::write(&aggregated_output_path, aggregate.text)
+                .await
+                .map_err(|err| {
+                    StrategyError::Backend(
+                        crate::backend::BackendError::ExecutionFailed {
+                            message: format!(
+                                "failed to write aggregate output to {aggregate_output_path_ref}: {err}"
+                            ),
+                            exit_code: None,
+                        },
+                    )
+                })?;
+
+            output.verify = Some(VerifyOutcome::passed("Vote"));
+        }
+
         Ok(output)
     }
 }
@@ -396,7 +471,9 @@ fn pick_model_override(query: &QueryOutput, prompt: &Prompt, target: &TargetSpec
 #[cfg(test)]
 mod tests {
     use super::*;
-    use crate::aggregator::AnyFailReason;
+    use crate::aggregator::{
+        AnyFailReason, BallotSchema, BranchFailure, BranchSuccess, TieBreak, VoteConfig,
+    };
     use crate::backend::BackendError;
     use std::path::Path;
     use std::sync::atomic::{AtomicUsize, Ordering};
@@ -937,4 +1014,103 @@ mod tests {
             other => panic!("expected AnyFail, got {other:?}"),
         }
     }
+
+    #[test]
+    fn vote_success() {
+        let a = MockBackend::ok("a", "A");
+        let b = MockBackend::ok("b", "A");
+        let c = MockBackend::ok("c", "B");
+        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone(), c.clone()];
+        let strategy = ParallelFanOut::new(
+            vec![
+                TargetSpec::new("a"),
+                TargetSpec::new("b"),
+                TargetSpec::new("c"),
+            ],
+            2,
+            "render-me",
+            Aggregator::vote(VoteConfig {
+                ballot_schema: BallotSchema::FreeText,
+                tie_break: TieBreak::FirstResponder,
+                abstain_threshold: 0,
+            }),
+        );
+
+        let out = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
+        assert_eq!(out.strategy, StrategyKind::Parallel);
+        assert_eq!(out.attempts.len(), 3);
+        assert_eq!(
+            out.verify.as_ref().unwrap().status,
+            crate::strategy::VerifyStatus::Pass
+        );
+        assert_eq!(out.verify.as_ref().unwrap().hook.as_deref(), Some("Vote"));
+        assert_eq!(out.aggregator.as_ref().unwrap().as_str(), "vote");
+    }
+
+    #[test]
+    fn vote_tie_random_deterministic() {
+        let a = MockBackend::ok("a", "A");
+        let b = MockBackend::ok("b", "B");
+        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone()];
+        let strategy = ParallelFanOut::new(
+            vec![TargetSpec::new("a"), TargetSpec::new("b")],
+            2,
+            "render-me",
+            Aggregator::vote(VoteConfig {
+                ballot_schema: BallotSchema::FreeText,
+                tie_break: TieBreak::Random { seed: 123 },
+                abstain_threshold: 0,
+            }),
+        );
+
+        let out1 = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
+        let out2 = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap();
+        assert_eq!(
+            out1.verify.unwrap().status,
+            crate::strategy::VerifyStatus::Pass
+        );
+        assert_eq!(
+            out2.verify.unwrap().status,
+            crate::strategy::VerifyStatus::Pass
+        );
+        // Same winner on repeated runs because the seed and inputs are identical
+    }
+
+    #[test]
+    fn vote_quorum_lost() {
+        let a = MockBackend::ok("a", "yes");
+        let b = MockBackend::fail("b", || BackendError::Network {
+            message: "boom".into(),
+        });
+        let c = MockBackend::fail("c", || BackendError::Auth {
+            message: "bad key".into(),
+        });
+        let backends: Vec<Arc<dyn Backend>> = vec![a.clone(), b.clone(), c.clone()];
+        let strategy = ParallelFanOut::new(
+            vec![
+                TargetSpec::new("a"),
+                TargetSpec::new("b"),
+                TargetSpec::new("c"),
+            ],
+            1,
+            "render-me",
+            Aggregator::vote(VoteConfig {
+                ballot_schema: BallotSchema::FreeText,
+                tie_break: TieBreak::FirstResponder,
+                abstain_threshold: 0,
+            }),
+        );
+
+        let err = run(strategy.execute(&backends, &Prompt::new(), &ctx())).unwrap_err();
+        match err {
+            StrategyError::Phase(phase_err) => {
+                let msg = phase_err.to_string();
+                assert!(
+                    msg.contains("quorum lost"),
+                    "expected quorum lost, got: {msg}"
+                );
+            }
+            other => panic!("expected PhaseError::QuorumLost, got {other:?}"),
+        }
+    }
 }

exec
/bin/zsh -lc "nl -ba src/aggregator/vote.rs | sed -n '1,260p'" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
     1	//! `Aggregator::Vote` implementation.
     2	//!
     3	//! Pure, synchronous vote counting over parallel branch outcomes.
     4	//! No secondary backend calls, no async, no I/O.
     5	
     6	use std::collections::BTreeMap;
     7	
     8	use crate::family::{family_of, Family};
     9	
    10	use super::{AggregatedArtifact, BranchOutcome};
    11	
    12	/// How a ballot is normalised and interpreted.
    13	#[derive(Debug, Clone, PartialEq, Eq)]
    14	#[non_exhaustive]
    15	pub enum BallotSchema {
    16	    /// Free text: each backend returns prose; normalise before bucketing.
    17	    FreeText,
    18	}
    19	
    20	/// How to resolve a tie when no strict majority exists.
    21	#[derive(Debug, Clone, PartialEq, Eq)]
    22	pub enum TieBreak {
    23	    /// Pick the candidate whose backend family matches the given family.
    24	    /// If multiple candidates match, first occurrence in arrival order wins.
    25	    ClosestToFamily(Family),
    26	    /// Deterministic shuffle from a fixed seed.
    27	    Random { seed: u64 },
    28	    /// Pick the candidate whose successful response arrived first.
    29	    FirstResponder,
    30	}
    31	
    32	/// Config payload for the Vote aggregator.
    33	#[derive(Debug, Clone, PartialEq, Eq)]
    34	pub struct VoteConfig {
    35	    pub ballot_schema: BallotSchema,
    36	    pub tie_break: TieBreak,
    37	    /// Number of abstentions (errors + malformed answers) that triggers
    38	    /// `QuorumLost`. Fires when **strictly more** than `abstain_threshold`
    39	    /// are abstentions.
    40	    pub abstain_threshold: usize,
    41	}
    42	
    43	/// A single candidate vote after normalisation.
    44	#[derive(Debug, Clone, PartialEq, Eq)]
    45	pub struct VoteCandidate {
    46	    pub backend_id: String,
    47	    pub family: String,
    48	    pub normalised: String,
    49	    pub arrival_order: usize,
    50	}
    51	
    52	/// Result of a vote aggregation, including metadata for traceability.
    53	#[derive(Debug, Clone, PartialEq, Eq)]
    54	pub struct VoteResult {
    55	    /// The winning text (normalised key).
    56	    pub winner: String,
    57	    /// Sorted descending by count for snapshot determinism.
    58	    pub vote_counts: Vec<(String, usize)>,
    59	    pub abstain_count: usize,
    60	    pub total_branches: usize,
    61	    pub tie_broken: bool,
    62	    pub tie_break_rule: String,
    63	}
    64	
    65	/// Errors specific to Vote aggregation.
    66	#[derive(Debug, thiserror::Error, PartialEq, Eq)]
    67	pub enum VoteError {
    68	    #[error("quorum lost: {abstains} abstentions exceed threshold {threshold}")]
    69	    QuorumLost { abstains: usize, threshold: usize },
    70	
    71	    #[error("no candidates available")]
    72	    NoCandidates,
    73	}
    74	
    75	/// Normalise a ballot text for comparison.
    76	pub fn normalise_ballot(text: &str) -> String {
    77	    text.trim().to_lowercase()
    78	}
    79	
    80	/// Aggregate vote outcomes from parallel branches.
    81	///
    82	/// Returns the aggregated artefact and structured result metadata.
    83	/// Pure synchronous function — no async, no backend calls.
    84	pub fn aggregate_vote(
    85	    branches: &[BranchOutcome],
    86	    config: &VoteConfig,
    87	) -> Result<(AggregatedArtifact, VoteResult), VoteError> {
    88	    let mut abstain_count = 0;
    89	    let mut candidates: Vec<VoteCandidate> = Vec::new();
    90	    let total = branches.len();
    91	
    92	    for (arrival_order, branch) in branches.iter().enumerate() {
    93	        match branch {
    94	            BranchOutcome::Success(success) => {
    95	                let normalised = normalise_ballot(&success.output);
    96	                if normalised.is_empty() {
    97	                    abstain_count += 1;
    98	                } else {
    99	                    candidates.push(VoteCandidate {
   100	                        backend_id: success.backend_id.clone(),
   101	                        family: success.family.clone(),
   102	                        normalised,
   103	                        arrival_order,
   104	                    });
   105	                }
   106	            }
   107	            BranchOutcome::Failure(_) => {
   108	                abstain_count += 1;
   109	            }
   110	        }
   111	    }
   112	
   113	    if abstain_count > config.abstain_threshold {
   114	        return Err(VoteError::QuorumLost {
   115	            abstains: abstain_count,
   116	            threshold: config.abstain_threshold,
   117	        });
   118	    }
   119	
   120	    if candidates.is_empty() {
   121	        return Err(VoteError::NoCandidates);
   122	    }
   123	
   124	    // Count votes by normalised bucket.
   125	    // BTreeMap ensures deterministic iteration order for tie-break determinism.
   126	    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
   127	    for (idx, c) in candidates.iter().enumerate() {
   128	        buckets.entry(c.normalised.clone()).or_default().push(idx);
   129	    }
   130	
   131	    let max_votes = buckets.values().map(|v| v.len()).max().unwrap_or(0);
   132	    let winners: Vec<&str> = buckets
   133	        .iter()
   134	        .filter(|(_, v)| v.len() == max_votes)
   135	        .map(|(k, _)| k.as_str())
   136	        .collect();
   137	
   138	    let mut result = if winners.len() == 1 {
   139	        let winner_text = winners[0];
   140	        VoteResult {
   141	            winner: winner_text.into(),
   142	            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
   143	            abstain_count,
   144	            total_branches: total,
   145	            tie_broken: false,
   146	            tie_break_rule: "none (strict majority)".into(),
   147	        }
   148	    } else {
   149	        let chosen_text = resolve_tie(&winners, &candidates, &buckets, &config.tie_break);
   150	        VoteResult {
   151	            winner: chosen_text,
   152	            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
   153	            abstain_count,
   154	            total_branches: total,
   155	            tie_broken: true,
   156	            tie_break_rule: format_tie_break_rule(&config.tie_break),
   157	        }
   158	    };
   159	
   160	    // Sort vote_counts descending for stable output
   161	    result.vote_counts.sort_by_key(|b| std::cmp::Reverse(b.1));
   162	
   163	    let text = build_aggregated_text(&result, &candidates, &buckets);
   164	    let artifact = AggregatedArtifact {
   165	        text,
   166	        successful: candidates.len(),
   167	        failed: abstain_count,
   168	    };
   169	
   170	    Ok((artifact, result))
   171	}
   172	
   173	fn resolve_tie(
   174	    tied_buckets: &[&str],
   175	    candidates: &[VoteCandidate],
   176	    buckets: &BTreeMap<String, Vec<usize>>,
   177	    tie_break: &TieBreak,
   178	) -> String {
   179	    match tie_break {
   180	        TieBreak::FirstResponder => tied_buckets
   181	            .iter()
   182	            .min_by_key(|&&bucket| {
   183	                buckets[bucket]
   184	                    .iter()
   185	                    .map(|&ci| candidates[ci].arrival_order)
   186	                    .min()
   187	                    .unwrap_or(usize::MAX)
   188	            })
   189	            .copied()
   190	            .unwrap_or(tied_buckets[0])
   191	            .to_string(),
   192	
   193	        TieBreak::ClosestToFamily(target_family) => {
   194	            let matching: Vec<&str> = tied_buckets
   195	                .iter()
   196	                .copied()
   197	                .filter(|&bucket| {
   198	                    buckets[bucket]
   199	                        .iter()
   200	                        .any(|&ci| family_of(&candidates[ci].backend_id) == *target_family)
   201	                })
   202	                .collect();
   203	
   204	            if matching.is_empty() {
   205	                resolve_tie(tied_buckets, candidates, buckets, &TieBreak::FirstResponder)
   206	            } else if matching.len() == 1 {
   207	                matching[0].to_string()
   208	            } else {
   209	                resolve_tie(&matching, candidates, buckets, &TieBreak::FirstResponder)
   210	            }
   211	        }
   212	
   213	        TieBreak::Random { seed } => {
   214	            use rand::rngs::StdRng;
   215	            use rand::Rng;
   216	            use rand::SeedableRng;
   217	
   218	            let mut rng = StdRng::seed_from_u64(*seed);
   219	            let idx = rng.random_range(0..tied_buckets.len());
   220	            tied_buckets[idx].to_string()
   221	        }
   222	    }
   223	}
   224	
   225	fn format_tie_break_rule(tie_break: &TieBreak) -> String {
   226	    match tie_break {
   227	        TieBreak::ClosestToFamily(f) => format!("closest_to_family({})", f),
   228	        TieBreak::Random { seed } => format!("random(seed={})", seed),
   229	        TieBreak::FirstResponder => "first_responder".into(),
   230	    }
   231	}
   232	
   233	fn build_aggregated_text(
   234	    result: &VoteResult,
   235	    candidates: &[VoteCandidate],
   236	    buckets: &BTreeMap<String, Vec<usize>>,
   237	) -> String {
   238	    // Pick the winner's original text from the first candidate in the winning bucket.
   239	    let winner_original = buckets
   240	        .get(&result.winner)
   241	        .and_then(|indices| {
   242	            indices
   243	                .first()
   244	                .and_then(|&idx| candidates.get(idx).map(|c| c.normalised.as_str()))
   245	        })
   246	        .unwrap_or(&result.winner);
   247	
   248	    let mut lines = Vec::new();
   249	    lines.push(winner_original.to_string());
   250	    lines.push(String::new());
   251	
   252	    // Build deterministic metadata comment block
   253	    lines.push("<!-- loker: Vote aggregator metadata".into());
   254	    lines.push(format!("  winner: {}", sanitize_comment(&result.winner)));
   255	    lines.push(format!("  total_branches: {}", result.total_branches));
   256	    lines.push("  vote_counts:".into());
   257	    for (text, count) in &result.vote_counts {
   258	        lines.push(format!("    {}: {}", sanitize_comment(text), count));
   259	    }
   260	    lines.push(format!("  abstain_count: {}", result.abstain_count));

exec
/bin/zsh -lc "nl -ba src/aggregator/vote.rs | sed -n '260,520p'" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
   260	    lines.push(format!("  abstain_count: {}", result.abstain_count));
   261	    lines.push(format!("  tie_broken: {}", result.tie_broken));
   262	    lines.push(format!(
   263	        "  tie_break_rule: {}",
   264	        sanitize_comment(&result.tie_break_rule)
   265	    ));
   266	    lines.push("-->".into());
   267	    lines.push(String::new());
   268	
   269	    lines.join("\n")
   270	}
   271	
   272	/// Replace `-->` with `-- >` to prevent premature HTML comment closure.
   273	fn sanitize_comment(text: &str) -> String {
   274	    text.replace("-->", "-- >")
   275	}
   276	
   277	#[cfg(test)]
   278	mod tests {
   279	    use super::super::BranchSuccess;
   280	    use super::*;
   281	
   282	    fn success(backend_id: &str, family: &str, output: &str) -> BranchOutcome {
   283	        BranchOutcome::Success(BranchSuccess {
   284	            backend_id: backend_id.into(),
   285	            family: family.into(),
   286	            index: 1,
   287	            output: output.into(),
   288	        })
   289	    }
   290	
   291	    fn failure(backend_id: &str, family: &str, reason: &str) -> BranchOutcome {
   292	        BranchOutcome::Failure(super::super::BranchFailure {
   293	            backend_id: backend_id.into(),
   294	            family: family.into(),
   295	            index: 1,
   296	            reason: reason.into(),
   297	        })
   298	    }
   299	
   300	    fn make_config(tie_break: TieBreak) -> VoteConfig {
   301	        VoteConfig {
   302	            ballot_schema: BallotSchema::FreeText,
   303	            tie_break,
   304	            abstain_threshold: 99,
   305	        }
   306	    }
   307	
   308	    #[test]
   309	    fn free_text_clear_winner() {
   310	        let branches = vec![
   311	            success("claude", "anthropic", "yes"),
   312	            success("codex", "openai", "yes"),
   313	            success("gemini", "google", "no"),
   314	        ];
   315	        let config = make_config(TieBreak::FirstResponder);
   316	        let (artifact, result) = aggregate_vote(&branches, &config).unwrap();
   317	        assert_eq!(result.winner, "yes");
   318	        assert!(!result.tie_broken);
   319	        assert_eq!(
   320	            result.vote_counts,
   321	            vec![("yes".into(), 2), ("no".into(), 1)]
   322	        );
   323	        assert_eq!(result.abstain_count, 0);
   324	        assert!(artifact.text.contains("yes"));
   325	    }
   326	
   327	    #[test]
   328	    fn free_text_tie_first_responder() {
   329	        let branches = vec![
   330	            success("claude", "anthropic", "yes"),
   331	            success("gemini", "google", "no"),
   332	        ];
   333	        let config = make_config(TieBreak::FirstResponder);
   334	        let (_, result) = aggregate_vote(&branches, &config).unwrap();
   335	        assert_eq!(result.winner, "yes");
   336	        assert!(result.tie_broken);
   337	        // "yes" arrives first (index 0), so FirstResponder picks it
   338	    }
   339	
   340	    #[test]
   341	    fn free_text_tie_closest_family() {
   342	        let branches = vec![
   343	            success("claude", "anthropic", "a"),
   344	            success("gemini", "google", "b"),
   345	        ];
   346	        let config = make_config(TieBreak::ClosestToFamily(Family::Anthropic));
   347	        let (_, result) = aggregate_vote(&branches, &config).unwrap();
   348	        assert_eq!(result.winner, "a");
   349	        assert!(result.tie_broken);
   350	        assert_eq!(result.tie_break_rule, "closest_to_family(anthropic)");
   351	    }
   352	
   353	    #[test]
   354	    fn free_text_tie_random_deterministic() {
   355	        let branches = vec![
   356	            success("claude", "anthropic", "a"),
   357	            success("gemini", "google", "b"),
   358	        ];
   359	        let config = make_config(TieBreak::Random { seed: 42 });
   360	        let (_, result1) = aggregate_vote(&branches, &config).unwrap();
   361	        let (_, result2) = aggregate_vote(&branches, &config).unwrap();
   362	        assert_eq!(result1.winner, result2.winner);
   363	        assert!(result1.tie_broken);
   364	    }
   365	
   366	    #[test]
   367	    fn abstain_backend_error() {
   368	        let branches = vec![
   369	            success("claude", "anthropic", "yes"),
   370	            success("gemini", "google", "yes"),
   371	            failure("codex", "openai", "network timeout"),
   372	        ];
   373	        let config = make_config(TieBreak::FirstResponder);
   374	        let (_, result) = aggregate_vote(&branches, &config).unwrap();
   375	        assert_eq!(result.abstain_count, 1);
   376	        assert_eq!(result.vote_counts, vec![("yes".into(), 2)]);
   377	    }
   378	
   379	    #[test]
   380	    fn quorum_lost() {
   381	        let branches = vec![
   382	            success("claude", "anthropic", "yes"),
   383	            failure("gemini", "google", "boom"),
   384	            failure("codex", "openai", "network"),
   385	        ];
   386	        let config = VoteConfig {
   387	            ballot_schema: BallotSchema::FreeText,
   388	            tie_break: TieBreak::FirstResponder,
   389	            abstain_threshold: 1,
   390	        };
   391	        let err = aggregate_vote(&branches, &config).unwrap_err();
   392	        assert_eq!(
   393	            err,
   394	            VoteError::QuorumLost {
   395	                abstains: 2,
   396	                threshold: 1,
   397	            }
   398	        );
   399	    }
   400	
   401	    #[test]
   402	    fn empty_input() {
   403	        let branches: Vec<BranchOutcome> = vec![];
   404	        let config = make_config(TieBreak::FirstResponder);
   405	        let err = aggregate_vote(&branches, &config).unwrap_err();
   406	        assert_eq!(err, VoteError::NoCandidates);
   407	    }
   408	
   409	    #[test]
   410	    fn all_abstain() {
   411	        let branches = vec![
   412	            failure("claude", "anthropic", "boom"),
   413	            failure("gemini", "google", "boom"),
   414	            failure("codex", "openai", "boom"),
   415	        ];
   416	        let config = VoteConfig {
   417	            ballot_schema: BallotSchema::FreeText,
   418	            tie_break: TieBreak::FirstResponder,
   419	            abstain_threshold: 1,
   420	        };
   421	        let err = aggregate_vote(&branches, &config).unwrap_err();
   422	        assert_eq!(
   423	            err,
   424	            VoteError::QuorumLost {
   425	                abstains: 3,
   426	                threshold: 1,
   427	            }
   428	        );
   429	    }
   430	
   431	    #[test]
   432	    fn normalise_case() {
   433	        let branches = vec![
   434	            success("a", "anthropic", "YES"),
   435	            success("b", "openai", "yes"),
   436	            success("c", "google", "Yes"),
   437	        ];
   438	        let config = make_config(TieBreak::FirstResponder);
   439	        let (_, result) = aggregate_vote(&branches, &config).unwrap();
   440	        assert_eq!(result.vote_counts.len(), 1);
   441	        assert_eq!(result.vote_counts[0].1, 3);
   442	    }
   443	
   444	    #[test]
   445	    fn normalise_whitespace() {
   446	        let branches = vec![
   447	            success("a", "anthropic", "  yes  "),
   448	            success("b", "openai", "yes\n"),
   449	        ];
   450	        let config = make_config(TieBreak::FirstResponder);
   451	        let (_, result) = aggregate_vote(&branches, &config).unwrap();
   452	        assert_eq!(result.vote_counts.len(), 1);
   453	        assert_eq!(result.vote_counts[0].1, 2);
   454	    }
   455	
   456	    #[test]
   457	    fn closest_family_no_match_fallback() {
   458	        let branches = vec![
   459	            success("gemini", "google", "a"),
   460	            success("openai", "openai", "b"),
   461	        ];
   462	        let config = make_config(TieBreak::ClosestToFamily(Family::Anthropic));
   463	        let (_, result) = aggregate_vote(&branches, &config).unwrap();
   464	        // No match for Anthropic: falls back to FirstResponder
   465	        assert!(result.tie_broken);
   466	    }
   467	
   468	    #[test]
   469	    fn closest_family_multiple_matching_buckets() {
   470	        let branches = vec![
   471	            success("claude", "anthropic", "a"),
   472	            success("gemini", "google", "b"),
   473	            success("loker_d1_anthropic", "anthropic", "a"),
   474	        ];
   475	        let config = make_config(TieBreak::ClosestToFamily(Family::Anthropic));
   476	        let (_, result) = aggregate_vote(&branches, &config).unwrap();
   477	        // "a" has two Anthropic candidates, "b" has zero.
   478	        // ClosestToFamily matches "a" uniquely, so no need for fallback.
   479	        assert_eq!(result.winner, "a");
   480	    }
   481	
   482	    #[test]
   483	    fn closest_family_multiple_buckets_match() {
   484	        // Tie between "a" (anthropic + google) and "b" (anthropic + openai)
   485	        // When both tied buckets contain Anthropic, FirstResponder fallback
   486	        // among the matching subset should pick the one arriving first.
   487	        let branches = vec![
   488	            success("claude", "anthropic", "a"),
   489	            success("gemini", "google", "a"),
   490	            success("loker_d1_anthropic", "anthropic", "b"),
   491	            success("openai", "openai", "b"),
   492	        ];
   493	        let config = make_config(TieBreak::ClosestToFamily(Family::Anthropic));
   494	        let (_, result) = aggregate_vote(&branches, &config).unwrap();
   495	        // Both "a" and "b" have Anthropic candidates; tie -> FirstResponder
   496	        // picks the bucket whose first candidate arrived earliest.
   497	        // "a" arrives at 0, "b" arrives at 2.
   498	        assert_eq!(result.winner, "a");
   499	    }
   500	
   501	    #[test]
   502	    fn empty_ballot_counts_as_abstain() {
   503	        let branches = vec![
   504	            success("a", "anthropic", ""),
   505	            success("b", "openai", "yes"),
   506	            success("c", "google", "yes"),
   507	        ];
   508	        let config = make_config(TieBreak::FirstResponder);
   509	        let (_, result) = aggregate_vote(&branches, &config).unwrap();
   510	        assert_eq!(result.abstain_count, 1);
   511	        assert_eq!(result.winner, "yes");
   512	    }
   513	
   514	    #[test]
   515	    fn whitespace_only_ballot_counts_as_abstain() {
   516	        let branches = vec![
   517	            success("a", "anthropic", "   "),
   518	            success("b", "openai", "yes"),
   519	            success("c", "google", "yes"),
   520	        ];

exec
/bin/zsh -lc "nl -ba src/aggregator/vote.rs | sed -n '520,760p'" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
   520	        ];
   521	        let config = make_config(TieBreak::FirstResponder);
   522	        let (_, result) = aggregate_vote(&branches, &config).unwrap();
   523	        assert_eq!(result.abstain_count, 1);
   524	        assert_eq!(result.winner, "yes");
   525	    }
   526	
   527	    #[test]
   528	    fn vote_counts_sorted_descending() {
   529	        let branches = vec![
   530	            success("a", "anthropic", "yes"),
   531	            success("b", "openai", "yes"),
   532	            success("c", "google", "no"),
   533	            success("d", "zhipu", "maybe"),
   534	        ];
   535	        let config = make_config(TieBreak::FirstResponder);
   536	        let (_, result) = aggregate_vote(&branches, &config).unwrap();
   537	        assert_eq!(
   538	            result.vote_counts,
   539	            vec![
   540	                ("yes".into(), 2),
   541	                // "maybe" and "no" tie at 1; BTreeMap iteration order is alphabetical,
   542	                // and stable_sort preserves that relative order.
   543	                ("maybe".into(), 1),
   544	                ("no".into(), 1),
   545	            ]
   546	        );
   547	    }
   548	
   549	    #[test]
   550	    fn sanitize_comment_in_metadata() {
   551	        let branches = vec![success("a", "anthropic", "ok --> bad")];
   552	        let config = make_config(TieBreak::FirstResponder);
   553	        let (artifact, _) = aggregate_vote(&branches, &config).unwrap();
   554	        // The metadata should have sanitized the `-->` in the winner text.
   555	        assert!(artifact.text.contains("ok -- > bad"));
   556	        // Ensure the comment block is intact.
   557	        assert!(artifact.text.contains("-->\n"));
   558	    }
   559	
   560	    #[test]
   561	    fn vote_snapshot() {
   562	        let branches = vec![
   563	            success("claude", "anthropic", " YES "),
   564	            success("codex", "openai", "yes"),
   565	            success("gemini", "google", "no"),
   566	        ];
   567	        let config = make_config(TieBreak::FirstResponder);
   568	        let (artifact, _) = aggregate_vote(&branches, &config).unwrap();
   569	        insta::assert_snapshot!(artifact.text);
   570	    }
   571	
   572	    #[test]
   573	    fn normalise_ballot_basic() {
   574	        assert_eq!(normalise_ballot("  YES  "), "yes");
   575	        assert_eq!(normalise_ballot("Yes\n"), "yes");
   576	        assert_eq!(normalise_ballot(""), "");
   577	    }
   578	}

exec
/bin/zsh -lc 'git diff 48cb96fc8c70f3f8cd70490580c279833fef6ef9 -- src/aggregator/concat.rs' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
diff --git a/src/aggregator/concat.rs b/src/aggregator/concat.rs
index 47d998b..e96c8c0 100644
--- a/src/aggregator/concat.rs
+++ b/src/aggregator/concat.rs
@@ -1,5 +1,6 @@
 use std::sync::Arc;
 
+use crate::aggregator::VoteConfig;
 use crate::backend::Backend;
 use crate::strategy::PhaseContext;
 
@@ -39,6 +40,9 @@ pub enum Aggregator {
         prompt_template: String,
         require_judge_different_family: bool,
     },
+    Vote {
+        config: VoteConfig,
+    },
 }
 
 impl Aggregator {
@@ -62,6 +66,11 @@ impl Aggregator {
         }
     }
 
+    /// Build a Vote aggregator with the provided configuration.
+    pub fn vote(config: VoteConfig) -> Self {
+        Self::Vote { config }
+    }
+
     /// Build an AnyFail aggregator (no configuration needed).
     pub fn any_fail() -> Self {
         Self::AnyFail
@@ -73,6 +82,7 @@ impl Aggregator {
             Self::Concat { .. } => crate::strategy::Aggregator::Concat,
             Self::AnyFail => crate::strategy::Aggregator::AnyFail,
             Self::LLMJudge { .. } => crate::strategy::Aggregator::LLMJudge,
+            Self::Vote { .. } => crate::strategy::Aggregator::Vote,
         }
     }
 
@@ -91,6 +101,9 @@ impl Aggregator {
             Self::LLMJudge { .. } => Err(AggregatorError::Unsupported(
                 "LLMJudge requires async backend access; use aggregate_llm_judge()".into(),
             )),
+            Self::Vote { .. } => Err(AggregatorError::Unsupported(
+                "Vote is evaluated inline by ParallelFanOut, not via aggregate()".into(),
+            )),
         }
     }
 }
@@ -462,6 +475,19 @@ mod tests {
         );
     }
 
+    #[test]
+    fn vote_kind_maps_to_strategy_label() {
+        assert_eq!(
+            Aggregator::vote(crate::aggregator::VoteConfig {
+                ballot_schema: crate::aggregator::BallotSchema::FreeText,
+                tie_break: crate::aggregator::TieBreak::FirstResponder,
+                abstain_threshold: 0,
+            })
+            .kind(),
+            crate::strategy::Aggregator::Vote
+        );
+    }
+
     #[test]
     fn concat_mixed_success_failure_snapshot() {
         let artifact = Aggregator::concat("## {index}. {backend_id} ({family})")

exec
/bin/zsh -lc 'git diff 48cb96fc8c70f3f8cd70490580c279833fef6ef9 -- src/aggregator/mod.rs' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
diff --git a/src/aggregator/mod.rs b/src/aggregator/mod.rs
index e946160..f65ec9c 100644
--- a/src/aggregator/mod.rs
+++ b/src/aggregator/mod.rs
@@ -8,6 +8,7 @@
 
 mod concat;
 mod llm_judge;
+mod vote;
 
 pub use concat::{
     AggregateInput, AggregatedArtifact, Aggregator, AggregatorError, BranchFailure, BranchOutcome,
@@ -19,6 +20,11 @@ pub use llm_judge::{
     render_ballot_prompt, Ballot, Candidate, LLMJudgeError,
 };
 
+pub use vote::{
+    aggregate_vote, normalise_ballot, BallotSchema, TieBreak, VoteCandidate, VoteConfig, VoteError,
+    VoteResult,
+};
+
 use serde_json::Value;
 
 #[non_exhaustive]

exec
/bin/zsh -lc 'cargo test aggregator::vote::tests::free_text_clear_winner -- --nocapture' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 830ms:
warning: unused imports: `BranchFailure` and `BranchSuccess`
   --> src/strategy/parallel_fanout.rs:475:38
    |
475 |         AnyFailReason, BallotSchema, BranchFailure, BranchSuccess, TieBreak, VoteConfig,
    |                                      ^^^^^^^^^^^^^  ^^^^^^^^^^^^^
    |
    = note: `#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default

warning: `loker` (lib test) generated 1 warning (run `cargo fix --lib -p loker --tests` to apply 1 suggestion)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.45s
     Running unittests src/lib.rs (target/debug/deps/loker-e0bbd5653d27b3fc)

running 1 test
test aggregator::vote::tests::free_text_clear_winner ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 612 filtered out; finished in 0.01s

     Running unittests src/main.rs (target/debug/deps/loker-be6d1c9068a5690d)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 524 filtered out; finished in 0.00s

     Running tests/aggregator_llm_judge.rs (target/debug/deps/aggregator_llm_judge-65bdce833c2f7139)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.00s

     Running tests/integration.rs (target/debug/deps/integration-fb6d6e97ae604984)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.00s

     Running tests/schema_validation.rs (target/debug/deps/schema_validation-490a24c6bc49f113)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s

     Running tests/strategy_escalating_retry.rs (target/debug/deps/strategy_escalating_retry-668bdb2f5492a863)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 15 filtered out; finished in 0.00s

     Running tests/strategy_parallel_fanout.rs (target/debug/deps/strategy_parallel_fanout-6f4b2549020db6e4)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.00s

     Running tests/strategy_single_model.rs (target/debug/deps/strategy_single_model-4f6ef3d0d771ae3a)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.00s

     Running tests/tensorzero_backend.rs (target/debug/deps/tensorzero_backend-b7b7a49b4a764a7a)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.00s

     Running tests/tensorzero_integration.rs (target/debug/deps/tensorzero_integration-73c1ffd0efc539fe)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.00s


exec
/bin/zsh -lc 'cargo test' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 exited 101 in 369ms:
warning: unused imports: `BranchFailure` and `BranchSuccess`
   --> src/strategy/parallel_fanout.rs:475:38
    |
475 |         AnyFailReason, BallotSchema, BranchFailure, BranchSuccess, TieBreak, VoteConfig,
    |                                      ^^^^^^^^^^^^^  ^^^^^^^^^^^^^
    |
    = note: `#[warn(unused_imports)]` (part of `#[warn(unused)]`) on by default

warning: `loker` (lib test) generated 1 warning (run `cargo fix --lib -p loker --tests` to apply 1 suggestion)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.08s
     Running unittests src/lib.rs (target/debug/deps/loker-e0bbd5653d27b3fc)

running 613 tests
test aggregator::concat::tests::vote_kind_maps_to_strategy_label ... ok
test aggregator::llm_judge::tests::llm_judge_family_diverse_ok ... ok
test aggregator::llm_judge::tests::llm_judge_family_overlap_opt_out_warns ... ok
test aggregator::concat::tests::llm_judge_kind_maps_to_strategy_label ... ok
test aggregator::concat::tests::concat_kind_maps_to_strategy_label ... ok
test aggregator::llm_judge::tests::llm_judge_family_overlap_blocks ... ok
test aggregator::llm_judge::tests::llm_judge_parse_markdown_fenced_ballot ... ok
test aggregator::llm_judge::tests::llm_judge_parse_out_of_bounds_index ... ok
test aggregator::llm_judge::tests::llm_judge_parse_out_of_bounds_index_clamped ... ok
test aggregator::llm_judge::tests::llm_judge_parse_valid_ballot ... ok
test aggregator::llm_judge::tests::llm_judge_parse_within_bounds_index_clamped ... ok
test aggregator::llm_judge::tests::llm_judge_parse_zero_candidates_index ... ok
test aggregator::llm_judge::tests::llm_judge_parse_negative_chosen_index ... ok
test aggregator::llm_judge::tests::llm_judge_parse_missing_chosen_index ... ok
test aggregator::llm_judge::tests::llm_judge_parse_missing_reason ... ok
test aggregator::llm_judge::tests::llm_judge_parse_malformed_json ... ok
test aggregator::tests::extra_keys_ok ... ok
test aggregator::tests::markdown_fenced_json ... ok
test aggregator::tests::markdown_fenced_fail ... ok
test aggregator::tests::missing_pass ... ok
test aggregator::tests::pass_false ... ok
test aggregator::tests::pass_true ... ok
test aggregator::vote::tests::all_abstain ... ok
test aggregator::tests::wrong_pass_type ... ok
test aggregator::tests::empty_text ... ok
test aggregator::vote::tests::abstain_backend_error ... ok
test aggregator::vote::tests::closest_family_multiple_matching_buckets ... ok
test aggregator::vote::tests::empty_ballot_counts_as_abstain ... ok
test aggregator::vote::tests::empty_input ... ok
test aggregator::vote::tests::free_text_clear_winner ... ok
test aggregator::vote::tests::free_text_tie_first_responder ... ok
test aggregator::vote::tests::free_text_tie_closest_family ... ok
test aggregator::vote::tests::closest_family_multiple_buckets_match ... ok
test aggregator::vote::tests::closest_family_no_match_fallback ... ok
test aggregator::vote::tests::normalise_ballot_basic ... ok
test aggregator::vote::tests::normalise_case ... ok
test aggregator::vote::tests::normalise_whitespace ... ok
test aggregator::vote::tests::quorum_lost ... ok
test aggregator::vote::tests::sanitize_comment_in_metadata ... ok
test aggregator::vote::tests::vote_counts_sorted_descending ... ok
test aggregator::vote::tests::whitespace_only_ballot_counts_as_abstain ... ok
test aggregator::vote::tests::free_text_tie_random_deterministic ... ok
test apply_verify::diff_applier::tests::test_apply_empty_edits ... ok
test apply_verify::diff_applier::tests::test_apply_empty_file_path_is_invalid_edit ... ok
test apply_verify::diff_applier::tests::test_apply_file_not_found ... ok
test apply_verify::diff_applier::tests::test_apply_empty_old_in_find_replace_is_invalid ... ok
test apply_verify::diff_applier::tests::test_apply_ambiguous_match ... ok
test apply_verify::diff_applier::tests::test_apply_full_file_overwrite ... ok
test apply_verify::diff_applier::tests::test_apply_full_file_create_new ... ok
test apply_verify::diff_applier::tests::test_apply_json_single_file ... ok
test apply_verify::diff_applier::tests::test_apply_old_text_not_found ... ok
test aggregator::concat::tests::concat_empty_input_returns_sentinel ... ok
test aggregator::concat::tests::concat_does_not_reexpand_placeholders_inside_metadata ... ok
test aggregator::concat::tests::concat_preserves_unknown_placeholders ... ok
test aggregator::concat::tests::concat_preserves_braced_unknown_expressions_containing_known_tokens ... ok
test aggregator::concat::tests::concat_whitespace_only_success_output_keeps_newline_invariants ... ok
test aggregator::concat::tests::concat_renders_success_sections_in_input_order ... ok
test apply_verify::edit_parser::tests::test_detect_diff ... ok
test apply_verify::edit_parser::tests::test_detect_full_file ... ok
test apply_verify::edit_parser::tests::test_detect_json_array ... ok
test apply_verify::edit_parser::tests::test_detect_json_object ... ok
test apply_verify::edit_parser::tests::test_detect_with_lang_hint_diff ... ok
test apply_verify::edit_parser::tests::test_detect_with_lang_hint_json ... ok
test apply_verify::diff_applier::tests::test_apply_partial_failure ... ok
test apply_verify::diff_applier::tests::test_apply_rejects_absolute_path ... ok
test apply_verify::diff_applier::tests::test_apply_rejects_path_traversal ... ok
test apply_verify::edit_parser::tests::test_diff_no_hunks ... ok
test aggregator::concat::tests::concat_escapes_multiline_failure_reason ... ok
test aggregator::concat::tests::concat_counts_success_and_failure ... ok
test aggregator::concat::tests::concat_normalizes_crlf_failure_reason ... ok
test apply_verify::diff_applier::tests::test_apply_multi_file_success ... ok
test apply_verify::edit_parser::tests::test_empty_input ... ok
test apply_verify::edit_parser::tests::test_full_file_empty_path ... ok
test apply_verify::diff_applier::tests::test_apply_unified_diff_multi_hunk_fails ... ok
test apply_verify::edit_parser::tests::test_full_file_no_path ... ok
test aggregator::llm_judge::tests::llm_judge_prompt_includes_phase_name ... ok
test aggregator::llm_judge::tests::llm_judge_prompt_renders_candidates ... ok
test apply_verify::edit_parser::tests::test_input_too_large ... ok
test apply_verify::diff_applier::tests::test_apply_unified_diff_single_hunk ... ok
test apply_verify::edit_parser::tests::test_diff_no_newline_marker ... ok
test apply_verify::edit_parser::tests::test_crlf_normalization ... ok
test apply_verify::edit_parser::tests::test_diff_multi_file ... ok
test apply_verify::edit_parser::tests::test_diff_context_lines ... ok
test apply_verify::edit_parser::tests::test_diff_single_file ... ok
test apply_verify::edit_parser::tests::test_diff_strips_ab_prefix ... ok
test apply_verify::edit_parser::tests::test_malformed_diff ... ok
test apply_verify::edit_parser::tests::test_markdown_diff_block ... ok
test apply_verify::edit_parser::tests::test_whitespace_only_input ... ok
test apply_verify::edit_parser::tests::test_json_empty_edits ... ok
test apply_verify::edit_parser::tests::test_full_file ... ok
test apply_verify::edit_parser::tests::test_full_file_with_dash_header ... ok
test apply_verify::edit_parser::tests::test_markdown_backticks_in_content ... ok
test apply_verify::edit_parser::tests::test_json_bare_array ... ok
test apply_verify::edit_parser::tests::test_markdown_generic_block ... ok
test apply_verify::edit_parser::tests::test_json_trailing_newlines_normalized ... ok
test apply_verify::edit_parser::tests::test_json_control_chars ... ok
test apply_verify::edit_parser::tests::test_markdown_json_block ... ok
test apply_verify::edit_parser::tests::test_json_malformed ... ok
test apply_verify::edit_parser::tests::test_json_with_message_field ... ok
test apply_verify::edit_parser::tests::test_json_agentic_output ... ok
test apply_verify::rollback::tests::test_is_fully_restored_false ... ok
test apply_verify::rollback::tests::test_is_fully_restored_true ... ok
test apply_verify::retry_loop::tests::test_parse_error_stop ... ok
test apply_verify::retry_loop::tests::test_apply_partial_failure_rolls_back ... ok
test apply_verify::rollback::tests::test_rollback_delete_tolerates_already_missing ... ok
test apply_verify::rollback::tests::test_rollback_continues_on_failure ... ok
test apply_verify::rollback::tests::test_rollback_empty_result_is_noop ... ok
test apply_verify::rollback::tests::test_rollback_deletes_new_file ... ok
test apply_verify::rollback::tests::test_rollback_single_file ... ok
test apply_verify::rollback::tests::test_rollback_mixed_restore_and_delete ... ok
test apply_verify::rollback::tests::test_rollback_reverse_order ... ok
test aggregator::vote::tests::vote_snapshot ... ok
test aggregator::concat::tests::concat_mixed_success_failure_snapshot ... ok
test apply_verify::verification::tests::test_verify_captures_stdout ... ok
test apply_verify::verification::tests::test_verify_captures_stderr ... ok
test apply_verify::verification::tests::test_verify_captures_both_streams ... ok
test apply_verify::retry_loop::tests::test_max_retries_zero_runs_once ... ok
test apply_verify::retry_loop::tests::test_requester_error_surfaced ... ok
test apply_verify::retry_loop::tests::test_success_first_attempt ... ok
test apply_verify::retry_loop::tests::test_verify_failure_triggers_rollback ... ok
test apply_verify::retry_loop::tests::test_parse_error_on_last_retry_exits ... ok
test backend::claude::tests::capabilities_match_current_wiring ... ok
test backend::claude::tests::test_claude_response_deserialize_with_usage ... ok
test backend::claude::tests::test_claude_response_deserialize_without_usage ... ok
test backend::codex::tests::capabilities_match_current_wiring ... ok
test backend::gemini::tests::capabilities_match_current_wiring ... ok
test backend::genai_error::tests::classify_404_body_detects_unknown_function_fixture ... ok
test backend::genai_error::tests::classify_5xx_body_detects_anthropic_auth_fixture ... ok
test apply_verify::retry_loop::tests::test_apply_error_triggers_rollback_and_retry ... ok
test apply_verify::retry_loop::tests::test_parse_error_retries ... ok
test backend::genai_error::tests::classify_5xx_body_detects_rate_limit_signature ... ok
test backend::genai_error::tests::contains_status_code_handles_punctuation_boundaries ... ok
test backend::genai_error::tests::classify_5xx_body_returns_none_for_generic_5xx ... ok
test backend::genai_error::tests::map_status_401_to_auth ... ok
test backend::genai_error::tests::map_status_403_to_auth ... ok
test backend::genai_error::tests::map_status_500_to_network_retryable ... ok
test backend::genai_error::tests::map_status_404_other_to_execution_failed ... ok
test backend::genai_error::tests::map_status_404_unknown_function_to_config ... ok
test backend::genai_error::tests::map_status_429_to_rate_limit_retryable ... ok
test backend::genai_error::tests::map_status_502_generic_to_network_retryable ... ok
test backend::genai_error::tests::map_status_502_upstream_rate_limit_to_rate_limit_retryable ... ok
test backend::genai_error::tests::map_status_502_upstream_auth_to_auth_not_retryable ... ok
test backend::genai_error::tests::map_status_503_to_network_retryable ... ok
test backend::genai_error::tests::map_status_unknown_to_execution_failed ... ok
test backend::ollama::tests::test_ollama_response_deserialize_partial_counts ... ok
test backend::ollama::tests::test_ollama_response_deserialize_with_counts ... ok
test backend::retry::tests::test_get_delay_attempt_zero_is_zero ... ok
test backend::retry::tests::test_get_delay_clamped_at_max ... ok
test backend::retry::tests::test_get_delay_grows_exponentially ... ok
test apply_verify::verification::tests::test_verify_failure_exit_code ... ok
test backend::retry::tests::test_retry_executor_does_not_retry_non_retryable ... ok
test backend::ollama::tests::test_ollama_response_deserialize_without_model ... ok
test backend::tensorzero::tests::canonicalize_wire_model_strips_to_canonical_on_wire ... ok
test backend::tensorzero::tests::capabilities_match_current_wiring ... ok
test apply_verify::retry_loop::tests::test_integration_end_to_end ... ok
test apply_verify::verification::tests::test_verify_invalid_command_exits_127 ... ok
test backend::tensorzero::tests::maps_401_to_auth_not_retryable ... FAILED
test backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable ... FAILED
test backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime ... FAILED
test backend::tensorzero::tests::maps_429_to_rate_limit_retryable ... FAILED
test backend::tensorzero::tests::maps_500_to_retryable_error ... FAILED
test backend::tensorzero::tests::maps_502_generic_to_network_retryable ... FAILED
test backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable ... FAILED
test backend::tensorzero::tests::maps_malformed_json_to_parse_error ... FAILED
test backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable ... FAILED
test backend::tensorzero::tests::normalize_endpoint_appends_when_missing ... ok
test backend::tensorzero::tests::normalize_endpoint_does_not_double_suffix ... ok
test backend::tensorzero::tests::maps_request_timeout_to_timeout_error ... FAILED
test backend::tensorzero::tests::returns_text_on_200_success ... FAILED
test backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model ... FAILED
test backend::tests::backend_capabilities_none_is_all_false ... ok
test backend::tests::capabilities_for_name_matches_static_expectations ... ok
test backend::tests::capabilities_for_name_unknown_returns_none ... ok
test backend::tests::default_capabilities_are_none ... ok
test backend::retry::tests::test_retry_exhausted ... ok
test backend::tests::test_backend_error_display ... ok
test backend::retry::tests::test_retry_success_after_failures ... ok
test backend::tests::test_backend_error_not_retryable ... ok
test backend::tests::test_backend_error_retryable ... ok
test backend::tests::test_backend_error_from_anyhow ... ok
test backend::tests::test_query_output_from_process_empty_stderr_normalized ... ok
test backend::tests::test_query_output_from_process_empty_stdout ... ok
test backend::tests::test_query_output_from_process_populates_backend_and_duration ... ok
test backend::tests::test_query_output_from_process_with_stderr ... ok
test backend::tests::test_query_output_from_text ... ok
test backend::tests::test_query_output_from_text_populates_backend_and_duration ... ok
test backend::tests::test_query_output_with_model_none ... ok
test backend::tests::test_query_output_with_model_some ... ok
test backend::tests::test_query_output_with_structured_none ... ok
test backend::tests::test_query_output_with_usage_none ... ok
test backend::tests::test_query_output_with_structured_some ... ok
test backend::tests::test_query_output_with_usage_some ... ok
test backend::tests::test_token_usage_default_zero ... ok
test backend::tests::test_token_usage_new_computes_total ... ok
test backend::tests::test_token_usage_new_saturates_on_overflow ... ok
test backend::tests::with_elapsed_is_idempotent_on_repeated_calls ... ok
test backend::tests::test_token_usage_saturating_add ... ok
test backend::tests::with_elapsed_is_noop_on_non_timeout_variants ... ok
test backend::tests::with_elapsed_overrides_timeout_elapsed_ms ... ok
test cache::tests::test_cache_disabled ... ok
test apply_verify::verification::tests::test_verify_success ... ok
test cache::tests::test_cache_key_deterministic ... ok
test cache::tests::test_cache_key_different_prompts ... ok
test cache::tests::test_cache_key_different_backends ... ok
test cache::tests::test_cache_warnings_deduplicated ... ok
test cache::tests::test_cache_warnings_on_parse_failure ... ok
test config::tests::test_command_wrapper_default_none ... ok
test apply_verify::verification::tests::test_verify_uses_passed_cwd ... ok
test apply_verify::retry_loop::tests::test_max_retries_exhausted ... ok
test config::tests::test_conductor_defaults ... ok
test config::tests::test_claude_backend_defaults ... ok
test config::tests::test_codex_backend_defaults ... ok
test apply_verify::verification::tests::test_verify_output_truncated ... ok
test config::tests::test_backend_config_defaults ... ok
test config::tests::test_deep_merge_boolean_override ... ok
test config::tests::test_command_wrapper_docker_example ... ok
test config::tests::test_conductor_custom_config ... ok
test config::tests::test_command_wrapper_config ... ok
test config::tests::test_deep_merge_empty_overlay ... ok
test config::tests::test_deep_merge_hashmap_add ... ok
test config::tests::test_default_config ... ok
test config::tests::test_gemini_backend_defaults ... ok
test config::tests::test_deep_merge_hashmap_override ... ok
test config::tests::test_deep_merge_partial_config ... ok
test config::tests::test_deep_merge_scalar_override ... ok
test config::tests::test_deny_unknown_fields ... ok
test config::tests::test_hunt_task_defaults ... ok
test config::tests::test_config_serialization_roundtrip ... ok
test config::tests::test_deep_merge_vec_replace ... ok
test config::tests::test_parse_custom_backend ... ok
test config::tests::test_parse_minimal_config ... ok
test config::tests::test_parse_custom_task ... ok
test config::tests::test_tensorzero_missing_endpoint_fails ... ok
test config::tests::test_load_config_from_paths_no_files ... ok
test apply_verify::retry_loop::tests::test_success_on_retry_after_verify_failure ... ok
test config::tests::test_load_config_from_paths_project_only ... ok
test consensus::tests::test_majority_vote_clear_winner ... ok
test consensus::tests::test_majority_vote_empty ... ok
test config::tests::test_load_config_from_paths_explicit_bypasses ... ok
test consensus::tests::test_majority_vote_tie_first_wins ... ok
test config::tests::test_tensorzero_config_serialization_roundtrip ... ok
test consensus::tests::test_weighted_vote ... ok
test config::tests::test_tensorzero_zero_timeout_fails ... ok
test consensus::tests::test_weighted_vote_clear_winner ... ok
test consensus::tests::test_whitespace_normalization ... ok
test config::tests::test_tensorzero_to_backend_opts_resolves_env ... ok
test family::tests::aggregator_rejected_display ... ok
test family::tests::as_str_openai ... ok
test family::tests::as_str_other ... ok
test family::tests::display_anthropic ... ok
test family::tests::display_other ... ok
test config::tests::test_tensorzero_invalid_url_fails ... ok
test family::tests::enforce_all_anthropic_rejected ... ok
test family::tests::enforce_distinct_other_ok ... ok
test family::tests::enforce_empty_slice_ok ... ok
test family::tests::enforce_mixed_families_ok ... ok
test family::tests::enforce_cross_family_deterministic ... ok
test family::tests::enforce_same_other_rejected ... ok
test family::tests::enforce_single_backend_ok ... ok
test family::tests::enforce_three_same_family ... ok
test family::tests::enforce_two_distinct_others_ok ... ok
test family::tests::family_of_bedrock ... ok
test family::tests::family_of_claude ... ok
test family::tests::family_of_codex ... ok
test family::tests::family_of_empty_string ... ok
test family::tests::family_of_gemini ... ok
test family::tests::family_of_loker_no_suffix ... ok
test family::tests::family_of_loker_prefix_anthropic ... ok
test family::tests::family_of_loker_prefix_gemini ... ok
test family::tests::family_of_loker_prefix_google ... ok
test family::tests::family_of_loker_prefix_local ... ok
test family::tests::family_of_loker_prefix_ollama ... ok
test family::tests::family_of_loker_prefix_openai ... ok
test family::tests::family_of_loker_zhipu_suffix ... ok
test family::tests::family_of_ollama ... ok
test family::tests::family_of_openai ... ok
test family::tests::family_of_tensorzero ... ok
test context::tests::test_no_context ... ok
test family::tests::family_of_tensorzero_function_name ... ok
test family::tests::family_of_tensorzero_slash_only ... ok
test family::tests::family_of_tensorzero_unknown_suffix ... ok
test family::tests::family_of_tensorzero_zhipu_suffix ... ok
test family::tests::family_of_unknown ... ok
test family::tests::family_of_zhipu ... ok
test family::tests::judge_unavailable_display ... ok
test family::tests::quorum_lost_display ... ok
test role::tests::test_resolution_is_empty ... ok
test role::tests::test_backend_filtering ... ok
test role::tests::test_resolution_builder ... ok
test role::tests::test_role_config_new ... ok
test role::tests::test_role_resolver_default_team ... ok
test role::tests::test_role_resolver_no_backends_available ... ok
test role::tests::test_role_config_serialization ... ok
test role::tests::test_role_resolver_resolve_global_role ... ok
test role::tests::test_role_resolver_role_not_found ... ok
test context::tests::test_detect_rails_with_goldiloader ... ok
test role::tests::test_role_resolver_team_can_define_custom_role ... ok
test role::tests::test_role_resolver_team_override ... ok
test role::tests::test_routing_strategy_default_is_fallback ... ok
test role::tests::test_role_resolver_team_override_takes_precedence ... ok
test role::tests::test_team_config_default ... ok
test role::tests::test_role_resolution_error_display ... ok
test role::tests::test_valid_parallel_config ... ok
test git_agent::tests::test_is_initialized_false_for_nonexistent ... ok
test role::tests::test_validation_parallel_min_success_exceeds_backends ... ok
test role::tests::test_validation_parallel_min_success_too_low ... ok
test role::tests::test_validation_unknown_backend ... ok
test role::tests::test_team_config_serialization ... ok
test strategy::escalating_retry::tests::config_default_false ... ok
test strategy::escalating_retry::tests::config_round_trip_false ... ok
test strategy::escalating_retry::tests::config_round_trip_true ... ok
test context::tests::test_detect_typescript ... ok
test config::tests::test_load_config_from_paths_user_parse_error ... ok
test config::tests::test_load_config_from_paths_three_layers ... ok
test git_agent::tests::test_is_available_returns_bool ... ok
test backend::tensorzero::tests::name_is_tensorzero ... FAILED
test apply_verify::retry_loop::tests::test_attempt_records ... ok
test backend::ollama::tests::capabilities_match_current_wiring ... FAILED
test strategy::escalating_retry::tests::truncate_exact_boundary ... ok
test strategy::escalating_retry::tests::truncate_multibyte_safe ... ok
test strategy::escalating_retry::tests::truncate_no_op_when_under_budget ... ok
test strategy::escalating_retry::tests::truncate_with_suffix_fits_within_budget ... ok
test strategy::future_variant_compiles::stub_fan_out_implements_strategy ... ok
test strategy::parallel_fanout::tests::any_fail_all_pass ... ok
test strategy::parallel_fanout::tests::any_fail_all_fail ... ok
test strategy::parallel_fanout::tests::any_fail_backend_error_treated_as_failure ... ok
test strategy::parallel_fanout::tests::any_fail_empty_query_text ... ok
test strategy::parallel_fanout::tests::any_fail_markdown_fenced_fail ... ok
test strategy::parallel_fanout::tests::any_fail_first_fails ... ok
test strategy::parallel_fanout::tests::any_fail_markdown_fenced_json ... ok
test strategy::parallel_fanout::tests::any_fail_missing_pass_field ... ok
test strategy::escalating_retry::tests::redaction_bearer_token ... ok
test strategy::escalating_retry::tests::envelope_verify_reason_only_when_no_response ... ok
test strategy::escalating_retry::tests::envelope_backend_error_shows_null_response ... ok
test strategy::escalating_retry::tests::redaction_api_key_value ... ok
test strategy::escalating_retry::tests::redaction_aws_key ... ok
test strategy::escalating_retry::tests::envelope_under_budget_no_truncation ... ok
test strategy::escalating_retry::tests::envelope_hard_caps_when_body_alone_exceeds_budget ... ok
test strategy::escalating_retry::tests::redaction_does_not_false_positive_short_text ... ok
test strategy::escalating_retry::tests::redaction_long_blob_heuristic ... ok
test strategy::parallel_fanout::tests::empty_targets_yields_no_backends ... ok
test strategy::parallel_fanout::tests::prompt_render_failure_no_dispatch ... ok
test strategy::parallel_fanout::tests::any_fail_valid_json_extra_keys ... ok
test strategy::parallel_fanout::tests::happy_path_all_succeed ... ok
test strategy::parallel_fanout::tests::floor_violation ... ok
test strategy::parallel_fanout::tests::backend_not_found ... ok
test template::context::tests::test_arg_out_of_bounds ... ok
test template::context::tests::test_arg_zero_undefined ... ok
test template::context::tests::test_arg_access ... ok
test template::context::tests::test_env_lookup ... ok
test template::context::tests::test_env_missing ... ok
test strategy::parallel_fanout::tests::one_fails_floor_still_met ... ok
test template::context::tests::test_loop_vars_object_item ... ok
test strategy::parallel_fanout::tests::vote_quorum_lost ... ok
test template::context::tests::test_loop_vars_string_item ... ok
test template::context::tests::test_loop_vars_preserve_existing_namespaces ... ok
test template::context::tests::test_step_field_fallback_no_parsed_output ... ok
test template::context::tests::test_step_output ... ok
test template::context::tests::test_step_field_with_parsed_output ... ok
test template::context::tests::test_step_success_false ... ok
test template::context::tests::test_step_success_true ... ok
test template::context::tests::test_workflow_backends ... ok
test template::filters::tests::test_default_val_defined ... ok
test template::filters::tests::test_default_val_empty_string ... ok
test template::context::tests::test_workflow_backends_empty ... ok
test template::filters::tests::test_default_val_undefined ... ok
test template::filters::tests::test_first_empty ... ok
test template::filters::tests::test_first_normal ... ok
test template::filters::tests::test_first_single ... ok
test template::filters::tests::test_join_default_separator ... ok
test template::filters::tests::test_join_empty ... ok
test template::filters::tests::test_join_with_separator ... ok
test template::filters::tests::test_json_encode_number ... ok
test template::filters::tests::test_json_encode_nested ... ok
test template::filters::tests::test_json_encode_string ... ok
test template::filters::tests::test_last_empty ... ok
test template::filters::tests::test_last_normal ... ok
test strategy::escalating_retry::tests::envelope_over_budget_truncates_excerpt ... ok
test template::filters::tests::test_last_single ... ok
test template::filters::tests::test_lines_empty ... ok
test template::filters::tests::test_lines_multiline ... ok
test template::filters::tests::test_lines_single ... ok
test template::filters::tests::test_shell_escape_basic ... ok
test template::filters::tests::test_shell_escape_backticks_and_dollar ... ok
test template::filters::tests::test_shell_escape_injection ... ok
test template::filters::tests::test_shell_escape_newlines ... ok
test template::filters::tests::test_shell_escape_null_bytes ... ok
test template::filters::tests::test_shell_escape_single_quotes ... ok
test template::filters::tests::test_shell_escape_unicode ... ok
test strategy::parallel_fanout::tests::vote_success ... ok
test template::filters::tests::test_trim_already_trimmed ... ok
test strategy::parallel_fanout::tests::any_fail_non_deterministic_offender ... ok
test template::filters::tests::test_trim_newlines ... ok
test template::filters::tests::test_trim_whitespace ... ok
test template::tests::test_combined_env_arg_step ... ok
test template::tests::test_eval_expression_falsy ... ok
test template::tests::test_eval_expression_truthy ... ok
test template::tests::test_eval_expression_undefined ... ok
test template::tests::test_parse_error ... ok
test template::tests::test_no_reexpansion_of_braces_in_output ... ok
test utils::tests::test_backend_error_kind_from_typed ... ok
test utils::tests::test_classify_auth_401 ... ok
test template::tests::test_undefined_variable ... ok
test utils::tests::test_classify_auth_invalid_key ... ok
test template::tests::test_render_mixed ... ok
test utils::tests::test_classify_capacity_exhausted ... ok
test utils::tests::test_classify_network_refused ... ok
test utils::tests::test_classify_not_installed ... ok
test utils::tests::test_classify_rate_limit_429 ... ok
test utils::tests::test_classify_rate_limit_quota ... ok
test utils::tests::test_classify_resource_exhausted ... ok
test utils::tests::test_classify_unknown ... ok
test utils::tests::test_summarize_capacity ... ok
test utils::tests::test_summarize_rate_limit ... ok
test utils::tests::test_summarize_typed_backend_error ... ok
test utils::tests::test_truncate_exact_length ... ok
test utils::tests::test_truncate_long_string ... ok
test utils::tests::test_summarize_unknown_truncates ... ok
test utils::tests::test_truncate_short_string ... ok
test utils::tests::test_truncate_unicode ... ok
test utils::tests::test_truncate_utf8_ascii ... ok
test utils::tests::test_truncate_utf8_empty_string ... ok
test utils::tests::test_truncate_utf8_exact_boundary ... ok
test utils::tests::test_truncate_utf8_multibyte_boundary ... ok
test utils::tests::test_truncate_utf8_within_limit ... ok
test utils::tests::test_truncate_utf8_zero_cap ... ok
test workflow::tests::required_capabilities_returns_empty_for_plain_step ... ok
test workflow::tests::required_capabilities_returns_file_edit_for_apply_edits ... ok
test strategy::parallel_fanout::tests::vote_tie_random_deterministic ... ok
test workflow::tests::test_apply_lenient_mode_non_empty_passes_with_cleaned_output ... ok
test workflow::tests::test_apply_lenient_mode_preserves_internal_whitespace ... ok
test workflow::tests::test_apply_parse_error_policy_explicit_fail_matches_default ... ok
test workflow::tests::test_apply_parse_error_policy_pass_succeeds_without_output ... ok
test workflow::tests::test_apply_parse_error_policy_default_fails ... ok
test workflow::tests::test_apply_parse_error_policy_skip_drops_validation ... ok
test workflow::tests::test_apply_lenient_mode_whitespace_only_fails ... ok
test workflow::tests::test_apply_parse_error_policy_unknown_value_falls_back_to_fail ... ok
test workflow::tests::test_apply_lenient_mode_empty_response_fails ... ok
test workflow::tests::test_build_apply_fix_prompt_includes_partial_paths ... ok
test workflow::tests::test_build_parse_fix_prompt_contains_previous_raw ... ok
test strategy::parallel_fanout::tests::any_fail_wrong_pass_type ... ok
test workflow::tests::test_build_verify_fix_prompt_with_exit_code ... ok
test workflow::tests::test_build_verify_fix_prompt_with_timeout_uses_timeout_string ... ok
test workflow::tests::test_apply_once_parse_error_returns_err ... ok
test workflow::tests::test_continue_on_error_toml_parsing ... ok
test workflow::tests::test_apply_once_apply_error_rolls_back ... ok
test workflow::tests::test_apply_once_success_without_format ... ok
test workflow::tests::test_extract_json_field_bool ... ok
test workflow::tests::test_extract_json_field_multiline ... ok
test workflow::tests::test_duplicate_step_names_error ... ok
test workflow::tests::test_extract_json_field_not_found ... ok
test workflow::tests::test_extract_json_field_number ... ok
test workflow::tests::test_extract_json_field_string ... ok
test strategy::parallel_fanout::tests::any_fail_mid_list_fails ... ok
test workflow::tests::test_extract_json_from_markdown_block ... ok
test workflow::tests::test_extract_json_from_plain_block ... ok
test workflow::tests::test_extract_json_raw ... ok
test workflow::tests::test_extract_json_with_text_before ... ok
test workflow::tests::test_extract_json_with_literal_newlines ... ok
test workflow::tests::test_find_closing_fence ... ok
test workflow::tests::test_group_by_depth_forward_declared_dependency ... ok
test workflow::tests::test_heuristic_contains_double_quotes ... ok
test workflow::tests::test_heuristic_contains_empty_string_always_passes ... ok
test workflow::tests::test_heuristic_contains_fail ... ok
test workflow::tests::test_heuristic_contains_pass ... ok
test workflow::tests::test_heuristic_contains_single_quote_char ... ok
test workflow::tests::test_heuristic_contains_special_chars ... ok
test workflow::tests::test_heuristic_empty_check_string ... ok
test workflow::tests::test_heuristic_min_length_fail ... ok
test workflow::tests::test_heuristic_min_length_invalid_arg ... ok
test workflow::tests::test_heuristic_min_length_pass ... ok
test workflow::tests::test_heuristic_min_length_unicode ... ok
test workflow::tests::test_for_each_parsed_output_not_array ... ok
test workflow::tests::test_for_each_with_parsed_output ... ok
test workflow::tests::test_heuristic_min_length_whitespace_counts ... ok
test workflow::tests::test_heuristic_min_length_zero_always_passes ... ok
test workflow::tests::test_heuristic_not_empty_fail_empty ... ok
test workflow::tests::test_heuristic_not_empty_fail_whitespace ... ok
test workflow::tests::test_heuristic_not_empty_pass ... ok
test workflow::tests::test_heuristic_unknown_check ... ok
test workflow::tests::test_interpolate_loop_vars_index ... ok
test workflow::tests::test_interpolate_loop_vars_combined ... ok
test workflow::tests::test_interpolate_loop_vars_item_object ... ok
test workflow::tests::test_interpolate_loop_vars_item_string ... ok
test workflow::tests::test_interpolate_loop_vars_item_whole_object ... ok
test workflow::tests::test_evaluate_condition_error_recovery ... ok
test workflow::tests::test_interpolate_loop_vars_missing_field ... ok
test workflow::tests::test_condition_unparseable_returns_true ... ok
test workflow::tests::test_interpolate_validation_prompt_basic ... ok
test workflow::tests::test_interpolate_validation_prompt_injection_safety ... ok
test workflow::tests::test_interpolate_validation_prompt_no_stderr ... ok
test workflow::tests::test_condition_legacy_syntax ... ok
test workflow::tests::test_interpolate_validation_prompt_no_truncation_when_under_limit ... ok
test workflow::tests::test_interpolate_validation_prompt_truncation ... ok
test workflow::tests::test_condition_steps_success ... ok
test workflow::tests::test_interpolate_loop_vars_multiple_fields_one_missing ... ok
test workflow::tests::test_interpolate_validation_prompt_with_stderr ... ok
test workflow::tests::test_condition_equals ... ok
test workflow::tests::test_condition_contains ... ok
test workflow::tests::test_condition_not ... ok
test workflow::tests::test_jinja_missing_step_default_fallback ... ok
test workflow::tests::test_jinja_if_block ... ok
test workflow::tests::test_jinja_shell_escape_filter ... ok
test workflow::tests::test_jinja_trim_filter ... ok
test workflow::tests::test_jinja_default_filter ... ok
test workflow::tests::test_jinja_join_filter ... ok
test workflow::tests::test_jinja_inline_for_loop ... ok
test workflow::tests::test_jinja_chained_filters ... ok
test workflow::tests::test_interpolate_parsed_output_none_fallback ... ok
test workflow::tests::test_load_error_tracker_backoff_progression ... ok
test workflow::tests::test_interpolate_with_fields_json ... ok
test workflow::tests::test_load_error_tracker_bail_at_threshold ... ok
test workflow::tests::test_load_error_tracker_reset_on_success ... ok
test workflow::tests::test_load_error_tracker_success_with_no_prior_errors ... ok
test workflow::tests::test_map_retry_failure_apply_error_with_paths ... ok
test workflow::tests::test_map_retry_failure_apply_error_without_paths ... ok
test workflow::tests::test_map_retry_failure_attempt_count_from_retries ... ok
test workflow::tests::test_map_retry_failure_empty_attempts ... ok
test workflow::tests::test_map_retry_failure_parse_error ... ok
test workflow::tests::test_map_retry_failure_verify_exit_code ... ok
test workflow::tests::test_map_retry_failure_verify_has_priority_over_apply ... ok
test workflow::tests::test_map_retry_failure_verify_timeout ... ok
test workflow::tests::test_map_retry_failure_stderr_truncated_to_1kb ... ok
test workflow::tests::test_condition_json_field_access ... ok
test workflow::tests::test_parse_for_each_inline_array ... ok
test workflow::tests::test_map_template_error_reports_offending_variable_in_multi_expression ... ok
test workflow::tests::test_min_deps_success_without_depends_on_error ... ok
test workflow::tests::test_output_format_toml_parsing ... ok
test workflow::tests::test_parse_for_each_inline_array_objects ... ok
test workflow::tests::test_parse_step_output_json ... ok
test workflow::tests::test_parse_step_output_lines ... ok
test workflow::tests::test_parse_step_output_none ... ok
test workflow::tests::test_parse_step_output_text ... ok
test workflow::tests::test_min_deps_success_validation_empty_deps ... ok
test workflow::tests::test_min_deps_success_validation_valid ... ok
test workflow::tests::test_parse_validation_response_empty_string_is_error ... ok
test workflow::tests::test_parse_for_each_invalid_format ... ok
test workflow::tests::test_parse_validation_response_invalid_status ... ok
test workflow::tests::test_parse_validation_response_json_fail ... ok
test workflow::tests::test_parse_for_each_step_not_found ... ok
test workflow::tests::test_parse_for_each_step_reference ... ok
test workflow::tests::test_parse_for_each_not_array ... ok
test workflow::tests::test_parse_validation_response_json_in_fences ... ok
test workflow::tests::test_parse_validation_response_json_pass ... ok
test workflow::tests::test_parse_for_each_step_reference_with_code_block ... ok
test workflow::tests::test_parse_validation_response_json_pass_no_output ... ok
test workflow::tests::test_parse_validation_response_review_failed ... ok
test workflow::tests::test_min_deps_success_validation_exceeds_deps ... ok
test workflow::tests::test_parse_validation_response_unrecognized_is_error ... ok
test workflow::tests::test_sanitize_json_strings ... ok
test workflow::tests::test_step_failure_kind_copy_eq ... ok
test workflow::tests::test_step_failure_kind_display ... ok
test workflow::tests::test_parse_validate_config_absent ... ok
test workflow::tests::test_step_for_each_inline_array_toml ... ok
test workflow::tests::test_parse_validate_config_from_toml ... ok
test workflow::tests::test_step_result_error_backend_error ... ok
test workflow::tests::test_step_for_each_toml_parsing ... ok
test workflow::tests::test_step_if_alias ... ok
test workflow::tests::test_step_result_error_edit_failed ... ok
test workflow::tests::test_step_result_error_has_no_validation ... ok
test workflow::tests::test_step_result_error_output_matches_failure_message ... ok
test workflow::tests::test_step_result_error_produces_failure ... ok
test workflow::tests::test_step_result_error_skipped ... ok
test workflow::tests::test_step_result_error_verify_failed ... ok
test workflow::tests::test_strip_markdown_fences_json ... ok
test workflow::tests::test_strip_markdown_fences_none ... ok
test workflow::tests::test_strip_markdown_fences_plain ... ok
test workflow::tests::test_strip_markdown_fences_with_whitespace ... ok
test workflow::tests::test_success_step_has_no_failure ... ok
test workflow::tests::test_parse_validate_config_mixed_fields ... ok
test workflow::tests::test_translate_contains_with_steps_prefix ... ok
test workflow::tests::test_translate_contains_call ... ok
test workflow::tests::test_translate_contains_with_single_quoted_literal_containing_double_quote ... ok
test workflow::tests::test_translate_contains_with_escaped_quotes ... ok
test workflow::tests::test_translate_equals_call ... ok
test workflow::tests::test_translate_fast_path_whitespace_variants ... ok
test workflow::tests::test_translate_equals_with_steps_prefix ... ok
test workflow::tests::test_translate_mixed_legacy_new ... ok
test workflow::tests::test_translate_multiple_contains ... ok
test workflow::tests::test_translate_passthrough_already_valid ... ok
test workflow::tests::test_translate_nested_not ... ok
test workflow::tests::test_translate_legacy_steps_output_contains ... ok
test workflow::tests::test_translate_passthrough_empty ... ok
test workflow::tests::test_translate_legacy_double_quotes ... ok
test workflow::tests::test_truncate_for_prompt_over_limit ... ok
test workflow::tests::test_truncate_for_prompt_under_limit ... ok
test workflow::tests::test_validation_failure_has_no_step_failure ... ok
test workflow::tests::test_parse_for_each_field_access ... ok
test workflow::tests::test_verify_command_composition_pattern ... ok
test workflow::tests::validate_accepts_apply_edits_on_claude ... ok
test workflow::tests::test_timeout_at_minimum_allowed ... ok
test workflow::tests::validate_rejects_apply_edits_with_multiple_backends ... ok
test workflow::tests::test_workflow_level_continue_on_error ... ok
test workflow::tests::validate_rejects_apply_edits_on_ollama ... ok
test workflow::tests::validate_rejects_apply_edits_with_no_backend ... ok
test workflow::tests::validate_skips_shell_only_steps ... ok
test workflow::tests::validate_treats_unknown_backend_as_none ... ok
test workflow::tests::validate_with_capabilities_handles_empty_steps ... ok
test workflows::tests::test_embedded_workflows_exist ... ok
test workflow::tests::test_apply_once_with_format_runs_after_apply ... ok
test workflow::tests::test_timeout_zero_allowed ... ok
test workflow::tests::test_timeout_too_small_validation ... ok
test workflow::tests::test_timeout_normal_value_allowed ... ok
test workflow::tests::test_validate_config_new_fields_default_to_none ... ok
test workflow::tests::test_validate_config_parses_mode_lenient_field ... ok
test workflow::tests::test_validate_config_new_fields_parsing ... ok
test workflow::tests::test_validate_config_defaults ... ok
test workflow::tests::test_validate_config_parses_on_parse_error_field ... ok
test workflows::tests::test_embedded_workflows_parse ... ok
test backend::retry::tests::test_retry_executor_honors_rate_limit_retry_after ... ok
test apply_verify::verification::tests::test_verify_elapsed_ms_nonzero ... ok
test apply_verify::verification::tests::test_verify_timeout_real_elapsed ... ok
test apply_verify::verification::tests::test_verify_timeout_kills_process_group ... ok

failures:

---- backend::tensorzero::tests::maps_401_to_auth_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_401_to_auth_not_retryable' (40902168) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

---- backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable' (40902278) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime stdout ----

thread 'backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime' (40902167) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_429_to_rate_limit_retryable stdout ----

thread 'backend::tensorzero::tests::maps_429_to_rate_limit_retryable' (40902284) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_500_to_retryable_error stdout ----

thread 'backend::tensorzero::tests::maps_500_to_retryable_error' (40902285) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_generic_to_network_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_generic_to_network_retryable' (40902286) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable' (40902287) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_malformed_json_to_parse_error stdout ----

thread 'backend::tensorzero::tests::maps_malformed_json_to_parse_error' (40902289) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable stdout ----

thread 'backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable' (40902288) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::maps_request_timeout_to_timeout_error stdout ----

thread 'backend::tensorzero::tests::maps_request_timeout_to_timeout_error' (40902292) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::returns_text_on_200_success stdout ----

thread 'backend::tensorzero::tests::returns_text_on_200_success' (40902296) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model stdout ----

thread 'backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model' (40902297) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/wiremock-0.6.5/src/mock_server/builder.rs:107:46:
Failed to bind an OS port for a mock server.: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" }

---- backend::tensorzero::tests::name_is_tensorzero stdout ----

thread 'backend::tensorzero::tests::name_is_tensorzero' (40902293) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
Attempted to create a NULL object.

---- backend::ollama::tests::capabilities_match_current_wiring stdout ----

thread 'backend::ollama::tests::capabilities_match_current_wiring' (40902135) panicked at /Users/mk/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/system-configuration-0.6.1/src/dynamic_store.rs:154:1:
Attempted to create a NULL object.


failures:
    backend::ollama::tests::capabilities_match_current_wiring
    backend::tensorzero::tests::endpoint_normalizes_to_openai_v1_at_runtime
    backend::tensorzero::tests::maps_401_to_auth_not_retryable
    backend::tensorzero::tests::maps_404_unknown_function_to_config_not_retryable
    backend::tensorzero::tests::maps_429_to_rate_limit_retryable
    backend::tensorzero::tests::maps_500_to_retryable_error
    backend::tensorzero::tests::maps_502_generic_to_network_retryable
    backend::tensorzero::tests::maps_502_upstream_auth_to_auth_not_retryable
    backend::tensorzero::tests::maps_502_upstream_rate_limit_to_rate_limit_retryable
    backend::tensorzero::tests::maps_malformed_json_to_parse_error
    backend::tensorzero::tests::maps_request_timeout_to_timeout_error
    backend::tensorzero::tests::name_is_tensorzero
    backend::tensorzero::tests::returns_text_on_200_success
    backend::tensorzero::tests::sends_bearer_auth_json_content_and_function_name_model

test result: FAILED. 599 passed; 14 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.24s

error: test failed, to rerun pass `--lib`

exec
/bin/zsh -lc "grep -R \"Serializer\\|serde\\(.*deserialize\\|derive\\(Serialize\\).*Vote\\|AggregatorConfig\\|strategy\\s*:\\s*\" -n src | head -n 300" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
grep: parentheses not balanced

exec
/bin/zsh -lc "grep -R \"struct Parallel\\|parallel\\s*:\\|vote\\s*:\\s*\" -n src/config.rs src/config/*.rs 2>/dev/null | head -n 200" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
zsh:1: no matches found: src/config/*.rs

exec
/bin/zsh -lc 'ls src' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
aggregator
apply_verify
backend
cache.rs
conductor.rs
config.rs
consensus.rs
context.rs
debate.rs
delegation.rs
family.rs
git_agent.rs
lib.rs
main.rs
output.rs
role
spawn.rs
strategy
tasks
team.rs
template
utils.rs
workflow.rs
workflows

exec
/bin/zsh -lc "grep -n \"parallel\\|Aggregator\\|LLMJudge\\|Vote\\|AnyFail\\|min_responses\\|targets\" src/config.rs | head -n 400" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
31:    #[serde(default = "default_parallel")]
32:    pub parallel: bool,
48:fn default_parallel() -> bool {
67:            parallel: default_parallel(),
459:        assert!(config.defaults.parallel);
542:parallel = false
547:        assert!(!config.defaults.parallel);
610:        assert_eq!(original.defaults.parallel, deserialized.defaults.parallel);
654:parallel = true
673:parallel = true
687:        assert!(config.defaults.parallel); // not overridden, stays true
695:parallel = true
703:parallel = false
709:        assert!(!config.defaults.parallel); // false overrides true
771:        assert!(config.defaults.parallel);
800:        assert!(config.defaults.parallel);
810:        assert!(config.defaults.parallel);
829:        assert!(config.defaults.parallel);
854:        // Project config: override timeout, add parallel=false
860:parallel = false
869:        // Project wins for parallel
870:        assert!(!config.defaults.parallel);
895:        assert!(config.defaults.parallel);

exec
/bin/zsh -lc "grep -R \"min_responses\\|tie_break\\|ballot_schema\\|aggregator\" -n src | head -n 400" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
src/lib.rs:8:pub mod aggregator;
src/aggregator/snapshots/loker__aggregator__concat__tests__concat_mixed_success_failure_snapshot.snap:2:source: src/aggregator/concat.rs
src/aggregator/snapshots/loker__aggregator__vote__tests__vote_snapshot.snap:2:source: src/aggregator/vote.rs
src/aggregator/snapshots/loker__aggregator__vote__tests__vote_snapshot.snap:7:<!-- loker: Vote aggregator metadata
src/aggregator/snapshots/loker__aggregator__vote__tests__vote_snapshot.snap:15:  tie_break_rule: none (strict majority)
src/aggregator/concat.rs:3:use crate::aggregator::VoteConfig;
src/aggregator/concat.rs:9:// This module contains behavioral aggregator config and pure aggregation logic.
src/aggregator/concat.rs:19:    "<!-- loker: concat aggregator received no target outputs -->";
src/aggregator/concat.rs:21:/// Behavioral aggregator configuration.
src/aggregator/concat.rs:49:    /// Build a concat aggregator with the provided heading template.
src/aggregator/concat.rs:56:    /// Build an LLM judge aggregator with the provided configuration.
src/aggregator/concat.rs:69:    /// Build a Vote aggregator with the provided configuration.
src/aggregator/concat.rs:74:    /// Build an AnyFail aggregator (no configuration needed).
src/aggregator/concat.rs:79:    /// Return the schema-facing strategy aggregator label for this behavior.
src/aggregator/concat.rs:111:/// Ordered input to an aggregator.
src/aggregator/concat.rs:153:/// Errors produced by aggregators.
src/aggregator/concat.rs:156:    #[error("unsupported aggregator operation: {0}")]
src/aggregator/concat.rs:481:            Aggregator::vote(crate::aggregator::VoteConfig {
src/aggregator/concat.rs:482:                ballot_schema: crate::aggregator::BallotSchema::FreeText,
src/aggregator/concat.rs:483:                tie_break: crate::aggregator::TieBreak::FirstResponder,
src/aggregator/mod.rs:3://! Three aggregator behaviors live here:
src/aggregator/vote.rs:32:/// Config payload for the Vote aggregator.
src/aggregator/vote.rs:35:    pub ballot_schema: BallotSchema,
src/aggregator/vote.rs:36:    pub tie_break: TieBreak,
src/aggregator/vote.rs:62:    pub tie_break_rule: String,
src/aggregator/vote.rs:146:            tie_break_rule: "none (strict majority)".into(),
src/aggregator/vote.rs:149:        let chosen_text = resolve_tie(&winners, &candidates, &buckets, &config.tie_break);
src/aggregator/vote.rs:156:            tie_break_rule: format_tie_break_rule(&config.tie_break),
src/aggregator/vote.rs:177:    tie_break: &TieBreak,
src/aggregator/vote.rs:179:    match tie_break {
src/aggregator/vote.rs:225:fn format_tie_break_rule(tie_break: &TieBreak) -> String {
src/aggregator/vote.rs:226:    match tie_break {
src/aggregator/vote.rs:253:    lines.push("<!-- loker: Vote aggregator metadata".into());
src/aggregator/vote.rs:263:        "  tie_break_rule: {}",
src/aggregator/vote.rs:264:        sanitize_comment(&result.tie_break_rule)
src/aggregator/vote.rs:300:    fn make_config(tie_break: TieBreak) -> VoteConfig {
src/aggregator/vote.rs:302:            ballot_schema: BallotSchema::FreeText,
src/aggregator/vote.rs:303:            tie_break,
src/aggregator/vote.rs:350:        assert_eq!(result.tie_break_rule, "closest_to_family(anthropic)");
src/aggregator/vote.rs:387:            ballot_schema: BallotSchema::FreeText,
src/aggregator/vote.rs:388:            tie_break: TieBreak::FirstResponder,
src/aggregator/vote.rs:417:            ballot_schema: BallotSchema::FreeText,
src/aggregator/vote.rs:418:            tie_break: TieBreak::FirstResponder,
src/aggregator/llm_judge.rs:9:use crate::aggregator::{strip_markdown_fences, AggregatedArtifact, BranchSuccess};
src/aggregator/llm_judge.rs:60:    #[error("aggregator contract violation: {message}")]
src/template/mod.rs:95:    /// Used by aggregators (e.g. LLMJudge) that need to inject arbitrary
src/family.rs:6://! downstream code (strategies, aggregators, phase runners) can call
src/family.rs:136:    #[error("aggregator contract violation: {message}")]
src/family.rs:142:    #[error("aggregator rejected: {message}")]
src/family.rs:425:    fn aggregator_rejected_display() {
src/family.rs:429:        assert_eq!(err.to_string(), "aggregator rejected: no candidates");
src/strategy/parallel_fanout.rs:7://! order.  Once `min_responses` successful responses have arrived the
src/strategy/parallel_fanout.rs:10://! If fewer than `min_responses` targets succeed before the whole set
src/strategy/parallel_fanout.rs:14:use crate::aggregator::{aggregate_llm_judge, Aggregator, BranchSuccess};
src/strategy/parallel_fanout.rs:53:    pub min_responses: usize,
src/strategy/parallel_fanout.rs:55:    pub aggregator: Aggregator,
src/strategy/parallel_fanout.rs:61:        min_responses: usize,
src/strategy/parallel_fanout.rs:63:        aggregator: Aggregator,
src/strategy/parallel_fanout.rs:66:            min_responses > 0,
src/strategy/parallel_fanout.rs:67:            "min_responses must be greater than 0, got {min_responses}"
src/strategy/parallel_fanout.rs:71:            min_responses,
src/strategy/parallel_fanout.rs:73:            aggregator,
src/strategy/parallel_fanout.rs:121:        let mut successful_candidates: Vec<crate::aggregator::BranchSuccess> =
src/strategy/parallel_fanout.rs:123:        let mut vote_branches: Vec<crate::aggregator::BranchOutcome> = Vec::new();
src/strategy/parallel_fanout.rs:124:        let is_any_fail = matches!(self.aggregator, Aggregator::AnyFail);
src/strategy/parallel_fanout.rs:125:        let is_llm_judge = matches!(self.aggregator, Aggregator::LLMJudge { .. });
src/strategy/parallel_fanout.rs:126:        let is_vote = matches!(self.aggregator, Aggregator::Vote { .. });
src/strategy/parallel_fanout.rs:142:                        if let Err(reason) = crate::aggregator::any_fail_evaluate(&query.stdout) {
src/strategy/parallel_fanout.rs:161:                                aggregator: Some(self.aggregator.kind()),
src/strategy/parallel_fanout.rs:187:                            .push(crate::aggregator::BranchOutcome::Success(branch_success));
src/strategy/parallel_fanout.rs:201:                    if !is_any_fail && !is_llm_judge && !is_vote && successes >= self.min_responses
src/strategy/parallel_fanout.rs:206:                        // cannot short-circuit on min_responses.
src/strategy/parallel_fanout.rs:243:                            aggregator: Some(self.aggregator.kind()),
src/strategy/parallel_fanout.rs:252:                            reason: crate::aggregator::AnyFailReason::BackendError {
src/strategy/parallel_fanout.rs:261:                        vote_branches.push(crate::aggregator::BranchOutcome::Failure(
src/strategy/parallel_fanout.rs:262:                            crate::aggregator::BranchFailure {
src/strategy/parallel_fanout.rs:287:                aggregator: Some(self.aggregator.kind()),
src/strategy/parallel_fanout.rs:293:        if successes < self.min_responses {
src/strategy/parallel_fanout.rs:301:                aggregator: Some(self.aggregator.kind()),
src/strategy/parallel_fanout.rs:307:                min_responses: self.min_responses,
src/strategy/parallel_fanout.rs:320:            aggregator: Some(self.aggregator.kind()),
src/strategy/parallel_fanout.rs:329:        } = &self.aggregator
src/strategy/parallel_fanout.rs:341:                use crate::aggregator::LLMJudgeError;
src/strategy/parallel_fanout.rs:402:            let config = match &self.aggregator {
src/strategy/parallel_fanout.rs:407:            let (aggregate, _result) = crate::aggregator::aggregate_vote(&vote_branches, config)
src/strategy/parallel_fanout.rs:409:                    crate::aggregator::VoteError::QuorumLost {
src/strategy/parallel_fanout.rs:416:                    crate::aggregator::VoteError::NoCandidates => {
src/strategy/parallel_fanout.rs:474:    use crate::aggregator::{
src/strategy/parallel_fanout.rs:626:        // settles; attempt count is therefore >= min_responses and <= targets.
src/strategy/parallel_fanout.rs:668:                min_responses,
src/strategy/parallel_fanout.rs:672:                assert_eq!(min_responses, 3);
src/strategy/parallel_fanout.rs:1033:                ballot_schema: BallotSchema::FreeText,
src/strategy/parallel_fanout.rs:1034:                tie_break: TieBreak::FirstResponder,
src/strategy/parallel_fanout.rs:1047:        assert_eq!(out.aggregator.as_ref().unwrap().as_str(), "vote");
src/strategy/parallel_fanout.rs:1060:                ballot_schema: BallotSchema::FreeText,
src/strategy/parallel_fanout.rs:1061:                tie_break: TieBreak::Random { seed: 123 },
src/strategy/parallel_fanout.rs:1098:                ballot_schema: BallotSchema::FreeText,
src/strategy/parallel_fanout.rs:1099:                tie_break: TieBreak::FirstResponder,
src/strategy/mod.rs:175:/// `aggregator`, `aggregate_output_path`, and `verify` are emitted at the
src/strategy/mod.rs:205:    pub aggregator: Option<Aggregator>,
src/strategy/mod.rs:219:            aggregator: None,
src/strategy/mod.rs:236:            field_count += 4; // branches, aggregator, aggregate_output_path, verify
src/strategy/mod.rs:276:                "aggregator",
src/strategy/mod.rs:277:                self.aggregator
src/strategy/mod.rs:423:    #[error("parallel floor violated: {successes}/{min_responses} targets succeeded")]
src/strategy/mod.rs:426:        min_responses: usize,
src/strategy/mod.rs:430:    #[error("aggregator any_fail: first failure from backend {backend}: {reason}")]
src/strategy/mod.rs:433:        reason: crate::aggregator::AnyFailReason,
src/strategy/single_model.rs:89:            aggregator: None,
src/strategy/escalating_retry.rs:397:                                    aggregator: None,
src/strategy/escalating_retry.rs:481:            aggregator: None,

exec
/bin/zsh -lc 'grep -R "ParallelFanOut::new" -n src | head -n 200' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
src/strategy/parallel_fanout.rs:594:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:617:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:653:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:683:        let strategy = ParallelFanOut::new(vec![], 1, "x", Aggregator::concat("## {backend_id}"));
src/strategy/parallel_fanout.rs:693:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:709:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:726:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:748:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:777:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:805:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:836:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:860:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:884:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:908:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:932:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:952:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:976:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:996:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:1024:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:1055:        let strategy = ParallelFanOut::new(
src/strategy/parallel_fanout.rs:1089:        let strategy = ParallelFanOut::new(

exec
/bin/zsh -lc "grep -n \"min_responses\\|quorum\\|threshold\\|vote\" -n docs/prds/clo-269-aggregator-vote.md" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
20:   as votes).
24:   configurable threshold (or when too few votes are cast to reach majority).
44:  - Backend errors during parallel execution → abstain (not a vote).
46:  - Configurable `abstain_threshold: usize` (or `max_abstain_fraction: f64`):
47:    if abstentions exceed the threshold, return `PhaseError::QuorumLost`.
69:- [ ] `PhaseError::QuorumLost` raised when abstentions exceed threshold.
91:- Existing `majority_vote` in `src/consensus.rs` (different concern,

exec
/bin/zsh -lc "sed -n '1,220p' docs/prds/clo-269-aggregator-vote.md" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
# PRD: CLO-269 — Aggregator::Vote with ballot schema and tie-breakers

## Problem

Workflow authors using `Strategy::ParallelFanOut` need a way to ask each
backend a structured ballot question and pick the winner by majority, not
just join outputs (`Concat`) or judge them externally (`LLMJudge`). The
`Vote` enum variant exists as a schema label but has zero behavioural
implementation. Without it, majority-based consensus (e.g. “which approach
is simpler: A or B?”) requires hand-crafting an LLMJudge prompt, which is
over-engineered for mechanical counting.

## Goal

Implement `Aggregator::Vote { ballot_schema: BallotSchema, tie_break: TieBreak }`
that:

1. Collects free-text or enum-style responses from each successful branch.
2. Abstains on malformed responses or backend errors (does not count them
   as votes).
3. Declares a winner when one response commands a strict majority (> 50 %).
4. Applies a deterministic tie-break rule when no strict majority exists.
5. Fails the phase with `PhaseError::QuorumLost` when abstentions exceed a
   configurable threshold (or when too few votes are cast to reach majority).

## Scope (in)

- `Aggregator` variant `Vote { ballot_schema, tie_break }` added to the
  behavioural enum in `src/aggregator/concat.rs`.
- `BallotSchema` enum:
  - `FreeText` (default, v0): each backend returns free text; text is
    normalised (trimmed, case-folded by config) before bucket counting.
  - `Enum { variants: Vec<String> }` (optional but strongly desired): each
    backend must pick one variant; anything outside the set is treated as
    abstain.
- `TieBreak` enum:
  - `ClosestToFamily(Family)` — resolve toward the first candidate whose
    `family_of(backend_id)` matches the given `Family`.
  - `Random { seed: u64 }` — deterministic shuffle from a per-run seed
    (seed sourced from manifest / workflow config).
  - `FirstResponder` — choose the candidate that arrived first in
    `ParallelFanOut` branch completion order.
- Abstention handling:
  - Backend errors during parallel execution → abstain (not a vote).
  - Malformed ballot (garbled text, invalid enum choice) → abstain.
  - Configurable `abstain_threshold: usize` (or `max_abstain_fraction: f64`):
    if abstentions exceed the threshold, return `PhaseError::QuorumLost`.
- Unit tests for every tie-break path with fixed seeds.
- Snapshot of phase-result file shape matching
  `docs/schemas/phase_result_parallel.schema.json`.

## Scope (out)

- Weighted voting (already exists in `src/consensus.rs` as a distinct
  `ConsensusStrategy`, not an `Aggregator`).
- Adaptive or recursive tie-breaking (e.g. re-prompt tied candidates).
- Ballot validation using a JSON schema or external parser.
- Prompt engineering for the ballot question itself (the question is
  rendered by `ParallelFanOut`'s existing template engine; Vote only
  interprets answers).

## Acceptance Criteria

- [ ] Tests pin ballot parsing, majority math, abstention handling, and
      each of the three tie-break rules.
- [ ] Random tie-break is reproducible from a logged seed (assert in a
      test).
- [ ] Snapshot of phase result file shape.
- [ ] `PhaseError::QuorumLost` raised when abstentions exceed threshold.
- [ ] `Vote` aggregator registered in `src/aggregator/concat.rs`
      `Aggregator::kind()` so the schema label round-trips.

## Demotion clause

If no concrete first use case lands by M3 start, close as Won't-do (v0)
and document the deferral in the roadmap. (Per roadmap; M3 date not yet
fixed.)

## Dependencies

- `family_of` lookup from [CLO-265](https://linear.app/cloud-ai/issue/CLO-265)
  is merged to main and exercised by LLMJudge.
- `PhaseError::QuorumLost` variant may need to be added to
  `src/family.rs` if it does not already exist.

## Related

- PRD FR-12 (Vote aggregator, Should)
- Design doc §7 aggregators, §8 open question on ballot schema
- Roadmap task T-019 in `docs/plans/001-implementation-roadmap.md`
- Existing `majority_vote` in `src/consensus.rs` (different concern,
  but shares normalisation logic).
- CLO-268 (LLMJudge) demonstrates how to wire a new aggregator into
  `ParallelFanOut`.

exec
/bin/zsh -lc "grep -R \"strict majority\\|tie-break\\|FirstResponder\\|majority\" -n src/aggregator/concat.rs src/aggregator/vote.rs docs/designs/clo-269-aggregator-vote.md docs/discovery/clo-269.md docs/prds/clo-269-aggregator-vote.md" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
src/aggregator/concat.rs:483:                tie_break: crate::aggregator::TieBreak::FirstResponder,
src/aggregator/vote.rs:20:/// How to resolve a tie when no strict majority exists.
src/aggregator/vote.rs:29:    FirstResponder,
src/aggregator/vote.rs:125:    // BTreeMap ensures deterministic iteration order for tie-break determinism.
src/aggregator/vote.rs:146:            tie_break_rule: "none (strict majority)".into(),
src/aggregator/vote.rs:180:        TieBreak::FirstResponder => tied_buckets
src/aggregator/vote.rs:205:                resolve_tie(tied_buckets, candidates, buckets, &TieBreak::FirstResponder)
src/aggregator/vote.rs:209:                resolve_tie(&matching, candidates, buckets, &TieBreak::FirstResponder)
src/aggregator/vote.rs:229:        TieBreak::FirstResponder => "first_responder".into(),
src/aggregator/vote.rs:315:        let config = make_config(TieBreak::FirstResponder);
src/aggregator/vote.rs:333:        let config = make_config(TieBreak::FirstResponder);
src/aggregator/vote.rs:337:        // "yes" arrives first (index 0), so FirstResponder picks it
src/aggregator/vote.rs:373:        let config = make_config(TieBreak::FirstResponder);
src/aggregator/vote.rs:388:            tie_break: TieBreak::FirstResponder,
src/aggregator/vote.rs:404:        let config = make_config(TieBreak::FirstResponder);
src/aggregator/vote.rs:418:            tie_break: TieBreak::FirstResponder,
src/aggregator/vote.rs:438:        let config = make_config(TieBreak::FirstResponder);
src/aggregator/vote.rs:450:        let config = make_config(TieBreak::FirstResponder);
src/aggregator/vote.rs:464:        // No match for Anthropic: falls back to FirstResponder
src/aggregator/vote.rs:485:        // When both tied buckets contain Anthropic, FirstResponder fallback
src/aggregator/vote.rs:495:        // Both "a" and "b" have Anthropic candidates; tie -> FirstResponder
src/aggregator/vote.rs:508:        let config = make_config(TieBreak::FirstResponder);
src/aggregator/vote.rs:521:        let config = make_config(TieBreak::FirstResponder);
src/aggregator/vote.rs:535:        let config = make_config(TieBreak::FirstResponder);
src/aggregator/vote.rs:552:        let config = make_config(TieBreak::FirstResponder);
src/aggregator/vote.rs:567:        let config = make_config(TieBreak::FirstResponder);
docs/designs/clo-269-aggregator-vote.md:1:# Design: CLO-269 — Aggregator::Vote with ballot schema and tie-breakers
docs/designs/clo-269-aggregator-vote.md:16:question and pick the majority winner cannot express this today — they must hand-craft
docs/designs/clo-269-aggregator-vote.md:20:error-time answers, applies tie-breakers, and produces a structured
docs/designs/clo-269-aggregator-vote.md:33:  `TieBreak::FirstResponder`.
docs/designs/clo-269-aggregator-vote.md:38:  structured metadata comment (vote counts, abstain count, tie-break used).
docs/designs/clo-269-aggregator-vote.md:39:- Unit-test every tie-break path with fixed seeds / deterministic inputs.
docs/designs/clo-269-aggregator-vote.md:45:- Recursive tie-breaking (e.g. second-round runoff between tied candidates).
docs/designs/clo-269-aggregator-vote.md:63:2. `aggregate_vote` returns a strict-majority winner in O(N) time for N branches.
docs/designs/clo-269-aggregator-vote.md:66:4. `TieBreak::FirstResponder` selects the bucket whose earliest-completed branch
docs/designs/clo-269-aggregator-vote.md:88:    vote.rs         # NEW: Vote config, normalisation, counting, tie-breakers
docs/designs/clo-269-aggregator-vote.md:139:/// How to resolve a tie when no strict majority exists.
docs/designs/clo-269-aggregator-vote.md:150:    FirstResponder,
docs/designs/clo-269-aggregator-vote.md:193:// removed because a unanimous single-bucket result is a valid strict majority,
docs/designs/clo-269-aggregator-vote.md:255:compute a majority or detect a quorum loss. Therefore the early-break condition is
docs/designs/clo-269-aggregator-vote.md:280:        // (for FirstResponder) and backend_id (for ClosestToFamily).
docs/designs/clo-269-aggregator-vote.md:336:But `Vote` cares about **arrival order** for `FirstResponder`, while
docs/designs/clo-269-aggregator-vote.md:339:`FirstResponder`, which needs the absolute arrival order among *all* branches
docs/designs/clo-269-aggregator-vote.md:393:    // BTreeMap ensures deterministic iteration order for tie-break determinism.
docs/designs/clo-269-aggregator-vote.md:417:            tie_break_rule: "none (strict majority)".into(),
docs/designs/clo-269-aggregator-vote.md:457:        TieBreak::FirstResponder => {
docs/designs/clo-269-aggregator-vote.md:486:                // Fallback: if no bucket matches target family, use FirstResponder.
docs/designs/clo-269-aggregator-vote.md:487:                resolve_tie(tied_buckets, candidates, buckets, &TieBreak::FirstResponder)
docs/designs/clo-269-aggregator-vote.md:491:                // Multiple matching buckets: apply FirstResponder among the matching subset.
docs/designs/clo-269-aggregator-vote.md:492:                resolve_tie(&matching, candidates, buckets, &TieBreak::FirstResponder)
docs/designs/clo-269-aggregator-vote.md:553:| `free_text_tie_first_responder` | 3 branches: "yes", "no", "yes" + tie=FirstResponder | winner="yes" (arrival 0) |
docs/designs/clo-269-aggregator-vote.md:562:| `closest_family_no_match_fallback` | tie with no matching family | falls back to FirstResponder |
docs/designs/clo-269-aggregator-vote.md:597:| Quorum threshold semantics | Absolute count: `abstain_threshold: usize`. The phase fails when `abstain_count > abstain_threshold`. This matches the AC: "document the threshold where abstain-majority returns PhaseError::QuorumLost". Example: 3 targets, threshold=1, 2 abstentions → QuorumLost. |
docs/designs/clo-269-aggregator-vote.md:604:| What does `FirstResponder` mean for a branch that backend-errored? | Errors are abstentions, so they never win. `FirstResponder` applies among *successful* branches only, ranked by their absolute arrival order in the FuturesUnordered loop. |
docs/discovery/clo-269.md:1:# Discovery Report: CLO-269 — Implement Aggregator::Vote with ballot schema and tie-breakers
docs/discovery/clo-269.md:13:want a lightweight majority vote over N candidate responses without
docs/discovery/clo-269.md:15:classification to three different-family models and taking the majority
docs/discovery/clo-269.md:23:error-time answers, or apply `ClosestToFamily` / `Random` / `FirstResponder`
docs/discovery/clo-269.md:24:tie-breakers in the aggregation path.
docs/discovery/clo-269.md:33:- A strict majority (> 50 %) wins outright.
docs/discovery/clo-269.md:34:- If no strict majority exists, the configured `TieBreak` rule resolves:
docs/discovery/clo-269.md:38:  - `FirstResponder` picks the candidate that arrived first.
docs/discovery/clo-269.md:40:  metadata: vote counts per response, abstain count, tie-break rule used.
docs/discovery/clo-269.md:73:- `majority_vote(responses: &[BackendResponse]) -> Option<VoteResult>`
docs/discovery/clo-269.md:81:- The normalisation logic (`trim()`) can be reused; the tie-breaking logic
docs/discovery/clo-269.md:98:  actually we need every single branch's response to compute a majority.
docs/discovery/clo-269.md:102:  flip the majority. The loop already collects until all futures resolve.
docs/discovery/clo-269.md:116:- Missing: Vote config enum, vote counting logic, tie-breaker logic,
docs/discovery/clo-269.md:155:  - Snapshot tests for tie-breaker determinism are easy.
docs/discovery/clo-269.md:166:Refactor `majority_vote` to accept `BranchSuccess` slices, add
docs/discovery/clo-269.md:172:  - `majority_vote` already has property-like tests.
docs/discovery/clo-269.md:175:    concern) and uses first-occurrence tie-breaking baked into the
docs/discovery/clo-269.md:177:    deterministic `Random`, and `FirstResponder` would refactor it into
docs/discovery/clo-269.md:180:    needed by the new tie-breakers.
docs/discovery/clo-269.md:210:  "document the threshold where abstain-majority returns `QuorumLost`".
docs/prds/clo-269-aggregator-vote.md:1:# PRD: CLO-269 — Aggregator::Vote with ballot schema and tie-breakers
docs/prds/clo-269-aggregator-vote.md:6:backend a structured ballot question and pick the winner by majority, not
docs/prds/clo-269-aggregator-vote.md:9:implementation. Without it, majority-based consensus (e.g. “which approach
docs/prds/clo-269-aggregator-vote.md:21:3. Declares a winner when one response commands a strict majority (> 50 %).
docs/prds/clo-269-aggregator-vote.md:22:4. Applies a deterministic tie-break rule when no strict majority exists.
docs/prds/clo-269-aggregator-vote.md:24:   configurable threshold (or when too few votes are cast to reach majority).
docs/prds/clo-269-aggregator-vote.md:41:  - `FirstResponder` — choose the candidate that arrived first in
docs/prds/clo-269-aggregator-vote.md:48:- Unit tests for every tie-break path with fixed seeds.
docs/prds/clo-269-aggregator-vote.md:56:- Adaptive or recursive tie-breaking (e.g. re-prompt tied candidates).
docs/prds/clo-269-aggregator-vote.md:64:- [ ] Tests pin ballot parsing, majority math, abstention handling, and
docs/prds/clo-269-aggregator-vote.md:65:      each of the three tie-break rules.
docs/prds/clo-269-aggregator-vote.md:66:- [ ] Random tie-break is reproducible from a logged seed (assert in a
docs/prds/clo-269-aggregator-vote.md:91:- Existing `majority_vote` in `src/consensus.rs` (different concern,

exec
/bin/zsh -lc "grep -R \"match .*PhaseError\\|PhaseError::\" -n src | head -n 400" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
src/family.rs:151:/// `Err(PhaseError::FamilyOverlap)`. An empty slice is allowed.
src/family.rs:170:            return Err(PhaseError::FamilyOverlap { family, count });
src/family.rs:390:            PhaseError::FamilyOverlap { family, count } => (family, count),
src/family.rs:397:                PhaseError::FamilyOverlap { family, count } => {
src/family.rs:414:        let err = PhaseError::QuorumLost {
src/family.rs:426:        let err = PhaseError::AggregatorRejected {
src/family.rs:434:        let err = PhaseError::JudgeUnavailable {
src/strategy/parallel_fanout.rs:350:                        StrategyError::Phase(crate::family::PhaseError::FamilyOverlap {
src/strategy/parallel_fanout.rs:356:                        StrategyError::Phase(crate::family::PhaseError::AggregatorContract {
src/strategy/parallel_fanout.rs:361:                        StrategyError::Phase(crate::family::PhaseError::JudgeUnavailable {
src/strategy/parallel_fanout.rs:412:                    } => StrategyError::Phase(crate::family::PhaseError::QuorumLost {
src/strategy/parallel_fanout.rs:417:                        StrategyError::Phase(crate::family::PhaseError::AggregatorRejected {
src/strategy/parallel_fanout.rs:1113:            other => panic!("expected PhaseError::QuorumLost, got {other:?}"),

exec
/bin/zsh -lc "sed -n '340,460p' src/family.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
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

    #[test]
    fn enforce_cross_family_deterministic() {
        // Two Anthropic + two OpenAI overlap. The reported family must be
        // identical across repeated calls (BTreeMap iteration is sorted by
        // key, so the first-by-sort-order overlapping family wins every run).
        let targets = &[
            "claude",
            "loker_review_anthropic",
            "openai",
            "loker_d1_openai",
        ];
        let first = enforce_cross_family(targets).unwrap_err();
        let (first_family, first_count) = match first {
            PhaseError::FamilyOverlap { family, count } => (family, count),
            _ => panic!("expected FamilyOverlap, got {first:?}"),
        };

        for _ in 0..100 {
            let next = enforce_cross_family(targets).unwrap_err();
            match next {
                PhaseError::FamilyOverlap { family, count } => {
                    assert_eq!(
                        first_family, family,
                        "FamilyOverlap family must be deterministic"
                    );
                    assert_eq!(
                        first_count, count,
                        "FamilyOverlap count must be deterministic"
                    );
                }
                _ => panic!("expected FamilyOverlap, got {next:?}"),
            }
        }
    }

    #[test]
    fn quorum_lost_display() {
        let err = PhaseError::QuorumLost {
            abstains: 3,
            threshold: 2,
        };
        assert_eq!(
            err.to_string(),
            "quorum lost: 3 abstentions exceed threshold 2"
        );
    }

    #[test]
    fn aggregator_rejected_display() {
        let err = PhaseError::AggregatorRejected {
            message: "no candidates".into(),
        };
        assert_eq!(err.to_string(), "aggregator rejected: no candidates");
    }

    #[test]
    fn judge_unavailable_display() {
        let err = PhaseError::JudgeUnavailable {
            detail: "network timeout".into(),
        };
        assert_eq!(err.to_string(), "judge unavailable: network timeout");
    }
}

exec
/bin/zsh -lc "grep -R '\"vote\"\\|vote' -n docs/schemas | head -n 200" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
docs/schemas/phase_result_parallel.schema.json:32:      "enum": ["concat", "llm_judge", "any_fail", "vote"]

exec
/bin/zsh -lc 'grep -R "aggregator" -n src/conductor.rs src/workflow.rs src | head -n 250' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
src/lib.rs:8:pub mod aggregator;
src/aggregator/snapshots/loker__aggregator__concat__tests__concat_mixed_success_failure_snapshot.snap:2:source: src/aggregator/concat.rs
src/aggregator/snapshots/loker__aggregator__vote__tests__vote_snapshot.snap:2:source: src/aggregator/vote.rs
src/aggregator/snapshots/loker__aggregator__vote__tests__vote_snapshot.snap:7:<!-- loker: Vote aggregator metadata
src/aggregator/concat.rs:3:use crate::aggregator::VoteConfig;
src/aggregator/concat.rs:9:// This module contains behavioral aggregator config and pure aggregation logic.
src/aggregator/concat.rs:19:    "<!-- loker: concat aggregator received no target outputs -->";
src/aggregator/concat.rs:21:/// Behavioral aggregator configuration.
src/aggregator/concat.rs:49:    /// Build a concat aggregator with the provided heading template.
src/aggregator/concat.rs:56:    /// Build an LLM judge aggregator with the provided configuration.
src/aggregator/concat.rs:69:    /// Build a Vote aggregator with the provided configuration.
src/aggregator/concat.rs:74:    /// Build an AnyFail aggregator (no configuration needed).
src/aggregator/concat.rs:79:    /// Return the schema-facing strategy aggregator label for this behavior.
src/aggregator/concat.rs:111:/// Ordered input to an aggregator.
src/aggregator/concat.rs:153:/// Errors produced by aggregators.
src/aggregator/concat.rs:156:    #[error("unsupported aggregator operation: {0}")]
src/aggregator/concat.rs:481:            Aggregator::vote(crate::aggregator::VoteConfig {
src/aggregator/concat.rs:482:                ballot_schema: crate::aggregator::BallotSchema::FreeText,
src/aggregator/concat.rs:483:                tie_break: crate::aggregator::TieBreak::FirstResponder,
src/aggregator/mod.rs:3://! Three aggregator behaviors live here:
src/aggregator/vote.rs:32:/// Config payload for the Vote aggregator.
src/aggregator/vote.rs:253:    lines.push("<!-- loker: Vote aggregator metadata".into());
src/aggregator/llm_judge.rs:9:use crate::aggregator::{strip_markdown_fences, AggregatedArtifact, BranchSuccess};
src/aggregator/llm_judge.rs:60:    #[error("aggregator contract violation: {message}")]
src/template/mod.rs:95:    /// Used by aggregators (e.g. LLMJudge) that need to inject arbitrary
src/family.rs:6://! downstream code (strategies, aggregators, phase runners) can call
src/family.rs:136:    #[error("aggregator contract violation: {message}")]
src/family.rs:142:    #[error("aggregator rejected: {message}")]
src/family.rs:425:    fn aggregator_rejected_display() {
src/family.rs:429:        assert_eq!(err.to_string(), "aggregator rejected: no candidates");
src/strategy/parallel_fanout.rs:14:use crate::aggregator::{aggregate_llm_judge, Aggregator, BranchSuccess};
src/strategy/parallel_fanout.rs:55:    pub aggregator: Aggregator,
src/strategy/parallel_fanout.rs:63:        aggregator: Aggregator,
src/strategy/parallel_fanout.rs:73:            aggregator,
src/strategy/parallel_fanout.rs:121:        let mut successful_candidates: Vec<crate::aggregator::BranchSuccess> =
src/strategy/parallel_fanout.rs:123:        let mut vote_branches: Vec<crate::aggregator::BranchOutcome> = Vec::new();
src/strategy/parallel_fanout.rs:124:        let is_any_fail = matches!(self.aggregator, Aggregator::AnyFail);
src/strategy/parallel_fanout.rs:125:        let is_llm_judge = matches!(self.aggregator, Aggregator::LLMJudge { .. });
src/strategy/parallel_fanout.rs:126:        let is_vote = matches!(self.aggregator, Aggregator::Vote { .. });
src/strategy/parallel_fanout.rs:142:                        if let Err(reason) = crate::aggregator::any_fail_evaluate(&query.stdout) {
src/strategy/parallel_fanout.rs:161:                                aggregator: Some(self.aggregator.kind()),
src/strategy/parallel_fanout.rs:187:                            .push(crate::aggregator::BranchOutcome::Success(branch_success));
src/strategy/parallel_fanout.rs:243:                            aggregator: Some(self.aggregator.kind()),
src/strategy/parallel_fanout.rs:252:                            reason: crate::aggregator::AnyFailReason::BackendError {
src/strategy/parallel_fanout.rs:261:                        vote_branches.push(crate::aggregator::BranchOutcome::Failure(
src/strategy/parallel_fanout.rs:262:                            crate::aggregator::BranchFailure {
src/strategy/parallel_fanout.rs:287:                aggregator: Some(self.aggregator.kind()),
src/strategy/parallel_fanout.rs:301:                aggregator: Some(self.aggregator.kind()),
src/strategy/parallel_fanout.rs:320:            aggregator: Some(self.aggregator.kind()),
src/strategy/parallel_fanout.rs:329:        } = &self.aggregator
src/strategy/parallel_fanout.rs:341:                use crate::aggregator::LLMJudgeError;
src/strategy/parallel_fanout.rs:402:            let config = match &self.aggregator {
src/strategy/parallel_fanout.rs:407:            let (aggregate, _result) = crate::aggregator::aggregate_vote(&vote_branches, config)
src/strategy/parallel_fanout.rs:409:                    crate::aggregator::VoteError::QuorumLost {
src/strategy/parallel_fanout.rs:416:                    crate::aggregator::VoteError::NoCandidates => {
src/strategy/parallel_fanout.rs:474:    use crate::aggregator::{
src/strategy/parallel_fanout.rs:1047:        assert_eq!(out.aggregator.as_ref().unwrap().as_str(), "vote");
src/strategy/mod.rs:175:/// `aggregator`, `aggregate_output_path`, and `verify` are emitted at the
src/strategy/mod.rs:205:    pub aggregator: Option<Aggregator>,
src/strategy/mod.rs:219:            aggregator: None,
src/strategy/mod.rs:236:            field_count += 4; // branches, aggregator, aggregate_output_path, verify
src/strategy/mod.rs:276:                "aggregator",
src/strategy/mod.rs:277:                self.aggregator
src/strategy/mod.rs:430:    #[error("aggregator any_fail: first failure from backend {backend}: {reason}")]
src/strategy/mod.rs:433:        reason: crate::aggregator::AnyFailReason,
src/strategy/single_model.rs:89:            aggregator: None,
src/strategy/escalating_retry.rs:397:                                    aggregator: None,
src/strategy/escalating_retry.rs:481:            aggregator: None,

exec
/bin/zsh -lc "sed -n '80,190p' src/consensus.rs" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
            weights,
            default_weight: 1.0,
        }
    }

    /// Get weight for a backend
    pub fn get(&self, backend: &str) -> f64 {
        self.weights
            .get(backend)
            .copied()
            .unwrap_or(self.default_weight)
    }
}

/// Response from a backend for voting
#[derive(Debug, Clone)]
pub struct BackendResponse {
    pub backend: String,
    pub content: String,
}

/// Perform majority vote on responses
///
/// Returns the most common response. Ties broken by first occurrence.
pub fn majority_vote(responses: &[BackendResponse]) -> Option<VoteResult> {
    if responses.is_empty() {
        return None;
    }

    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut first_seen: HashMap<String, usize> = HashMap::new();

    for (idx, resp) in responses.iter().enumerate() {
        // Normalize whitespace for comparison
        let normalized = resp.content.trim().to_string();
        *counts.entry(normalized.clone()).or_default() += 1;
        first_seen.entry(normalized).or_insert(idx);
    }

    let max_count = *counts.values().max().unwrap_or(&0);
    let winners: Vec<_> = counts
        .iter()
        .filter(|(_, &count)| count == max_count)
        .collect();

    let was_tie = winners.len() > 1;

    // Break tie by first occurrence
    let winner = winners
        .into_iter()
        .min_by_key(|(content, _)| first_seen.get(*content).unwrap_or(&usize::MAX))
        .map(|(content, _)| content.clone())?;

    Some(VoteResult {
        winner,
        breakdown: counts,
        total: responses.len(),
        was_tie,
    })
}

/// Perform weighted vote on responses
///
/// Each backend's vote is weighted by its tier. Returns the response with highest total weight.
pub fn weighted_vote(
    responses: &[BackendResponse],
    weights: &BackendWeights,
) -> Option<WeightedVoteResult> {
    if responses.is_empty() {
        return None;
    }

    let mut scores: HashMap<String, f64> = HashMap::new();
    let mut first_seen: HashMap<String, usize> = HashMap::new();

    for (idx, resp) in responses.iter().enumerate() {
        let normalized = resp.content.trim().to_string();
        let weight = weights.get(&resp.backend);
        *scores.entry(normalized.clone()).or_default() += weight;
        first_seen.entry(normalized).or_insert(idx);
    }

    let max_score = scores.values().cloned().fold(0.0, f64::max);
    let winners: Vec<_> = scores
        .iter()
        .filter(|(_, &score)| (score - max_score).abs() < f64::EPSILON)
        .collect();

    let was_tie = winners.len() > 1;

    // Break tie by first occurrence
    let winner = winners
        .into_iter()
        .min_by_key(|(content, _)| first_seen.get(*content).unwrap_or(&usize::MAX))
        .map(|(content, _)| content.clone())?;

    Some(WeightedVoteResult {
        winner,
        breakdown: scores,
        was_tie,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_majority_vote_clear_winner() {
        let responses = vec![
            BackendResponse {

exec
/bin/zsh -lc "grep -n \"min_responses\\|independence\" -n docs/designs/clo-269-aggregator-vote.md" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
253:`ParallelFanOut::execute` currently short-circuits when `successes >= min_responses`.
259:if !is_any_fail && !is_llm_judge && !is_vote && successes >= self.min_responses {
606:| How do `min_responses` and `abstain_threshold` interact? | They are independent gates. `min_responses` controls when `ParallelFanOut` stops collecting additional branches; `abstain_threshold` controls whether Vote rejects the result after all branches are collected. Vote disables the `min_responses` short-circuit so it always sees the full set. |

exec
/bin/zsh -lc "nl -ba src/strategy/parallel_fanout.rs | sed -n '280,315p'" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
   280	            return Ok(StrategyOutput {
   281	                schema_version: SCHEMA_VERSION,
   282	                strategy: StrategyKind::Parallel,
   283	                phase: ctx.phase_name.clone(),
   284	                run_id: ctx.run_id,
   285	                attempts,
   286	                final_status: None,
   287	                aggregator: Some(self.aggregator.kind()),
   288	                aggregate_output_path: Some(format!("{}/aggregated.txt", ctx.phase_name)),
   289	                verify: Some(VerifyOutcome::passed("Aggregator::AnyFail")),
   290	            });
   291	        }
   292	
   293	        if successes < self.min_responses {
   294	            let output = StrategyOutput {
   295	                schema_version: SCHEMA_VERSION,
   296	                strategy: StrategyKind::Parallel,
   297	                phase: ctx.phase_name.clone(),
   298	                run_id: ctx.run_id,
   299	                attempts,
   300	                final_status: None,
   301	                aggregator: Some(self.aggregator.kind()),
   302	                aggregate_output_path: Some(format!("{}/aggregated.txt", ctx.phase_name)),
   303	                verify: Some(VerifyOutcome::skipped()),
   304	            };
   305	            return Err(StrategyError::FloorViolation {
   306	                successes,
   307	                min_responses: self.min_responses,
   308	                output: Box::new(output),
   309	            });
   310	        }
   311	
   312	        let aggregated_output_path = format!("{}/aggregated.txt", ctx.phase_name);
   313	        let mut output = StrategyOutput {
   314	            schema_version: SCHEMA_VERSION,
   315	            strategy: StrategyKind::Parallel,

exec
/bin/zsh -lc "nl -ba src/aggregator/vote.rs | sed -n '233,250p'" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
   233	fn build_aggregated_text(
   234	    result: &VoteResult,
   235	    candidates: &[VoteCandidate],
   236	    buckets: &BTreeMap<String, Vec<usize>>,
   237	) -> String {
   238	    // Pick the winner's original text from the first candidate in the winning bucket.
   239	    let winner_original = buckets
   240	        .get(&result.winner)
   241	        .and_then(|indices| {
   242	            indices
   243	                .first()
   244	                .and_then(|&idx| candidates.get(idx).map(|c| c.normalised.as_str()))
   245	        })
   246	        .unwrap_or(&result.winner);
   247	
   248	    let mut lines = Vec::new();
   249	    lines.push(winner_original.to_string());
   250	    lines.push(String::new());

exec
/bin/zsh -lc "nl -ba src/aggregator/vote.rs | sed -n '120,160p'" in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
   120	    if candidates.is_empty() {
   121	        return Err(VoteError::NoCandidates);
   122	    }
   123	
   124	    // Count votes by normalised bucket.
   125	    // BTreeMap ensures deterministic iteration order for tie-break determinism.
   126	    let mut buckets: BTreeMap<String, Vec<usize>> = BTreeMap::new();
   127	    for (idx, c) in candidates.iter().enumerate() {
   128	        buckets.entry(c.normalised.clone()).or_default().push(idx);
   129	    }
   130	
   131	    let max_votes = buckets.values().map(|v| v.len()).max().unwrap_or(0);
   132	    let winners: Vec<&str> = buckets
   133	        .iter()
   134	        .filter(|(_, v)| v.len() == max_votes)
   135	        .map(|(k, _)| k.as_str())
   136	        .collect();
   137	
   138	    let mut result = if winners.len() == 1 {
   139	        let winner_text = winners[0];
   140	        VoteResult {
   141	            winner: winner_text.into(),
   142	            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
   143	            abstain_count,
   144	            total_branches: total,
   145	            tie_broken: false,
   146	            tie_break_rule: "none (strict majority)".into(),
   147	        }
   148	    } else {
   149	        let chosen_text = resolve_tie(&winners, &candidates, &buckets, &config.tie_break);
   150	        VoteResult {
   151	            winner: chosen_text,
   152	            vote_counts: buckets.iter().map(|(k, v)| (k.clone(), v.len())).collect(),
   153	            abstain_count,
   154	            total_branches: total,
   155	            tie_broken: true,
   156	            tie_break_rule: format_tie_break_rule(&config.tie_break),
   157	        }
   158	    };
   159	
   160	    // Sort vote_counts descending for stable output

exec
/bin/zsh -lc 'nl -ba !' in /Users/mk/Code/orchestrator/loker--feat-clo-269
 succeeded in 0ms:
codex
There are functional regressions in vote semantics that can lead to incorrect outcome selection and incorrect error classification. In particular, winner selection does not enforce a strict-majority condition as described, and vote-specific quorum handling can be short-circuited by the existing floor-violation check before aggregation runs.

Full review comments:

- [P2] Require >50% majority before winner selection — /Users/mk/Code/orchestrator/loker--feat-clo-269/src/aggregator/vote.rs:138-146
  `aggregate_vote` only treats ties specially and otherwise accepts the bucket with the highest count as the winner, even when that bucket is not a strict majority of ballots. For example, with four successful votes `a,a,b,c`, the current logic returns `a` because it is the unique max (`2`) although it is not >50%; this contradicts the documented “strict-majority” behavior in this feature and can produce an unintended winner.

- [P2] Bypass floor-violation guard for vote quorum handling — /Users/mk/Code/orchestrator/loker--feat-clo-269/src/strategy/parallel_fanout.rs:293-307
  `ParallelFanOut::execute` still performs `if successes < self.min_responses` before entering vote aggregation, so Vote can fail with `StrategyError::FloorViolation` even after all branches have been collected. In vote mode that suppresses the vote-specific `aggregate_vote` path (and therefore `PhaseError::QuorumLost`) for scenarios where many branches fail but abstention threshold logic should decide the outcome.

- [P3] Preserve raw winning response text in artifact output — /Users/mk/Code/orchestrator/loker--feat-clo-269/src/aggregator/vote.rs:238-245
  `build_aggregated_text` says it selects the winner’s original text, but it pulls `c.normalised` from the candidate bucket, which has already been lowercased and trimmed. This rewrites user-facing output (case/whitespace changes) instead of preserving the actual winning response content from the backend.
2026-04-29T20:01:45.327653Z ERROR codex_core::session: failed to record rollout items: thread 019ddad3-9fee-7512-aa07-b5993256cdee not found
2026-04-29T20:01:45.363876Z ERROR codex_core::session: failed to record rollout items: thread 019ddad3-9faa-78e2-bfa8-d78aa5b84316 not found
There are functional regressions in vote semantics that can lead to incorrect outcome selection and incorrect error classification. In particular, winner selection does not enforce a strict-majority condition as described, and vote-specific quorum handling can be short-circuited by the existing floor-violation check before aggregation runs.

Full review comments:

- [P2] Require >50% majority before winner selection — /Users/mk/Code/orchestrator/loker--feat-clo-269/src/aggregator/vote.rs:138-146
  `aggregate_vote` only treats ties specially and otherwise accepts the bucket with the highest count as the winner, even when that bucket is not a strict majority of ballots. For example, with four successful votes `a,a,b,c`, the current logic returns `a` because it is the unique max (`2`) although it is not >50%; this contradicts the documented “strict-majority” behavior in this feature and can produce an unintended winner.

- [P2] Bypass floor-violation guard for vote quorum handling — /Users/mk/Code/orchestrator/loker--feat-clo-269/src/strategy/parallel_fanout.rs:293-307
  `ParallelFanOut::execute` still performs `if successes < self.min_responses` before entering vote aggregation, so Vote can fail with `StrategyError::FloorViolation` even after all branches have been collected. In vote mode that suppresses the vote-specific `aggregate_vote` path (and therefore `PhaseError::QuorumLost`) for scenarios where many branches fail but abstention threshold logic should decide the outcome.

- [P3] Preserve raw winning response text in artifact output — /Users/mk/Code/orchestrator/loker--feat-clo-269/src/aggregator/vote.rs:238-245
  `build_aggregated_text` says it selects the winner’s original text, but it pulls `c.normalised` from the candidate bucket, which has already been lowercased and trimmed. This rewrites user-facing output (case/whitespace changes) instead of preserving the actual winning response content from the backend.
