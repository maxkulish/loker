# mentis integration findings (task-kickoff + task-implement build-out)

Date: 2026-06-06
Status: 2 fixed on `fix/parallel-target-model-concat`, rest open
Context: built the full TensorZero-backed orchestrator flow in mentis
(`.loker/` there) and drove it end to end - toy smoke spec through
draft-PR creation. Every issue below was hit live; repro evidence in
mentis `runs/` and ClickHouse (`tensorzero.ChatInference`).

## Fixed on this branch

### 1. Parallel targets drop the backend model/function (FIXED: 4b868e1)

`build_phase_config` built fan-out targets with
`TargetSpec::new(backend_scheme(b))`, discarding the `<rest>` of
`tensorzero/<fn>` and `ollama/<model>`. Every TensorZero leg silently
hit `[backends.tensorzero].model` instead of the named function.
ClickHouse proof: a 2-leg probe (`loker_d1_anthropic` +
`loker_d1_google`) produced two `loker_d1_anthropic` inferences and
zero google. Cross-family review/vote guarantees were silently void
through TensorZero. Fix threads the rest via `TargetSpec::with_model`.

Still open for single/escalating: `PhaseRung` has no model slot, so
`ollama/<model>` in those strategies still routes to the config default.

### 2. Parallel aggregation defaulted to First, docs say concat (FIXED: 4b868e1)

`AggregatorName::First` returned the first listed leg's output as the
phase artefact. In a real review fan-out the first leg (claude) returned
a 124-byte skill-announcement fragment, which beat a full
APPROVE_WITH_SUGGESTIONS review from the TensorZero leg purely by list
position. Default is now `Concat` (matches the developer guide §6.2).
Cosmetic follow-up: the concat header template is `## {backend_id}` -
two tensorzero legs both render `## tensorzero`; include the
model/function in the label.

### 3. Codex backend returns the preamble, not the answer (FIXED: 74aaeca)

`parse_output` returned the FIRST `agent_message` item. Codex emits one
per assistant message - preambles first, the real answer last. A
99-second reconcile run produced "I'll audit from git evidence first…"
as its entire captured output. Fix keeps the last match.

## Open

### 4. Resume treats never-started phases as complete

A run that failed in phase 2 of 3 (only `markers/design.completed`
exists) resumes with "All phases already complete. Nothing to resume."
The planner only walks marker files; phases with no marker at all fall
through. Repro: any phase-based run that errors during a middle phase.
Compounds with `--rerun phase=` being a no-op for phase grammar
(CLO-327), so the only recovery from a mid-run failure is a full re-run.

### 5. Parallel join hangs when a leg dies while others are in flight

With the TensorZero gateway down (Docker died mid-run), the tz leg
failed fast with connection-refused while claude/codex/ollama were
mid-call. After all subprocesses exited, the runner sat parked forever:
18 threads idle, zero sockets, zero children, `review.started.0` only.
Looks like a lost-waker/join issue in the fan-out when early-error and
late-success legs mix. A clean `parallel floor violated` failure (which
the same setup produces when the gateway is down from the start) would
be correct.

### 6. Backend failures swallow stdout

`claude.rs` bails with `Claude CLI failed: {stderr}` - claude prints
errors like `API Error: 529 Overloaded` and
`API Error: socket connection closed unexpectedly` to STDOUT, so the
workflow error is empty. Same class: `-v` adds nothing for phase legs,
and a failed phase persists only `failure-summary.json`
(`error_class: strategy_failed`) - no prompt, no raw output. Diagnosis
required digging in `~/.claude/projects/<cwd-slug>/*.jsonl`. Include
captured stdout (tail) in backend errors and persist per-attempt
prompt/stdout on failure.

### 7. prompt_template path footgun - confirmed live

Exactly as diagnosed in
`docs/spikes/2026-05-18-prompt-template-path-footgun.md`: mentis's
original workflow passed `.tmpl` paths; claude received the literal
path string as its whole prompt (visible in `ps`). Inlined the
templates as the unblock. Option 1 (`prompt_template_file`) + Option 3
(path-shape validator) remain the right fix; the missing-placeholder
lint would also have caught it in one second.

### 8. Phase grammar silently ignores unknown keys

`Phase` lacks `deny_unknown_fields`; `verify = {...}` and `aggregator =
{...}` from the developer guide §6 parse fine, validate fine
(`loker workflow validate` also reports `Steps: 0` for phase files), and
do nothing. `loker explain` even prints `verify: none`. Until the
grammar grows those fields, reject unknown keys so authors find out at
validate time, and make `workflow validate` phase-aware.

### 9. Undocumented: tensorzero phases need [backends.tensorzero]

The developer guide documents the top-level `[tensorzero]` block, but
`resolve_phase_backends` resolves every scheme through
`config.backends`, so a phase using `tensorzero/<fn>` fails with
"Backend 'tensorzero' not configured" unless a `[backends.tensorzero]`
entry exists (`command` = endpoint URL, `model` = default function).
Document it (or fall back to the top-level block).

### 10. Deploy compose: gateway/ClickHouse startup race

First `docker compose up -d` on a cold stack: gateway exits 1
("ClickHouse is not healthy") because ClickHouse wasn't ready yet;
a second `up -d` fixes it. The healthcheck/depends_on ordering from
87021f8 doesn't cover the cold-pull first boot.

## Operational notes from the integration (no loker change needed)

- Headless `claude -p` inherits the user's skills and may answer with a
  skill announcement instead of the requested artefact - phase prompts
  must say "do not invoke skills or tools".
- claude transport flakes (529, socket reset) killed 2 of 4 long design
  runs; `[defaults] max_retries = 2` in the consumer's loker.toml wraps
  backends in RetryExecutor and absorbs them.
- macOS `/bin/sh` is bash 3.2: a heredoc nested inside `"$(...)"` dies
  on ASCII apostrophes in the body ("unexpected EOF while looking for
  matching `'"). Step shells must write prompt files with plain heredoc
  redirection and substitute with `$(cat file)`.
