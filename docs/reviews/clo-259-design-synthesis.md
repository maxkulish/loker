# Design Review Synthesis: CLO-259

**Reviewer**: Self-review (pi harness)
**Date**: 2026-04-27
**Verdict**: approve_with_changes

## Summary

The design doc `docs/designs/clo-259-parallel-fanout.md` was reviewed against:
- Discovery report requirements
- `phase_result_parallel.schema.json` schema constraints
- Existing `Strategy` trait object-safety invariants
- CLO-259 acceptance criteria

## Applied Suggestions

1. **Schema compliance — aggregate_output_path + verify**
   The parallel schema requires `aggregate_output_path` and `verify`. Added notes in §2.3 and §4.1 that `StrategyOutput` must emit these conditionally for `StrategyKind::Parallel`. In v0, `aggregate_output_path` is a sentinel path and `verify` is `skipped()` since aggregation logic is deferred.

2. **Schema compliance — family field**
   The schema requires `family` in every branch. Since `family_of()` is an M3 concern, the design now records that `Attempt::family` defaults to `"local"` in v0, with a `#[serde(skip_serializing_if)]` guard for non-parallel strategies.

3. **Test completeness — floor violation schema validation**
   Added explicit test case `floor_violation_schema_validates` to ensure the `Box<StrategyOutput>` carried by `StrategyError::FloorViolation` still satisfies the parallel schema.

## Flagged Suggestions

None.

## Rationale for approve_with_changes

The design satisfies all CLO-259 acceptance criteria with minimal blast radius:
- `Strategy` trait signature unchanged → no call-site breakage
- `StrategyKind::Parallel` + `StrategyError::FloorViolation` are additive
- `ParallelFanOut` is a new module; existing `single_model.rs` and `escalating_retry.rs` untouched
- Test plan mirrors the proven patterns from CLO-257 and CLO-258
- Schema compliance handled via conditional serialization on `StrategyOutput`

## Risk assessment

- **Low risk**: The approach is minimally invasive and follows established patterns.
- **Known limitation**: Cooperative cancellation via `drop(FuturesUnordered)`; documented in design and rustdoc.
- **Deferred scope**: Full aggregator trait, cross-family enforcement, family resolution — all tracked for M3 follow-up.
