//! Distance-based LOD selection with configurable thresholds and hysteresis.

use nebula_math::WorldPosition;

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
    ///
    /// # Panics
    ///
    /// Panics if thresholds are not strictly increasing or contain non-positive values.
    pub fn custom(thresholds: Vec<f64>) -> Self {
        assert!(!thresholds.is_empty(), "must have at least one threshold");
        for (i, &t) in thresholds.iter().enumerate() {
            assert!(t > 0.0, "thresholds must be positive");
            if i > 0 {
                assert!(
                    t > thresholds[i - 1],
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

    /// Return a reference to the threshold distances.
    pub fn thresholds(&self) -> &[f64] {
        &self.thresholds
    }
}

/// Selects LOD levels based on distance from the camera.
pub struct LodSelector {
    thresholds: LodThresholds,
}

impl LodSelector {
    /// Create a new LOD selector with the given thresholds.
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

    /// Access the underlying thresholds.
    pub fn thresholds(&self) -> &LodThresholds {
        &self.thresholds
    }
}

/// Compute the Euclidean distance from a chunk center to the camera, in millimeters.
///
/// Uses i128 displacements converted to f64 for the final calculation.
/// The f64 conversion is safe because LOD thresholds operate at scales
/// where f64 precision (~15 significant digits) is more than sufficient.
pub fn chunk_distance_to_camera(chunk_center: &WorldPosition, camera_pos: &WorldPosition) -> f64 {
    let dx = (chunk_center.x - camera_pos.x) as f64;
    let dy = (chunk_center.y - camera_pos.y) as f64;
    let dz = (chunk_center.z - camera_pos.z) as f64;
    (dx * dx + dy * dy + dz * dz).sqrt()
}

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

    /// LOD levels at exact threshold boundaries.
    #[test]
    fn test_threshold_boundary_behavior() {
        let selector = default_selector();
        assert_eq!(selector.select_lod(255.999), 0);
        assert_eq!(selector.select_lod(256.0), 1);
        assert_eq!(selector.select_lod(511.999), 1);
        assert_eq!(selector.select_lod(512.0), 2);
    }

    /// LOD level should be monotonically non-decreasing with distance.
    #[test]
    fn test_monotonically_increasing_with_distance() {
        let selector = default_selector();
        let distances = [
            0.0, 50.0, 256.0, 400.0, 512.0, 800.0, 1024.0, 2000.0, 5000.0, 100_000.0,
        ];
        let mut prev_lod = 0u8;
        for &d in &distances {
            let lod = selector.select_lod(d);
            assert!(
                lod >= prev_lod,
                "LOD must not decrease with distance: d={d}, lod={lod}, prev={prev_lod}"
            );
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
        assert_eq!(selector.select_lod(500.0), 3);
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

    /// Distance calculation between two world positions.
    #[test]
    fn test_chunk_distance_to_camera() {
        let a = WorldPosition::new(0, 0, 0);
        let b = WorldPosition::new(3000, 4000, 0);
        let dist = chunk_distance_to_camera(&a, &b);
        assert!((dist - 5000.0).abs() < 0.001);
    }
}
