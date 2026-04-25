# Atomic run-state write protocol (D3)

**Status**: decided. Closes PRD §11 D3, blocks T-024 (manifest writer),
T-025 (phase status markers), T-031 (resumability).
**Scope**: how loker writes everything under `runs/<id>/` so any crash
leaves the run resumable without manual cleanup. Read this once;
T-024/T-025/T-031 reference it instead of relitigating.

## Why

FR-21 mandates atomic per-phase status markers. FR-23b - FR-23e mandate
a manifest plus an `attempts/<phase>/<n>/` archive for failed retries.
Crash-injection tests (FR-23c) demand a single, well-defined order so
the reader can tell "completed" apart from "torn mid-write" without
guessing. Without a written protocol, every call site (manifest writer,
markers, per-phase artefacts, HITL pending/responses, summary) would
re-derive its own ordering and we would discover divergence the hard
way - in a half-resumed run.

## Run directory layout (recap from FR-22)

```
runs/<workflow>-<timestamp>-<short-uuid>/
├── manifest.json              # canonical artefact index (full rewrite via tmp+rename)
├── trace.jsonl                # append-only OTel GenAI events (D2 / T-002)
├── markers/
│   ├── <phase>.started        # one per phase that began
│   ├── <phase>.completed      # one per phase that produced a verified artefact
│   └── <phase>.failed         # one per phase that exhausted retries
├── heartbeat.json             # writer pid + monotonic clock tick (refreshed periodically)
├── <phase>/                   # canonical artefacts produced by the phase
│   └── <artefact>             # e.g. design/design.md, verify/verify.json
├── attempts/<phase>/<n>/      # debris from failed attempts (FR-23e)
│   └── <artefact>             # not loaded by downstream phases
└── pending/, responses/       # HITL only (M10 / D4)
```

Phase names are stable identifiers from the workflow definition
(`design`, `review`, `verify`, ...). `<n>` is a 1-indexed attempt
counter that increments per failure within the same phase invocation.

## Decision: tmp + rename + status marker (canonical at top-level)

Two candidates were considered, both compatible with FR-21 and FR-23e:

**A. tmp + rename + status marker.** Canonical artefact lives at
`<phase>/<artefact>`. Each write is `<artefact>.<rand>.tmp` -> fsync
file -> rename to canonical -> fsync parent dir. `phase.completed`
written *after* artefact and manifest entry are durable. Failed
attempts copied to `attempts/<phase>/<n>/` for postmortem.

**B. Attempt-directory canonical.** All attempts live under
`attempts/<phase>/<n>/`. Canonical is a symlink (or pointer file)
`<phase>/current` swung to the latest successful attempt directory.
Markers live next to the pointer.

We pick **A**. Trade-offs:

| Concern | A (chosen) | B (rejected) |
|---|---|---|
| Reader path | Stable: `runs/<id>/<phase>/<artefact>` | Two hops: read `current` then path-build |
| Manifest entries | Path field is canonical and stable | Path must include `<n>` or be re-resolved per read |
| Crash-mid-write detection | Hash check on canonical + missing marker | Pointer race window: pointer swung but `phase.completed` not yet written |
| Portability | Plain `rename(2)`; no symlink at all | Symlinks are unprivileged-blocked on Windows; pointer-file workaround adds indirection |
| File watch (`notify`, M5+ SSE) | Single canonical path per artefact | Watcher fires on every attempt write; UI must filter by current pointer |
| History inspection | `attempts/` dir always there | Native to layout, no extra dir |
| Concurrent writers (same artefact) | rename(2) is atomic; loser visible only via tmp filename collision (random suffix) | Pointer swing race; needs lock or compare-and-swap on pointer file |

A's only concession is that retry history lives in a sibling `attempts/`
dir rather than as the canonical layout itself. We accept that: the
canonical path is the load-bearing read path, and stable plain-file
semantics matter more than retro-aesthetic of "everything is an
attempt". B's pointer indirection breaks the simple `cat
runs/<id>/<phase>/<artefact>` mental model that operators will reach
for in production.

## Write protocol

### Atomic file commit primitive

For any file that must be readable as "either the old contents or the
new contents, never torn":

1. Open `<final>.<rand64>.tmp` in the **same directory** as `<final>`
   (rename across directories is not atomic on most POSIX filesystems).
2. Write payload.
3. `fsync(file_fd)` - commit data and metadata to disk.
4. `rename(<tmp>, <final>)` - POSIX-atomic replace.
5. `fsync(parent_dir_fd)` - commit the directory entry update.

In Rust, `tempfile::NamedTempFile::new_in(dir)` + `.persist(<final>)`
implements (1)-(4) portably. Step 5 needs a manual
`File::open(parent).sync_all()` because `tempfile` does not fsync the
directory.

The random suffix (64 bits is enough) prevents collisions when two
processes - or a previously-crashed writer's leftover - attempt the
same final path. Reader sweeps any `*.tmp` files older than the
heartbeat TTL on resume (see "Stale tmp" below).

### Phase markers

Markers are tiny JSON files written via the atomic-commit primitive
into `runs/<id>/markers/`.

`<phase>.started`:

```json
{
  "phase": "design",
  "attempt": 1,
  "started_at": "2026-04-25T20:45:00Z",
  "writer_pid": 12345,
  "writer_host": "loker-runner-3",
  "heartbeat_ttl_seconds": 300
}
```

`<phase>.completed`:

```json
{
  "phase": "design",
  "attempt": 1,
  "completed_at": "2026-04-25T20:48:13Z",
  "manifest_entry_sha256": "ab12...",
  "artefact_paths": ["design/design.md"]
}
```

`<phase>.failed`:

```json
{
  "phase": "design",
  "attempts_made": 3,
  "failed_at": "2026-04-25T20:51:02Z",
  "error_class": "BackendTimeout",
  "last_attempt_path": "attempts/design/3/"
}
```

### Per-phase commit order

For each successful attempt of a phase, the writer MUST execute steps
in this order and complete each step's fsync before starting the next:

1. **Started**: write `markers/<phase>.started` (only on attempt 1; on
   retry the original started marker is reused, only `attempt` count
   in the body would change - but the body is not rewritten - the
   reader uses `attempts_made` from `<phase>.failed` if present, else
   the manifest entries to count attempts).
2. **Artefact**: write each artefact file via the atomic-commit
   primitive into `<phase>/<artefact>`.
3. **Manifest**: rewrite `manifest.json` via the atomic-commit
   primitive with the new entries appended logically (full rewrite
   physically).
4. **Completed**: write `markers/<phase>.completed` via the
   atomic-commit primitive.

**Critical invariant**: `<phase>.completed` is the single source of
truth for "phase done". Its presence implies (2) and (3) are durable.
Its absence implies the phase MUST be rerun, even if (2) and (3)
appear complete on disk.

### Manifest rewrite

The manifest is "append-only" in the sense that entries are only ever
added, never mutated or deleted. Physically, each phase commit
rewrites the entire file via tmp+rename. This is intentional:
line-append-with-fsync on JSON text invites partial-line corruption
on power loss (the OS may flush the rename but not the appended
bytes). The manifest is small enough (low hundreds of entries even
for large workflows) that full rewrite is cheap and avoids a class of
bugs.

Each entry is content-addressed by sha256 of the artefact bytes
(FR-23b/c). Resumption verifies hash before trusting the entry. A
mismatch (artefact tampered after marker written) surfaces as
`PhaseError::ArtefactSchemaMismatch` per FR-23d.

### Failed-attempt archive

On phase failure (retry budget exhausted, unrecoverable error):

1. Move (`rename`, same filesystem) the in-flight `<phase>/<artefact>`
   files - if any survived past their tmp - into
   `attempts/<phase>/<n>/<artefact>`.
2. Move (or copy + remove) any `<phase>/*.tmp` debris into the same
   `attempts/<phase>/<n>/` directory for postmortem.
3. Write `markers/<phase>.failed` via the atomic-commit primitive.

Archival is best-effort: failure to archive must not block the
`phase.failed` marker. If archival itself fails, log a warning to
`trace.jsonl` and proceed.

### Heartbeat

`heartbeat.json` is rewritten via the atomic-commit primitive every
`heartbeat_ttl_seconds / 3` (default 100s for a 300s TTL) by the
active writer. Body:

```json
{ "writer_pid": 12345, "writer_host": "loker-runner-3", "tick_at": "..." }
```

Stale heartbeats let the reader distinguish "writer is alive,
phase-in-progress" from "writer died, phase needs rerun" without
process-discovery RPC. TTL is conservative (5 min default) - tighter
TTLs trade false-positive reruns for faster crash recovery; the
default favors no-spurious-rerun.

## Read protocol

### Resume walk

For each phase declared in the workflow, in dependency order:

1. **`markers/<phase>.completed` present**:
   - Load each `artefact_paths` entry via the manifest.
   - sha256-verify against the manifest entry.
   - On mismatch: log `ArtefactSchemaMismatch`, treat phase as failed,
     rerun. (The completion marker plus a tampered artefact means
     someone edited files between runs; we do not silently trust
     edits.)
   - On match: skip phase, advance to next.
2. **`markers/<phase>.failed` present, no completed**:
   - Read `attempts_made` from the failed marker; rerun starting from
     attempt `attempts_made + 1`.
3. **`markers/<phase>.started` present, no completed, no failed**:
   - Compare `heartbeat.json` `tick_at` to `now()`.
   - If `now - tick_at < heartbeat_ttl_seconds`: another writer holds
     the run. Abort the resume with a clear error, do not start a
     parallel writer.
   - Else (stale): treat as failed (writer died mid-phase). Move any
     `<phase>/<artefact>` and `<phase>/*.tmp` to
     `attempts/<phase>/<n>/`. Rerun from attempt `n+1`.
4. **No markers for `<phase>`**: phase is fresh; run normally.

### Stale tmp sweep

On resume, before any phase work begins, sweep every
`runs/<id>/**/*.tmp` whose mtime is older than `heartbeat_ttl_seconds`.
Move into `attempts/_orphan_tmp/<isoformat>/` rather than deleting -
crash-debugging value outweighs disk cost.

### Hash verification

Manifest entries carry `sha256` and `schema_version`. Readers MUST
verify both:

- sha256 mismatch -> `ArtefactSchemaMismatch` (treat as failed).
- schema_version mismatch -> `ArtefactSchemaMismatch` (FR-23d).

Hash verification on every resume is cheap (small artefacts) and
catches both filesystem corruption and manual edits.

## Fault-injection test plan

The protocol is only as strong as the tests. T-031 (resumability)
must include the following kill matrix. Each row is one test: kill the
writer at the named transition, then resume and assert the expected
reader behaviour.

| # | Kill point | On-disk state after kill | Expected reader behaviour |
|---|---|---|---|
| 1 | Before `phase.started` write | No markers for phase | Treat as fresh; run from attempt 1 |
| 2 | Mid-write of `phase.started` (between tmp write and rename) | `<phase>.started.<rand>.tmp` only | Sweep tmp on resume; treat as fresh |
| 3 | After `phase.started` rename, before any artefact tmp opened | `<phase>.started`; heartbeat fresh | If heartbeat stale: rerun from attempt 1 (no archive needed). If fresh: refuse parallel writer |
| 4 | Mid-write of artefact (tmp present, not fsynced) | `<phase>.started`; `<phase>/<artefact>.<rand>.tmp` | Heartbeat stale -> archive tmp, rerun from attempt 2 |
| 5 | After artefact fsync, before rename | Same as (4) | Same as (4) |
| 6 | After artefact rename, before parent-dir fsync | `<phase>/<artefact>` may exist in page cache; not durably visible after power loss | After power loss: equivalent to (4). After process kill (no power loss): equivalent to (7) |
| 7 | After artefact rename + parent fsync, before manifest rewrite | `<phase>/<artefact>` durable; manifest does not reference it | Resume: `<phase>.completed` absent -> archive artefact to attempts, rerun. The orphan artefact has no manifest entry, so it is invisible to downstream phases either way |
| 8 | Mid-manifest rewrite (`manifest.json.tmp` present) | Old `manifest.json` intact; new tmp present | Sweep tmp; manifest is the old one; rerun phase |
| 9 | After manifest rename, before `phase.completed` rewrite | Manifest references new artefact; no completion marker | Rerun phase. The manifest entry is a leak (orphan entry referencing artefact at canonical path). On rerun success, manifest gets rewritten with the new attempt's entry, leaking the prior orphan. **Mitigation**: on resume, drop manifest entries for which no `<phase>.completed` marker references their sha256. T-024 must implement this sweep |
| 10 | Mid-write of `phase.completed` | `phase.completed.<rand>.tmp` present | Sweep tmp; rerun phase per (9) |
| 11 | After `phase.completed` rename, before parent-dir fsync | Marker may not survive power loss | Power loss: equivalent to (9). Process kill: equivalent to (12) |
| 12 | After `phase.completed` durable | Phase done | Skip phase; load via manifest with hash verify |
| 13 | After `phase.failed` durable | No partial state in `<phase>/` (archived) | Rerun from attempt `attempts_made + 1` |
| 14 | Two writers race on the same run (concurrent `loker run`) | Both see no completion marker; both write `<phase>.started.<rand>.tmp` | Whichever rename wins becomes `<phase>.started`; loser's started.tmp is sweep debris. Reader-side: heartbeat-fresh check makes second writer abort before doing artefact work. **Reinforced** by an advisory file lock on `runs/<id>/.lock` taken by the writer at start (out of scope for this doc; T-031 owns) |

Each row maps to one test in `tests/run_state_crash.rs` (to be created
under T-031). Crash injection uses `std::process::abort` from a test
hook installed at the named transition - no real signal delivery
needed.

## Open items deferred to downstream tasks

- **T-024 manifest writer**: implement the orphan-entry sweep
  described in row 9 of the kill matrix.
- **T-025 phase markers**: implement the started/completed/failed
  marker writers and the heartbeat refresher; expose attempt-counter
  state to retry strategies.
- **T-031 resumability**: implement the resume walk, the stale-tmp
  sweep, the kill-matrix tests above, and the advisory `.lock` for
  parallel-writer detection (row 14).
- **HITL `pending/<phase>.json` and `responses/<phase>.json`** (M10 /
  D4): same atomic-commit primitive, but first-write-wins semantics
  for `responses/` need a separate decision (open lock file vs
  rename-fails-if-exists). Out of scope here.

## Non-goals

- Cross-host coordination of writers. Loker assumes one writer per
  `runs/<id>/`; the heartbeat + advisory lock detect violations but do
  not arbitrate.
- Append-only `trace.jsonl` durability. Trace lines are best-effort
  with periodic fsync; losing the last few seconds of trace on power
  loss is acceptable.
- Compaction of `attempts/` debris. Operators can `rm -rf` failed
  attempt directories; loker never reads them after `<phase>.failed`
  is written.
