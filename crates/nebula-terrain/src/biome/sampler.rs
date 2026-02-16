//! Biome sampler: uses simplex noise fields for temperature and moisture
//! to sample biomes via the Whittaker diagram.

use noise::{NoiseFn, Simplex};

use super::{BiomeId, WhittakerDiagram};

/// Samples biomes at arbitrary 3D sphere-surface points using noise-driven
/// temperature and moisture fields fed into a [`WhittakerDiagram`].
pub struct BiomeSampler {
    temp_noise: Simplex,
    moisture_noise: Simplex,
    diagram: WhittakerDiagram,
    /// Frequency for temperature noise. Lower values produce broader temperature zones.
    pub temp_frequency: f64,
    /// Frequency for moisture noise.
    pub moisture_frequency: f64,
}

impl BiomeSampler {
    /// Creates a new sampler with the given seed and Whittaker diagram.
    ///
    /// Temperature and moisture noise fields use different seeds derived from
    /// `seed` to ensure they are decorrelated.
    pub fn new(seed: u64, diagram: WhittakerDiagram) -> Self {
        let temp_noise = Simplex::new(seed as u32);
        let moisture_noise = Simplex::new(seed.wrapping_add(0xDEAD_BEEF) as u32);
        Self {
            temp_noise,
            moisture_noise,
            diagram,
            temp_frequency: 0.0005,
            moisture_frequency: 0.0007,
        }
    }

    /// Samples the biome at a 3D sphere-surface point.
    ///
    /// Returns `(biome_id, temperature, moisture)` where temperature and moisture
    /// are normalized to `[0.0, 1.0]`.
    pub fn sample(&self, point: glam::DVec3) -> (BiomeId, f64, f64) {
        let temp_raw = self.temp_noise.get([
            point.x * self.temp_frequency,
            point.y * self.temp_frequency,
            point.z * self.temp_frequency,
        ]);
        let moisture_raw = self.moisture_noise.get([
            point.x * self.moisture_frequency,
            point.y * self.moisture_frequency,
            point.z * self.moisture_frequency,
        ]);

        // Normalize from [-1, 1] to [0, 1].
        let temperature = (temp_raw + 1.0) * 0.5;
        let moisture = (moisture_raw + 1.0) * 0.5;

        let biome_id = self.diagram.lookup(temperature, moisture);
        (biome_id, temperature, moisture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biome::{BiomeDef, BiomeRegistry, WhittakerRegion};
    use nebula_voxel::VoxelTypeId;

    fn make_test_registry_and_diagram() -> (BiomeRegistry, WhittakerDiagram) {
        let mut reg = BiomeRegistry::new();

        let tundra = reg
            .register(BiomeDef {
                name: "tundra".into(),
                surface_voxel: VoxelTypeId(10),
                subsurface_voxel: VoxelTypeId(11),
                vegetation_density: 0.02,
                tree_type: None,
            })
            .unwrap();

        let desert = reg
            .register(BiomeDef {
                name: "desert".into(),
                surface_voxel: VoxelTypeId(20),
                subsurface_voxel: VoxelTypeId(21),
                vegetation_density: 0.01,
                tree_type: None,
            })
            .unwrap();

        let plains = reg
            .register(BiomeDef {
                name: "plains".into(),
                surface_voxel: VoxelTypeId(30),
                subsurface_voxel: VoxelTypeId(31),
                vegetation_density: 0.3,
                tree_type: Some("oak".into()),
            })
            .unwrap();

        let forest = reg
            .register(BiomeDef {
                name: "forest".into(),
                surface_voxel: VoxelTypeId(40),
                subsurface_voxel: VoxelTypeId(41),
                vegetation_density: 0.8,
                tree_type: Some("birch".into()),
            })
            .unwrap();

        let diagram = WhittakerDiagram {
            regions: vec![
                WhittakerRegion {
                    temp_min: 0.0,
                    temp_max: 0.3,
                    moisture_min: 0.0,
                    moisture_max: 0.5,
                    biome_id: tundra,
                },
                WhittakerRegion {
                    temp_min: 0.7,
                    temp_max: 1.0,
                    moisture_min: 0.0,
                    moisture_max: 0.3,
                    biome_id: desert,
                },
                WhittakerRegion {
                    temp_min: 0.3,
                    temp_max: 0.7,
                    moisture_min: 0.0,
                    moisture_max: 0.5,
                    biome_id: plains,
                },
                WhittakerRegion {
                    temp_min: 0.3,
                    temp_max: 0.7,
                    moisture_min: 0.5,
                    moisture_max: 1.0,
                    biome_id: forest,
                },
                WhittakerRegion {
                    temp_min: 0.0,
                    temp_max: 0.3,
                    moisture_min: 0.5,
                    moisture_max: 1.0,
                    biome_id: tundra,
                },
                WhittakerRegion {
                    temp_min: 0.7,
                    temp_max: 1.0,
                    moisture_min: 0.3,
                    moisture_max: 1.0,
                    biome_id: forest,
                },
            ],
            fallback: plains,
        };

        (reg, diagram)
    }

    #[test]
    fn test_cold_dry_is_tundra() {
        let (reg, diagram) = make_test_registry_and_diagram();
        let biome_id = diagram.lookup(0.1, 0.2);
        assert_eq!(reg.get(biome_id).name, "tundra");
    }

    #[test]
    fn test_hot_dry_is_desert() {
        let (reg, diagram) = make_test_registry_and_diagram();
        let biome_id = diagram.lookup(0.8, 0.1);
        assert_eq!(reg.get(biome_id).name, "desert");
    }

    #[test]
    fn test_moderate_wet_is_forest() {
        let (reg, diagram) = make_test_registry_and_diagram();
        let biome_id = diagram.lookup(0.5, 0.7);
        assert_eq!(reg.get(biome_id).name, "forest");
    }

    #[test]
    fn test_diagram_covers_entire_range() {
        let (_reg, diagram) = make_test_registry_and_diagram();
        let steps = 100;
        let mut resolved_count = 0;
        for t in 0..steps {
            for m in 0..steps {
                let temp = t as f64 / steps as f64;
                let moisture = m as f64 / steps as f64;
                let _biome = diagram.lookup(temp, moisture);
                resolved_count += 1;
            }
        }
        assert_eq!(resolved_count, steps * steps);
    }

    #[test]
    fn test_biome_boundaries_smooth() {
        let (_reg, diagram) = make_test_registry_and_diagram();
        let moisture = 0.25;
        let mut transitions = 0;
        let mut prev_biome = diagram.lookup(0.0, moisture);

        for i in 1..=1000 {
            let temp = i as f64 / 1000.0;
            let biome = diagram.lookup(temp, moisture);
            if biome != prev_biome {
                transitions += 1;
                prev_biome = biome;
            }
        }

        assert!(
            transitions < 10,
            "Too many biome transitions ({transitions}) along temperature gradient -- \
             indicates noisy boundaries"
        );
    }

    #[test]
    fn test_biome_registry_contains_all_expected() {
        let (reg, _diagram) = make_test_registry_and_diagram();
        assert!(reg.lookup_by_name("tundra").is_some());
        assert!(reg.lookup_by_name("desert").is_some());
        assert!(reg.lookup_by_name("plains").is_some());
        assert!(reg.lookup_by_name("forest").is_some());
        assert_eq!(reg.len(), 4);
    }

    #[test]
    fn test_biome_registry_duplicate_rejected() {
        let mut reg = BiomeRegistry::new();
        reg.register(BiomeDef {
            name: "desert".into(),
            surface_voxel: VoxelTypeId(1),
            subsurface_voxel: VoxelTypeId(2),
            vegetation_density: 0.0,
            tree_type: None,
        })
        .unwrap();

        let result = reg.register(BiomeDef {
            name: "desert".into(),
            surface_voxel: VoxelTypeId(3),
            subsurface_voxel: VoxelTypeId(4),
            vegetation_density: 0.0,
            tree_type: None,
        });

        assert!(result.is_err());
    }

    #[test]
    fn test_biome_sampler_deterministic() {
        let (_reg, diagram) = make_test_registry_and_diagram();
        let sampler_a = BiomeSampler::new(42, diagram);
        let (_reg2, diagram2) = make_test_registry_and_diagram();
        let sampler_b = BiomeSampler::new(42, diagram2);

        let point = glam::DVec3::new(0.5, 0.3, 0.8).normalize();
        let (biome_a, temp_a, moist_a) = sampler_a.sample(point);
        let (biome_b, temp_b, moist_b) = sampler_b.sample(point);

        assert_eq!(biome_a, biome_b);
        assert!((temp_a - temp_b).abs() < 1e-12);
        assert!((moist_a - moist_b).abs() < 1e-12);
    }
}
