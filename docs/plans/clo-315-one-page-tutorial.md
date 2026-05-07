# CLO-315 Implementation Plan: One-page tutorial

**From design**: [docs/design-docs/clo-315-one-page-tutorial.md](../design-docs/clo-315-one-page-tutorial.md)
**Linear**: https://linear.app/cloud-ai/issue/CLO-315

---

## Goal

Produce `docs/tutorial.md` and cross-links so a new user can go from `git clone` to inspecting a run directory in under 10 minutes.

---

## Sub-tasks

### ST1: Create the tutorial file
**Files**: `docs/tutorial.md`
**Tests**: Manual — follow every command on current `main`
**Estimate**: M

Write `docs/tutorial.md` with the 9 sections from the design doc:
1. What you'll do
2. Prerequisites
3. Install
4. Check your setup (`loker doctor`)
5. Explore a workflow without backends (`loker explain design-doc-tdd`)
6. Run your first workflow (simple shell workflow, optionally Ollama-backed)
7. Locate the run directory
8. Read the trace (`loker trace`)
9. Next steps

Acceptance criteria for this sub-task:
- Every ` ```bash ` block has a matching ` ```text ` output block from an actual run.
- File is ≤200 lines.
- Calculator spec is referenced.

### ST2: Create the calculator tutorial workflow (optional)
**Files**: `examples/workflows/calculator-tutorial.toml`
**Tests**: `cargo run -- run examples/workflows/calculator-tutorial.toml --spec examples/specs/calculator.md`
**Estimate**: XS

If the tutorial needs a simple workflow that uses the calculator spec, add it to `examples/workflows/` so it is maintained alongside other examples.

### ST3: Cross-link from README and handoff
**Files**: `README.md`, `docs/handoff.md`
**Tests**: `rg -i tutorial README.md docs/handoff.md`
**Estimate**: XS

Add links to `docs/tutorial.md` in:
- `README.md` — in the "One-page example" or "Install" section
- `docs/handoff.md` — in the onboarding/getting-started section

### ST4: Final verification
**Files**: `docs/tutorial.md`
**Tests**: Manual read-through + timing
**Estimate**: XS

- Follow the tutorial on a fresh mental read-through.
- Time each section to confirm ≤10 minutes total.
- Ensure no invented outputs.

---

## Dependencies

- `examples/specs/calculator.md` must exist (T-036 / CLO-290 — ✅ exists)
- `README.md` restructure from T-045 / CLO-314 should not conflict (this task adds a link only)

---

## Execution order

```
ST1 ──→ ST2 ──→ ST3 ──→ ST4
  │        │        │        │
  └────────┴────────┴────────┘
         All can be in one commit
```

All sub-tasks are small and sequential; they can land in a single PR.

---

## Rollback plan

If the tutorial references a CLI command that changes behavior before merge, update the affected section and re-verify. If T-041 lands during implementation and changes the recommended first-run path, evaluate whether to update the tutorial or keep it scoped to the current CLI surface.
