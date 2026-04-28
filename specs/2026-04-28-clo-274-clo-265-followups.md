# Spec: CLO-274 — CLO-265 PR #18 review follow-ups

**Created**: 2026-04-28
**Estimated scope**: XS (2 production files, ~5 sub-tasks, +5 tests)
**Linear**: [CLO-274](https://linear.app/cloud-ai/issue/CLO-274/address-clo-265-pr-18-review-comments-zhipu-duplicate-phaseerror-non)
**Parent**: [CLO-265](https://linear.app/cloud-ai/issue/CLO-265) — `specs/2026-04-28-clo-265-family-of.md`
**Source PR**: [maxkulish/loker#18](https://github.com/maxkulish/loker/pull/18)

## 1. Problem Statement

CLO-265 (PR #18) merged before automated review bots posted comments. Five
review comments arrived 2-4 minutes post-merge and were not addressed:

1. **Zhipu unreachable in `family_of`** (Copilot, `family.rs:103`) — `Family::Zhipu`
   exists and `as_str()` returns `"zhipu"`, but `family_of("zhipu")` falls through
   to `Family::Other("zhipu")` because the match arm is missing. Any configured
   Zhipu backend is bucketed as Other.
2. **Zhipu missing in `family_of_suffix`** (Copilot, `family.rs:114`) — Same
   omission for gateway-style ids `loker_*_zhipu` and `tensorzero/loker_*_zhipu`.
3. **Duplicate `PhaseError`** (Copilot, `family.rs:133` + `strategy/mod.rs:444`) —
   `PhaseError` is defined twice with the same `FamilyOverlap` variant. CLO-265
   spec ST2 says re-export from `src/strategy/mod.rs`, not redefine.
4. **Non-deterministic `enforce_cross_family`** (Gemini, `family.rs:158`) —
   `HashMap` iteration order is undefined. With multiple overlapping families
   (e.g. 2× Anthropic + 2× OpenAI) the reported family is arbitrary across runs,
   making `FamilyOverlap` error messages non-reproducible.

Out of scope: Gemini's `pub(crate)` suggestion for `enforce_cross_family`. Per
CLO-265 spec the function is part of the public API.

## 2. Acceptance Criteria

- [ ] **AC1**: `family_of("zhipu") == Family::Zhipu`. Add a `"zhipu" =>
      Family::Zhipu` arm in the match block at `src/family.rs` (currently
      lines 99-107).
- [ ] **AC2**: `family_of("loker_d1_zhipu") == Family::Zhipu` and
      `family_of("tensorzero/loker_review_zhipu") == Family::Zhipu`. Add a
      `"zhipu" => Family::Zhipu` arm in `family_of_suffix` (currently
      lines 110-118).
- [ ] **AC3**: `PhaseError` is defined exactly once, in `src/family.rs`. The
      duplicate definition in `src/strategy/mod.rs` (lines 428-444) is removed
      and replaced with a re-export so `use crate::strategy::PhaseError`
      continues to work for downstream callers (CLO-265 spec ST2).
- [ ] **AC4**: `enforce_cross_family` returns the same `FamilyOverlap.family`
      across repeated runs when multiple families overlap. Replace the
      internal `HashMap<Family, usize>` with `BTreeMap<Family, usize>` (which
      requires `Family: Ord`). Add `PartialOrd, Ord` derives to `Family`. The
      first overlapping family by sort order is reported (deterministic).
- [ ] **AC5**: New unit tests in `src/family.rs::tests`:
      - `family_of_zhipu` → `Family::Zhipu`
      - `family_of_loker_zhipu_suffix` (`"loker_d1_zhipu"`) → `Family::Zhipu`
      - `family_of_tensorzero_zhipu_suffix` (`"tensorzero/loker_review_zhipu"`) → `Family::Zhipu`
      - `enforce_cross_family_deterministic`: 2× Anthropic + 2× OpenAI, called
        100× in a loop — every call returns `FamilyOverlap` with the same
        `family` value (reproducible).
- [ ] **AC6**: All 31 existing `family::tests` plus the 4 new tests pass.
- [ ] **AC7**: `make check` exits 0 (fmt + clippy + lib + integration tests).

## 3. Constraints

**Must**:
- Keep `PhaseError` `pub` (re-exported from `crate::strategy`).
- Keep `enforce_cross_family` `pub` (Gemini suggested `pub(crate)`; spec rejects).
- Adding `Ord` to `Family` is additive under `#[non_exhaustive]` — safe.

**Must-not**:
- Touch any other backend / strategy logic.
- Add a new dependency. `BTreeMap` is in `std`.
- Change `Attempt.family` schema (still `Option<String>`).

**Prefer**:
- Keep the determinism fix as small as possible: `BTreeMap` swap is one type
  change in `enforce_cross_family` plus two derive additions on `Family`.

## 4. Decomposition

Five sub-tasks. ST1-ST3 independent; ST4 depends on ST1; ST5 final gate.

1. **ST1: Add Zhipu match arms.** Two one-line additions in `src/family.rs`
   (`family_of` and `family_of_suffix`).
2. **ST2: De-duplicate `PhaseError`.** Delete the enum block in
   `src/strategy/mod.rs` (lines 428-444), replace with `pub use
   crate::family::PhaseError;`.
3. **ST3: Determinism.** Add `PartialOrd, Ord` to `Family` derives. Switch
   `HashMap` → `BTreeMap` inside `enforce_cross_family`. Note: `BTreeMap`
   iteration is deterministic by key order; the first overlapping family by
   `Family` sort order is reported.
4. **ST4: Tests.** Three Zhipu tests + one determinism loop test in
   `src/family.rs::tests`.
5. **ST5: Gate.** `make check` green; commit; PR.

## 5. Evaluation

| # | Test | Expected | Run |
|---|------|----------|-----|
| 1 | `family_of("zhipu")` | `Family::Zhipu` | `cargo test --lib family_of_zhipu` |
| 2 | `family_of("loker_d1_zhipu")` | `Family::Zhipu` | `cargo test --lib family_of_loker_zhipu_suffix` |
| 3 | `family_of("tensorzero/loker_review_zhipu")` | `Family::Zhipu` | `cargo test --lib family_of_tensorzero_zhipu_suffix` |
| 4 | Existing `PhaseError` callers compile | `cargo build` | `cargo build` |
| 5 | `enforce_cross_family` deterministic | Same family reported across 100 runs | `cargo test --lib enforce_cross_family_deterministic` |
| 6 | Pre-merge gate | fmt + clippy + tests | `make check` |
