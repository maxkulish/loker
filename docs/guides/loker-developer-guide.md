# loker developer guide

A self-contained reference for building flows with **loker** v0
(`v20260509.0.0`). Drop the absolute path of this file into a fresh
agent session ("read this guide and build a flow that does X") and the
session should have everything it needs to write a working `lok.toml`,
a workflow TOML, and run it end-to-end.

> Repo root: this guide assumes loker is cloned at the path you point
> the agent at, and `loker` is on `PATH` (`make install` or
> `cargo install --path .`). When in doubt, run `loker --version` —
> v0 reports `loker 20260509.0.0`.

---

## 1. What loker is

loker is a Rust LLM orchestration engine. It routes prompts across
multiple model families (Anthropic, Google, OpenAI, Zhipu, Ollama),
aggregates their responses, and verifies outputs before they ship — all
from a single declarative TOML workflow.

It's a **hard fork of [`ducks/lok`](https://github.com/ducks/lok)**.
It preserves lok's command surface (`ask`, `hunt`, `audit`, `diff`)
while adding the orchestration primitives:

1. **Cross-family aggregation** — the same prompt fans out to N
   backends in parallel and answers are merged with cross-family
   guards (a single family cannot win a vote alone).
2. **Escalating retry** — try a cheap model, verify, retry on a
   stronger model only if the verify hook fails.
3. **Verify hooks** — gate retries by exit code, by an LLM judge, or
   by parsed test output (`cargo test`, `pytest`).

If you want single-shot multi-backend queries with no orchestration,
use upstream lok. If you want a pipeline that fans out, votes, retries
on failure, traces every step, and pauses for human review when
configured — use loker.

### What ships in v0

| Surface | What works |
|---------|------------|
| Backends | TensorZero (HTTP gateway), subprocess CLIs (`claude`, `codex`, `gemini`, `ollama`), bedrock (feature-gated) |
| Strategies | `single`, `parallel` (with `min_responses`), `escalating` (with `pass_failure_context`) |
| Aggregators | `concat`, `llm_judge`, `any_fail`, `vote` (cross-family enforced) |
| Verify hooks | `RunCommand` (sandboxed), `LLMVerifier`, `TestRunner` (cargo / pytest), `HumanVerifier` |
| Workflow shapes | Step-based (`[[steps]]`) and phase-based (`[[phases]]`) |
| Run state | `runs/<workflow>-<ts>-<uuid>/` with `manifest.json` + `trace.jsonl` (OpenTelemetry GenAI semantic conventions) |
| HITL | Pending/response file pair with severity ladder, 60s heartbeat advisory lock |
| UI | localhost-only daemon (`loker ui --serve`) with SSE tail-f |

---

## 2. Quick orientation: the three primitives

Every workflow composes three (optionally four) primitives:

```
              ┌────────────┐         ┌────────────┐         ┌────────────┐
prompt ─────► │  Strategy  │ ──────► │ Aggregator │ ──────► │   output   │
              │ how to call│         │ how to merge│        │            │
              └─────┬──────┘         └────────────┘         └────────────┘
                    │
                    ▼
              ┌────────────┐
              │  Backends  │  one or more model endpoints
              │ (genai/sub)│
              └────────────┘
                    │
                    ▼ (optional, per-attempt)
              ┌────────────┐
              │ VerifyHook │  pass/fail gate that controls retries
              └────────────┘
```

- **Backend** — wraps a model family. Subprocess (CLI) or HTTP
  (TensorZero). Normalises auth, timeouts, errors, token tracking.
- **Strategy** — `single`, `parallel`, or `escalating`. Decides how
  many calls to issue and in what order.
- **Aggregator** — `concat`, `llm_judge`, `any_fail`, or `vote`.
  Decides how to fold N responses into one.
- **VerifyHook** — `RunCommand`, `LLMVerifier`, `TestRunner`, or
  `HumanVerifier`. A pass/fail gate that controls whether the
  strategy retries or commits.

---

## 3. Install and verify

```bash
git clone https://github.com/maxkulish/loker.git
cd loker

# Pick one:
make install               # cargo install --path .
cargo install --path .     # same thing, explicit
make release               # maintainer path: auto-version + tag + push + install to /usr/local/bin

# Verify
loker --version            # → loker 20260509.0.0
loker doctor               # check which backends are installed and reachable
```

`loker doctor` prints something like:

```
Lok Doctor
==================================================
Checking backends...
  ✓ codex - ready
  ✓ gemini - ready
  ✓ claude - ready

Checking API keys...
  ○ ANTHROPIC_API_KEY - not set (claude backend)
  ○ GOOGLE_API_KEY - not set (gemini backend)

Checking TensorZero gateway...
  ○ tensorzero - not configured
```

The CLI subprocess backends (`claude`, `codex`, `gemini`, `ollama`)
need their respective binaries on `PATH`. The TensorZero backend needs
the gateway running (see §8.1).

---

## 4. Configure: `loker.toml`

loker reads `loker.toml` from the working directory (or a path passed via
`-c/--config`). Initialise one with:

```bash
loker init                 # writes a starter loker.toml
```

> **Coexistence with lok.** loker uses its own namespace so it can run
> alongside lok in the same repo. If `loker.toml` is absent loker falls
> back to `lok.toml` (and `.loker/workflows/` falls back to
> `.lok/workflows/`). Repos that already use lok can opt into loker by
> creating `loker.toml` and `.loker/workflows/` without touching their
> existing lok config.

The full canonical config (from this repo's own `lok.toml`, written
under the legacy name for now):

```toml
[defaults]
parallel = true
timeout = 300

[conductor]
max_rounds = 5
max_tokens = 4096

[cache]
enabled = true
ttl_hours = 24

# ── Backends ────────────────────────────────────────────────────────
[backends.ollama]
enabled = true
command = "http://localhost:11434"   # HTTP URL → talks to Ollama API
args = []
skip_lines = 0
model = "qwen3-coder-next:latest"

[backends.gemini]
enabled = true
command = "npx"
args = ["@google/gemini-cli"]
skip_lines = 1                       # drop preamble lines from CLI output
timeout = 600

[backends.codex]
enabled = true
command = "codex"
args = ["exec", "--json", "-s", "read-only"]
skip_lines = 0

[backends.claude]
enabled = true
command = "claude"
args = []
skip_lines = 0

# ── TensorZero gateway (optional) ───────────────────────────────────
# [tensorzero]
# endpoint = "http://localhost:3000"
# default_model = "loker_judge_anthropic"
# api_key_env = "TENSORZERO_API_KEY"
# timeout_secs = 60
#
# [tensorzero.retry_policy]
# max_attempts = 3
# initial_backoff_ms = 250
# max_backoff_ms = 5000

# ── Predefined task templates (used by `loker audit`, `loker hunt`) ─
[tasks.audit]
description = "Security audit"
backends = ["gemini"]

[[tasks.audit.prompts]]
name = "injection"
prompt = "Search for injection vulnerabilities…"
```

Key conventions:

- A backend with `command = "http://…"` is treated as an HTTP target;
  anything else is a subprocess command.
- `skip_lines` strips N preamble lines from CLI output (e.g. Gemini's
  banner).
- TensorZero functions follow the **family suffix convention**:
  `loker_<purpose>_<family>` where `<family>` ∈
  `{anthropic, openai, google, zhipu, …}`. Cross-family guards
  (Vote, LLMJudge) derive the family from the suffix.

---

## 5. Writing a workflow

loker has **two workflow grammars** that share placeholder syntax and
templating. Choose one based on what you need.

| Grammar | Use when |
|---------|----------|
| **Step-based** (`[[steps]]`) | You want shell commands and single-backend prompts wired into a DAG. Closest to GitHub Actions. |
| **Phase-based** (`[[phases]]`) | You want strategy/aggregator/verify primitives. Closest to a multi-LLM pipeline DSL. |

Workflows live in:

- `.loker/workflows/<name>.toml` — primary lookup path; addressable by
  name (`loker run my-workflow`). `.lok/workflows/` is read as a
  fallback for repos forked from lok.
- Anywhere else — pass the path explicitly (`loker run ./flow.toml`).

### 5.1. Step-based: small DAG of shell + LLM calls

`examples/workflows/calculator-tutorial.toml` is the simplest example:

```toml
name = "calculator-tutorial"
description = "Read the calculator spec and echo its structure"

[[steps]]
name = "read_spec"
shell = "echo '--- Spec ---' && cat examples/specs/calculator.md"

[[steps]]
name = "count_sections"
shell = "echo 'Sections:' && grep '^## ' examples/specs/calculator.md"
depends_on = ["read_spec"]
```

Run it: `loker run calculator-tutorial`.

A step is one of two things:

| Step kind | Required keys | Optional keys |
|-----------|---------------|---------------|
| Shell step | `name`, `shell` | `depends_on`, `timeout`, `retries`, `continue_on_error`, `when`, `if` |
| Backend step | `name`, `backend`, `prompt` | `depends_on`, `timeout`, `retries`, `continue_on_error`, `when`, `if`, `apply_edits` |

Common step keys (from `examples/workflows/*.toml`):

- `depends_on = ["a", "b"]` — run after these steps.
- `timeout = 300000` — milliseconds.
- `retries = 3` — retry on failure.
- `continue_on_error = true` — failure does not abort the workflow.
- `when = "steps.x.success"` — small expression language; can use
  `success`, `not`, `and`, `or`.
- `if = 'contains(review.output, "\"approved\": true")'` — gate
  execution on string contents of an upstream output.
- `apply_edits = true` — when the backend returns an `edits` array, the
  runner applies them as `{file, old, new}` exact replacements.
- `[steps.validate]` (sub-table) — output validator; see §5.4.

#### Step placeholders

Inside `shell` or `prompt` strings:

| Placeholder | Resolves to |
|-------------|-------------|
| `{{ arg.1 }}`, `{{ arg.2 }}`, … | Positional CLI args (`loker run flow PR_NUM TASK_ID`) |
| `{{ var.name }}` | `--var name=value` from the CLI |
| `{{ spec }}` | Contents of `--spec FILE` |
| `{{ steps.foo.output }}` | Stdout of step `foo` |
| `{{ steps.foo.<field> }}` | If `foo`'s stdout was JSON, deep-access via dot path (e.g. `{{ steps.fix.summary }}`) |
| `{{ steps.foo.success }}` | Boolean for use in `when` |
| `{{ workflow.backends }}` | Comma-list of distinct backends used in the workflow |

Heredocs work natively. The runner expands templates **before** the
shell sees the body, so use a literal-quoted heredoc tag (`<<'EOF'`)
when the body contains `$` or backticks you do not want the shell to
expand. When the body itself contains template values you want
expanded, the heredoc tag is irrelevant — loker expands first.

### 5.2. Phase-based: strategy/aggregator/verify pipeline

`.lok/workflows/design-doc-tdd.toml` is the canonical reference:

```toml
name = "design-doc-tdd"
description = "Four-phase design → review → implement → verify pipeline."

[[phases]]
name = "design"
strategy = { single = {} }
backends = ["ollama/qwen3-coder-next"]
prompt_template = "../prompts/design-doc-tdd/design.md.tmpl"
inputs = ["spec"]
output = "design.md"

[[phases]]
name = "review"
strategy = { parallel = { min_responses = 2 } }
backends = ["claude/", "gemini/", "codex/", "ollama/qwen3-coder-next"]
prompt_template = "../prompts/design-doc-tdd/review.md.tmpl"
inputs = ["phase:design"]
output = "review.md"

[[phases]]
name = "implement"
strategy = { escalating = { pass_failure_context = true } }
backends = ["ollama/qwen3-coder-next", "claude/", "codex/"]
prompt_template = "../prompts/design-doc-tdd/implement.md.tmpl"
inputs = ["phase:design", "phase:review"]
output = "changes"

[[phases]]
name = "verify"
strategy = { parallel = { min_responses = 1 } }
backends = ["codex/", "gemini/"]
prompt_template = "../prompts/design-doc-tdd/verify.md.tmpl"
inputs = ["phase:implement", "phase:design"]
output = "verify.json"
```

Required phase keys:

- `name` — unique within the workflow.
- `strategy` — see §6.
- `backends` — list of `<family>/<model>` identifiers. Empty `<model>`
  (e.g. `"claude/"`) uses the backend's default model from `lok.toml`.
- `prompt_template` — path to a `.tmpl` file, relative to the
  workflow's directory.
- `inputs` — list of upstream artefacts. `"spec"` reads from
  `--spec FILE`; `"phase:<name>"` reads from a prior phase's `output`.
- `output` — file or directory under
  `runs/<id>/attempts/<phase>/<n>/`. Strings ending in `/` (or named
  `changes` by convention) are treated as directories.

Optional:

- `[phases.contract]` — forward-compat slot for hooks (e.g.
  `reviewer = "ollama/glm-5.1"` once grammar lands).
- `verify` — see §7.
- `aggregator` — see §6.3. Defaults are sensible: `single` →
  pass-through, `parallel` → `concat`, `escalating` → first pass wins.

#### Phase placeholders

Inside `prompt_template` files:

| Placeholder | Resolves to |
|-------------|-------------|
| `{{ spec }}` | Contents of `--spec FILE` |
| `{{ phase.<name>.output }}` | The full text content of the named phase's output artefact |
| `{{ phase.<name>.output.path }}` | The relative path of that artefact (for prompts that should reference but not embed) |
| `{{ var.<name> }}` | `--var name=value` from the CLI |

Templates are rendered by `src/workflow/template.rs`. A template file
lives anywhere; the canonical location is `.lok/prompts/<workflow>/`.

Example template (`.lok/prompts/design-doc-tdd/design.md.tmpl`):

```
You are a senior systems architect.

Read the specification below and produce a complete design document covering:
1. Problem statement
2. Goals and non-goals
3. Architecture (modules, data flow, concrete types)
4. Public API surface (Rust trait / struct signatures)
5. Test plan (unit, integration, manual)
6. Migration / rollout
7. Open questions

Output format: Markdown with numbered sections.

--- Specification ---

{{ spec }}
```

### 5.3. When to pick which grammar

| Situation | Grammar |
|-----------|---------|
| You need to chain shell commands with one or two LLM calls | step-based |
| You want a fan-out review with cross-family vote | phase-based |
| You want escalating retry with a verify hook | phase-based |
| You need GitHub-Actions-style conditional jobs | step-based |
| You want trace.jsonl with proper phase semantics | phase-based |

You **cannot** mix `[[steps]]` and `[[phases]]` in the same file.
Pick one.

### 5.4. Output validators (`[steps.validate]`)

A step can attach a validator that re-runs the output through a small
LLM and either passes or rewrites it. Used in the
`design-review` workflow:

```toml
[steps.validate]
check = "min_length(200)"      # or "json_schema(...)" etc.
backend = "claude"
model = "haiku"                # use a cheap model
replace_output = true          # if true, the validator's verdict replaces the step output
max_input_length = 100000
timeout_ms = 60000
on_error = "fail"              # or "pass"
on_parse_error = "fail"
prompt = """
{{ output }}

---

You are a review output validator. Produce ONE JSON object.
If valid: {"status": "pass"}
If garbage: {"status": "fail", "reason": "<one-liner>"}
"""
```

---

## 6. Strategy / aggregator / verify hook reference

### 6.1. Strategies

| Variant | TOML | Behaviour |
|---------|------|-----------|
| Single | `strategy = { single = {} }` | One backend, one call. Errors abort the phase. |
| Parallel | `strategy = { parallel = { min_responses = 2 } }` | Fans out to all `backends` simultaneously. Tolerates partial failure if at least `min_responses` succeed. |
| Escalating | `strategy = { escalating = { pass_failure_context = true } }` | Calls `backends[0]` first; if verify fails, retries on `backends[1]`, then `[2]`, …. With `pass_failure_context = true`, the next attempt receives the previous attempt's output and the verify failure reason. |

### 6.2. Aggregators

Default per strategy: single → pass-through, parallel → concat,
escalating → first-pass-wins. Override with an `aggregator` field on
the phase.

| Variant | TOML | Behaviour |
|---------|------|-----------|
| Concat | `aggregator = { concat = {} }` | Dump all responses with `--- backend: <id> ---` separators. Use for audit trails and debug. |
| LLMJudge | `aggregator = { llm_judge = { model = "loker_judge_anthropic" } }` | Sends all responses to a judge LLM that must be **a different family** from any responder. Cross-family guard is enforced at validation time. |
| AnyFail | `aggregator = { any_fail = {} }` | Any non-pass response aborts the phase. Used for verify gates where every reviewer must approve. |
| Vote | `aggregator = { vote = { quorum = 2, cross_family = true } }` | N-of-M agreement. With `cross_family = true`, at least one supporting voter must be from a different family than the leader. |

### 6.3. Verify hooks

Each phase can attach an optional verify hook. The strategy uses the
hook's pass/fail to decide whether to retry.

```toml
[[phases]]
name = "implement"
strategy = { escalating = { pass_failure_context = true } }
backends = ["ollama/qwen3-coder-next", "claude/", "codex/"]
verify = { run_command = { cmd = ["make", "check"], working_dir = "." } }
```

Variants:

| Variant | TOML | Pass condition |
|---------|------|----------------|
| RunCommand | `verify = { run_command = { cmd = ["make", "check"] } }` | Exit code 0. Sandbox-restricted (no network in CI). |
| LLMVerifier | `verify = { llm = { model = "loker_verifier_google", prompt_template = "verify.tmpl" } }` | Model returns the JSON `{"verdict": "pass"}`. |
| TestRunner | `verify = { tests = { runner = "cargo", path = "." } }` | Parses `cargo test` JSON output (or `pytest` with `runner = "pytest"`). All tests must pass. |
| HumanVerifier | `verify = { human = { severity = "medium" } }` | Pauses the run and writes `runs/<id>/pending/<phase>.json`. Resumes when `runs/<id>/responses/<phase>.json` is written. See §10. |

---

## 7. Backend catalogue

Backends are configured in `lok.toml` under `[backends.<id>]`. A
phase's `backends = [...]` list references them by `<id>/<model>`. An
empty model (`"claude/"`) uses the backend's `model` from `lok.toml`.

### 7.1. Subprocess CLI backends

Each subprocess backend wraps a CLI binary that emits text on stdout.

| ID | Binary | Notes |
|----|--------|-------|
| `claude` | `claude` (Anthropic CLI) | Streaming or `-p` text mode. |
| `codex` | `codex` | Run with `exec --json -s read-only` for sandbox safety. |
| `gemini` | `npx @google/gemini-cli` | Set `skip_lines = 1` to drop banner. |
| `ollama` | HTTP URL, e.g. `http://localhost:11434` | Set `model = "<name>:<tag>"`. |

The runner spawns the binary, pipes the rendered prompt to stdin (or
passes via `-p`/`--prompt` per CLI), captures stdout, applies
`skip_lines`, and returns. Per-call timeout via the backend's `timeout`
key (seconds) or the workflow's `timeout = <ms>` per step.

### 7.2. TensorZero (HTTP)

TensorZero is the supported HTTP backend; it sits in front of OpenAI,
Anthropic, Google, Zhipu, etc., and gives you observability +
ClickHouse-backed tracing.

- Local stack: `cd deploy/tensorzero && cp .env.example .env`, fill
  in `OPENAI_API_KEY` etc., then `docker compose up -d`. The gateway
  binds at `http://localhost:3000` (no path suffix).
- Function naming: `loker_<purpose>_<family>` where the suffix is the
  family the function actually calls. The cross-family guard (FR-13)
  derives the family from this suffix; pick wrong and the validator
  rejects the workflow.
- Config:
  ```toml
  [tensorzero]
  endpoint = "http://localhost:3000"
  default_model = "loker_judge_anthropic"
  api_key_env = "TENSORZERO_API_KEY"
  timeout_secs = 60

  [tensorzero.retry_policy]
  max_attempts = 3
  initial_backoff_ms = 250
  max_backoff_ms = 5000
  ```
- Integration tests are gated by `LOKER_TZ_INTEGRATION=1` and use
  `TENSORZERO_GATEWAY_URL` (root URL, no `/openai/v1` suffix).

### 7.3. Bedrock (feature-gated)

Compile with `--features bedrock`. Reads AWS credentials via the
standard chain (`AWS_PROFILE` / `AWS_ACCESS_KEY_ID` etc.). Treat as
experimental in v0.

---

## 8. CLI command catalogue

```
loker --version
loker --help
loker -c PATH …             # custom config path
loker -v …                  # verbose: show prompts, timing, debug
```

### 8.1. Workflow execution

| Command | Use |
|---------|-----|
| `loker run <name> [args…]` | Run a workflow. `name` is either the file under `.lok/workflows/` or a path to `*.toml`. Positional args become `{{ arg.1 }}`, `{{ arg.2 }}`. |
| `loker run <name> --spec FILE` | Inject `FILE` contents as `{{ spec }}`. |
| `loker run <name> --var k=v` | Set `{{ var.k }}`. Repeatable. |
| `loker run <name> --rerun phase=design` | Force re-execution of a completed phase even if its marker exists. Repeatable. |
| `loker run <name> --explain-validation` | Dump raw validator responses on parse failures. |
| `loker resume <run_id>` | Continue an interrupted run from the last completed-phase marker. `run_id` is a directory name under `runs/` or a path. |
| `loker resume <run_id> --ttl <secs>` | Override the heartbeat TTL (default: from the run's `heartbeat.json` or 300s). |
| `loker workflow list` | List discovered workflows in `.lok/workflows/`. |
| `loker workflow validate <path>` | Parse + schema-check without executing. |
| `loker explain <name>` | Print the DAG and phase summary for a workflow. |
| `loker explain <dir> -f <topic>` | Codebase explanation focused on a topic — uses backends. |
| `loker trace <run_id>` | Pretty-print `trace.jsonl`. Pass `--json` for raw JSONL. |
| `loker ls` | List recent runs (when invoked with no args, or filter by status — see `loker ls --help`). |

### 8.2. Single-shot LLM commands (lok-compatible)

| Command | Use |
|---------|-----|
| `loker ask <prompt>` | Query backends. `-b claude,codex` to restrict. `--no-cache` skips cache. |
| `loker smart <prompt> [--role <role>]` | Pick the best backend for the prompt via the team/role config. `--explain` shows resolution. |
| `loker suggest <task>` | Recommend a backend for the task (no LLM call). |
| `loker hunt [DIR] [--issues]` | Multi-prompt bug hunt. With `--issues` creates GitHub/GitLab issues. |
| `loker audit [DIR]` | Security audit (uses `[tasks.audit]` from `lok.toml`). |
| `loker diff <range>` | Multi-backend code review of a git range (`main..HEAD`). |
| `loker pr <num>` | Multi-backend PR review. |
| `loker spec <task>` | Generate ARF specs from a high-level description. |
| `loker implement <spec>` | Drive an implementation from an ARF spec. |
| `loker conduct <task>` | Multi-round conductor mode (Claude orchestrates other backends). |
| `loker debate <topic> [-o file.md]` | Multi-round debate transcript. |
| `loker context [--explain]` | Show resolved context (config + role + team). |
| `loker team` | Team / role management (see `loker team --help`). |
| `loker spawn <agent>` | Spawn a worktree-isolated agent. |

### 8.3. Operational

| Command | Use |
|---------|-----|
| `loker doctor` | Health-check backends, API keys, gateway. |
| `loker backends` | List configured backends and their state. |
| `loker init` | Write a starter `lok.toml`. |
| `loker report [--pr N]` | Generate an agent-history report from `.agent/` worktree. |
| `loker ui --serve [--bind 127.0.0.1:8080]` | Start the localhost UI daemon. |

---

## 9. Run directory layout

Every `loker run` creates a directory under `runs/` that is the
single source of truth for the run.

```
runs/<workflow>-<utc-timestamp>-<short-uuid>/
├── manifest.json              # artefact inventory + sha256 hashes
├── trace.jsonl                # one OTel-shaped span per phase / backend / verify
├── heartbeat.json             # advisory lock for resume (60s TTL)
├── markers/
│   └── phase-<name>.json      # completed-phase marker (for resume)
├── pending/                   # HITL pending requests
│   └── <phase>.json
├── responses/                 # HITL human responses
│   └── <phase>.json
└── attempts/
    └── <phase-name>/
        ├── 1/
        │   ├── prompt.txt     # rendered prompt
        │   ├── output         # phase output (file or dir)
        │   ├── stdout         # raw backend stdout
        │   └── verify.json    # verify hook result, if any
        └── 2/                 # retry attempt
            └── …
```

Atomic write protocol: every artefact is written to a `.tmp` sibling
and renamed; phase completion is signalled by writing
`markers/phase-<name>.json`. This is what `loker resume` walks.

`trace.jsonl` follows the OpenTelemetry GenAI semantic conventions
with custom fields under `loker.*` (phase, attempt, backend, verify
verdict). Inspect with `loker trace <run_id>` or pipe to `jq`:

```bash
jq -c 'select(.name == "phase.review") | {phase: .attributes."loker.phase", verdict: .attributes."loker.verdict"}' \
   runs/design-doc-tdd-20260509T120000Z-abc12345/trace.jsonl
```

---

## 10. Human-in-the-loop (HITL)

Attach `verify = { human = { severity = "<level>" } }` to a phase. On
trigger, loker writes `runs/<id>/pending/<phase>.json` and pauses the
run. A human (or automation) responds by writing
`runs/<id>/responses/<phase>.json` with a verdict. The run resumes
on the next `loker resume <id>` (or automatically if the heartbeat
is still live).

Severity ladder:

| Severity | Auto-timeout | Use for |
|----------|--------------|---------|
| `low` | 1 hour | Style nits, optional reviews |
| `medium` | 24 hours | Default for design / review approvals |
| `high` | infinite | Production cutovers, irreversible changes |

The 60-second heartbeat advisory lock prevents two `loker resume`
calls from racing on the same run.

---

## 11. UI daemon

```bash
loker ui --serve                      # http://127.0.0.1:8080
loker ui --serve --bind 127.0.0.1:9000
```

Localhost-only by design (binding non-loopback is rejected). Streams
`trace.jsonl` via SSE using `notify` for tail-f semantics. Useful for
watching a long fan-out without parsing JSONL by hand.

---

## 12. Recipes

These are the patterns most agents will need to assemble for a target
repo. Pick one and adapt.

### 12.1. "Review my PR with three families and post a synthesis"

Drop this into `.lok/workflows/review-pr.toml` in the target repo,
then `loker run review-pr <PR_NUM>`. Full reference at
`examples/workflows/review-pr.toml` in this repo. Shape:

```toml
name = "review-pr"

[[steps]]
name = "fetch-meta"
shell = "gh pr view {{ arg.1 }} --json number,title,additions,deletions,changedFiles"

[[steps]]
name = "fetch-diff"
shell = "gh pr diff {{ arg.1 }}"

[[steps]]
name = "review_claude"
backend = "claude"
depends_on = ["fetch-meta", "fetch-diff"]
prompt = """ … {{ steps.fetch-diff.output }} … """

# repeat for review_codex, review_gemini in parallel

[[steps]]
name = "synthesize"
backend = "claude"
depends_on = ["review_claude", "review_codex", "review_gemini"]
prompt = """ JSON-shaped consensus … """

[[steps]]
name = "comment"
depends_on = ["synthesize"]
shell = "gh pr comment {{ arg.1 }} --body \"$(cat <<'EOF'\n{{ steps.synthesize.summary }}\nEOF\n)\""
```

### 12.2. "Hunt for a bug, propose a fix, open an issue"

`examples/workflows/full-heal.toml` is the autonomous loop:
hunt → pick → issue → fix (with `apply_edits = true`) → branch →
commit → push → PR → review → conditional merge. Copy that file into
the target repo's `.lok/workflows/` and adjust prompts.

### 12.3. "Design → review → implement → verify pipeline"

Phase-based. Use `.lok/workflows/design-doc-tdd.toml` from this repo
(quoted in §5.2) as the template. The `verify` phase can swap in
`run_command = { cmd = ["make", "check"] }` for repos with a
`Makefile`, `["pytest"]` for Python, `["cargo", "test"]` for Rust.

### 12.4. "Cross-family vote with judge"

```toml
[[phases]]
name = "answer"
strategy = { parallel = { min_responses = 2 } }
backends = [
  "tensorzero/loker_answer_anthropic",
  "tensorzero/loker_answer_openai",
  "tensorzero/loker_answer_google",
]
prompt_template = "../prompts/answer.md.tmpl"
inputs = ["spec"]
output = "answer.md"
aggregator = { vote = { quorum = 2, cross_family = true } }
```

The `cross_family = true` flag means the winning quorum cannot all
come from one family — at least one supporter must be a different
family. Family is derived from the function-name suffix
(`anthropic`, `openai`, `google`, `zhipu`).

### 12.5. "Cheap → strong escalation with verify"

```toml
[[phases]]
name = "patch"
strategy = { escalating = { pass_failure_context = true } }
backends = [
  "ollama/qwen3-coder-next",   # cheap, local
  "claude/sonnet",              # medium
  "claude/opus",                # strong
]
prompt_template = "../prompts/patch.md.tmpl"
inputs = ["phase:design"]
output = "changes"
verify = { run_command = { cmd = ["make", "check"] } }
```

The runner stops at the first attempt whose verify returns 0. Each
retry receives the previous attempt's output plus a structured
failure-context block.

---

## 13. Building a flow in a fresh repository (the meta-recipe)

This is the workflow an agent should follow when handed
*"build a flow in this repo"*:

1. **Confirm prerequisites in the target repo.**
   - `loker --version` → expect `loker 20260509.0.0` (or newer).
   - `loker doctor` → at least one ready backend.
   - If TensorZero is required: `cd deploy/tensorzero && docker compose up -d` in the loker checkout, then ensure the target repo's `lok.toml` has the `[tensorzero]` block above.

2. **Create or update `lok.toml` in the target repo.**
   - Copy the §4 template; trim to the backends you actually need.
   - For predefined `loker hunt` / `loker audit` prompts, copy the
     `[tasks.*]` blocks too.

3. **Decide grammar.**
   - Step-based for "shell + LLM" automation (PR review, issue
     triage, fix-and-merge).
   - Phase-based for "fan-out + verify" pipelines (design-doc TDD,
     cross-family vote, escalating retry).

4. **Write the workflow file.**
   - Place it under `.lok/workflows/<name>.toml`.
   - Reference templates from `.lok/prompts/<name>/*.tmpl` if
     phase-based; inline `prompt = """…"""` if step-based.
   - Validate: `loker workflow validate .lok/workflows/<name>.toml`.

5. **Dry-run the DAG.**
   - `loker explain <name>` — prints the phases, backends, inputs,
     outputs. Catches typos before any LLM call.

6. **Execute.**
   - `loker run <name> [args…] [--spec FILE] [--var k=v]`.
   - Watch progress: `loker ui --serve` in another terminal, point
     a browser at `http://127.0.0.1:8080`.

7. **Inspect the run.**
   - `runs/<name>-<ts>-<uuid>/manifest.json` — artefact list with
     hashes.
   - `runs/<name>-<ts>-<uuid>/trace.jsonl` — every phase, backend,
     and verify call as an OTel span. Use `loker trace <id>` for a
     readable view.
   - `runs/<name>-<ts>-<uuid>/attempts/<phase>/<n>/` — the actual
     prompt sent and the raw output received.

8. **Resume on failure / paused state.**
   - `loker resume <id>` walks `markers/` and re-enters from the
     last completed phase.
   - For HITL pauses, write the response file as in §10.

9. **Iterate.**
   - To force a phase to re-run: `loker run <name> --rerun phase=<name> [args…]`.
   - To pin a different backend without editing the workflow: edit
     `lok.toml` (`[backends.<id>]`) — but prefer explicit
     `<family>/<model>` strings in the workflow for reproducibility.

---

## 14. Troubleshooting

| Symptom | Likely cause | Where to look |
|---------|--------------|---------------|
| `loker doctor` shows a backend ✗ | Binary not on `PATH`, or HTTP backend unreachable | `which claude`, `which codex`, `curl http://localhost:11434` |
| `loker run` hangs on a phase | LLM call blocked / no timeout set | Check the backend's per-step `timeout` (ms) |
| Validator rejects a workflow | Bad `prompt_template` path, missing `inputs`, family mismatch | Run `loker workflow validate <path>`; pass `--explain-validation` to `loker run` |
| Phase output is empty | Backend returned no stdout, or `skip_lines` ate the whole response | Inspect `runs/<id>/attempts/<phase>/1/stdout` |
| Cross-family vote always fails | All backends derived to the same family from their function-name suffixes | Rename TensorZero functions to follow `loker_<purpose>_<family>` |
| Resume thinks the run is already in progress | Stale `heartbeat.json` | Wait 60s + retry, or pass `--ttl <secs>` to `loker resume` |
| TensorZero integration tests skipped | Env not set | `LOKER_TZ_INTEGRATION=1 TENSORZERO_GATEWAY_URL=http://localhost:3000 cargo test` |
| `make check` fails before commit | Pre-merge gate (fmt + clippy + test) | Read the failing line; `cargo fmt`, `cargo clippy --fix`, run failing test alone |

---

## 15. Cross-references

Authoritative sources in this repo:

- [`README.md`](../../README.md) — primitives overview, milestone snapshot.
- [`docs/handoff.md`](../handoff.md) — project WHY/Intent/HOW, constraints, conventions.
- [`docs/plans/001-implementation-roadmap.md`](../plans/001-implementation-roadmap.md) — canonical task list, milestone status.
- [`docs/migration-from-lok.md`](../migration-from-lok.md) — diff vs upstream lok.
- [`docs/tutorial.md`](../tutorial.md) — full end-to-end run walkthrough.
- [`docs/prd/2026-04-25-loker.md`](../prd/2026-04-25-loker.md) — milestone PRD; §6 is the v0 out-of-scope list.
- [`.lok/workflows/`](../../.lok/workflows/) — the in-repo workflows used to build loker itself.
- [`examples/workflows/`](../../examples/workflows/) — copy-pasteable starting points.
- [`deploy/tensorzero/`](../../deploy/tensorzero/) — Docker Compose for the TensorZero gateway + ClickHouse + UI.

External:

- [loker-design.md](https://github.com/maxkulish/investigations/blob/main/sakana-fugu/loker-design.md) — full design doc with grammar, milestones, and the FR list referenced throughout this guide.
