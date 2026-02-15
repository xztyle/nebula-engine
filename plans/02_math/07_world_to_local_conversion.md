# World-to-Local Conversion

## Problem

This is the most critical precision bridge in the entire engine. Every frame, thousands of world-space positions (chunks, entities, particles) must be converted to camera-relative f32 coordinates for rendering. If done naively — e.g., casting a huge i128 directly to f32 — catastrophic precision loss occurs. The correct approach is to subtract the camera's world position (in i128) first, producing a small displacement, and only then convert the small delta to f32. This story implements that conversion and its inverse.

## Solution

### World-to-local conversion

```rust
/// Convert a world position to a local position relative to the given origin.
///
/// This is the precision-critical path:
/// 1. Subtract origin from world_pos in i128 (exact, no precision loss).
/// 2. The resulting delta is small (within rendering distance).
/// 3. Cast the small delta to f32 (minimal precision loss).
///
/// # Precision guarantee
/// If |world_pos - origin| < 8,388,608 mm (~8.4 km) per axis,
/// the conversion is exact to 1 mm (f32 has 23 mantissa bits).
///
/// If |world_pos - origin| < 83,886,080 mm (~83.9 km) per axis,
/// precision is within 10 mm.
///
/// # Panics
/// Does not panic. For positions extremely far from origin,
/// the f32 result will have poor precision but will not crash.
pub fn to_local(world_pos: WorldPosition, origin: WorldPosition) -> LocalPosition {
    let delta = world_pos - origin; // Vec3I128, exact
    LocalPosition::new(
        delta.x as f32,
        delta.y as f32,
        delta.z as f32,
    )
}
```

### Local-to-world conversion (inverse)

```rust
/// Convert a local position back to a world position given the origin.
///
/// Rounds each f32 component to the nearest i128 (millimeter)
/// and adds it to the origin.
///
/// # Precision
/// The roundtrip world -> local -> world introduces error equal to
/// the f32 quantization at the given delta magnitude. For deltas
/// under 8.4 km, this is at most ±1 mm.
pub fn to_world(local: LocalPosition, origin: WorldPosition) -> WorldPosition {
    let dx = local.x.round() as i128;
    let dy = local.y.round() as i128;
    let dz = local.z.round() as i128;
    origin + Vec3I128::new(dx, dy, dz)
}
```

### Safe conversion with distance check

```rust
/// Maximum recommended delta (per axis) for accurate conversion.
/// 8,388,608 mm = 2²³ mm ≈ 8.389 km.
pub const MAX_SAFE_LOCAL_DELTA: i128 = 8_388_608;

/// Converts world to local, returning Err if any axis delta exceeds
/// the safe range. The conversion still produces a result in the
/// Err variant, but callers should be aware of precision degradation.
pub fn to_local_checked(
    world_pos: WorldPosition,
    origin: WorldPosition,
) -> Result<LocalPosition, LocalPosition> {
    let delta = world_pos - origin;
    let local = LocalPosition::new(
        delta.x as f32,
        delta.y as f32,
        delta.z as f32,
    );
    if delta.x.abs() > MAX_SAFE_LOCAL_DELTA
        || delta.y.abs() > MAX_SAFE_LOCAL_DELTA
        || delta.z.abs() > MAX_SAFE_LOCAL_DELTA
    {
        Err(local)
    } else {
        Ok(local)
    }
}
```

### Batch conversion

```rust
/// Convert a slice of world positions to local positions.
/// This is the hot path called every frame for visible chunks/entities.
///
/// Future optimization: SIMD or rayon parallelism.
pub fn to_local_batch(
    positions: &[WorldPosition],
    origin: WorldPosition,
    out: &mut Vec<LocalPosition>,
) {
    out.clear();
    out.reserve(positions.len());
    for &pos in positions {
        let delta = pos - origin;
        out.push(LocalPosition::new(
            delta.x as f32,
            delta.y as f32,
            delta.z as f32,
        ));
    }
}
```

### Design notes

- The `to_local` function is intentionally simple — a subtraction and a cast. The subtraction is where all the precision magic happens.
- The camera origin should be updated every frame to the camera's current `WorldPosition`.
- For chunk meshes, the origin can be the chunk center rather than the camera, since chunk meshes are pre-built relative to their chunk origin. The per-frame conversion then only needs to handle the chunk-origin-to-camera offset, which is done in the vertex shader via a uniform matrix.

## Outcome

After this story is complete:

- `to_local()` converts any `WorldPosition` to camera-relative `LocalPosition`
- `to_world()` inverts the conversion for picking, physics feedback, etc.
- `to_local_checked()` warns when precision degradation is likely
- `to_local_batch()` provides an efficient path for per-frame bulk conversion
- The rendering pipeline has a clear, auditable precision bridge

## Demo Integration

**Demo crate:** `nebula-demo`

Both world and local coordinates are displayed; local stays small even as world coordinates grow enormous, proving the floating-origin precision bridge works.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| *(none)* | — | Uses only types from `nebula_math` (this crate) |

Rust edition 2024. No new external dependencies beyond what `WorldPosition`, `Vec3I128`, and `LocalPosition` already require.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_at_origin_is_zero() {
        let origin = WorldPosition::new(1000, 2000, 3000);
        let local = to_local(origin, origin);
        assert!(local.approx_eq(LocalPosition::zero(), 1e-6));
    }

    #[test]
    fn test_small_offset() {
        let origin = WorldPosition::new(0, 0, 0);
        let pos = WorldPosition::new(100, 200, 300);
        let local = to_local(pos, origin);
        assert!(local.approx_eq(LocalPosition::new(100.0, 200.0, 300.0), 1e-6));
    }

    #[test]
    fn test_roundtrip_nearby() {
        let origin = WorldPosition::new(1_000_000_000, 2_000_000_000, 3_000_000_000);
        let original = WorldPosition::new(1_000_001_000, 2_000_002_000, 3_000_003_000);
        let local = to_local(original, origin);
        let recovered = to_world(local, origin);
        assert_eq!(recovered, original);
    }

    #[test]
    fn test_large_world_small_delta() {
        // Camera is at 10 light-years from origin.
        // Object is 1 meter away from camera.
        let ly_mm: i128 = 9_460_730_472_580_800_000; // ~9.46×10¹⁸ mm
        let origin = WorldPosition::new(
            10 * ly_mm,
            10 * ly_mm,
            10 * ly_mm,
        );
        let pos = WorldPosition::new(
            10 * ly_mm + 1000, // 1 meter offset
            10 * ly_mm,
            10 * ly_mm,
        );
        let local = to_local(pos, origin);
        // Delta is only 1000 mm — well within f32 precision
        assert!(local.approx_eq(LocalPosition::new(1000.0, 0.0, 0.0), 0.1));
    }

    #[test]
    fn test_negative_delta() {
        let origin = WorldPosition::new(1000, 1000, 1000);
        let pos = WorldPosition::new(500, 500, 500);
        let local = to_local(pos, origin);
        assert!(local.approx_eq(LocalPosition::new(-500.0, -500.0, -500.0), 1e-6));
    }

    #[test]
    fn test_checked_within_safe_range() {
        let origin = WorldPosition::new(0, 0, 0);
        let pos = WorldPosition::new(1_000_000, 0, 0); // 1 km, well under 8.4 km
        assert!(to_local_checked(pos, origin).is_ok());
    }

    #[test]
    fn test_checked_outside_safe_range() {
        let origin = WorldPosition::new(0, 0, 0);
        let pos = WorldPosition::new(100_000_000, 0, 0); // 100 km
        assert!(to_local_checked(pos, origin).is_err());
    }

    #[test]
    fn test_batch_conversion() {
        let origin = WorldPosition::new(0, 0, 0);
        let positions = vec![
            WorldPosition::new(100, 200, 300),
            WorldPosition::new(400, 500, 600),
        ];
        let mut out = Vec::new();
        to_local_batch(&positions, origin, &mut out);
        assert_eq!(out.len(), 2);
        assert!(out[0].approx_eq(LocalPosition::new(100.0, 200.0, 300.0), 1e-6));
        assert!(out[1].approx_eq(LocalPosition::new(400.0, 500.0, 600.0), 1e-6));
    }

    #[test]
    fn test_roundtrip_precision_at_5km() {
        let origin = WorldPosition::new(0, 0, 0);
        let pos = WorldPosition::new(5_000_000, 5_000_000, 5_000_000); // 5 km
        let local = to_local(pos, origin);
        let recovered = to_world(local, origin);
        // At 5 km, f32 should preserve 1mm precision
        let delta = recovered - pos;
        assert!(delta.x.abs() <= 1);
        assert!(delta.y.abs() <= 1);
        assert!(delta.z.abs() <= 1);
    }
}
```
