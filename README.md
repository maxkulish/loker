# loker — LLM orchestration engine

Routes prompts across multiple model families, aggregates their responses,
and verifies outputs before they ship — all from a single declarative TOML
workflow.

![status](https://img.shields.io/badge/status-pre--v0-yellow)
[![MIT License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

---

## What is loker?

loker is a Rust LLM orchestration engine that fills two gaps in the current
ecosystem: **cross-family aggregation** (sending the same prompt to
Anthropic, Google, OpenAI, and Zhipu in parallel, then merging answers to
reduce correlated failures) and **escalating retry** (try a cheap model →
verify → retry on a stronger model if verification fails). Both are shipped
primitives, not aspirational features.

loker is a **hard fork** of [`ducks/lok`](https://github.com/ducks/lok) that
preserves lok's command surface (`ask`, `hunt`, `audit`, `diff`) while adding
the orchestration layer. If you want single-shot multi-backend queries
without orchestration primitives, use upstream lok directly.

---

## Three primitives

Every loker workflow composes three primitives:

### Backend — talk to models

loker speaks to LLMs through **backends**. Each backend wraps a model family
(Anthropic, OpenAI, Google, Ollama, Zhipu) and normalises authentication,
timeouts, error handling, and token tracking. Backends can run as
subprocesses (CLI tools like `claude`, `codex`, `gemini`, `ollama`) or
through an HTTP gateway ([TensorZero](https://www.tensorzero.com/) with
ClickHouse observability).

### Strategy — how to call backends

- **SingleModel** — one backend, one call.
- **ParallelFanOut** — N backends in parallel, partial-failure tolerant.
- **EscalatingRetry** — cheap → medium → strong, stopping at the first
  verify-pass.

Shipped in M2.

### Aggregator — how to merge results

- **Concat** — dump all responses (debug, audit trails).
- **LLMJudge** — use an LLM (different family) to merge.
- **AnyFail** — any non-pass aborts the phase.
- **Vote** — N-of-M agreement with cross-family enforcement.

Shipped in M3.

### VerifyHook — how to gate retries

- **RunCommand** — exit code 0 = pass.
- **LLMVerifier** — ask a model "does this answer the question?"
- **TestRunner** — parse `cargo test` or `pytest` JSON output.

Shipped in M4.

---

## What works today

```bash
loker doctor               # check which backends are installed
loker ask "Find N+1"       # query all backends
loker hunt .               # multi-prompt bug hunt
loker audit .              # security audit
loker diff main..HEAD      # multi-backend code review
loker run <wf> --spec <f>  # run a workflow (--var, --rerun)
loker resume <run_id>      # resume a paused run
loker explain <wf>         # show DAG and phase summary
loker trace <run_id>       # pretty-print the trace
```

## Install

```bash
git clone https://github.com/maxkulish/loker.git
cd loker
make release          # auto-versions, builds, installs to /usr/local/bin
```

Once published on crates.io: `cargo install loker`.

> **Note:** loker is pre-v0. `make release` is the maintainer path (auto-versions, tags, installs to `/usr/local/bin`).
> For a local build: `make install` or `cargo install --path .`.

## One-page example

This runs `loker explain design-doc-tdd` to show the four-phase workflow
structure — no backends needed.

```bash
loker explain design-doc-tdd
```

**Expected output:**

The explain command prints a DAG of the four phases with their strategies,
backends, inputs, and outputs:

```text
loker explain design-doc-tdd

Design-doc-tdd workflow (4 phases):

  design (ollama/qwen3-coder-next)
    ↓ design.md
  review (claude/ + gemini/ + codex/ + ollama/qwen3-coder-next)
    ↓ review.md
  implement (ollama/qwen3-coder-next → claude/ → codex/)
    ↓ changes/
  verify (codex/ + gemini/)
    ↓ verify.json
```

Each run lands in `runs/<workflow>-<timestamp>-<id>/` with:
- `trace.jsonl` — spans for each phase, backend, and verify step
- `manifest.json` — artefact inventory with sha256 hashes
- Per-phase artefacts under `attempts/<phase>/<n>/`

Workflows are TOML files under `.lok/workflows/`. See
[`.lok/workflows/design-doc-tdd.toml`](.lok/workflows/design-doc-tdd.toml)
for the full definition.

> For a full end-to-end run example, see the **[Tutorial](docs/tutorial.md)**.

---

## Architecture

```
trigger          ─►  engine            ─►  transport          ─►  providers
(CLI / hook)         (loker)               (TensorZero +          (Anthropic /
                                            subprocess CLIs)       OpenAI / Google /
                                                                   Ollama / ...)
```

loker is provider-agnostic. It speaks to subprocess backends (`claude`,
`codex`, `gemini`, `ollama`) and an HTTP backend via TensorZero gateway
with ClickHouse-backed observability.

---

## Design docs & roadmap

Migrating from lok? See [docs/migration-from-lok.md](docs/migration-from-lok.md).

For depth, read:

- **[`docs/handoff.md`](docs/handoff.md)** — project WHY, intent, constraints
- **[`docs/plans/001-implementation-roadmap.md`](docs/plans/001-implementation-roadmap.md)** — canonical task list with dependency tracking
- **[loker-design.md](https://github.com/maxkulish/investigations/blob/main/sakana-fugu/loker-design.md)** — full design: primitives, TOML grammar, milestones
- **[`docs/prds/`](docs/prds/)** — PRD files per milestone
- **[`docs/discovery/`](docs/discovery/)** — discovery reports per task
- **[`docs/designs/`](docs/designs/)** — design documents per task
- **[`deploy/tensorzero/`](deploy/tensorzero/)** — Docker Compose for TensorZero gateway + ClickHouse + UI

## Milestone snapshot

| M | Milestone | Status |
|---|-----------|--------|
| M0 | Fork prep: rename, deps, attribution | ✅ |
| M1 | TensorZero backend via `genai` crate | ✅ |
| M2 | Strategy primitives | ✅ |
| M3 | Aggregator vocabulary | ✅ |
| M4 | Verify hooks | ✅ |
| M5 | Phase runner + `trace.jsonl` | ✅ |
| M6 | `design-doc-tdd` reference workflow | ✅ |
| M7 | TensorZero deployment recipe | ✅ |
| M8 | CLI polish: `explain`, `resume`, `trace` | ✅ |
| M9 | Documentation, migration guide | **← In progress** |
| M10 | HITL hook | Next |
| M11 | Browser UI | Future |

---

## License

MIT — see [LICENSE](LICENSE). Copyright held jointly: original work by
ducks, derivative work by Max Kulish.
