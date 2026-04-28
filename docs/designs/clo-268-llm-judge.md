# Design: CLO-268 — Aggregator::LLMJudge with cross-family enforcement

| Field | Value |
|-------|-------|
| Task | CLO-268 |
| Date | 2026-04-28 |
| Phase | design |
| Discovery | docs/discovery/clo-268.md |
| PRD | docs/prds/clo-268-llm-judge.md |

## 1. Problem

`ParallelFanOut` currently supports `Concat` (text joining) and `AnyFail` (pessimistic verdict evaluation). The `Aggregator` enum stub also declares `LLMJudge`, but there is no behavioural implementation. Workflow authors who want to fan out to N models and then use a separate-family model as a judge cannot express this in loker. This design closes that gap by adding a self-contained `llm_judge.rs` module under `src/aggregator/` that constructs a ballot prompt, enforces cross-family separation, calls the judge backend, parses the structured response, and produces an `AggregatedArtifact`.

Reference: discovery report §Problem Framing.

## 2. Goals & Non-goals

### Goals
- Add `Aggregator::LLMJudge { judge_backend, prompt_template, require_judge_different_family }` to the behavioural enum.
- Render a ballot prompt from the user-supplied template + candidate outputs.
- Reuse `family_of` / `enforce_cross_family` (CLO-265) to guarantee judge ≠ candidate family overlap.
- Parse judge response as `{ chosen_index: usize, reason: string }`.
- Map malformed responses → `PhaseError::AggregatorContract`.
- Map judge backend transport errors → `PhaseError::JudgeUnavailable`.
- Unit-test prompt construction and ballot parsing without live backends.
- Integration-test the full judge call via Wiremock-backed mock backend.
- Snapshot-test serialised `StrategyOutput` against the parallel schema.

### Non-goals
- Retry policy for judge calls (v0: single attempt).
- Prompt-injection sanitisation of candidate outputs (deferred; assumes trusted workflow context).
- `Vote` aggregator (T-019).
- Pluggable judge response schemas (v0 locks to the single JSON schema below).

## 3. Architecture

### 3.1 Modules

```
src/
  aggregator/
    mod.rs          # AnyFail logic, re-exports, shared helpers (strip_markdown_fences)
    concat.rs       # Concat config + behaviour (unchanged)
    llm_judge.rs   # NEW: LLMJudge config, prompt builder, parser, family check
  strategy/
    mod.rs          # Aggregator schema label enum (unchanged)
    parallel_fanout.rs  # Extended: post-collection judge call dispatch
  family.rs         # CLO-265: add PhaseError::JudgeUnavailable
```

### 3.2 Data flow

```
ParallelFanOut::execute
  │
  ├─ FuturesUnordered loop collects all attempts (same as today)
  │
  ├─ match self.aggregator
  │    ├─ Concat  → concat::aggregate_concat(...)
  │    ├─ AnyFail → already handled inline
  │    └─ LLMJudge → llm_judge::aggregate_llm_judge(
  │                     candidates,
  │                     judge_backend_name,
  │                     prompt_template,
  │                     require_judge_different_family,
  │                     backends,      // &amp;[Arc&lt;dyn Backend&gt;]
  │                     ctx,            // PhaseContext (for template engine + cwd)
  │                  )
  │
  └─ Returns StrategyOutput with
       aggregator: Some(Aggregator::LLMJudge),
       aggregate_output_path: "{phase}/aggregated.txt",
       verify: VerifyOutcome::passed("LLMJudge")
```

### 3.3 New types

```rust
// src/aggregator/llm_judge.rs
use serde::{Deserialize, Serialize};

/// Candidate exposed to the prompt template.
#[derive(Debug, Clone, Serialize)]
pub struct Candidate {
    pub index: usize,
    pub backend_id: String,
    pub family: String,
    pub output: String,
}

/// Judge ballot response schema (v0).
#[derive(Debug, Clone, Deserialize)]
pub struct Ballot {
    pub chosen_index: usize,
    pub reason: String,
}

/// Errors specific to LLMJudge aggregation.
#[derive(Debug, thiserror::Error)]
pub enum LLMJudgeError {
    #[error("family overlap: judge backend {judge} shares family {family} with candidate {candidate}")]
    FamilyOverlap {
        judge: String,
        candidate: String,
        family: String,
    },

    #[error("judge backend not found: {0}")]
    BackendNotFound(String),

    #[error("judge call failed: {0}")]
    JudgeCall(#[from] crate::backend::BackendError),

    #[error("aggregator contract violation: {message}")]
    Contract { message: String },
}
```

### 3.4 Public API surface

```rust
// src/aggregator/concat.rs  (behavioural Aggregator enum expanded)
pub enum Aggregator {
    Concat { heading_template: String },
    LLMJudge {
        judge_backend: String,
        prompt_template: String,
        require_judge_different_family: bool,
    },
}

impl Aggregator {
    pub fn kind(&amp;self) -&gt; crate::strategy::Aggregator { ... }

    pub fn aggregate(
        &amp;self,
        input: AggregateInput,
        backends: &amp;[Arc&lt;dyn Backend&gt;],
        ctx: &amp;PhaseContext,
    ) -&gt; Result&lt;AggregatedArtifact, AggregatorError&gt;;
}
```

**Note on signature change:** `aggregate()` currently takes only `AggregateInput`. To support `LLMJudge`, it must also receive `backends` and `ctx`. This is a minor interface expansion documented as v0 boundary debt; T-029 may formalise a proper `Aggregator` trait.

```rust
// src/aggregator/llm_judge.rs
pub fn aggregate_llm_judge(
    candidates: &amp;[BranchSuccess],
    judge_backend: &amp;str,
    prompt_template: &amp;str,
    require_judge_different_family: bool,
    backends: &amp;[Arc&lt;dyn Backend&gt;],
    ctx: &amp;PhaseContext,
) -&gt; Result&lt;AggregatedArtifact, AggregatorError&gt;;
```

### 3.5 PhaseError expansion

```rust
// src/family.rs
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum PhaseError {
    #[error("family overlap: found {family} on {count} backends")]
    FamilyOverlap { family: Family, count: usize },

    #[error("aggregator contract violation: {message}")]
    AggregatorContract { message: String },

    #[error("judge unavailable: {detail}")]
    JudgeUnavailable { detail: String },
}
```

## 4. Implementation details

### 4.1 Family check

```rust
fn check_cross_family(
    judge_backend: &amp;str,
    candidates: &amp;[BranchSuccess],
    require_different: bool,
) -&gt; Result&lt;(), LLMJudgeError&gt; {
    let judge_family = family_of(judge_backend);
    for c in candidates {
        let c_family = family_of(&amp;c.backend_id);
        if c_family == judge_family {
            if require_different {
                return Err(LLMJudgeError::FamilyOverlap {
                    judge: judge_backend.into(),
                    candidate: c.backend_id.clone(),
                    family: judge_family.to_string(),
                });
            } else {
                log::warn!(
                    "judge backend {} shares family {} with candidate {} (opted out)",
                    judge_backend, judge_family, c.backend_id
                );
            }
        }
    }
    Ok(())
}
```

### 4.2 Prompt rendering

Use `ctx.template_engine` (Tera already wired in `PhaseContext`). Build a `Context` with:
- `candidates: Vec&lt;Candidate&gt;`
- `phase_name: &amp;str`

The `Candidate` struct derives `Serialize` so Tera can iterate with `{% for c in candidates %}`.

### 4.3 Judge call

```rust
let judge = backends
    .iter()
    .find(|b| b.name() == judge_backend)
    .ok_or(LLMJudgeError::BackendNotFound(judge_backend.into()))?;

let query = judge.query(&amp;rendered_prompt, &amp;ctx.cwd, None).await?;
```

Transport errors (`BackendError`) are converted via `From` into `LLMJudgeError::JudgeCall`, then mapped at the call site:

```rust
// In parallel_fanout.rs
Err(LLMJudgeError::JudgeCall(be)) =&gt; {
    return Err(PhaseError::JudgeUnavailable {
        detail: be.to_string(),
    });
}
```

### 4.4 Ballot parsing

```rust
fn parse_ballot(text: &amp;str) -&gt; Result&lt;Ballot, LLMJudgeError&gt; {
    let stripped = strip_markdown_fences(text.trim());
    let value: serde_json::Value = serde_json::from_str(stripped)
        .map_err(|e| LLMJudgeError::Contract {
            message: format!("JSON parse error: {e}"),
        })?;

    let chosen_index = value.get("chosen_index")
        .and_then(|v| v.as_u64())
        .map(|u| u as usize)
        .ok_or_else(|| LLMJudgeError::Contract {
            message: "missing or non-integer 'chosen_index'".into(),
        })?;

    let reason = value.get("reason")
        .and_then(|v| v.as_str())
        .ok_or_else(|| LLMJudgeError::Contract {
            message: "missing or non-string 'reason'".into(),
        })?;

    Ok(Ballot { chosen_index, reason: reason.into() })
}
```

Index clamping happens after parsing:
```rust
let chosen_index = ballot.chosen_index.min(candidates.len().saturating_sub(1));
```

### 4.5 AggregatedArtifact construction

```rust
let chosen = &amp;candidates[chosen_index];
let text = format!(
    "{}\n\n<!-- loker: LLMJudge chose candidate {chosen_index} ({}) --\u003e\n{}\n",
    chosen.output,
    chosen.backend_id,
    ballot.reason
);

Ok(AggregatedArtifact {
    text,
    successful: candidates.len(),
    failed: 0,
})
```

## 5. Test plan

### Unit tests (`src/aggregator/llm_judge.rs`)

| Test | Input | Expected |
|------|-------|----------|
| `prompt_renders_candidates` | 2 candidates + template | Markdown headings with correct index/backend_id/family |
| `parse_valid_ballot` | `{"chosen_index":1,"reason":"better"}` | `Ballot { chosen_index: 1, reason: "better" }` |
| `parse_markdown_fenced_ballot` | `` \`\`\`json\n{...}\n\`\`\` `` | Same as above |
| `parse_missing_chosen_index` | `{"reason":"better"}` | `Contract` error |
| `parse_negative_chosen_index` | `{"chosen_index":-1,"reason":"better"}` | Rejected by `as_u64()` → `Contract` error |
| `parse_out_of_bounds_index` | valid ballot with index 5 for 2 candidates | Clamped to 1 |
| `family_overlap_blocks` | judge_family == candidate_family, require=true | `FamilyOverlap` error |
| `family_overlap_opt_out_warns` | same overlap, require=false | Ok + warning logged |
| `family_diverse_ok` | 3 different families | Ok |

### Integration tests (`tests/strategy_parallel_fanout.rs` or new `tests/aggregator_llm_judge.rs`)

| Test | Setup | Expected |
|------|-------|----------|
| `llm_judge_success` | 2 mock candidates + 1 mock judge returning valid ballot | `StrategyOutput` with correct `aggregate_output_path`, schema validates |
| `llm_judge_malformed_json` | judge returns `not json` | `PhaseError::AggregatorContract` |
| `llm_judge_backend_error` | judge returns `BackendError::Network` | `PhaseError::JudgeUnavailable` |
| `llm_judge_family_overlap_refused` | judge == candidate family, require=true | `PhaseError::FamilyOverlap` |
| `llm_judge_family_overlap_opt_out` | judge == candidate family, require=false | Phase succeeds, warning |

### Snapshot test

1. Run `llm_judge_success`.
2. Serialise `StrategyOutput` to JSON.
3. Validate against `docs/schemas/phase_result_parallel.schema.json`.
4. Use `insta` snapshot on the `aggregated.txt` content (chosen output + rationale comment).

## 6. Migration / rollout

- No breaking changes to existing `Concat` or `AnyFail` aggregators.
- `PhaseError` gains `JudgeUnavailable` (additive, `#[non_exhaustive]`).
- `Aggregator` enum gains `LLMJudge` variant (already present as schema label; behavioural payload is new).
- Workflow files that previously referenced `"llm_judge"` as a schema label but had no implementation will now work if the `judge_backend`, `prompt_template`, etc. are configured. This is a net-new capability, not a migration.

## 7. Open questions

| Question | Resolution |
|----------|------------|
| Should `aggregate()` signature change for all aggregators? | Yes, minimal expansion: add `backends` + `ctx` parameters. `Concat` ignores them. Document as v0 debt. |
| Where does the judge `Attempt` record go in `StrategyOutput.attempts`? | Not in `attempts`. The judge is not a candidate branch; its call is part of aggregation. Future T-029 may add a `judge_attempt` field if observability requires it. |
| What if the judge backend name is not in the `backends` slice? | `BackendNotFound` error surfaced through `LLMJudgeError`. |

## 8. References

- Discovery report: `docs/discovery/clo-268.md`
- PRD: `docs/prds/clo-268-llm-judge.md`
- CLO-265 `family.rs`: `src/family.rs`
- CLO-266 concat aggregator: `src/aggregator/concat.rs`
- CLO-267 AnyFail: `src/aggregator/mod.rs`
- ParallelFanOut: `src/strategy/parallel_fanout.rs`
- loker-design.md §4.3, §14 (cross-family enforcement)
