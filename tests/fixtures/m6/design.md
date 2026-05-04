# Calculator Library Design

## Architecture

A minimal Rust library providing `add`, `subtract`, `multiply`, `divide` as pure functions.
Division by zero returns a typed error instead of panicking.

## Public API

```rust
pub enum CalcError {
    DivisionByZero,
}

pub fn add(a: f64, b: f64) -> f64;
pub fn subtract(a: f64, b: f64) -> f64;
pub fn multiply(a: f64, b: f64) -> f64;
pub fn divide(a: f64, b: f64) -> Result<f64, CalcError>;
```

## Implementation

Each function is a one-liner delegating to the corresponding operator.
`divide` wraps `/` in a `match` that returns `Err(DivisionByZero)` when `b == 0.0`.

## Acceptance Tests

```
add(2, 3) == 5
divide(1, 0) -> Err(DivisionByZero)
```
