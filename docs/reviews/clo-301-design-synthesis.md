# Design Review Synthesis — CLO-301: Wire ResumeRunner Execution End-to-End

## Review Sources

| Source | Model | Verdict | Notes |
|---|---|---|---|
| `clo-301-design-gemini.md` | Gemini 3.1 Pro | APPROVE_WITH_SUGGESTIONS | 3 actionable suggestions |

## Summary

The design is sound. Gemini's review identified three concrete issues:

1. **F1 (Shell Steps)**: `to_phase_configs()` excludes shell steps — resume will skip them. Noted as acceptable for v0, should be documented.
2. **F2 (Manifest Sweeping)**: When adding `workflow_name` to `manifest.json`, must trigger orphan-entry sweep before execution.
3. **F3 (Binary Artefacts)**: `with_artefact` prompt helper must handle `Vec<u8>` for binary artefact kinds, not assume UTF-8.

All three are minor and addressable in implement without changing the core approach.

## Actionable Changes

| ID | Classification | Action |
|---|---|---|
| F1 | **Additive** | Add shell-step limitation note to design doc §4.2 |
| F2 | **Additive** | Add manifest orphan-sweep callout to §4.3 |
| F3 | **Refinement** | Note `Vec<u8>` handling in §4.4 |

## Verdict

**APPROVE_WITH_CHANGES** — apply F1 and F2 as additive documentation fixes before plan phase. F3 is a nit to carry forward into implementation notes.
