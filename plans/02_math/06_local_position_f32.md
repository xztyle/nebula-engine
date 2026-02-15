# Local Position f32

## Problem

GPUs operate in 32-bit floating point. Rendering, physics simulation, audio spatialization, and animation all require f32 coordinates. The engine cannot send i128 world positions to the GPU directly — they must be converted to camera-relative or chunk-relative f32 coordinates. A dedicated `LocalPosition` type makes this conversion explicit in the type system, preventing accidental use of world positions where local positions are expected (and vice versa).

## Solution

Define a `LocalPosition` struct in `nebula_math`:

```rust
/// Position relative to a local origin (camera, chunk center, etc.)
/// in f32 space. Each unit is 1 millimeter, same as WorldPosition.
///
/// Precision: f32 has 23 mantissa bits, giving ~7 decimal digits.
/// At 1mm resolution, positions are exact up to ±8,388 meters (~8.4 km).
/// Beyond that, sub-millimeter precision degrades but sub-centimeter
/// precision holds to ~83 km.
///
/// For rendering, the origin should be set to the camera position
/// so that nearby geometry has maximum precision.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct LocalPosition {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}
```

### Constructors

```rust
impl LocalPosition {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    pub fn zero() -> Self {
        Self::default()
    }
}
```

### glam interop

```rust
use glam::Vec3;

impl From<LocalPosition> for Vec3 {
    fn from(lp: LocalPosition) -> Vec3 {
        Vec3::new(lp.x, lp.y, lp.z)
    }
}

impl From<Vec3> for LocalPosition {
    fn from(v: Vec3) -> LocalPosition {
        LocalPosition::new(v.x, v.y, v.z)
    }
}
```

### Arithmetic

Implement `Add`, `Sub`, `Neg`, `Mul<f32>`, `Div<f32>` via `std::ops`, delegating to component-wise f32 operations. These mirror the f32 vector math that glam provides, but on the `LocalPosition` type so the type system keeps local and world coordinates distinct.

### Display

Format as `"Local(x, y, z)"` with 3 decimal places.

### Design notes

- `LocalPosition` does **not** implement `Eq` or `Hash` because f32 is not `Eq`. Comparisons should use approximate equality helpers.
- The struct is deliberately not `#[repr(C)]` by default since glam handles GPU layout. Add `#[repr(C)]` only if direct GPU upload is needed.
- An `approx_eq` method is provided with a configurable epsilon for testing.

```rust
impl LocalPosition {
    /// Returns true if all components are within epsilon of the other.
    pub fn approx_eq(self, other: LocalPosition, epsilon: f32) -> bool {
        (self.x - other.x).abs() < epsilon
            && (self.y - other.y).abs() < epsilon
            && (self.z - other.z).abs() < epsilon
    }
}
```

## Outcome

After this story is complete:

- `LocalPosition` is available as the f32 rendering-space position type
- Conversion to/from `glam::Vec3` is seamless
- Arithmetic operators work for local-space math
- The type system prevents accidentally mixing world and local coordinates
- An `approx_eq` method supports floating-point-aware testing

## Demo Integration

**Demo crate:** `nebula-demo`

The title shows a `LocalPosition` in f32 alongside the world i128 coordinates: `Local: (0.00, 0.00, 1.67)` -- the same position re-expressed in meters.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `glam` | `0.29` | f32 vector/matrix math, GPU-ready types |

Rust edition 2024. `glam` is the standard math library for Rust game engines and provides `Vec3`, `Mat4`, `Quat`, etc.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn test_construction() {
        let lp = LocalPosition::new(1.0, 2.0, 3.0);
        assert_eq!(lp.x, 1.0);
        assert_eq!(lp.y, 2.0);
        assert_eq!(lp.z, 3.0);
    }

    #[test]
    fn test_default_is_zero() {
        let lp = LocalPosition::default();
        assert_eq!(lp.x, 0.0);
        assert_eq!(lp.y, 0.0);
        assert_eq!(lp.z, 0.0);
    }

    #[test]
    fn test_to_glam_vec3() {
        let lp = LocalPosition::new(1.0, 2.0, 3.0);
        let v: Vec3 = lp.into();
        assert_eq!(v, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn test_from_glam_vec3() {
        let v = Vec3::new(4.0, 5.0, 6.0);
        let lp: LocalPosition = v.into();
        assert_eq!(lp, LocalPosition::new(4.0, 5.0, 6.0));
    }

    #[test]
    fn test_roundtrip_glam() {
        let original = LocalPosition::new(1.5, -2.7, 3.14);
        let roundtrip: LocalPosition = Vec3::from(original).into();
        assert!(original.approx_eq(roundtrip, 1e-6));
    }

    #[test]
    fn test_precision_at_500m() {
        // At 500,000 mm from origin, f32 should resolve to 1mm
        let lp = LocalPosition::new(500_000.0, 500_000.0, 500_000.0);
        let nudged = LocalPosition::new(500_001.0, 500_000.0, 500_000.0);
        // The 1mm difference should be preserved
        assert!((nudged.x - lp.x - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_approx_eq() {
        let a = LocalPosition::new(1.0, 2.0, 3.0);
        let b = LocalPosition::new(1.0 + 1e-7, 2.0, 3.0);
        assert!(a.approx_eq(b, 1e-6));
        assert!(!a.approx_eq(LocalPosition::new(2.0, 2.0, 3.0), 0.5));
    }

    #[test]
    fn test_add() {
        let a = LocalPosition::new(1.0, 2.0, 3.0);
        let b = LocalPosition::new(0.5, 0.5, 0.5);
        let c = a + b;
        assert!(c.approx_eq(LocalPosition::new(1.5, 2.5, 3.5), 1e-6));
    }

    #[test]
    fn test_sub() {
        let a = LocalPosition::new(3.0, 4.0, 5.0);
        let b = LocalPosition::new(1.0, 1.0, 1.0);
        let c = a - b;
        assert!(c.approx_eq(LocalPosition::new(2.0, 3.0, 4.0), 1e-6));
    }

    #[test]
    fn test_scalar_mul() {
        let a = LocalPosition::new(1.0, 2.0, 3.0);
        let b = a * 2.0;
        assert!(b.approx_eq(LocalPosition::new(2.0, 4.0, 6.0), 1e-6));
    }

    #[test]
    fn test_neg() {
        let a = LocalPosition::new(1.0, -2.0, 3.0);
        let b = -a;
        assert!(b.approx_eq(LocalPosition::new(-1.0, 2.0, -3.0), 1e-6));
    }
}
```
