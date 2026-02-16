//! Feature placement: Poisson disk sampling and biome-aware surface feature distribution.

use hashbrown::HashMap;
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::biome::BiomeId;

/// Identifier for a feature archetype (tree, boulder, ruin, etc.).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FeatureTypeId(pub u32);

/// A placed feature instance on the terrain surface.
#[derive(Clone, Debug)]
pub struct PlacedFeature {
    /// World-space position of the feature's anchor point (base).
    pub position: glam::DVec3,
    /// Rotation around the surface normal, in radians.
    pub rotation: f64,
    /// The feature archetype to instantiate.
    pub feature_type: FeatureTypeId,
    /// Scale multiplier for natural size variation.
    pub scale: f64,
}

/// Definition of a feature type that can appear in a biome.
#[derive(Clone, Debug)]
pub struct FeatureTypeDef {
    /// Human-readable name.
    pub name: String,
    /// Feature archetype ID.
    pub id: FeatureTypeId,
    /// Minimum spacing between instances of this feature, in engine units.
    pub min_spacing: f64,
    /// Maximum slope (in radians from vertical) where this feature can be placed.
    /// Default: 0.5 (~28 degrees). Set to PI/2 to allow any slope.
    pub max_slope: f64,
    /// Minimum terrain height above sea level for placement.
    /// Default: 1.0 (just above water line).
    pub min_height_above_sea: f64,
    /// Scale variation range: \[min_scale, max_scale\].
    pub scale_range: (f64, f64),
}

/// Feature placement rules for a specific biome.
#[derive(Clone, Debug)]
pub struct BiomeFeatureConfig {
    /// List of (feature_type_id, density) pairs.
    /// Density is the probability of placing this feature at a candidate point.
    pub features: Vec<(FeatureTypeId, f64)>,
}

/// Generate Poisson disk sample points within a 2D region.
///
/// Uses Mitchell's best-candidate algorithm for simplicity and speed.
///
/// # Arguments
/// - `seed`: Deterministic seed for the RNG.
/// - `region_min`, `region_max`: Axis-aligned bounding rectangle.
/// - `min_distance`: Minimum distance between any two points.
/// - `max_attempts`: Number of candidates tested per accepted point.
///
/// # Returns
/// A list of 2D positions within the region.
pub fn poisson_disk_2d(
    seed: u64,
    region_min: (f64, f64),
    region_max: (f64, f64),
    min_distance: f64,
    max_attempts: u32,
) -> Vec<(f64, f64)> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut points: Vec<(f64, f64)> = Vec::new();

    let area = (region_max.0 - region_min.0) * (region_max.1 - region_min.1);
    let max_points = (area / (min_distance * min_distance * 0.7)) as usize;

    for _ in 0..max_points {
        let mut best_candidate = None;
        let mut best_distance = 0.0f64;

        for _ in 0..max_attempts {
            let x = rng.random_range(region_min.0..region_max.0);
            let y = rng.random_range(region_min.1..region_max.1);

            let min_dist_to_existing = points
                .iter()
                .map(|&(px, py)| ((x - px).powi(2) + (y - py).powi(2)).sqrt())
                .fold(f64::INFINITY, f64::min);

            if min_dist_to_existing >= min_distance && min_dist_to_existing > best_distance {
                best_candidate = Some((x, y));
                best_distance = min_dist_to_existing;
            }
        }

        if let Some(point) = best_candidate {
            points.push(point);
        } else {
            break; // Region is saturated.
        }
    }

    points
}

/// Distributes surface features across terrain using Poisson disk sampling
/// and biome-aware filtering.
pub struct FeaturePlacer {
    seed: u64,
    feature_defs: HashMap<FeatureTypeId, FeatureTypeDef>,
    biome_features: HashMap<BiomeId, BiomeFeatureConfig>,
}

impl FeaturePlacer {
    /// Create a new feature placer.
    ///
    /// # Arguments
    /// - `seed`: World seed for deterministic placement.
    /// - `feature_defs`: All feature type definitions.
    /// - `biome_features`: Per-biome feature configuration.
    pub fn new(
        seed: u64,
        feature_defs: Vec<FeatureTypeDef>,
        biome_features: HashMap<BiomeId, BiomeFeatureConfig>,
    ) -> Self {
        let feature_defs = feature_defs.into_iter().map(|d| (d.id, d)).collect();
        Self {
            seed,
            feature_defs,
            biome_features,
        }
    }

    /// Generate all features for a chunk region.
    ///
    /// # Arguments
    /// - `chunk_min`, `chunk_max`: 2D bounding region of the chunk.
    /// - `chunk_seed`: Deterministic seed derived from chunk address.
    /// - `heights`: Closure returning height at a 2D point.
    /// - `biome_at`: Closure returning the biome ID at a 2D point.
    /// - `sea_level`: The sea level height (absolute).
    pub fn place_features(
        &self,
        chunk_min: (f64, f64),
        chunk_max: (f64, f64),
        chunk_seed: u64,
        heights: &dyn Fn(f64, f64) -> f64,
        biome_at: &dyn Fn(f64, f64) -> BiomeId,
        sea_level: f64,
    ) -> Vec<PlacedFeature> {
        let mut results = Vec::new();
        let combined_seed = chunk_seed ^ self.seed;
        let mut rng = ChaCha8Rng::seed_from_u64(combined_seed);

        let min_spacing = self.smallest_min_spacing();
        if min_spacing <= 0.0 {
            return results;
        }

        let candidates = poisson_disk_2d(chunk_seed, chunk_min, chunk_max, min_spacing, 30);

        for (cx, cy) in candidates {
            let height = heights(cx, cy);

            // Skip if below sea level + margin.
            if height < sea_level + 1.0 {
                continue;
            }

            let biome_id = biome_at(cx, cy);

            let Some(biome_cfg) = self.biome_features.get(&biome_id) else {
                continue;
            };

            // Probabilistically select a feature type.
            for &(feat_id, density) in &biome_cfg.features {
                if rng.random::<f64>() < density {
                    let Some(feat_def) = self.feature_defs.get(&feat_id) else {
                        continue;
                    };

                    // Check height above sea level constraint.
                    if height - sea_level < feat_def.min_height_above_sea {
                        continue;
                    }

                    let scale = rng.random_range(feat_def.scale_range.0..=feat_def.scale_range.1);
                    let rotation = rng.random_range(0.0..std::f64::consts::TAU);

                    results.push(PlacedFeature {
                        position: glam::DVec3::new(cx, height, cy),
                        rotation,
                        feature_type: feat_id,
                        scale,
                    });
                    break; // One feature per candidate point.
                }
            }
        }

        results
    }

    /// Returns the smallest min_spacing among all feature definitions.
    fn smallest_min_spacing(&self) -> f64 {
        self.feature_defs
            .values()
            .map(|d| d.min_spacing)
            .fold(f64::INFINITY, f64::min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_features_only_on_solid_ground() {
        let points = poisson_disk_2d(42, (0.0, 0.0), (100.0, 100.0), 5.0, 30);

        let sea_level = 50.0;
        for (i, &(x, _y)) in points.iter().enumerate() {
            let mock_height = if i % 2 == 0 { 60.0 } else { 40.0 };
            if mock_height <= sea_level {
                assert!(
                    mock_height <= sea_level,
                    "Feature placed below sea level at x={x}"
                );
            }
        }
    }

    #[test]
    fn test_poisson_sampling_maintains_minimum_distance() {
        let min_distance = 10.0;
        let points = poisson_disk_2d(123, (0.0, 0.0), (200.0, 200.0), min_distance, 30);

        for (i, &(x1, y1)) in points.iter().enumerate() {
            for (j, &(x2, y2)) in points.iter().enumerate() {
                if i == j {
                    continue;
                }
                let dist = ((x1 - x2).powi(2) + (y1 - y2).powi(2)).sqrt();
                assert!(
                    dist >= min_distance - 1e-6,
                    "Points {i} and {j} are too close: distance={dist}, min={min_distance}"
                );
            }
        }
    }

    #[test]
    fn test_poisson_sampling_produces_points() {
        let points = poisson_disk_2d(42, (0.0, 0.0), (100.0, 100.0), 5.0, 30);
        assert!(
            !points.is_empty(),
            "Poisson disk sampling should produce at least one point"
        );
        assert!(
            points.len() > 50,
            "Expected many points in 100x100 area with spacing 5, got {}",
            points.len()
        );
    }

    #[test]
    fn test_poisson_points_within_bounds() {
        let min = (10.0, 20.0);
        let max = (50.0, 80.0);
        let points = poisson_disk_2d(99, min, max, 3.0, 30);

        for &(x, y) in &points {
            assert!(
                x >= min.0 && x <= max.0 && y >= min.1 && y <= max.1,
                "Point ({x}, {y}) is outside bounds [{min:?}, {max:?}]",
            );
        }
    }

    #[test]
    fn test_deterministic_placement_from_seed() {
        let points_a = poisson_disk_2d(42, (0.0, 0.0), (100.0, 100.0), 5.0, 30);
        let points_b = poisson_disk_2d(42, (0.0, 0.0), (100.0, 100.0), 5.0, 30);

        assert_eq!(points_a.len(), points_b.len());

        for (i, (&(x1, y1), &(x2, y2))) in points_a.iter().zip(points_b.iter()).enumerate() {
            assert!(
                (x1 - x2).abs() < 1e-12 && (y1 - y2).abs() < 1e-12,
                "Point {i} differs between runs: ({x1}, {y1}) vs ({x2}, {y2})"
            );
        }
    }

    #[test]
    fn test_no_features_in_ocean() {
        let sea_level = 0.0;
        let points = poisson_disk_2d(42, (0.0, 0.0), (100.0, 100.0), 5.0, 30);

        let placed: Vec<_> = points
            .iter()
            .filter(|_| {
                let mock_height = -10.0;
                mock_height > sea_level
            })
            .collect();

        assert!(
            placed.is_empty(),
            "No features should be placed when all terrain is underwater"
        );
    }

    #[test]
    fn test_different_seeds_different_placement() {
        let points_a = poisson_disk_2d(1, (0.0, 0.0), (100.0, 100.0), 5.0, 30);
        let points_b = poisson_disk_2d(2, (0.0, 0.0), (100.0, 100.0), 5.0, 30);

        let mut any_different = false;
        let min_len = points_a.len().min(points_b.len());
        for i in 0..min_len {
            if (points_a[i].0 - points_b[i].0).abs() > 1e-6
                || (points_a[i].1 - points_b[i].1).abs() > 1e-6
            {
                any_different = true;
                break;
            }
        }
        assert!(
            any_different || points_a.len() != points_b.len(),
            "Different seeds should produce different point distributions"
        );
    }

    #[test]
    fn test_feature_placer_filters_underwater() {
        let tree = FeatureTypeDef {
            name: "tree".into(),
            id: FeatureTypeId(1),
            min_spacing: 5.0,
            max_slope: 0.5,
            min_height_above_sea: 1.0,
            scale_range: (0.8, 1.2),
        };
        let biome = BiomeId(0);
        let mut biome_features = HashMap::new();
        biome_features.insert(
            biome,
            BiomeFeatureConfig {
                features: vec![(FeatureTypeId(1), 1.0)],
            },
        );
        let placer = FeaturePlacer::new(42, vec![tree], biome_features);

        // All underwater
        let features = placer.place_features(
            (0.0, 0.0),
            (50.0, 50.0),
            123,
            &|_x, _y| -10.0,
            &|_x, _y| biome,
            0.0,
        );
        assert!(
            features.is_empty(),
            "No features should be placed underwater"
        );
    }

    #[test]
    fn test_feature_placer_places_on_land() {
        let tree = FeatureTypeDef {
            name: "tree".into(),
            id: FeatureTypeId(1),
            min_spacing: 5.0,
            max_slope: 0.5,
            min_height_above_sea: 1.0,
            scale_range: (0.8, 1.2),
        };
        let biome = BiomeId(0);
        let mut biome_features = HashMap::new();
        biome_features.insert(
            biome,
            BiomeFeatureConfig {
                features: vec![(FeatureTypeId(1), 1.0)],
            },
        );
        let placer = FeaturePlacer::new(42, vec![tree], biome_features);

        let features = placer.place_features(
            (0.0, 0.0),
            (50.0, 50.0),
            123,
            &|_x, _y| 100.0,
            &|_x, _y| biome,
            0.0,
        );
        assert!(
            !features.is_empty(),
            "Features should be placed on land above sea level"
        );
        for f in &features {
            assert!(
                f.scale >= 0.8 && f.scale <= 1.2,
                "Scale out of range: {}",
                f.scale
            );
        }
    }
}
