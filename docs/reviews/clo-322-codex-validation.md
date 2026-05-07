# Codex Validation Report: CLO-322

**Reviewer**: Codex (gpt-5.5 with high reasoning effort via `codex review`)
**Reviewed**: 2026-05-07
**Status**: OK

## Verdict
approve_with_changes

## Findings

### [P1] Validate run_id before joining POST paths — SECURITY
**File**: `src/ui/routes.rs:152`
For `approve`/`reject`, `run_id` is joined directly into the filesystem path,
but Axum percent-decodes path params; a request like `/gates/%2Ftmp/review/approve`
can make `run_dir` resolve outside `<project_root>/runs` and write `locks/` and
`responses/` under any existing writable directory. Apply the same traversal/
absolute-path validation used by `GET /runs/:id` before constructing `run_dir`.

### [P2] Skip gates that already have responses
**File**: `src/ui/gate_discovery.rs:108`
This scan treats every `pending/<phase>.json` as active, but approving or
rejecting only writes `responses/<phase>.json`; the pending file remains, so
the 303 back to `/pending` will still show the resolved gate and allow repeated
submissions. Match the existing `ls --blocked` behavior by skipping pending files
with a sibling response already present.

## Missing Items
- Path traversal validation on approve/reject handlers
- Filter resolved gates from pending panel

## Recommendations
Apply both fixes before PR.
