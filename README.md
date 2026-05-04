# loker

LLM orchestration engine. Routes prompts across multiple model families,
aggregates their responses, and verifies outputs before they ship.

> **Status: pre-v0 (M0 - fork preparation).** loker is a hard fork of
> [`ducks/lok`](https://github.com/ducks/lok). The lok command surface still
> works under the new `loker` binary. The new primitives described below
> (Strategy, Aggregator, VerifyHook, TensorZero backend) land in milestones
> M1-M9. See [`loker-design.md`](https://github.com/maxkulish/investigations/blob/main/sakana-fugu/loker-design.md)
> for the full design.

## Why loker exists

Two gaps in the current Rust LLM-orchestration ecosystem:

1. **No production cross-family aggregation.** Sending the same prompt to
   Anthropic, Google, OpenAI, and Zhipu in parallel and merging their answers
   reduces correlated failures - but no Rust crate ships this as a primitive.
2. **No escalating-retry primitive.** The pattern "try cheap model -> verify ->
   retry on stronger model if it fails" is widely used informally but absent
   from existing Rust orchestrators (`rig`, `genai`, `graniet/llm`, `swarms-rs`).

loker exists to put both behind a stable, declarative TOML interface.

## The three primitives

These are the public surface loker is built around. None ship yet (M2-M4).

### Strategy - how to call backends

```rust
enum Strategy {
    SingleModel,        // one backend, one call
    ParallelFanOut,     // N backends in parallel, partial-failure tolerant
    EscalatingRetry,    // cheap -> medium -> strong, stop on first verify-pass
}
```

### Aggregator - how to merge results

```rust
enum Aggregator {
    Concat,             // dump all responses (debug, audit trails)
    LLMJudge,           // use an LLM (different family) to merge
    AnyFail,            // any non-pass aborts the phase
    Vote,               // N-of-M agreement with cross-family enforcement
}
```

### VerifyHook - how to gate retries

```rust
enum VerifyHook {
    RunCommand,         // exit code 0 = pass
    LLMVerifier,        // ask a model "does this answer the question?"
    TestRunner,         // parse `cargo test` JSON output
}
```

## Architecture

```
trigger          ─►  engine            ─►  transport          ─►  providers
(CLI / hook)         (loker)               (TensorZero +          (Anthropic /
                                            existing CLIs)         OpenAI / Google /
                                                                   Ollama / ...)
```

loker is provider-agnostic. It speaks to:
- **Subprocess backends** (`claude`, `codex`, `gemini`, `ollama`) - inherited from lok
- **HTTP backend** via [TensorZero](https://www.tensorzero.com/) gateway
  with ClickHouse-backed observability - new in M1

## Deployment (TensorZero Tier 2)

A one-command Docker Compose recipe for the TensorZero stack (gateway +
ClickHouse + UI) lives at [`deploy/tensorzero/`](deploy/tensorzero/).

```bash
cd deploy/tensorzero
cp ../../tensorzero/.env.example .env   # fill in OPENAI_API_KEY
docker compose up -d
```

See [`deploy/tensorzero/README.md`](deploy/tensorzero/README.md) for setup
instructions, service details, and how to run the integration test.

## What works today (inherited from lok)

```bash
cargo install --path .                 # build from source while M0 lands
loker doctor                           # check which backend CLIs are installed
loker ask "Find N+1 queries"           # query all backends
loker hunt .                           # multi-prompt bug hunt
loker audit .                          # security audit
loker diff main..HEAD                  # multi-backend code review
```

Workflows are TOML files under `.lok/workflows/` (config filename will move to
`loker.toml` / `.loker/workflows/` in a later milestone with a deprecation
window for both names).

## Roadmap

| M | Milestone | Test contract |
|---|-----------|---------------|
| M0 | Fork prep: rename, `genai = "0.6"` dep, attribution | `cargo test` green |
| M1 | TensorZero backend via `genai` crate | wiremock-based unit tests + opt-in integration |
| M2 | Strategy primitives (`SingleModel`, `ParallelFanOut`, `EscalatingRetry`) | unit tests with `MockBackend` |
| M3 | Aggregator vocabulary (`Concat`, `LLMJudge`, `AnyFail`, `Vote`) + family-overlap guard | property tests + family-conflict tests |
| M4 | Verify hooks (`RunCommand`, `LLMVerifier`, `TestRunner`) | per-hook unit tests against fixtures |
| M5 | Phase runner + `trace.jsonl` (OpenTelemetry GenAI semconv) | resumability test, schema test |
| M6 | `design-doc-tdd` reference workflow | end-to-end against local TensorZero |
| M7 | TensorZero deployment recipe (gateway + ClickHouse + UI, Tier 2) | smoke script with health/query/observe |
| M8 | CLI polish: `explain`, `resume`, `trace` pretty-printer | snapshot tests |
| M9 | Documentation, migration guide from lok | doc-tests on README samples |
| M10/M11 | HITL hook + browser UI - separate doc (`loker-hitl-design.md`) | depends on M4/M5 |

## Design documents

The actual decision-making lives in design docs, not this README:

- **[`loker-design.md`](https://github.com/maxkulish/investigations/blob/main/sakana-fugu/loker-design.md)** - the engine: primitives, TOML grammar, milestones, related work, references
- **`loker-hitl-design.md`** - human-in-the-loop hook and the localhost UI
- **`workflow-design.md`** - broader system context across trigger / engine / transport / providers

## Lineage

loker is a hard fork of [`ducks/lok`](https://github.com/ducks/lok), MIT
licensed, started in early 2026. Every commit before M0 in this repository's
history was authored by ducks. The fork preserves lok's command surface while
adding the orchestration primitives needed for cross-family aggregation and
escalating retry.

If you want lok's original direction (single-shot multi-backend queries, code
review, audit) without the orchestration-primitive layer, use upstream lok
directly.

## License

MIT - see [LICENSE](LICENSE). Copyright is held jointly: original work by
ducks, derivative work by Max Kulish.
