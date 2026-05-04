/// Errors that can occur during calculator operations.
#[derive(Debug, PartialEq)]
pub enum CalcError {
    DivisionByZero,
}

/// Adds two numbers.
pub fn add(a: f64, b: f64) -> f64 {
    a + b
}

/// Subtracts `b` from `a`.
pub fn subtract(a: f64, b: f64) -> f64 {
    a - b
}

/// Multiplies two numbers.
pub fn multiply(a: f64, b: f64) -> f64 {
    a * b
}

/// Divides `a` by `b`. Returns `Err(DivisionByZero)` if `b == 0.0`.
pub fn divide(a: f64, b: f64) -> Result<f64, CalcError> {
    if b == 0.0 {
        Err(CalcError::DivisionByZero)
    } else {
        Ok(a / b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        assert_eq!(add(2.0, 3.0), 5.0);
    }

    #[test]
    fn test_divide_by_zero() {
        assert_eq!(divide(1.0, 0.0), Err(CalcError::DivisionByZero));
    }
}
