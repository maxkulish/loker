# PRD: CLO-266 — Aggregator::Concat with per-source headings

## Problem

Workflow authors using `ParallelFanOut` need a deterministic way to turn multiple branch outputs into one downstream artefact. Today the parallel strategy records branch metadata and a placeholder aggregate path, but there is no implementation that concatenates successful branch text or preserves failed target reasons in the merged output. This blocks the phase runner from consuming parallel review outputs as a single phase artefact.

## Goal

Implement `Aggregator::Concat { heading_template: String }` as the simplest aggregator: structured paste of per-target outputs under labelled headings, followed by an error footer when any targets failed.

## Scope

- Add a real aggregation seam for parallel branch results, preserving the existing schema-facing `Aggregator` label semantics.
- Support `{backend_id}`, `{family}`, and `{index}` placeholders in the heading template and document that supported set.
- Produce a single string artefact with one section per successful target in arrival order, matching the order currently exposed by `ParallelFanOut`.
- Surface failed targets in a `## Errors` footer with structured per-target reason; never silently drop failures.
- Return a documented sentinel for empty input instead of panicking.
- Validate the output path through the existing D2 parallel phase-result schema path.

## Out of scope

- LLM judge, vote, or any-fail aggregation logic.
- Phase runner integration that writes the aggregate artefact as the canonical phase output (tracked by T-029).
- New cross-family enforcement rules; `family_of()` already exists and non-concat aggregators will consume it separately.

## Acceptance Criteria

- Snapshot test (`insta`) on a 3-target merge with mixed success/failure captures heading rendering and footer shape.
- Empty input returns the documented sentinel.
- Result metadata still round-trips through `docs/schemas/phase_result_parallel.schema.json` validation.
