# Distance-Based LOD

## Problem

In a voxel engine rendering planetary-scale terrain with 128-bit coordinates, it is computationally impossible to render every chunk at full resolution. A planet with a radius of millions of meters contains billions of potential chunks — rendering them all at 32x32x32 resolution would overwhelm any GPU's vertex budget and exhaust system memory. The engine needs a systematic way to select a Level of Detail (LOD) for each chunk based on how far it is from the camera. Chunks near the player must be rendered at full fidelity so individual voxels are visible, while distant chunks should be progressively coarser to keep triangle counts manageable. The LOD selection must be fast (evaluated for thousands of chunks per frame), deterministic (same distance always yields same LOD), and configurable (different planet types or hardware profiles may need different thresholds).

## Solution

Implement a `LodSelector` in the `nebula_lod` crate that maps a distance value to a LOD level using configurable distance thresholds. LOD levels are represented as `u8` values where 0 is full detail and higher values are progressively coarser. Each LOD level halves the voxel resolution of a chunk along each axis:

| LOD Level | Voxels per Chunk | Distance Range (default) |
|-----------|-----------------|--------------------------|
| 0 | 32x32x32 | 0 - 256 m |
| 1 | 16x16x16 | 256 - 512 m |
| 2 | 8x8x8 | 512 - 1,024 m |
| 3 | 4x4x4 | 1,024 - 2,048 m |
| 4 | 2x2x2 | 2,048 - 4,096 m |

### Data Structures

```rust
/// Configuration for distance-based LOD selection.
#[derive(Clone, Debug)]
pub struct LodThresholds {
    /// Distance boundaries between LOD levels.
    /// `thresholds[i]` is the maximum distance for LOD level `i`.
    /// Length determines the number of LOD levels minus one (the last level extends to infinity).
    thresholds: Vec<f64>,
}

impl LodThresholds {
    /// Create default thresholds: 256, 512, 1024, 2048, 4096 meters.
    pub fn default_planet() -> Self {
        Self {
            thresholds: vec![256.0, 512.0, 1024.0, 2048.0, 4096.0],
        }
    }

    /// Create custom thresholds from a list of distance boundaries.
    /// Panics if thresholds are not strictly increasing or contain non-positive values.
    pub fn custom(thresholds: Vec<f64>) -> Self {
        assert!(!thresholds.is_empty(), "must have at least one threshold");
        for i in 0..thresholds.len() {
            assert!(thresholds[i] > 0.0, "thresholds must be positive");
            if i > 0 {
                assert!(
                    thresholds[i] > thresholds[i - 1],
                    "thresholds must be strictly increasing"
                );
            }
        }
        Self { thresholds }
    }

    /// Return the maximum LOD level (the coarsest level of detail).
    pub fn max_lod(&self) -> u8 {
        self.thresholds.len() as u8
    }
}

/// Selects LOD levels based on distance from the camera.
pub struct LodSelector {
    thresholds: LodThresholds,
}

impl LodSelector {
    pub fn new(thresholds: LodThresholds) -> Self {
        Self { thresholds }
    }

    /// Determine the LOD level for a chunk at the given distance from the camera.
    ///
    /// Returns 0 for the closest chunks (full detail) and higher values for
    /// progressively more distant chunks. Distances beyond the last threshold
    /// return `max_lod`.
    pub fn select_lod(&self, distance: f64) -> u8 {
        debug_assert!(distance >= 0.0, "distance must be non-negative");
        for (i, &threshold) in self.thresholds.thresholds.iter().enumerate() {
            if distance < threshold {
                return i as u8;
            }
        }
        self.thresholds.max_lod()
    }

    /// Return the voxel resolution for a given LOD level.
    /// LOD 0 = 32, LOD 1 = 16, LOD 2 = 8, etc.
    pub fn resolution_for_lod(lod: u8) -> u32 {
        32 >> lod
    }
}
```

### Distance Calculation

The distance used for LOD selection is the Euclidean distance from the camera's world position to the nearest point on the chunk's bounding volume. Because the engine uses 128-bit world coordinates, this distance is first computed in full precision as an `i128` displacement, then converted to `f64` for the threshold comparison. The conversion to `f64` is safe because LOD thresholds operate at scales where `f64` precision (approximately 15 significant digits) is more than sufficient.

```rust
pub fn chunk_distance_to_camera(
    chunk_center: &WorldPosition,
    camera_pos: &WorldPosition,
) -> f64 {
    let dx = (chunk_center.x - camera_pos.x) as f64;
    let dy = (chunk_center.y - camera_pos.y) as f64;
    let dz = (chunk_center.z - camera_pos.z) as f64;
    (dx * dx + dy * dy + dz * dz).sqrt()
}
```

### Hysteresis

To avoid rapid LOD switching when the camera is near a threshold boundary, the selector supports an optional hysteresis band. When a chunk is currently at LOD N, it must cross `threshold + hysteresis` before upgrading to LOD N+1, and must come back within `threshold - hysteresis` before downgrading back to LOD N. The default hysteresis is 10% of each threshold distance.

## Outcome

The `nebula_lod` crate exports `LodSelector`, `LodThresholds`, and `chunk_distance_to_camera()`. Given a camera position and a chunk center, the selector returns the appropriate LOD level as a `u8`. Running `cargo test -p nebula_lod` passes all LOD selection tests. The selector evaluates in constant time (a linear scan of the small threshold array, typically 5-8 entries).

## Demo Integration

**Demo crate:** `nebula-demo`

Nearby chunks render at full detail; distant chunks use progressively lower LOD. The console logs `LOD distribution: L0=8, L1=16, L2=32, L3=64`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_math` | workspace | `WorldPosition` (128-bit coordinate type), distance utilities |

No external crates required. LOD selection is pure arithmetic on floating-point thresholds. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn default_selector() -> LodSelector {
        LodSelector::new(LodThresholds::default_planet())
    }

    /// A chunk at distance 0 (directly at the camera) should return LOD 0.
    #[test]
    fn test_zero_distance_returns_lod_0() {
        let selector = default_selector();
        assert_eq!(selector.select_lod(0.0), 0);
    }

    /// A chunk very close to the camera (within 256m) should return LOD 0.
    #[test]
    fn test_very_close_returns_lod_0() {
        let selector = default_selector();
        assert_eq!(selector.select_lod(100.0), 0);
        assert_eq!(selector.select_lod(255.9), 0);
    }

    /// A chunk far beyond all thresholds should return the maximum LOD level.
    #[test]
    fn test_far_distance_returns_max_lod() {
        let selector = default_selector();
        let max = selector.thresholds.max_lod();
        assert_eq!(selector.select_lod(100_000.0), max);
        assert_eq!(selector.select_lod(f64::MAX), max);
    }

    /// LOD levels at exact threshold boundaries: the threshold value itself
    /// should transition to the next LOD level.
    #[test]
    fn test_threshold_boundary_behavior() {
        let selector = default_selector();
        // Just below 256m -> LOD 0
        assert_eq!(selector.select_lod(255.999), 0);
        // At 256m -> LOD 1 (threshold is exclusive lower bound for next level)
        assert_eq!(selector.select_lod(256.0), 1);
        // Just below 512m -> LOD 1
        assert_eq!(selector.select_lod(511.999), 1);
        // At 512m -> LOD 2
        assert_eq!(selector.select_lod(512.0), 2);
    }

    /// LOD level should be monotonically non-decreasing with distance.
    #[test]
    fn test_monotonically_increasing_with_distance() {
        let selector = default_selector();
        let distances = [0.0, 50.0, 256.0, 400.0, 512.0, 800.0, 1024.0, 2000.0, 5000.0, 100_000.0];
        let mut prev_lod = 0u8;
        for &d in &distances {
            let lod = selector.select_lod(d);
            assert!(lod >= prev_lod, "LOD must not decrease with distance: d={d}, lod={lod}, prev={prev_lod}");
            prev_lod = lod;
        }
    }

    /// Custom thresholds should override the defaults and work correctly.
    #[test]
    fn test_custom_thresholds_work() {
        let thresholds = LodThresholds::custom(vec![100.0, 200.0, 400.0]);
        let selector = LodSelector::new(thresholds);
        assert_eq!(selector.select_lod(50.0), 0);
        assert_eq!(selector.select_lod(150.0), 1);
        assert_eq!(selector.select_lod(300.0), 2);
        assert_eq!(selector.select_lod(500.0), 3); // max LOD
    }

    /// Resolution halves with each LOD level.
    #[test]
    fn test_resolution_for_lod() {
        assert_eq!(LodSelector::resolution_for_lod(0), 32);
        assert_eq!(LodSelector::resolution_for_lod(1), 16);
        assert_eq!(LodSelector::resolution_for_lod(2), 8);
        assert_eq!(LodSelector::resolution_for_lod(3), 4);
        assert_eq!(LodSelector::resolution_for_lod(4), 2);
    }

    /// Invalid custom thresholds (non-increasing) should panic.
    #[test]
    #[should_panic(expected = "strictly increasing")]
    fn test_non_increasing_thresholds_panic() {
        LodThresholds::custom(vec![100.0, 50.0, 200.0]);
    }
}
```
