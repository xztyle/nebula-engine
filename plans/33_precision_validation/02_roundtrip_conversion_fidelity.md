# Roundtrip Conversion Fidelity

## Problem

The Nebula Engine's precision architecture relies on converting between coordinate spaces: `WorldPosition` (i128, millimeter units) to `LocalPosition` (f32, camera-relative) for rendering, `WorldPosition` to `SectorCoord` for spatial partitioning, and i128 to f64 for distance calculations. Each conversion introduces potential precision loss. If the roundtrip error is not measured, bounded, and documented, systems will silently disagree about where an entity is -- physics places a block at one position, rendering draws it at another, and the player sees a visible crack between chunks. This story builds a test suite that quantifies the exact error at every distance scale the engine operates at and establishes hard limits the rest of the engine can rely on.

## Solution

Create a test module in `nebula_math` (file: `src/tests/roundtrip_fidelity.rs`) that systematically measures conversion error at increasing distances from the local origin.

### Conversion paths under test

1. **WorldPosition -> LocalPosition -> WorldPosition** (the primary rendering path)
2. **WorldPosition -> SectorCoord -> WorldPosition** (the spatial partitioning path)
3. **i128 -> f64 -> i128** (the distance calculation path)

### Path 1: World -> Local -> World

The `to_local` function subtracts the camera origin in i128 space and casts the delta to f32. The `to_world` function rounds the f32 back to i128 and adds the origin. The roundtrip error depends entirely on the magnitude of the delta -- not the absolute position (the subtraction is exact in i128):

```rust
/// Measure the per-axis roundtrip error in millimeters.
fn roundtrip_error_mm(world_pos: WorldPosition, origin: WorldPosition) -> Vec3I128 {
    let local = to_local(world_pos, origin);
    let recovered = to_world(local, origin);
    recovered - world_pos
}
```

Error expectations by distance from origin (single axis):

| Distance from origin | Delta (mm) | f32 mantissa bits | Quantization step | Max roundtrip error |
|---------------------|-----------|-------------------|-------------------|-------------------|
| 0 m | 0 | N/A | 0 | 0 mm (exact) |
| 1 m | 1,000 | 23 bits | 1 mm | 0 mm (exact, delta < 2^23) |
| 100 m | 100,000 | 23 bits | 1 mm | 0 mm (exact, delta < 2^23) |
| 1 km | 1,000,000 | 23 bits | 1 mm | < 1 mm (delta < 2^23 = 8,388,608) |
| 10 km | 10,000,000 | 23 bits | 2 mm | < 2 mm (delta > 2^23) |
| 100 km | 100,000,000 | 23 bits | 8 mm | < 16 mm |
| 1,000 km | 1,000,000,000 | 23 bits | 64 mm | < 128 mm |

The hard requirement: **error must be < 1 mm for any position within 1 km of the origin** (delta < 1,000,000 mm, well below f32's exact integer range of 2^23 = 8,388,608).

### Path 2: World -> Sector -> World

This conversion uses only bit shifts and masks (see story 03_coords/02). It is mathematically lossless by construction. The test confirms this for every test position including extreme values.

```rust
fn sector_roundtrip(pos: WorldPosition) -> WorldPosition {
    let sector = SectorCoord::from_world(&pos);
    sector.to_world()
}
```

### Path 3: i128 -> f64 -> i128

When computing distances, the engine sometimes casts i128 deltas to f64 for square root. f64 has 52 mantissa bits, so it represents integers exactly up to 2^53 = 9,007,199,254,740,992 (approximately 9,007 km). Beyond that, precision degrades:

```rust
fn f64_roundtrip(value: i128) -> i128 {
    (value as f64).round() as i128
}
```

| Value magnitude | f64 exact? | Roundtrip error |
|----------------|-----------|-----------------|
| < 2^53 (~9,007 km) | Yes | 0 |
| 2^53 to 2^54 | +/- 1 unit | <= 2 mm |
| 2^63 (~9.2 * 10^18, ~1 light-year) | +/- 2^10 | <= 1,024 mm (~1 m) |
| 2^80 | +/- 2^27 | ~134 km |

## Outcome

After this story is complete:

- The WorldPosition -> LocalPosition -> WorldPosition roundtrip error is measured at 7 distance scales (0 m, 1 m, 100 m, 1 km, 10 km, 100 km, 1,000 km)
- The < 1 mm error guarantee within 1 km is enforced by a hard-failing test
- The SectorCoord roundtrip is proven lossless across all test positions including `i128::MAX` and `i128::MIN`
- The i128 -> f64 -> i128 roundtrip error is documented for planet-scale and interstellar-scale values
- A precision degradation table is embedded in the source as a doc comment for other developers to reference
- Running `cargo test -p nebula_math -- roundtrip_fidelity` passes all tests

## Demo Integration

**Demo crate:** `nebula-demo`

The demo converts positions between i128 and f32 at various distances and displays the accumulated error: `At 1km: 0mm. At 1000km: 0.1mm. At 1Mkm: 100mm error.`

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| *(none)* | -- | Uses only types from `nebula_math` and `nebula_coords` (workspace crates) |

Rust edition 2024. No external crates needed.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    // --- World -> Local -> World roundtrip ---

    #[test]
    fn test_roundtrip_error_at_zero_is_exact() {
        let origin = WorldPosition::new(999_999_999, 999_999_999, 999_999_999);
        let pos = origin; // Same as origin: delta is 0
        let local = to_local(pos, origin);
        let recovered = to_world(local, origin);
        assert_eq!(recovered, pos, "Roundtrip at zero delta must be exact");
    }

    #[test]
    fn test_roundtrip_error_at_1m() {
        let origin = WorldPosition::new(0, 0, 0);
        let pos = WorldPosition::new(1_000, 1_000, 1_000); // 1 meter
        let local = to_local(pos, origin);
        let recovered = to_world(local, origin);
        assert_eq!(recovered, pos, "Roundtrip at 1 m must be exact (delta < 2^23)");
    }

    #[test]
    fn test_roundtrip_error_at_100m() {
        let origin = WorldPosition::new(0, 0, 0);
        let pos = WorldPosition::new(100_000, 100_000, 100_000); // 100 meters
        let local = to_local(pos, origin);
        let recovered = to_world(local, origin);
        assert_eq!(recovered, pos, "Roundtrip at 100 m must be exact");
    }

    #[test]
    fn test_roundtrip_error_at_1km_under_1mm() {
        let origin = WorldPosition::new(0, 0, 0);
        let pos = WorldPosition::new(1_000_000, 1_000_000, 1_000_000); // 1 km
        let local = to_local(pos, origin);
        let recovered = to_world(local, origin);
        let error = recovered - pos;
        assert!(
            error.x.abs() < 1 && error.y.abs() < 1 && error.z.abs() < 1,
            "Roundtrip error at 1 km must be < 1 mm, got: ({}, {}, {})",
            error.x, error.y, error.z,
        );
    }

    #[test]
    fn test_roundtrip_error_at_100km_documented() {
        let origin = WorldPosition::new(0, 0, 0);
        let pos = WorldPosition::new(100_000_000, 100_000_000, 100_000_000); // 100 km
        let local = to_local(pos, origin);
        let recovered = to_world(local, origin);
        let error = recovered - pos;
        // At 100 km, f32 quantizes at ~8 mm steps; roundtrip error should be < 16 mm
        assert!(
            error.x.abs() <= 16 && error.y.abs() <= 16 && error.z.abs() <= 16,
            "Roundtrip error at 100 km must be <= 16 mm, got: ({}, {}, {})",
            error.x, error.y, error.z,
        );
    }

    #[test]
    fn test_roundtrip_large_origin_small_delta() {
        // Camera is 10 light-years away; object is 50 cm from camera
        let ly_mm: i128 = 9_460_730_472_580_800_000;
        let origin = WorldPosition::new(10 * ly_mm, 10 * ly_mm, 10 * ly_mm);
        let pos = WorldPosition::new(
            10 * ly_mm + 500, // 50 cm offset
            10 * ly_mm + 500,
            10 * ly_mm + 500,
        );
        let local = to_local(pos, origin);
        let recovered = to_world(local, origin);
        assert_eq!(
            recovered, pos,
            "Small delta must be exact regardless of absolute position"
        );
    }

    // --- Sector roundtrip ---

    #[test]
    fn test_sector_roundtrip_is_lossless_origin() {
        let pos = WorldPosition::new(0, 0, 0);
        let sector = SectorCoord::from_world(&pos);
        let recovered = sector.to_world();
        assert_eq!(recovered, pos, "Sector roundtrip at origin must be lossless");
    }

    #[test]
    fn test_sector_roundtrip_is_lossless_negative() {
        let pos = WorldPosition::new(-1, -4_294_967_296, -999_999_999_999);
        let sector = SectorCoord::from_world(&pos);
        let recovered = sector.to_world();
        assert_eq!(recovered, pos, "Sector roundtrip for negative coords must be lossless");
    }

    #[test]
    fn test_sector_roundtrip_is_lossless_max() {
        let pos = WorldPosition::new(i128::MAX, i128::MIN, 42);
        let sector = SectorCoord::from_world(&pos);
        let recovered = sector.to_world();
        assert_eq!(recovered, pos, "Sector roundtrip at extreme values must be lossless");
    }

    // --- i128 -> f64 -> i128 roundtrip ---

    #[test]
    fn test_f64_roundtrip_exact_within_2pow53() {
        // f64 represents integers exactly up to 2^53
        let value: i128 = 9_007_199_254_740_992; // 2^53, ~9,007 km
        let roundtripped = (value as f64).round() as i128;
        assert_eq!(roundtripped, value, "f64 roundtrip must be exact within 2^53");
    }

    #[test]
    fn test_f64_roundtrip_preserves_at_planet_scale() {
        // Earth radius: 6,371 km = 6_371_000_000 mm (well under 2^53)
        let value: i128 = 6_371_000_000;
        let roundtripped = (value as f64).round() as i128;
        let error = (roundtripped - value).abs();
        assert!(
            error <= 1,
            "f64 roundtrip at planet scale must preserve to +/-1 unit, got error: {}",
            error,
        );
    }

    #[test]
    fn test_f64_roundtrip_at_interstellar_scale() {
        // 1 light-year: ~9.46 * 10^18 mm (between 2^62 and 2^63)
        let value: i128 = 9_460_730_472_580_800_000;
        let roundtripped = (value as f64).round() as i128;
        let error = (roundtripped - value).abs();
        // f64 has 52 mantissa bits; value needs ~63 bits; error up to 2^(63-52) = 2^11 = 2048
        assert!(
            error <= 2048,
            "f64 roundtrip at interstellar scale: error {} must be <= 2048 mm (~2 m)",
            error,
        );
    }

    // --- Systematic sweep ---

    #[test]
    fn test_roundtrip_sweep_distances() {
        let origin = WorldPosition::new(0, 0, 0);
        let distances_mm: &[i128] = &[
            0,                  // 0 m
            1_000,              // 1 m
            100_000,            // 100 m
            1_000_000,          // 1 km
            10_000_000,         // 10 km
            100_000_000,        // 100 km
            1_000_000_000,      // 1,000 km
        ];
        for &d in distances_mm {
            let pos = WorldPosition::new(d, d, d);
            let local = to_local(pos, origin);
            let recovered = to_world(local, origin);
            let error = recovered - pos;
            // Just assert no catastrophic failure; specific bounds are tested above
            let max_err = error.x.abs().max(error.y.abs()).max(error.z.abs());
            assert!(
                max_err < 1_000, // Less than 1 meter error even at 1,000 km
                "Roundtrip error at distance {} mm was {} mm (must be < 1000)",
                d, max_err,
            );
        }
    }
}
```
