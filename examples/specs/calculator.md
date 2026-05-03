# Calculator Library Specification

A minimal calculator library providing pure, deterministic arithmetic operations over integers and floats.

## Problem Statement

This library serves as the integration test target for the Loker M6 phase-runner. It provides four fundamental arithmetic operations (`add`, `subtract`, `multiply`, `divide`) as pure functions. Division by zero must not panic — it returns a typed error. The library has no external dependencies and targets Rust's standard library only.

## Requirements

- `add(a, b)` returns the sum of two integers or floats.
- `subtract(a, b)` returns the difference of two integers or floats.
- `multiply(a, b)` returns the product of two integers or floats.
- `divide(a, b)` returns the quotient of two integers or floats.
- All operations support both integer and floating-point inputs.
- Division by zero returns a typed error (not a panic or sentinel value).
- All operations are pure: no side effects, no mutation of inputs.
- Results are deterministic: same inputs always produce the same output.

## Constraints

- No external dependencies — standard library only.
- Library (not a binary) — compiled as a Rust crate.
- Tests live alongside code (in `src/` or adjacent `tests/` module).

## Out of Scope

- Arbitrary-precision arithmetic.
- Expression parsing (e.g., `"2 + 3 * 4"`).
- Persistence or serialization.
- Pytest variant (Rust-only for v0).

## Acceptance

```
add(2, 3) == 5
add(-1, 1) == 0
subtract(10, 4) == 6
multiply(3, 7) == 21
divide(10, 2) == 5.0
divide(7.0, 2.0) == 3.5
divide(1, 0) -> Err(DivisionByZero)
```

All operations are pure, deterministic, and return consistent results for the same inputs. Division by zero returns an explicit `Err(DivisionByZero)` variant rather than panicking.