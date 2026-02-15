# Fixed-Point Type

## Problem

Some computations require sub-voxel (sub-millimeter) precision in world space without leaving the integer domain. Examples include smoothed terrain surfaces, physics interpolation between ticks, and precise entity placement within a voxel. Floating-point would lose precision at large coordinates, but plain i128 only resolves to 1mm. A fixed-point type bridges this gap: 96 bits of integer range (still ±3.9×10²⁸, or ±4.2×10²² km) with 32 bits of fractional precision (resolution of ~0.00000000023 mm, i.e., sub-nanometer).

## Solution

Define a `FixedI128` struct that stores a single `i128` where the lower 32 bits represent the fractional part:

```rust
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
```

### Constructors and conversions

```rust
impl FixedI128 {
    /// Create from the raw i128 representation directly.
    pub const fn from_raw(raw: i128) -> Self { Self { raw } }

    /// Access the raw i128 value.
    pub const fn to_raw(self) -> i128 { self.raw }

    /// Create from a whole integer (no fractional part).
    pub const fn from_int(value: i128) -> Self {
        Self { raw: value << FRAC_BITS }
    }

    /// Truncate to the integer part (toward zero).
    pub const fn to_int(self) -> i128 {
        self.raw >> FRAC_BITS
    }
}

impl From<i128> for FixedI128 {
    fn from(value: i128) -> Self { Self::from_int(value) }
}

impl From<f64> for FixedI128 {
    fn from(value: f64) -> Self {
        Self { raw: (value * FRAC_SCALE as f64) as i128 }
    }
}

impl From<FixedI128> for f64 {
    fn from(fixed: FixedI128) -> f64 {
        fixed.raw as f64 / FRAC_SCALE as f64
    }
}
```

### Arithmetic

| Operation | Implementation |
|-----------|----------------|
| `Add` | `raw + raw` (same scale, direct addition) |
| `Sub` | `raw - raw` (same scale, direct subtraction) |
| `Mul` | `(self.raw as i256 * rhs.raw as i256) >> FRAC_BITS` — Since Rust lacks native `i256`, implement via two `i128` multiplications using a widening multiply helper. Alternatively, split each operand into high and low 64-bit halves and combine partial products. |
| `Div` | `(self.raw << FRAC_BITS) / rhs.raw` — pre-shift the dividend to maintain precision. Guard against overflow by checking if the shift would overflow before executing. |

### Widening multiply implementation

```rust
impl FixedI128 {
    /// Multiply two FixedI128 values.
    ///
    /// Internally performs a 256-bit multiply by splitting each
    /// operand into two 64-bit halves, computing four partial
    /// products, and recombining. The result is right-shifted
    /// by FRAC_BITS to maintain the fixed-point scale.
    pub fn fixed_mul(self, rhs: FixedI128) -> FixedI128 {
        // Split into (high, low) 64-bit parts
        let a_hi = (self.raw >> 64) as i128;
        let a_lo = (self.raw & ((1i128 << 64) - 1)) as i128;
        let b_hi = (rhs.raw >> 64) as i128;
        let b_lo = (rhs.raw & ((1i128 << 64) - 1)) as i128;

        // Partial products (each fits in i128)
        let lo_lo = a_lo * b_lo;
        let lo_hi = a_lo * b_hi;
        let hi_lo = a_hi * b_lo;
        // hi_hi would need the top 128 bits; only needed if
        // the result exceeds 128-bit integer range.

        // Combine: result = (lo_lo >> FRAC_BITS)
        //                  + (lo_hi << (64 - FRAC_BITS))
        //                  + (hi_lo << (64 - FRAC_BITS))
        let result = (lo_lo >> FRAC_BITS)
            + (lo_hi << (64 - FRAC_BITS))
            + (hi_lo << (64 - FRAC_BITS));

        FixedI128 { raw: result }
    }
}
```

### Display

Format as a decimal number with up to 9 fractional digits (sufficient to represent 2⁻³² precision).

## Outcome

After this story is complete:

- `FixedI128` enables sub-voxel precision in the integer domain
- Conversion to/from `i128` and `f64` is seamless
- Arithmetic operators work correctly with fixed-point scaling
- The type can be used for smooth terrain heights, physics interpolation, and precise entity placement

## Demo Integration

**Demo crate:** `nebula-demo`

No visible demo change; fixed-point infrastructure is available for sub-millimeter interpolation in later epics.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| *(none)* | — | Pure `std` only |

Rust edition 2024. The widening multiply is implemented manually without external big-integer crates to avoid a heavy dependency for a single operation.

## Unit Tests

```rust
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
        let original: f64 = 3.14159;
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
```
