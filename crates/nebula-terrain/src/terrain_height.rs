//! Cubesphere terrain height sampling.
//!
//! Maps 3D sphere-surface noise to physically meaningful terrain heights
//! and integrates with the cubesphere face coordinate system for seamless,
//! distortion-free terrain across all cube faces.

use glam::DVec3;
use nebula_cubesphere::{FaceCoord, face_coord_to_sphere_everitt};

use crate::heightmap::{HeightmapParams, HeightmapSampler};

/// Configuration for how noise maps to physical terrain height.
#[derive(Clone, Debug)]
pub struct TerrainHeightConfig {
    /// Minimum terrain height relative to sea level, in engine units.
    /// Negative values represent depths below sea level (e.g., ocean trenches).
    /// Default: -2_000 (2 km below sea level).
    pub min_height: f64,
    /// Maximum terrain height relative to sea level, in engine units.
    /// Default: 8_000 (8 km above sea level).
    pub max_height: f64,
    /// Sea level as an offset from the planet's base radius, in engine units.
    /// Default: 0 (sea level == base radius).
    pub sea_level: f64,
    /// Planet base radius in engine units.
    /// Default: 6_371_000 (Earth-like, ~6371 km).
    pub planet_radius: f64,
}

impl Default for TerrainHeightConfig {
    fn default() -> Self {
        Self {
            min_height: -2_000.0,
            max_height: 8_000.0,
            sea_level: 0.0,
            planet_radius: 6_371_000.0,
        }
    }
}

/// Samples terrain height on a cubesphere surface using 3D noise.
///
/// Wraps [`HeightmapSampler`] with planet-specific height mapping, producing
/// seamless terrain across all cube faces by sampling in 3D sphere coordinates.
pub struct TerrainHeightSampler {
    heightmap: HeightmapSampler,
    config: TerrainHeightConfig,
}

impl TerrainHeightSampler {
    /// Create a new terrain height sampler.
    pub fn new(heightmap_params: HeightmapParams, config: TerrainHeightConfig) -> Self {
        Self {
            heightmap: HeightmapSampler::new(heightmap_params),
            config,
        }
    }

    /// Sample terrain height at a point on the unit sphere.
    ///
    /// Returns the height above sea level in engine units, clamped to
    /// [`TerrainHeightConfig::min_height`, `TerrainHeightConfig::max_height`].
    pub fn sample_height(&self, sphere_point: DVec3) -> f64 {
        let max_amp = self.heightmap.max_amplitude();

        // Handle zero-amplitude case: map to midpoint of range.
        if max_amp == 0.0 {
            let midpoint = (self.config.min_height + self.config.max_height) * 0.5;
            return midpoint;
        }

        let raw = self.heightmap.sample_3d(sphere_point);

        // Normalize raw noise from [-max_amp, +max_amp] to [0, 1].
        let normalized = (raw / max_amp + 1.0) * 0.5;

        // Map [0, 1] to [min_height, max_height].
        let height =
            self.config.min_height + normalized * (self.config.max_height - self.config.min_height);

        height.clamp(self.config.min_height, self.config.max_height)
    }

    /// Compute the absolute distance from the planet center for a surface point.
    ///
    /// This is the planet radius + sea level offset + terrain height.
    /// Used when converting to `WorldPosition` (128-bit coordinates).
    pub fn sample_radius(&self, sphere_point: DVec3) -> f64 {
        let height = self.sample_height(sphere_point);
        self.config.planet_radius + self.config.sea_level + height
    }

    /// Compute the 3D world-space position (in f64) for a point on the sphere.
    ///
    /// The result is `sphere_point.normalize() * sample_radius(sphere_point)`.
    pub fn sample_world_position(&self, sphere_point: DVec3) -> DVec3 {
        let dir = sphere_point.normalize();
        let radius = self.sample_radius(sphere_point);
        dir * radius
    }

    /// Return a reference to the terrain height configuration.
    pub fn config(&self) -> &TerrainHeightConfig {
        &self.config
    }
}

/// Determine the terrain surface height for a column at the given face coordinate.
///
/// Converts the face coordinate to a 3D sphere point via the Everitt projection,
/// then samples terrain height. Returns height above sea level in engine units.
pub fn column_surface_height(fc: &FaceCoord, terrain: &TerrainHeightSampler) -> f64 {
    let sphere_point = face_coord_to_sphere_everitt(fc);
    terrain.sample_height(sphere_point)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_cubesphere::CubeFace;

    const EPSILON: f64 = 1e-6;

    fn default_sampler() -> TerrainHeightSampler {
        TerrainHeightSampler::new(
            HeightmapParams {
                seed: 42,
                ..Default::default()
            },
            TerrainHeightConfig::default(),
        )
    }

    #[test]
    fn test_terrain_continuous_across_cube_face_edges() {
        let sampler = default_sampler();

        // PosX face at u=0 and NegZ face at u=1 share an edge.
        let steps = 50;
        for i in 0..=steps {
            let v = i as f64 / steps as f64;
            let fc_a = FaceCoord::new(CubeFace::PosX, 0.0, v);
            let fc_b = FaceCoord::new(CubeFace::NegZ, 1.0, v);

            let pt_a = face_coord_to_sphere_everitt(&fc_a);
            let pt_b = face_coord_to_sphere_everitt(&fc_b);

            if (pt_a - pt_b).length() < 0.01 {
                let h_a = sampler.sample_height(pt_a);
                let h_b = sampler.sample_height(pt_b);
                assert!(
                    (h_a - h_b).abs() < 1.0,
                    "Height discontinuity at shared edge v={v}: {h_a} vs {h_b}"
                );
            }
        }
    }

    #[test]
    fn test_height_at_poles_is_reasonable() {
        let sampler = default_sampler();
        let config = &TerrainHeightConfig::default();

        let north = DVec3::new(0.0, 1.0, 0.0);
        let south = DVec3::new(0.0, -1.0, 0.0);

        let h_north = sampler.sample_height(north);
        let h_south = sampler.sample_height(south);

        assert!(
            h_north >= config.min_height && h_north <= config.max_height,
            "North pole height {h_north} out of range [{}, {}]",
            config.min_height,
            config.max_height
        );
        assert!(
            h_south >= config.min_height && h_south <= config.max_height,
            "South pole height {h_south} out of range [{}, {}]",
            config.min_height,
            config.max_height
        );
    }

    #[test]
    fn test_height_never_exceeds_defined_range() {
        let sampler = default_sampler();
        let config = &TerrainHeightConfig::default();

        for face in CubeFace::ALL {
            for u_step in 0..=20 {
                for v_step in 0..=20 {
                    let u = u_step as f64 / 20.0;
                    let v = v_step as f64 / 20.0;
                    let fc = FaceCoord::new(face, u, v);
                    let sphere_pt = face_coord_to_sphere_everitt(&fc);
                    let h = sampler.sample_height(sphere_pt);

                    assert!(
                        h >= config.min_height - EPSILON && h <= config.max_height + EPSILON,
                        "Height {h} out of range at face {face:?} ({u}, {v})"
                    );
                }
            }
        }
    }

    #[test]
    fn test_flat_planet_has_uniform_radius() {
        let sampler = TerrainHeightSampler::new(
            HeightmapParams {
                seed: 42,
                amplitude: 0.0,
                ..Default::default()
            },
            TerrainHeightConfig::default(),
        );

        let points = [
            DVec3::new(1.0, 0.0, 0.0),
            DVec3::new(0.0, 1.0, 0.0),
            DVec3::new(0.0, 0.0, 1.0),
            DVec3::new(-1.0, 0.0, 0.0),
        ];

        let radii: Vec<f64> = points.iter().map(|p| sampler.sample_radius(*p)).collect();
        let first = radii[0];
        for (i, &r) in radii.iter().enumerate() {
            assert!(
                (r - first).abs() < EPSILON,
                "Flat planet should have uniform radius. Point {i}: {r} vs {first}"
            );
        }
    }

    #[test]
    fn test_sample_radius_includes_planet_radius() {
        let config = TerrainHeightConfig {
            planet_radius: 1_000_000.0,
            min_height: -100.0,
            max_height: 100.0,
            sea_level: 0.0,
        };
        let sampler = TerrainHeightSampler::new(
            HeightmapParams {
                seed: 7,
                ..Default::default()
            },
            config,
        );

        let point = DVec3::new(1.0, 0.0, 0.0);
        let radius = sampler.sample_radius(point);

        assert!(
            radius > 999_800.0 && radius < 1_000_200.0,
            "Radius {radius} should be near planet radius 1_000_000"
        );
    }

    #[test]
    fn test_sample_world_position_direction_matches_input() {
        let sampler = default_sampler();
        let dir = DVec3::new(1.0, 1.0, 1.0).normalize();
        let world_pos = sampler.sample_world_position(dir);

        let result_dir = world_pos.normalize();
        assert!(
            (result_dir - dir).length() < EPSILON,
            "World position direction should match input sphere point"
        );
    }

    #[test]
    fn test_column_surface_height_matches_direct_sample() {
        let sampler = default_sampler();
        let fc = FaceCoord::new(CubeFace::PosX, 0.3, 0.7);
        let sphere_pt = face_coord_to_sphere_everitt(&fc);

        let h_direct = sampler.sample_height(sphere_pt);
        let h_column = column_surface_height(&fc, &sampler);

        assert!(
            (h_direct - h_column).abs() < EPSILON,
            "column_surface_height should match direct sample: {h_direct} vs {h_column}"
        );
    }
}
