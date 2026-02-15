# Cubesphere Terrain Height

## Problem

The engine represents planets as cubespheres -- cubes projected onto spheres. Terrain height must be applied to this curved surface. A naive approach of sampling 2D noise using UV coordinates on each cube face creates visible seams at face edges and severe distortion at cube corners, because the UV parameterization is discontinuous across face boundaries. The engine needs a method that produces seamless, distortion-free terrain height across the entire sphere surface, and that maps the noise output to a physically meaningful height range (e.g., -2 km below sea level to +8 km above). The final terrain height must be added to the planet's base radius when computing the `WorldPosition` (128-bit) of each surface voxel.

## Solution

Sample the heightmap noise using the 3D unit-sphere coordinate of each surface point, rather than per-face 2D UV coordinates. Since the 3D coordinate is continuous everywhere on the sphere, there are no seams at face boundaries or distortion at corners. The `HeightmapSampler::sample_3d` method (from story 01) accepts a `DVec3` on the unit sphere and returns a raw noise value. This story wraps that sampler with planet-specific height mapping.

### Terrain Height Configuration

```rust
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
```

### Terrain Height Sampler

```rust
use crate::heightmap::{HeightmapSampler, HeightmapParams};

pub struct TerrainHeightSampler {
    heightmap: HeightmapSampler,
    config: TerrainHeightConfig,
}

impl TerrainHeightSampler {
    pub fn new(heightmap_params: HeightmapParams, config: TerrainHeightConfig) -> Self {
        Self {
            heightmap: HeightmapSampler::new(heightmap_params),
            config,
        }
    }

    /// Sample terrain height at a point on the unit sphere.
    ///
    /// Returns the height above sea level in engine units, clamped to
    /// [config.min_height, config.max_height].
    pub fn sample_height(&self, sphere_point: glam::DVec3) -> f64 {
        let raw = self.heightmap.sample_3d(sphere_point);
        let max_amp = self.heightmap.max_amplitude();

        // Normalize raw noise from [-max_amp, +max_amp] to [0, 1].
        let normalized = (raw / max_amp + 1.0) * 0.5;

        // Map [0, 1] to [min_height, max_height].
        let height = self.config.min_height
            + normalized * (self.config.max_height - self.config.min_height);

        height.clamp(self.config.min_height, self.config.max_height)
    }

    /// Compute the absolute distance from the planet center for a surface point.
    ///
    /// This is the planet radius + sea level offset + terrain height.
    /// Used when converting to WorldPosition (128-bit coordinates).
    pub fn sample_radius(&self, sphere_point: glam::DVec3) -> f64 {
        let height = self.sample_height(sphere_point);
        self.config.planet_radius + self.config.sea_level + height
    }

    /// Compute the 3D world-space position (in f64) for a point on the sphere.
    ///
    /// The result is `sphere_point.normalize() * sample_radius(sphere_point)`.
    pub fn sample_world_position(&self, sphere_point: glam::DVec3) -> glam::DVec3 {
        let dir = sphere_point.normalize();
        let radius = self.sample_radius(sphere_point);
        dir * radius
    }
}
```

### Integration with CubeFace Coordinates

When generating a chunk on a specific cube face, the system:

1. Converts the chunk's `FaceCoord` to a `DVec3` on the unit sphere via `face_coord_to_sphere_everitt()` (from cubesphere story 02).
2. Passes that `DVec3` to `TerrainHeightSampler::sample_height()`.
3. Uses the returned height to determine which voxels in the chunk column are solid and which are air.

```rust
use crate::cubesphere::face_coord_to_sphere_everitt;

/// Determine the terrain surface height for a column at the given face coordinate.
pub fn column_surface_height(
    fc: &FaceCoord,
    terrain: &TerrainHeightSampler,
) -> f64 {
    let sphere_point = face_coord_to_sphere_everitt(fc);
    terrain.sample_height(sphere_point)
}
```

### Seamlessness Across Face Edges

Because noise is sampled using the 3D sphere coordinate (which is identical for two face coordinates that map to the same sphere point at a shared edge), height values are automatically continuous. There is no stitching required.

## Outcome

A `TerrainHeightSampler` and `TerrainHeightConfig` in `nebula-terrain` that map cubesphere surface points to terrain heights via 3D noise sampling. Heights are clamped to a configurable physical range and added to the planet radius for world-space positioning. Running `cargo test -p nebula-terrain` passes all cubesphere terrain height tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Terrain follows the cubesphere surface with height variation. Mountains rise above the base sphere radius and valleys dip below it. The curvature of the planet is visible at chunk edges.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `noise` | 0.9 | Underlying simplex noise (via `HeightmapSampler`) |
| `glam` | 0.29 | `DVec3` for sphere-surface and world-space coordinates |

Depends on `nebula-cubesphere` for `face_coord_to_sphere_everitt()` and related types. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::cubesphere::{CubeFace, FaceCoord, face_coord_to_sphere_everitt};
    use glam::DVec3;

    const EPSILON: f64 = 1e-6;

    fn default_sampler() -> TerrainHeightSampler {
        TerrainHeightSampler::new(
            HeightmapParams { seed: 42, ..Default::default() },
            TerrainHeightConfig::default(),
        )
    }

    #[test]
    fn test_terrain_continuous_across_cube_face_edges() {
        let sampler = default_sampler();

        // PosX face at u=0 and NegZ face at u=1 share an edge.
        // Heights should match at corresponding points along the shared edge.
        let steps = 50;
        for i in 0..=steps {
            let v = i as f64 / steps as f64;
            let fc_a = FaceCoord::new(CubeFace::PosX, 0.0, v);
            let fc_b = FaceCoord::new(CubeFace::NegZ, 1.0, v);

            let pt_a = face_coord_to_sphere_everitt(&fc_a);
            let pt_b = face_coord_to_sphere_everitt(&fc_b);

            // If the sphere points are close, the heights must be close.
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

        // North pole (0, 1, 0) and south pole (0, -1, 0).
        let north = DVec3::new(0.0, 1.0, 0.0);
        let south = DVec3::new(0.0, -1.0, 0.0);

        let h_north = sampler.sample_height(north);
        let h_south = sampler.sample_height(south);

        assert!(
            h_north >= config.min_height && h_north <= config.max_height,
            "North pole height {h_north} out of range [{}, {}]",
            config.min_height, config.max_height
        );
        assert!(
            h_south >= config.min_height && h_south <= config.max_height,
            "South pole height {h_south} out of range [{}, {}]",
            config.min_height, config.max_height
        );
    }

    #[test]
    fn test_height_never_exceeds_defined_range() {
        let sampler = default_sampler();
        let config = &TerrainHeightConfig::default();

        // Sample many points across all cube faces.
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
        // With amplitude=0, noise output is always 0. The normalized value maps
        // to the midpoint of [min_height, max_height].
        let sampler = TerrainHeightSampler::new(
            HeightmapParams {
                seed: 42,
                amplitude: 0.0,
                ..Default::default()
            },
            TerrainHeightConfig::default(),
        );

        let config = TerrainHeightConfig::default();
        // With zero amplitude, max_amplitude() is 0. Division by zero is
        // handled: the height should be a consistent value.
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
            HeightmapParams { seed: 7, ..Default::default() },
            config,
        );

        let point = DVec3::new(1.0, 0.0, 0.0);
        let radius = sampler.sample_radius(point);

        // Radius should be approximately planet_radius +/- max terrain height.
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
}
```
