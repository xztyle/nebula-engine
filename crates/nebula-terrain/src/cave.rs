//! 3D noise-based cave generation using the Swiss cheese model.
//!
//! Uses multi-octave 3D simplex noise to carve cave systems into subsurface
//! volume. Respects depth bounds and ocean floor buffers.

use noise::{NoiseFn, Simplex};

/// Configuration for 3D noise-based cave generation.
#[derive(Clone, Debug)]
pub struct CaveConfig {
    /// World seed for deterministic cave generation.
    pub seed: u64,
    /// Noise threshold. Voxels where `noise_value <= threshold` become air (cave).
    /// Lower thresholds produce fewer, smaller caves. Typical range: -0.3 to 0.0.
    /// Default: -0.15.
    pub threshold: f64,
    /// Number of noise octaves. More octaves create more detailed cave shapes.
    /// Default: 3.
    pub octaves: u32,
    /// Base frequency of the cave noise. Controls the spatial scale of cave tunnels.
    /// Higher frequency = narrower, more frequent tunnels.
    /// Default: 0.02.
    pub frequency: f64,
    /// Lacunarity (frequency multiplier per octave). Default: 2.0.
    pub lacunarity: f64,
    /// Persistence (amplitude multiplier per octave). Default: 0.5.
    pub persistence: f64,
    /// Maximum depth below the terrain surface where caves can exist, in engine units.
    /// Caves are suppressed below this depth to prevent infinitely deep tunnels.
    /// Default: 500.0 (~500 meters).
    pub max_depth: f64,
    /// Minimum distance below the terrain surface before caves can appear.
    /// Prevents thin "eggshell" surfaces that break easily. Default: 5.0.
    pub min_depth: f64,
    /// Minimum distance above the ocean floor where caves are suppressed.
    /// Prevents water from draining into caves. Default: 10.0.
    pub ocean_floor_buffer: f64,
}

impl Default for CaveConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            threshold: -0.15,
            octaves: 3,
            frequency: 0.02,
            lacunarity: 2.0,
            persistence: 0.5,
            max_depth: 500.0,
            min_depth: 5.0,
            ocean_floor_buffer: 10.0,
        }
    }
}

/// Carves cave systems into subsurface volume using 3D simplex noise.
pub struct CaveCarver {
    noise: Simplex,
    config: CaveConfig,
}

impl CaveCarver {
    /// Create a new cave carver with the given configuration.
    pub fn new(config: CaveConfig) -> Self {
        // Use a different seed offset to decorrelate cave noise from terrain noise.
        let noise = Simplex::new(config.seed.wrapping_add(0xCAFE_BABE) as u32);
        Self { noise, config }
    }

    /// Returns the cave configuration.
    pub fn config(&self) -> &CaveConfig {
        &self.config
    }

    /// Determine if a voxel at the given 3D position should be carved as a cave.
    ///
    /// # Arguments
    /// - `voxel_pos`: The 3D position of the voxel in world space (f64).
    /// - `surface_height`: The terrain surface height at this column (from heightmap).
    /// - `sea_level_height`: The absolute height of sea level at this column.
    ///
    /// # Returns
    /// `true` if the voxel should be air (cave), `false` if it remains solid.
    pub fn is_cave(
        &self,
        voxel_pos: glam::DVec3,
        surface_height: f64,
        sea_level_height: f64,
    ) -> bool {
        let voxel_height = voxel_pos.length(); // distance from planet center

        // Only carve below the terrain surface.
        if voxel_height >= surface_height {
            return false;
        }

        let depth_below_surface = surface_height - voxel_height;

        // Don't carve too close to the surface (eggshell prevention).
        if depth_below_surface < self.config.min_depth {
            return false;
        }

        // Don't carve deeper than max_depth.
        if depth_below_surface > self.config.max_depth {
            return false;
        }

        // Don't carve near or below the ocean floor to prevent water drainage.
        if surface_height <= sea_level_height
            && depth_below_surface < self.config.ocean_floor_buffer
        {
            return false;
        }
        // Also suppress if we're close to sea level from below.
        if voxel_height <= sea_level_height + self.config.ocean_floor_buffer
            && surface_height <= sea_level_height
        {
            return false;
        }

        // Sample 3D noise at the voxel position.
        let noise_val = self.sample_cave_noise(voxel_pos);

        // Apply depth-based fade: caves become less likely near max_depth.
        let depth_factor = 1.0
            - ((depth_below_surface - self.config.min_depth)
                / (self.config.max_depth - self.config.min_depth))
                .powf(2.0);

        let adjusted_threshold = self.config.threshold * depth_factor;

        noise_val <= adjusted_threshold
    }

    /// Sample multi-octave 3D cave noise at a position.
    fn sample_cave_noise(&self, pos: glam::DVec3) -> f64 {
        let mut total = 0.0;
        let mut frequency = self.config.frequency;
        let mut amplitude = 1.0;
        let mut max_amplitude = 0.0;

        for _ in 0..self.config.octaves {
            let val = self
                .noise
                .get([pos.x * frequency, pos.y * frequency, pos.z * frequency]);
            total += val * amplitude;
            max_amplitude += amplitude;

            frequency *= self.config.lacunarity;
            amplitude *= self.config.persistence;
        }

        // Normalize to [-1, 1].
        total / max_amplitude
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    const PLANET_RADIUS: f64 = 6_371_000.0;

    fn default_carver() -> CaveCarver {
        CaveCarver::new(CaveConfig {
            seed: 42,
            ..Default::default()
        })
    }

    #[test]
    fn test_cave_voxels_are_air() {
        let carver = default_carver();
        let surface_height = PLANET_RADIUS + 100.0;
        let sea_level = PLANET_RADIUS;

        let mut found_cave = false;
        for i in 0..1000 {
            let depth = 10.0 + (i as f64) * 0.5;
            let voxel_pos = DVec3::new(surface_height - depth, 0.0, 0.0);
            if carver.is_cave(voxel_pos, surface_height, sea_level) {
                found_cave = true;
            }
        }
        assert!(
            found_cave,
            "Should find at least some cave voxels in 1000 samples"
        );
    }

    #[test]
    fn test_non_cave_voxels_are_solid() {
        let carver = default_carver();
        let surface_height = PLANET_RADIUS + 100.0;
        let sea_level = PLANET_RADIUS;

        let mut found_solid = false;
        for i in 0..1000 {
            let depth = 10.0 + (i as f64) * 0.5;
            let voxel_pos = DVec3::new(surface_height - depth, 0.0, 0.0);
            if !carver.is_cave(voxel_pos, surface_height, sea_level) {
                found_solid = true;
            }
        }
        assert!(
            found_solid,
            "Should find solid (non-cave) voxels in subsurface"
        );
    }

    #[test]
    fn test_caves_exist_below_surface_only() {
        let carver = default_carver();
        let surface_height = PLANET_RADIUS + 100.0;
        let sea_level = PLANET_RADIUS;

        for i in 1..=50 {
            let above = surface_height + (i as f64);
            let voxel_pos = DVec3::new(above, 0.0, 0.0);
            assert!(
                !carver.is_cave(voxel_pos, surface_height, sea_level),
                "No caves should exist above the surface (height={above})"
            );
        }
    }

    #[test]
    fn test_cave_density_controllable_via_threshold() {
        let surface_height = PLANET_RADIUS + 200.0;
        let sea_level = PLANET_RADIUS;

        let count_caves = |threshold: f64| -> usize {
            let carver = CaveCarver::new(CaveConfig {
                seed: 42,
                threshold,
                ..Default::default()
            });
            let mut count = 0;
            for i in 0..500 {
                let depth = 10.0 + (i as f64);
                let voxel_pos = DVec3::new(surface_height - depth, i as f64 * 0.3, 0.0);
                if carver.is_cave(voxel_pos, surface_height, sea_level) {
                    count += 1;
                }
            }
            count
        };

        let caves_low_threshold = count_caves(-0.5);
        let caves_high_threshold = count_caves(0.2);

        assert!(
            caves_high_threshold > caves_low_threshold,
            "Higher threshold should produce more caves: low={caves_low_threshold}, high={caves_high_threshold}"
        );
    }

    #[test]
    fn test_deterministic_with_seed() {
        let carver_a = CaveCarver::new(CaveConfig {
            seed: 123,
            ..Default::default()
        });
        let carver_b = CaveCarver::new(CaveConfig {
            seed: 123,
            ..Default::default()
        });

        let surface_height = PLANET_RADIUS + 100.0;
        let sea_level = PLANET_RADIUS;

        for i in 0..200 {
            let voxel_pos = DVec3::new(
                surface_height - 20.0 - (i as f64),
                i as f64 * 1.7,
                i as f64 * 0.3,
            );
            assert_eq!(
                carver_a.is_cave(voxel_pos, surface_height, sea_level),
                carver_b.is_cave(voxel_pos, surface_height, sea_level),
                "Cave determination must be deterministic at index {i}"
            );
        }
    }

    #[test]
    fn test_caves_dont_breach_ocean_floor() {
        let carver = CaveCarver::new(CaveConfig {
            seed: 42,
            threshold: 0.5,
            ocean_floor_buffer: 10.0,
            ..Default::default()
        });

        let surface_height = PLANET_RADIUS - 50.0;
        let sea_level = PLANET_RADIUS;

        for i in 0..20 {
            let depth = (i as f64) + 1.0;
            let voxel_pos = DVec3::new(surface_height - depth, 0.0, 0.0);
            if depth < 10.0 {
                assert!(
                    !carver.is_cave(voxel_pos, surface_height, sea_level),
                    "Caves must not breach within ocean_floor_buffer ({depth} units below ocean floor)"
                );
            }
        }
    }

    #[test]
    fn test_no_caves_in_eggshell_zone() {
        let carver = CaveCarver::new(CaveConfig {
            seed: 42,
            threshold: 0.5,
            min_depth: 5.0,
            ..Default::default()
        });

        let surface_height = PLANET_RADIUS + 100.0;
        let sea_level = PLANET_RADIUS;

        for i in 0..50 {
            let depth = (i as f64) * 0.1;
            let voxel_pos = DVec3::new(surface_height - depth, i as f64, 0.0);
            assert!(
                !carver.is_cave(voxel_pos, surface_height, sea_level),
                "No caves within min_depth ({depth} units below surface)"
            );
        }
    }
}
