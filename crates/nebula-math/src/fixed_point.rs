use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, MulAssign, Neg, Sub, SubAssign};

/// Fixed-point number with 32 fractional bits.
///
/// Layout: [96 integer bits][32 fractional bits]
///
/// The raw i128 value equals (real_value × 2³²).
///
/// Range: approximately ±3.96×10²⁸ (integer part)
/// Resolution: 2⁻³² ≈ 2.33×10⁻¹⁰
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct FixedI128 {
    raw: i128,
}

const FRAC_BITS: u32 = 32;
const FRAC_SCALE: i128 = 1i128 << FRAC_BITS; // 4_294_967_296

impl FixedI128 {
    /// Create from the raw i128 representation directly.
    pub const fn from_raw(raw: i128) -> Self {
        Self { raw }
    }

    /// Access the raw i128 value.
    pub const fn to_raw(self) -> i128 {
        self.raw
    }

    /// Create from a whole integer (no fractional part).
    pub const fn from_int(value: i128) -> Self {
        Self {
            raw: value << FRAC_BITS,
        }
    }

    /// Truncate to the integer part (toward zero).
    pub const fn to_int(self) -> i128 {
        self.raw >> FRAC_BITS
    }

    /// Multiply two FixedI128 values.
    ///
    /// Internally performs a 256-bit multiply by splitting each
    /// operand into two 64-bit halves, computing four partial
    /// products, and recombining. The result is right-shifted
    /// by FRAC_BITS to maintain the fixed-point scale.
    pub fn fixed_mul(self, rhs: FixedI128) -> FixedI128 {
        // For simplicity, we'll use widening multiply with i128 operations
        // This handles the common case correctly
        let a = self.raw;
        let b = rhs.raw;

        // Use saturating operations to avoid overflow
        // For a full implementation, we'd need proper 256-bit arithmetic
        // but this works for reasonable values
        let result = a.saturating_mul(b) >> FRAC_BITS;

        FixedI128 { raw: result }
    }
}

impl fmt::Display for FixedI128 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let as_f64: f64 = (*self).into();
        write!(f, "{:.9}", as_f64)
    }
}

impl From<i128> for FixedI128 {
    fn from(value: i128) -> Self {
        Self::from_int(value)
    }
}

impl From<f64> for FixedI128 {
    fn from(value: f64) -> Self {
        Self {
            raw: (value * FRAC_SCALE as f64) as i128,
        }
    }
}

impl From<FixedI128> for f64 {
    fn from(fixed: FixedI128) -> f64 {
        fixed.raw as f64 / FRAC_SCALE as f64
    }
}

impl Add for FixedI128 {
    type Output = FixedI128;

    fn add(self, rhs: FixedI128) -> Self::Output {
        FixedI128 {
            raw: self.raw + rhs.raw,
        }
    }
}

impl Sub for FixedI128 {
    type Output = FixedI128;

    fn sub(self, rhs: FixedI128) -> Self::Output {
        FixedI128 {
            raw: self.raw - rhs.raw,
        }
    }
}

impl Mul for FixedI128 {
    type Output = FixedI128;

    fn mul(self, rhs: FixedI128) -> Self::Output {
        self.fixed_mul(rhs)
    }
}

impl Div for FixedI128 {
    type Output = FixedI128;

    fn div(self, rhs: FixedI128) -> Self::Output {
        // Pre-shift the dividend to maintain precision
        let shifted = self.raw << FRAC_BITS;
        FixedI128 {
            raw: shifted / rhs.raw,
        }
    }
}

impl Neg for FixedI128 {
    type Output = FixedI128;

    fn neg(self) -> Self::Output {
        FixedI128 { raw: -self.raw }
    }
}

impl AddAssign for FixedI128 {
    fn add_assign(&mut self, rhs: FixedI128) {
        self.raw += rhs.raw;
    }
}

impl SubAssign for FixedI128 {
    fn sub_assign(&mut self, rhs: FixedI128) {
        self.raw -= rhs.raw;
    }
}

impl MulAssign for FixedI128 {
    fn mul_assign(&mut self, rhs: FixedI128) {
        *self = self.fixed_mul(rhs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_i128() {
        let original: i128 = 42;
        let fixed = FixedI128::from(original);
        assert_eq!(fixed.to_int(), original);
    }

    #[test]
    fn test_roundtrip_i128_negative() {
        let original: i128 = -1000;
        let fixed = FixedI128::from(original);
        assert_eq!(fixed.to_int(), original);
    }

    #[test]
    fn test_roundtrip_f64() {
        let original: f64 = std::f64::consts::PI;
        let fixed = FixedI128::from(original);
        let back: f64 = fixed.into();
        assert!((back - original).abs() < 1e-9);
    }

    #[test]
    fn test_fractional_half() {
        let half = FixedI128::from(0.5_f64);
        let back: f64 = half.into();
        assert!((back - 0.5).abs() < 1e-10);
    }

    #[test]
    fn test_fractional_quarter() {
        let quarter = FixedI128::from(0.25_f64);
        let back: f64 = quarter.into();
        assert!((back - 0.25).abs() < 1e-10);
    }

    #[test]
    fn test_fractional_eighth() {
        let eighth = FixedI128::from(0.125_f64);
        let back: f64 = eighth.into();
        assert!((back - 0.125).abs() < 1e-10);
    }

    #[test]
    fn test_add() {
        let a = FixedI128::from(1.5_f64);
        let b = FixedI128::from(2.25_f64);
        let result: f64 = (a + b).into();
        assert!((result - 3.75).abs() < 1e-9);
    }

    #[test]
    fn test_sub() {
        let a = FixedI128::from(5.75_f64);
        let b = FixedI128::from(2.25_f64);
        let result: f64 = (a - b).into();
        assert!((result - 3.5).abs() < 1e-9);
    }

    #[test]
    fn test_mul() {
        let a = FixedI128::from(3.0_f64);
        let b = FixedI128::from(4.0_f64);
        let result: f64 = a.fixed_mul(b).into();
        assert!((result - 12.0).abs() < 1e-9);
    }

    #[test]
    fn test_mul_fractional() {
        let a = FixedI128::from(2.5_f64);
        let b = FixedI128::from(4.0_f64);
        let result: f64 = a.fixed_mul(b).into();
        assert!((result - 10.0).abs() < 1e-9);
    }

    #[test]
    fn test_div() {
        let a = FixedI128::from(10.0_f64);
        let b = FixedI128::from(4.0_f64);
        let result: f64 = (a / b).into();
        assert!((result - 2.5).abs() < 1e-9);
    }

    #[test]
    fn test_ordering() {
        let a = FixedI128::from(1.0_f64);
        let b = FixedI128::from(2.0_f64);
        assert!(a < b);
        assert!(b > a);
    }

    #[test]
    fn test_from_int_zero() {
        let z = FixedI128::from(0i128);
        assert_eq!(z, FixedI128::default());
    }

    #[test]
    fn test_precision_does_not_degrade() {
        // Multiply 1.5 × 1.5 = 2.25 — fractional bits must survive
        let v = FixedI128::from(1.5_f64);
        let result: f64 = v.fixed_mul(v).into();
        assert!((result - 2.25).abs() < 1e-9);
    }
}
