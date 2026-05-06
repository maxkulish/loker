# Design Review Synthesis: CLO-312

## Verdict

approve_with_changes

## Applied suggestions

1. **F1 — Add `--color <auto|always|never>` CLI flag**: Apply. Add an inline clap arg `--color` with an enum mapping to `colored::control::set_override`. This matches the project's existing use of `colored::control::SHOULD_COLORIZE` and is common across Rust CLI tools.

2. **F2 — Emit stderr warning for malformed lines**: Apply. When a line fails to parse, emit `<malformed>` on stdout **and** write `eprintln!("warning: malformed span on line {}", line_no)` to stderr. This preserves the resilient behavior while giving the user an audit trail.

## Flagged suggestions

1. **F3 — Rename `TracePrinter` → `TraceFormatter`**: Flagged, do not apply now. The name `TracePrinter` is already in the draft and the module is small enough that renaming is a trivial refactor. If a future TUI or web surface is added, a rename will naturally fall out of that design. Documenting the intent in a module-level doc comment is sufficient for v0.

## Final recommendation

Proceed to the plan phase after applying F1 and F2. The design is additive (CLI surface only), introduces no new dependencies, does not touch the trace writer or runner, and follows existing repo conventions. The implementation can follow the standard `make check` gate.
