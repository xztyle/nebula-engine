# Distance & Magnitude

## Problem

Measuring the distance between two `WorldPosition` values is fundamental to the engine — needed for LOD selection, chunk loading radius, physics proximity, audio attenuation, and more. However, computing Euclidean distance naively requires squaring i128 values, which can overflow since `(i128::MAX)²` requires 256 bits. The engine needs both exact integer-domain functions (squared distance) and approximate floating-point functions (actual distance via `f64`), plus an overflow-safe alternative (Manhattan distance).

## Solution

Implement distance and magnitude functions as methods on `Vec3I128` and as free functions that accept `WorldPosition` pairs.

### Squared magnitude and distance (i128 domain)

```rust
impl Vec3I128 {
    /// Returns x² + y² + z².
    ///
    /// # Overflow
    /// Each component squared can reach i128::MAX if the component
    /// itself exceeds ~1.3×10¹⁹ (√(i128::MAX) ≈ 1.3×10¹⁹).
    /// The sum of three squares overflows if any component exceeds
    /// ~7.75×10¹⁸ (~2⁶² / √3).
    ///
    /// For positions within a single solar system (~10¹⁵ mm), this
    /// is safe. For interstellar distances, use magnitude_f64().
    pub fn magnitude_squared(self) -> i128 {
        self.x * self.x + self.y * self.y + self.z * self.z
    }
}

/// Squared Euclidean distance between two world positions.
pub fn distance_squared(a: WorldPosition, b: WorldPosition) -> i128 {
    (a - b).magnitude_squared()
}
```

### Floating-point magnitude and distance

```rust
impl Vec3I128 {
    /// Converts to f64 and computes √(x² + y² + z²).
    ///
    /// Precision: f64 has 53 bits of mantissa, so values above 2⁵³
    /// (~9×10¹⁵) lose precision. For distances up to ~9×10¹² km
    /// this is exact to the millimeter.
    pub fn magnitude_f64(self) -> f64 {
        let x = self.x as f64;
        let y = self.y as f64;
        let z = self.z as f64;
        (x * x + y * y + z * z).sqrt()
    }
}

/// Euclidean distance between two world positions, returned as f64.
pub fn distance_f64(a: WorldPosition, b: WorldPosition) -> f64 {
    (a - b).magnitude_f64()
}
```

### Manhattan distance (overflow-safe)

```rust
impl Vec3I128 {
    /// Manhattan (L1) magnitude: |x| + |y| + |z|.
    ///
    /// Cannot overflow unless the sum of three i128::MAX-magnitude
    /// values is taken, which requires components near i128::MAX/3.
    /// For all practical game distances, this is completely safe.
    pub fn manhattan_magnitude(self) -> i128 {
        self.x.abs() + self.y.abs() + self.z.abs()
    }
}

/// Manhattan distance between two world positions.
pub fn manhattan_distance(a: WorldPosition, b: WorldPosition) -> i128 {
    (a - b).manhattan_magnitude()
}
```

### Checked squared magnitude

```rust
impl Vec3I128 {
    /// Returns None if any intermediate multiplication or the
    /// final sum overflows i128.
    pub fn checked_magnitude_squared(self) -> Option<i128> {
        let x2 = self.x.checked_mul(self.x)?;
        let y2 = self.y.checked_mul(self.y)?;
        let z2 = self.z.checked_mul(self.z)?;
        x2.checked_add(y2)?.checked_add(z2)
    }
}
```

### Design notes

- Squared distance is preferred for comparisons (LOD thresholds, radius checks) since it avoids the sqrt entirely.
- The engine should store LOD thresholds as squared distances wherever possible.
- `magnitude_f64` performs the squaring in f64 space, avoiding i128 overflow entirely at the cost of precision for very large displacements.

## Outcome

After this story is complete:

- `distance_squared(a, b)` provides exact integer distance comparison
- `distance_f64(a, b)` provides approximate Euclidean distance in millimeters
- `manhattan_distance(a, b)` provides overflow-safe distance estimation
- LOD and chunk-loading systems can compare squared distances without sqrt
- Checked variants protect against overflow in interstellar-scale computations

## Demo Integration

**Demo crate:** `nebula-demo`

The title displays distance from origin alongside the position: `Dist: 2,345,678 mm`. The distance grows each tick as the position moves.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| *(none)* | — | Pure `std` only (`f64::sqrt` is in std) |

Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distance_to_self_is_zero() {
        let p = WorldPosition::new(1000, 2000, 3000);
        assert_eq!(distance_squared(p, p), 0);
        assert_eq!(distance_f64(p, p), 0.0);
    }

    #[test]
    fn test_distance_3_4_5_triangle() {
        // Displacement of (3000, 4000, 0) mm -> distance = 5000 mm
        let a = WorldPosition::new(0, 0, 0);
        let b = WorldPosition::new(3000, 4000, 0);
        assert_eq!(distance_squared(a, b), 25_000_000);
        assert!((distance_f64(a, b) - 5000.0).abs() < 1e-10);
    }

    #[test]
    fn test_distance_symmetric() {
        let a = WorldPosition::new(10, 20, 30);
        let b = WorldPosition::new(40, 50, 60);
        assert_eq!(distance_squared(a, b), distance_squared(b, a));
        assert_eq!(distance_f64(a, b), distance_f64(b, a));
    }

    #[test]
    fn test_manhattan_distance() {
        let a = WorldPosition::new(0, 0, 0);
        let b = WorldPosition::new(3, 4, 5);
        assert_eq!(manhattan_distance(a, b), 12);
    }

    #[test]
    fn test_manhattan_distance_with_negatives() {
        let a = WorldPosition::new(10, 10, 10);
        let b = WorldPosition::new(7, 14, 10);
        // |3| + |-4| + |0| = 7
        assert_eq!(manhattan_distance(a, b), 7);
    }

    #[test]
    fn test_squared_distance_manual_calc() {
        let a = WorldPosition::new(1, 2, 3);
        let b = WorldPosition::new(4, 6, 8);
        // (3² + 4² + 5²) = 9 + 16 + 25 = 50
        assert_eq!(distance_squared(a, b), 50);
    }

    #[test]
    fn test_magnitude_squared() {
        let v = Vec3I128::new(3, 4, 0);
        assert_eq!(v.magnitude_squared(), 25);
    }

    #[test]
    fn test_magnitude_f64() {
        let v = Vec3I128::new(3, 4, 0);
        assert!((v.magnitude_f64() - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_checked_magnitude_squared_safe() {
        let v = Vec3I128::new(1000, 2000, 3000);
        assert_eq!(v.checked_magnitude_squared(), Some(14_000_000));
    }

    #[test]
    fn test_checked_magnitude_squared_overflow() {
        let v = Vec3I128::new(i128::MAX, 0, 0);
        assert!(v.checked_magnitude_squared().is_none());
    }

    #[test]
    fn test_large_coordinates_f64_distance() {
        // Two points 1 AU apart along x-axis
        // 1 AU ≈ 1.496×10¹¹ m = 1.496×10¹⁴ mm
        let au_mm: i128 = 149_597_870_700_000;
        let a = WorldPosition::new(0, 0, 0);
        let b = WorldPosition::new(au_mm, 0, 0);
        let d = distance_f64(a, b);
        assert!((d - au_mm as f64).abs() / (au_mm as f64) < 1e-10);
    }
}
```
