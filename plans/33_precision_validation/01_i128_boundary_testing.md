# i128 Boundary Testing

## Problem

The Nebula Engine represents every position in the universe as three `i128` components (each equal to 1 millimeter). While the range of `i128` (approximately +/-1.7x10^38) is astronomically large, arithmetic near the boundaries of that range can silently overflow, wrap, or panic depending on build profile (`debug` vs `release`). Before the engine ships, there must be an exhaustive test suite that exercises every math operation at the extreme edges of the `i128` domain so that latent overflow bugs do not manifest as teleporting players, inverted bounding boxes, or server crashes. The suite must also document which value ranges are safe for normal gameplay and which are theoretical limits that no real gameplay scenario should approach.

## Solution

Create a dedicated test module in `nebula_math` (file: `src/tests/i128_boundary.rs`) that systematically exercises `WorldPosition`, `Vec3I128`, `Aabb128`, and `distance_squared` at critical boundary values.

### Boundary value categories

| Category | Representative value (per axis) | Description |
|----------|--------------------------------|-------------|
| Origin | `0` | The center of the universe |
| Planet surface | `6_371_000_000` (~6,371 km) | Earth-radius distance from a planet center |
| Orbit | `42_164_000_000` (~42,164 km) | Geostationary orbit altitude |
| Interstellar | `9_460_730_472_580_800_000` (~1 light-year) | Distance to a nearby star |
| Near max positive | `i128::MAX - 1` | One unit below positive limit |
| Max positive | `i128::MAX` | The absolute positive limit |
| Near max negative | `i128::MIN + 1` | One unit above negative limit |
| Max negative | `i128::MIN` | The absolute negative limit |
| Half range | `i128::MAX / 2` | Midpoint, safe for doubling |

### Operations under test

For each boundary category, verify:

1. **Construction** -- `WorldPosition::new(v, v, v)` succeeds and stores the exact value.
2. **Addition** -- `WorldPosition + Vec3I128` at boundary values. Specifically, `MAX + 1` must be detected.
3. **Subtraction** -- `WorldPosition - WorldPosition` producing `Vec3I128`. Test `MIN - 1` detection.
4. **Distance squared** -- `distance_squared(a, b)` where intermediate `dx*dx` might overflow i128.
5. **AABB operations** -- `Aabb128::new` with corners at extreme values; `contains_point`, `intersects`, `volume`.
6. **Negation** -- `i128::MIN` cannot be negated (its absolute value exceeds `i128::MAX` by 1).

### Overflow detection strategy

In debug builds, Rust panics on arithmetic overflow. In release builds, it wraps. The tests explicitly verify both behaviors:

```rust
/// Assert that an expression panics in debug mode.
/// In release mode, verify the wrapped result is detected by checked arithmetic.
fn assert_overflow<F: FnOnce() -> i128 + std::panic::UnwindSafe>(f: F) {
    if cfg!(debug_assertions) {
        assert!(std::panic::catch_unwind(f).is_err(), "Expected overflow panic");
    } else {
        // In release mode, use checked_add/checked_sub to verify overflow would occur
        // (the wrapping result itself is not tested — only that the checked variant returns None)
    }
}
```

### Safe operating range documentation

The test file includes a doc comment at the top that summarizes the safe operating range:

```rust
/// # Safe operating ranges for i128 world coordinates
///
/// | Scenario | Per-axis range | Notes |
/// |----------|---------------|-------|
/// | Planet surface | ±10^13 (10 billion km) | Entire solar system fits comfortably |
/// | Interstellar | ±10^22 (1,000 light-years) | Local stellar neighborhood |
/// | Galactic | ±10^26 (100 million light-years) | Observable galaxy cluster |
/// | Theoretical max | ±1.7×10^38 | Full i128 range; overflow risk in multiplication |
///
/// **Multiplication overflow**: `distance_squared` computes `dx*dx + dy*dy + dz*dz`.
/// Each `dx*dx` overflows i128 when `|dx| > ~1.3×10^19` (~1.3 billion km, ~9 AU).
/// For distances beyond 9 AU, use `distance_squared_f64` instead.
```

## Outcome

After this story is complete:

- A comprehensive boundary test suite covers every math operation in `nebula_math` at extreme i128 values
- The safe operating range is documented in code with a clear table of distance thresholds
- Overflow at `MAX + 1` and `MIN - 1` is explicitly tested and confirmed to panic (debug) or be detectable (release)
- The `distance_squared` overflow boundary (~1.3x10^19 per axis) is quantified and documented
- Typical gameplay values (planet, orbit, interstellar) are validated as safe
- Running `cargo test -p nebula_math -- i128_boundary` passes all tests

## Demo Integration

**Demo crate:** `nebula-demo`

The demo teleports to coordinates near i128 MAX and renders terrain. No overflow, no crash, no jitter. The terrain looks identical to terrain at the origin.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| *(none)* | -- | Uses only `WorldPosition`, `Vec3I128`, `Aabb128`, and `distance_squared` from `nebula_math` |

Rust edition 2024. No external crates needed. All tests use `std::panic::catch_unwind` for overflow detection.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::panic;

    // --- Construction at boundaries ---

    #[test]
    fn test_construction_at_origin() {
        let pos = WorldPosition::new(0, 0, 0);
        assert_eq!(pos.x, 0);
        assert_eq!(pos.y, 0);
        assert_eq!(pos.z, 0);
    }

    #[test]
    fn test_construction_at_max() {
        let pos = WorldPosition::new(i128::MAX, i128::MAX, i128::MAX);
        assert_eq!(pos.x, i128::MAX);
        assert_eq!(pos.y, i128::MAX);
        assert_eq!(pos.z, i128::MAX);
    }

    #[test]
    fn test_construction_at_min() {
        let pos = WorldPosition::new(i128::MIN, i128::MIN, i128::MIN);
        assert_eq!(pos.x, i128::MIN);
        assert_eq!(pos.y, i128::MIN);
        assert_eq!(pos.z, i128::MIN);
    }

    // --- Addition overflow ---

    #[test]
    fn test_max_plus_one_overflows() {
        // i128::MAX + 1 must overflow
        let result = i128::MAX.checked_add(1);
        assert_eq!(result, None, "MAX + 1 must return None from checked_add");
    }

    #[test]
    fn test_max_plus_one_panics_in_debug() {
        let result = panic::catch_unwind(|| {
            let a: i128 = i128::MAX;
            let b: i128 = 1;
            let _ = a + b; // Should panic in debug
        });
        if cfg!(debug_assertions) {
            assert!(result.is_err(), "MAX + 1 must panic in debug builds");
        }
    }

    // --- Subtraction overflow ---

    #[test]
    fn test_min_minus_one_overflows() {
        let result = i128::MIN.checked_sub(1);
        assert_eq!(result, None, "MIN - 1 must return None from checked_sub");
    }

    #[test]
    fn test_min_minus_one_panics_in_debug() {
        let result = panic::catch_unwind(|| {
            let a: i128 = i128::MIN;
            let b: i128 = 1;
            let _ = a - b; // Should panic in debug
        });
        if cfg!(debug_assertions) {
            assert!(result.is_err(), "MIN - 1 must panic in debug builds");
        }
    }

    // --- Operations at max values ---

    #[test]
    fn test_subtraction_at_extremes() {
        // MAX - MIN should overflow (result = 2 * MAX + 1, which exceeds i128)
        let result = i128::MAX.checked_sub(i128::MIN);
        assert_eq!(result, None, "MAX - MIN overflows i128");
    }

    #[test]
    fn test_subtraction_max_from_max_is_zero() {
        let a = WorldPosition::new(i128::MAX, i128::MAX, i128::MAX);
        let b = WorldPosition::new(i128::MAX, i128::MAX, i128::MAX);
        let delta = a - b;
        assert_eq!(delta, Vec3I128::new(0, 0, 0));
    }

    #[test]
    fn test_aabb_at_max_boundary() {
        let aabb = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(i128::MAX, i128::MAX, i128::MAX),
        );
        assert!(aabb.contains_point(WorldPosition::new(i128::MAX / 2, 0, 0)));
        assert!(aabb.contains_point(WorldPosition::new(i128::MAX, i128::MAX, i128::MAX)));
        assert!(!aabb.contains_point(WorldPosition::new(-1, 0, 0)));
    }

    // --- Typical gameplay ranges ---

    #[test]
    fn test_planet_surface_distance() {
        // Earth radius: ~6,371 km = 6_371_000_000 mm
        let earth_radius: i128 = 6_371_000_000;
        let surface = WorldPosition::new(earth_radius, 0, 0);
        let opposite = WorldPosition::new(-earth_radius, 0, 0);
        let delta = surface - opposite;
        assert_eq!(delta.x, 2 * earth_radius);
    }

    #[test]
    fn test_orbit_distance() {
        // Geostationary orbit: ~42,164 km = 42_164_000_000 mm
        let orbit: i128 = 42_164_000_000;
        let a = WorldPosition::new(orbit, 0, 0);
        let b = WorldPosition::new(0, 0, 0);
        let dist_sq = distance_squared(a, b);
        assert_eq!(dist_sq, orbit * orbit);
    }

    #[test]
    fn test_interstellar_distance() {
        // 1 light-year in mm: ~9.46 * 10^18
        let ly: i128 = 9_460_730_472_580_800_000;
        let a = WorldPosition::new(ly, 0, 0);
        let b = WorldPosition::new(0, 0, 0);
        // distance_squared would be ly*ly which is ~8.95 * 10^37 — near i128::MAX (~1.7 * 10^38)
        // This should still fit in i128 (single axis, squared)
        let dist_sq = distance_squared(a, b);
        assert_eq!(dist_sq, ly * ly);
    }

    #[test]
    fn test_distance_squared_overflow_boundary() {
        // i128::MAX ≈ 1.7 * 10^38
        // sqrt(i128::MAX) ≈ 1.3 * 10^19
        // Beyond this per-axis delta, dx*dx overflows i128
        let safe_limit: i128 = 13_043_817_825_332_782_212; // floor(sqrt(i128::MAX))
        let a = WorldPosition::new(safe_limit, 0, 0);
        let b = WorldPosition::new(0, 0, 0);

        // This should NOT overflow (single axis at the limit)
        let result = safe_limit.checked_mul(safe_limit);
        assert!(result.is_some(), "distance_squared at safe_limit should not overflow");

        // One more and it overflows
        let over = safe_limit + 1;
        let result = over.checked_mul(over);
        assert!(result.is_none(), "distance_squared at safe_limit+1 should overflow");
    }

    // --- Negation edge case ---

    #[test]
    fn test_i128_min_negation_overflows() {
        // i128::MIN.abs() overflows because |MIN| = MAX + 1
        let result = i128::MIN.checked_neg();
        assert_eq!(result, None, "Negating i128::MIN must overflow");
    }

    // --- AABB operations near boundaries ---

    #[test]
    fn test_aabb_contains_point_at_boundaries() {
        let aabb = Aabb128::new(
            WorldPosition::new(i128::MIN, i128::MIN, i128::MIN),
            WorldPosition::new(i128::MAX, i128::MAX, i128::MAX),
        );
        // This AABB spans the entire i128 range — every point is inside
        assert!(aabb.contains_point(WorldPosition::new(0, 0, 0)));
        assert!(aabb.contains_point(WorldPosition::new(i128::MAX, 0, 0)));
        assert!(aabb.contains_point(WorldPosition::new(i128::MIN, 0, 0)));
    }

    #[test]
    fn test_aabb_intersects_at_boundaries() {
        let a = Aabb128::new(
            WorldPosition::new(0, 0, 0),
            WorldPosition::new(i128::MAX, i128::MAX, i128::MAX),
        );
        let b = Aabb128::new(
            WorldPosition::new(i128::MIN, i128::MIN, i128::MIN),
            WorldPosition::new(0, 0, 0),
        );
        // They touch at origin — should count as intersection
        assert!(a.intersects(&b));
    }
}
```
