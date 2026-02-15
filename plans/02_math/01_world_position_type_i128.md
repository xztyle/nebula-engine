# WorldPosition Type (i128)

## Problem

Every object, voxel, chunk, player, and celestial body in the Nebula Engine universe needs a single, unambiguous position type that can represent locations across interstellar distances without floating-point drift. Standard `f64` coordinates lose sub-millimeter precision beyond a few kilometers from the origin, and `f32` breaks down within meters. A purpose-built integer position type is required that serves as the canonical "address" for everything in the universe, distinct from displacement vectors or local rendering coordinates.

## Solution

Define a `WorldPosition` struct in the `nebula_math` crate:

```rust
/// Canonical position in the universe. Each unit equals 1 millimeter.
///
/// The i128 range of ±1.7×10³⁸ units corresponds to ±1.7×10³⁵ kilometers,
/// or roughly ±18 billion light-years — more than the observable universe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct WorldPosition {
    pub x: i128,
    pub y: i128,
    pub z: i128,
}
```

### Trait implementations

- **`Default`** — returns the origin `(0, 0, 0)`, which is the derive-default behavior for `i128` fields.
- **`Display`** — format as `"WorldPosition(x, y, z)"` using the standard `fmt::Display` trait. Values are printed as plain decimal integers.
- **`From<(i128, i128, i128)>`** — construct from a tuple for ergonomic literal creation.
- **`PartialEq`, `Eq`, `Hash`** — derived, enabling use as `HashMap`/`HashSet` keys (critical for chunk lookup tables).
- **`Clone`, `Copy`** — the struct is 48 bytes (3 × 16), small enough to copy freely on the stack.

### Constructor

Provide a `WorldPosition::new(x: i128, y: i128, z: i128) -> Self` associated function as the primary construction path.

### Design constraints

- This type represents a **position**, not a **displacement**. Arithmetic between two `WorldPosition` values yields a `Vec3I128` (displacement), not another `WorldPosition`. The `Sub` impl (`WorldPosition - WorldPosition`) returns `Vec3I128`. Adding a `Vec3I128` to a `WorldPosition` returns a `WorldPosition`.
- No floating-point fields. Conversion to/from floats is handled by dedicated conversion functions elsewhere (story 07).
- The struct is `#[repr(C)]` to guarantee field layout for FFI and GPU upload if needed in the future.

## Outcome

A single, importable `WorldPosition` type that every other system in the engine can depend on. After this story is complete you can:

- Construct positions from literals: `WorldPosition::new(1000, 2000, 3000)`
- Use positions as hash-map keys for chunk storage
- Print positions for debug logging
- Convert tuples into positions with `.into()`
- Confirm the origin default is `(0, 0, 0)`

## Demo Integration

**Demo crate:** `nebula-demo`

The demo's window title displays a `WorldPosition` at the origin: `Pos: (0, 0, 0)`. The 128-bit coordinate type is alive and visible.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| *(none)* | — | Pure `std` only; no external dependencies for this type |

This type lives in the `nebula_math` crate with Rust edition 2024. No third-party crates are needed — all trait implementations use `std::fmt`, `std::hash`, `std::ops`, and `std::convert`.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn test_construction() {
        let pos = WorldPosition::new(10, -20, 30);
        assert_eq!(pos.x, 10);
        assert_eq!(pos.y, -20);
        assert_eq!(pos.z, 30);
    }

    #[test]
    fn test_default_is_origin() {
        let pos = WorldPosition::default();
        assert_eq!(pos.x, 0);
        assert_eq!(pos.y, 0);
        assert_eq!(pos.z, 0);
    }

    #[test]
    fn test_display_format() {
        let pos = WorldPosition::new(1, -2, 3);
        let s = format!("{}", pos);
        assert_eq!(s, "WorldPosition(1, -2, 3)");
    }

    #[test]
    fn test_equality() {
        let a = WorldPosition::new(5, 5, 5);
        let b = WorldPosition::new(5, 5, 5);
        let c = WorldPosition::new(5, 5, 6);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_hashing_same_position() {
        let mut set = HashSet::new();
        set.insert(WorldPosition::new(1, 2, 3));
        set.insert(WorldPosition::new(1, 2, 3));
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn test_hashing_different_positions() {
        let mut set = HashSet::new();
        set.insert(WorldPosition::new(1, 2, 3));
        set.insert(WorldPosition::new(4, 5, 6));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_from_tuple() {
        let pos: WorldPosition = (100i128, 200i128, 300i128).into();
        assert_eq!(pos, WorldPosition::new(100, 200, 300));
    }

    #[test]
    fn test_extreme_values() {
        let pos = WorldPosition::new(i128::MAX, i128::MIN, 0);
        assert_eq!(pos.x, i128::MAX);
        assert_eq!(pos.y, i128::MIN);
        assert_eq!(pos.z, 0);
    }

    #[test]
    fn test_copy_semantics() {
        let a = WorldPosition::new(1, 2, 3);
        let b = a; // Copy
        assert_eq!(a, b); // `a` is still valid
    }
}
```
