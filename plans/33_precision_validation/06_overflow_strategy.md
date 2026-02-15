# Overflow Strategy

## Problem

Rust's i128 arithmetic behaves differently depending on the build profile: in debug builds, overflow panics; in release builds, it wraps silently. Neither behavior is universally correct for a game engine. A panic crashes the server because a player flew too far. Silent wrapping teleports a player from the edge of the universe to the opposite edge -- a catastrophic, invisible bug. The engine needs an explicit, documented strategy for every arithmetic operation: which operations use checked arithmetic (fail-fast for bug detection), which use saturating arithmetic (clamp to bounds for graceful degradation), and which use wrapping arithmetic (if any). Without this strategy codified in helper functions and enforced by tests, individual developers will use raw `+` and `-` operators inconsistently, and overflow bugs will ship.

## Solution

Define an overflow strategy module in `nebula_math` (file: `src/overflow.rs`) that provides helper functions for every arithmetic pattern used in the engine, each implementing the appropriate overflow behavior.

### Strategy per operation

| Operation | Strategy | Rationale |
|-----------|----------|-----------|
| `WorldPosition + Vec3I128` (movement) | **Checked** | Overflow means an entity escaped the universe -- this is always a bug. Return `Result` so the caller can log and clamp. |
| `WorldPosition - WorldPosition` (delta) | **Checked** | The delta between two valid positions can exceed i128 if they span the full range. Return `Result`. |
| `Vec3I128 + Vec3I128` (velocity accumulation) | **Checked** | Accumulated velocity exceeding i128 is a physics bug. |
| `distance_squared` (dx*dx + dy*dy + dz*dz) | **Saturating** | Overflow in distance means "very far away." Clamping to `i128::MAX` preserves the ordering (farther things have larger distance) without wrapping to a small or negative number. |
| `Aabb128::volume` | **Saturating** | Planet-scale AABBs overflow volume. Saturating to `i128::MAX` means "very large volume," which is correct for sorting and comparison. |
| `Aabb128::expand_by` | **Checked** | Expanding past i128 range is a configuration bug. |
| Sector index computation (bit shift) | **Infallible** | Right-shift of i128 cannot overflow. No strategy needed. |
| `i128 * i128` (intermediate in distance) | **Checked, with f64 fallback** | If checked_mul returns None, fall back to f64 for the entire computation. |

### Helper functions

```rust
use std::num::Wrapping;

/// Error returned when an i128 operation overflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverflowError {
    pub operation: &'static str,
    pub context: &'static str,
}

impl std::fmt::Display for OverflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "i128 overflow in {} ({})", self.operation, self.context)
    }
}

impl std::error::Error for OverflowError {}

/// Checked addition of a displacement to a world position.
/// Returns Err if any axis overflows.
pub fn checked_add_displacement(
    pos: WorldPosition,
    disp: Vec3I128,
) -> Result<WorldPosition, OverflowError> {
    let x = pos.x.checked_add(disp.x).ok_or(OverflowError {
        operation: "add_displacement",
        context: "x axis overflow",
    })?;
    let y = pos.y.checked_add(disp.y).ok_or(OverflowError {
        operation: "add_displacement",
        context: "y axis overflow",
    })?;
    let z = pos.z.checked_add(disp.z).ok_or(OverflowError {
        operation: "add_displacement",
        context: "z axis overflow",
    })?;
    Ok(WorldPosition::new(x, y, z))
}

/// Checked subtraction of two world positions producing a displacement.
/// Returns Err if any axis overflows (positions span more than half the i128 range).
pub fn checked_sub_positions(
    a: WorldPosition,
    b: WorldPosition,
) -> Result<Vec3I128, OverflowError> {
    let x = a.x.checked_sub(b.x).ok_or(OverflowError {
        operation: "sub_positions",
        context: "x axis overflow",
    })?;
    let y = a.y.checked_sub(b.y).ok_or(OverflowError {
        operation: "sub_positions",
        context: "y axis overflow",
    })?;
    let z = a.z.checked_sub(b.z).ok_or(OverflowError {
        operation: "sub_positions",
        context: "z axis overflow",
    })?;
    Ok(Vec3I128::new(x, y, z))
}

/// Checked addition of two displacement vectors.
pub fn checked_add_vectors(
    a: Vec3I128,
    b: Vec3I128,
) -> Result<Vec3I128, OverflowError> {
    let x = a.x.checked_add(b.x).ok_or(OverflowError {
        operation: "add_vectors",
        context: "x axis overflow",
    })?;
    let y = a.y.checked_add(b.y).ok_or(OverflowError {
        operation: "add_vectors",
        context: "y axis overflow",
    })?;
    let z = a.z.checked_add(b.z).ok_or(OverflowError {
        operation: "add_vectors",
        context: "z axis overflow",
    })?;
    Ok(Vec3I128::new(x, y, z))
}

/// Saturating distance squared. If any intermediate multiplication or
/// the final sum overflows, returns i128::MAX instead of wrapping.
///
/// This preserves the distance ordering: if A is farther than B,
/// saturating_distance_squared(A) >= saturating_distance_squared(B).
pub fn saturating_distance_squared(
    a: WorldPosition,
    b: WorldPosition,
) -> i128 {
    let dx = a.x.saturating_sub(b.x);
    let dy = a.y.saturating_sub(b.y);
    let dz = a.z.saturating_sub(b.z);

    let dx2 = match dx.checked_mul(dx) {
        Some(v) => v,
        None => return i128::MAX,
    };
    let dy2 = match dy.checked_mul(dy) {
        Some(v) => v,
        None => return i128::MAX,
    };
    let dz2 = match dz.checked_mul(dz) {
        Some(v) => v,
        None => return i128::MAX,
    };

    dx2.saturating_add(dy2).saturating_add(dz2)
}

/// Saturating volume for Aabb128. Returns i128::MAX if the volume
/// exceeds i128 range.
pub fn saturating_volume(aabb: &Aabb128) -> i128 {
    let dx = aabb.max.x.saturating_sub(aabb.min.x);
    let dy = aabb.max.y.saturating_sub(aabb.min.y);
    let dz = aabb.max.z.saturating_sub(aabb.min.z);

    match dx.checked_mul(dy) {
        Some(dxy) => match dxy.checked_mul(dz) {
            Some(vol) => vol,
            None => i128::MAX,
        },
        None => i128::MAX,
    }
}

/// Checked AABB expansion. Returns Err if expanding by the margin
/// would push min or max past i128 bounds.
pub fn checked_expand(
    aabb: &Aabb128,
    margin: i128,
) -> Result<Aabb128, OverflowError> {
    let min = WorldPosition::new(
        aabb.min.x.checked_sub(margin).ok_or(OverflowError {
            operation: "expand_aabb",
            context: "min.x underflow",
        })?,
        aabb.min.y.checked_sub(margin).ok_or(OverflowError {
            operation: "expand_aabb",
            context: "min.y underflow",
        })?,
        aabb.min.z.checked_sub(margin).ok_or(OverflowError {
            operation: "expand_aabb",
            context: "min.z underflow",
        })?,
    );
    let max = WorldPosition::new(
        aabb.max.x.checked_add(margin).ok_or(OverflowError {
            operation: "expand_aabb",
            context: "max.x overflow",
        })?,
        aabb.max.y.checked_add(margin).ok_or(OverflowError {
            operation: "expand_aabb",
            context: "max.y overflow",
        })?,
        aabb.max.z.checked_add(margin).ok_or(OverflowError {
            operation: "expand_aabb",
            context: "max.z overflow",
        })?,
    );
    Ok(Aabb128::new(min, max))
}
```

### Usage guidelines (doc comment in `overflow.rs`)

```rust
//! # Overflow strategy for i128 arithmetic
//!
//! ## Rules for engine developers
//!
//! 1. **Never use raw `+`, `-`, `*` on i128 values in game logic.**
//!    Use the checked/saturating helpers from this module instead.
//!
//! 2. **Use `checked_*` for operations that indicate a bug if they overflow.**
//!    Movement, delta computation, velocity accumulation.
//!    Log the error, clamp the entity to the universe boundary, and continue.
//!
//! 3. **Use `saturating_*` for operations where overflow means "very large."**
//!    Distance comparison, volume estimation, LOD distance thresholds.
//!
//! 4. **Never use wrapping arithmetic** in game logic. There is no scenario
//!    where modular wrap-around of a world position is correct.
//!
//! 5. **Raw operators are acceptable only in tests and benchmarks**, where
//!    the values are known to be within safe range.
```

## Outcome

After this story is complete:

- Every arithmetic operation in `nebula_math` has a documented overflow strategy (checked or saturating)
- Helper functions `checked_add_displacement`, `checked_sub_positions`, `checked_add_vectors`, `saturating_distance_squared`, `saturating_volume`, and `checked_expand` are available in `nebula_math::overflow`
- An `OverflowError` type provides context about which operation and axis overflowed
- A usage guideline doc comment tells developers which helper to use and prohibits raw operators in game logic
- No silent wrapping can occur in critical paths when the helpers are used
- Running `cargo test -p nebula_math -- overflow` passes all tests

## Demo Integration

**Demo crate:** `nebula-demo`

The demo intentionally creates an overflow condition. The position saturates at i128 MAX instead of wrapping. The console logs the overflow event gracefully.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `fixed` | `1.30` | Fixed-point types for potential future saturating arithmetic on sub-millimeter values |

Rust edition 2024. The `fixed` crate is listed as a dependency for the broader precision system but is not required by the overflow helpers themselves, which operate purely on `i128`. It is included here to document the version pinned across the precision validation stories.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // --- Checked addition ---

    #[test]
    fn test_checked_add_displacement_normal() {
        let pos = WorldPosition::new(100, 200, 300);
        let disp = Vec3I128::new(10, 20, 30);
        let result = checked_add_displacement(pos, disp);
        assert_eq!(result.unwrap(), WorldPosition::new(110, 220, 330));
    }

    #[test]
    fn test_checked_add_displacement_overflow_returns_error() {
        let pos = WorldPosition::new(i128::MAX, 0, 0);
        let disp = Vec3I128::new(1, 0, 0);
        let result = checked_add_displacement(pos, disp);
        assert!(result.is_err(), "Adding 1 to MAX must return Err");
        assert_eq!(result.unwrap_err().operation, "add_displacement");
    }

    #[test]
    fn test_checked_add_displacement_negative_overflow() {
        let pos = WorldPosition::new(i128::MIN, 0, 0);
        let disp = Vec3I128::new(-1, 0, 0);
        let result = checked_add_displacement(pos, disp);
        assert!(result.is_err(), "Subtracting 1 from MIN must return Err");
    }

    // --- Checked subtraction ---

    #[test]
    fn test_checked_sub_positions_normal() {
        let a = WorldPosition::new(100, 200, 300);
        let b = WorldPosition::new(10, 20, 30);
        let result = checked_sub_positions(a, b);
        assert_eq!(result.unwrap(), Vec3I128::new(90, 180, 270));
    }

    #[test]
    fn test_checked_sub_positions_overflow_returns_error() {
        let a = WorldPosition::new(i128::MAX, 0, 0);
        let b = WorldPosition::new(i128::MIN, 0, 0);
        let result = checked_sub_positions(a, b);
        assert!(result.is_err(), "MAX - MIN must overflow");
    }

    // --- Checked vector addition ---

    #[test]
    fn test_checked_add_vectors_normal() {
        let a = Vec3I128::new(100, 200, 300);
        let b = Vec3I128::new(400, 500, 600);
        let result = checked_add_vectors(a, b);
        assert_eq!(result.unwrap(), Vec3I128::new(500, 700, 900));
    }

    #[test]
    fn test_checked_add_vectors_overflow() {
        let a = Vec3I128::new(i128::MAX, 0, 0);
        let b = Vec3I128::new(1, 0, 0);
        let result = checked_add_vectors(a, b);
        assert!(result.is_err());
    }

    // --- Saturating distance squared ---

    #[test]
    fn test_saturating_distance_squared_normal() {
        let a = WorldPosition::new(3_000, 4_000, 0);
        let b = WorldPosition::new(0, 0, 0);
        let dist = saturating_distance_squared(a, b);
        assert_eq!(dist, 25_000_000); // 3000^2 + 4000^2
    }

    #[test]
    fn test_saturating_distance_squared_clamps_on_overflow() {
        // Positions far enough apart that dx*dx overflows i128
        let a = WorldPosition::new(i128::MAX, 0, 0);
        let b = WorldPosition::new(0, 0, 0);
        let dist = saturating_distance_squared(a, b);
        assert_eq!(dist, i128::MAX, "Overflow must saturate to i128::MAX");
    }

    #[test]
    fn test_saturating_distance_preserves_ordering() {
        let origin = WorldPosition::new(0, 0, 0);
        let near = WorldPosition::new(1_000_000, 0, 0); // 1 km
        let far = WorldPosition::new(1_000_000_000, 0, 0); // 1000 km
        let very_far = WorldPosition::new(i128::MAX / 2, 0, 0); // huge

        let d_near = saturating_distance_squared(near, origin);
        let d_far = saturating_distance_squared(far, origin);
        let d_very_far = saturating_distance_squared(very_far, origin);

        assert!(d_near < d_far, "Near must be less than far");
        assert!(d_far < d_very_far, "Far must be less than very_far");
    }

    // --- Saturating volume ---

    #[test]
    fn test_saturating_volume_normal() {
        let aabb = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(10, 20, 30),
        );
        assert_eq!(saturating_volume(&aabb), 6_000);
    }

    #[test]
    fn test_saturating_volume_clamps_on_overflow() {
        let aabb = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(i128::MAX, i128::MAX, i128::MAX),
        );
        assert_eq!(
            saturating_volume(&aabb),
            i128::MAX,
            "Planet-scale AABB volume must saturate to MAX"
        );
    }

    // --- Checked expand ---

    #[test]
    fn test_checked_expand_normal() {
        let aabb = Aabb128::new(
            WorldPosition::new(10, 10, 10),
            WorldPosition::new(20, 20, 20),
        );
        let expanded = checked_expand(&aabb, 5).unwrap();
        assert_eq!(expanded.min, WorldPosition::new(5, 5, 5));
        assert_eq!(expanded.max, WorldPosition::new(25, 25, 25));
    }

    #[test]
    fn test_checked_expand_overflow_returns_error() {
        let aabb = Aabb128::new(
            WorldPosition::new(i128::MIN + 1, 0, 0),
            WorldPosition::new(i128::MAX, 0, 0),
        );
        let result = checked_expand(&aabb, 2);
        assert!(result.is_err(), "Expanding past MIN must return Err");
    }

    // --- Strategy documentation ---

    #[test]
    fn test_no_silent_wrapping_in_critical_path() {
        // Verify that using checked helpers, overflow is always detectable
        // and never silently wraps
        let critical_operations: Vec<Result<(), OverflowError>> = vec![
            checked_add_displacement(
                WorldPosition::new(i128::MAX, 0, 0),
                Vec3I128::new(1, 0, 0),
            ).map(|_| ()),
            checked_sub_positions(
                WorldPosition::new(i128::MAX, 0, 0),
                WorldPosition::new(i128::MIN, 0, 0),
            ).map(|_| ()),
            checked_add_vectors(
                Vec3I128::new(i128::MAX, 0, 0),
                Vec3I128::new(1, 0, 0),
            ).map(|_| ()),
        ];

        for (i, result) in critical_operations.iter().enumerate() {
            assert!(
                result.is_err(),
                "Critical operation {} must detect overflow, not wrap silently",
                i,
            );
        }
    }

    #[test]
    fn test_overflow_error_has_useful_context() {
        let err = checked_add_displacement(
            WorldPosition::new(i128::MAX, 0, 0),
            Vec3I128::new(1, 0, 0),
        ).unwrap_err();

        let message = format!("{}", err);
        assert!(
            message.contains("add_displacement"),
            "Error message must identify the operation"
        );
        assert!(
            message.contains("x axis"),
            "Error message must identify the axis"
        );
    }
}
```
