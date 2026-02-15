# I128 Vector Types

## Problem

`WorldPosition` represents an absolute location, but many operations — physics impulses, offsets between positions, directions, chunk strides — require a **displacement** or **direction** vector in the same i128 coordinate space. Conflating position and displacement into a single type leads to semantic bugs (e.g., adding two positions is meaningless). A separate vector type is needed, along with a 2D variant for operations on individual face planes of the cubesphere.

## Solution

Define two structs in `nebula_math`:

```rust
/// 3D displacement / direction vector in i128 space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Vec3I128 {
    pub x: i128,
    pub y: i128,
    pub z: i128,
}

/// 2D vector in i128 space, used for cubesphere face-local coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Vec2I128 {
    pub x: i128,
    pub y: i128,
}
```

### `std::ops` implementations for `Vec3I128`

| Trait | LHS | RHS | Output | Notes |
|-------|-----|-----|--------|-------|
| `Add<Vec3I128>` | `Vec3I128` | `Vec3I128` | `Vec3I128` | Component-wise add |
| `Sub<Vec3I128>` | `Vec3I128` | `Vec3I128` | `Vec3I128` | Component-wise sub |
| `Neg` | `Vec3I128` | — | `Vec3I128` | Negate all components |
| `Mul<i128>` | `Vec3I128` | `i128` | `Vec3I128` | Scalar multiply |
| `Div<i128>` | `Vec3I128` | `i128` | `Vec3I128` | Scalar divide (truncating) |
| `AddAssign` | `Vec3I128` | `Vec3I128` | `()` | In-place add |
| `SubAssign` | `Vec3I128` | `Vec3I128` | `()` | In-place sub |
| `MulAssign<i128>` | `Vec3I128` | `i128` | `()` | In-place scalar mul |

The same table applies to `Vec2I128` with the `z` component removed.

### Overflow strategy

All arithmetic uses Rust's default **checked (debug) / wrapping (release)** behavior. This means:

- In debug builds, overflow panics immediately, catching bugs early.
- In release builds, overflow wraps silently (two's complement).
- Provide explicit `checked_add`, `checked_sub`, `saturating_add`, `saturating_sub` methods that return `Option<Vec3I128>` or clamp to `i128::MIN`/`i128::MAX` for callers that need safety at interstellar scales.

### Interop with `WorldPosition`

```rust
impl Sub for WorldPosition {
    type Output = Vec3I128;
    // WorldPosition - WorldPosition = Vec3I128 (displacement)
}

impl Add<Vec3I128> for WorldPosition {
    type Output = WorldPosition;
    // WorldPosition + Vec3I128 = WorldPosition (offset position)
}

impl Sub<Vec3I128> for WorldPosition {
    type Output = WorldPosition;
    // WorldPosition - Vec3I128 = WorldPosition
}
```

### Constructors

- `Vec3I128::new(x, y, z)`
- `Vec3I128::zero()` — alias for `Default::default()`
- `Vec3I128::unit_x()`, `unit_y()`, `unit_z()` — basis vectors
- Same pattern for `Vec2I128`

## Outcome

Two vector types (`Vec3I128`, `Vec2I128`) that serve as the displacement counterpart to `WorldPosition`. After this story is complete:

- Subtracting two `WorldPosition` values yields a `Vec3I128`
- Vector arithmetic compiles and passes all operator tests
- Basis vectors are available for constructing directions
- Both types can be used as hash-map keys

## Demo Integration

**Demo crate:** `nebula-demo`

A displacement vector `Vec3I128(0, 0, 100)` moves the position each tick. The window title shows the Z coordinate climbing steadily.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| *(none)* | — | Pure `std` only |

Rust edition 2024. All implementations use `std::ops` traits and core integer methods.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        let a = Vec3I128::new(1, 2, 3);
        let b = Vec3I128::new(10, 20, 30);
        assert_eq!(a + b, Vec3I128::new(11, 22, 33));
    }

    #[test]
    fn test_sub() {
        let a = Vec3I128::new(10, 20, 30);
        let b = Vec3I128::new(1, 2, 3);
        assert_eq!(a - b, Vec3I128::new(9, 18, 27));
    }

    #[test]
    fn test_neg() {
        let v = Vec3I128::new(1, -2, 3);
        assert_eq!(-v, Vec3I128::new(-1, 2, -3));
    }

    #[test]
    fn test_scalar_mul() {
        let v = Vec3I128::new(2, 3, 4);
        assert_eq!(v * 10, Vec3I128::new(20, 30, 40));
    }

    #[test]
    fn test_scalar_div() {
        let v = Vec3I128::new(20, 30, 40);
        assert_eq!(v / 10, Vec3I128::new(2, 3, 4));
    }

    #[test]
    fn test_scalar_div_truncates() {
        let v = Vec3I128::new(7, 7, 7);
        assert_eq!(v / 2, Vec3I128::new(3, 3, 3)); // truncation toward zero
    }

    #[test]
    fn test_zero_vector() {
        let z = Vec3I128::zero();
        assert_eq!(z, Vec3I128::new(0, 0, 0));
        assert_eq!(z, Vec3I128::default());
    }

    #[test]
    fn test_basis_vectors() {
        assert_eq!(Vec3I128::unit_x(), Vec3I128::new(1, 0, 0));
        assert_eq!(Vec3I128::unit_y(), Vec3I128::new(0, 1, 0));
        assert_eq!(Vec3I128::unit_z(), Vec3I128::new(0, 0, 1));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic]
    fn test_overflow_panics_in_debug() {
        let v = Vec3I128::new(i128::MAX, 0, 0);
        let _ = v + Vec3I128::new(1, 0, 0); // overflow
    }

    #[test]
    fn test_checked_add_overflow() {
        let v = Vec3I128::new(i128::MAX, 0, 0);
        assert!(v.checked_add(Vec3I128::new(1, 0, 0)).is_none());
    }

    #[test]
    fn test_saturating_add() {
        let v = Vec3I128::new(i128::MAX, 0, 0);
        let result = v.saturating_add(Vec3I128::new(1, 0, 0));
        assert_eq!(result.x, i128::MAX);
    }

    #[test]
    fn test_vec2_add() {
        let a = Vec2I128::new(1, 2);
        let b = Vec2I128::new(10, 20);
        assert_eq!(a + b, Vec2I128::new(11, 22));
    }

    #[test]
    fn test_world_position_sub_yields_vec3() {
        let a = WorldPosition::new(100, 200, 300);
        let b = WorldPosition::new(10, 20, 30);
        let delta: Vec3I128 = a - b;
        assert_eq!(delta, Vec3I128::new(90, 180, 270));
    }

    #[test]
    fn test_world_position_add_vec3() {
        let pos = WorldPosition::new(10, 20, 30);
        let offset = Vec3I128::new(5, 5, 5);
        assert_eq!(pos + offset, WorldPosition::new(15, 25, 35));
    }
}
```
