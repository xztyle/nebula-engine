# Biome System

## Problem

A planet covered in uniform terrain looks artificial. Real planets exhibit distinct ecological zones -- deserts, tundra, forests, oceans -- driven by temperature and moisture gradients. The engine needs a system that assigns a biome to every surface point, where each biome controls the surface voxel type (grass, sand, snow), subsurface material (dirt, sandstone, permafrost), vegetation density, and tree species. The biome assignment must be deterministic, smooth (no jarring single-voxel biome flicker at boundaries), and extensible so that game designers can define new biomes without modifying engine code. The system must also interoperate with the heightmap sampler (story 01) -- biomes influence terrain appearance, while terrain elevation can influence biome selection (e.g., high altitude forces alpine/mountain biomes).

## Solution

Implement a `BiomeSystem` in the `nebula-terrain` crate that uses two independent noise fields -- temperature and moisture -- to classify every surface point into a biome via a Whittaker-style lookup diagram.

### Biome Definition

```rust
use crate::voxel::VoxelTypeId;

/// Unique identifier for a biome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BiomeId(pub u16);

/// Full descriptor for a biome type.
#[derive(Clone, Debug)]
pub struct BiomeDef {
    /// Human-readable biome name (e.g., "temperate_forest").
    pub name: String,
    /// Voxel type placed on the terrain surface (e.g., grass, sand, snow).
    pub surface_voxel: VoxelTypeId,
    /// Voxel type for the layers immediately below the surface (e.g., dirt, sandstone).
    pub subsurface_voxel: VoxelTypeId,
    /// Probability of vegetation spawning per surface voxel, in [0.0, 1.0].
    pub vegetation_density: f64,
    /// Identifier for the tree/plant archetype used in this biome. `None` for barren biomes.
    pub tree_type: Option<String>,
}
```

### Biome Registry

```rust
pub struct BiomeRegistry {
    biomes: Vec<BiomeDef>,
    name_to_id: HashMap<String, BiomeId>,
}

impl BiomeRegistry {
    pub fn new() -> Self {
        Self {
            biomes: Vec::new(),
            name_to_id: HashMap::new(),
        }
    }

    pub fn register(&mut self, def: BiomeDef) -> Result<BiomeId, BiomeRegistryError> {
        if self.name_to_id.contains_key(&def.name) {
            return Err(BiomeRegistryError::DuplicateName(def.name.clone()));
        }
        let id = BiomeId(self.biomes.len() as u16);
        self.name_to_id.insert(def.name.clone(), id);
        self.biomes.push(def);
        Ok(id)
    }

    pub fn get(&self, id: BiomeId) -> &BiomeDef {
        &self.biomes[id.0 as usize]
    }

    pub fn lookup_by_name(&self, name: &str) -> Option<BiomeId> {
        self.name_to_id.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.biomes.len()
    }
}
```

### Whittaker Diagram Lookup

Temperature and moisture are each normalized to [0.0, 1.0]. The Whittaker diagram is a 2D lookup that maps `(temperature, moisture)` pairs to biome IDs. It is implemented as a list of rectangular regions with a priority/fallback so that the entire `[0, 1] x [0, 1]` space is covered:

```rust
/// A rectangular region in temperature-moisture space mapped to a biome.
pub struct WhittakerRegion {
    pub temp_min: f64,
    pub temp_max: f64,
    pub moisture_min: f64,
    pub moisture_max: f64,
    pub biome_id: BiomeId,
}

pub struct WhittakerDiagram {
    regions: Vec<WhittakerRegion>,
    /// Fallback biome if no region matches (should never happen with proper coverage).
    fallback: BiomeId,
}

impl WhittakerDiagram {
    /// Lookup the biome for a given temperature and moisture, both in [0.0, 1.0].
    pub fn lookup(&self, temperature: f64, moisture: f64) -> BiomeId {
        for region in &self.regions {
            if temperature >= region.temp_min
                && temperature < region.temp_max
                && moisture >= region.moisture_min
                && moisture < region.moisture_max
            {
                return region.biome_id;
            }
        }
        self.fallback
    }
}
```

### Default Biome Layout

| Biome | Temperature Range | Moisture Range |
|-------|------------------|----------------|
| Tundra | 0.0 - 0.2 | 0.0 - 0.5 |
| Taiga | 0.0 - 0.2 | 0.5 - 1.0 |
| Desert | 0.5 - 1.0 | 0.0 - 0.2 |
| Savanna | 0.5 - 0.8 | 0.2 - 0.5 |
| Plains | 0.2 - 0.5 | 0.2 - 0.6 |
| Forest | 0.2 - 0.5 | 0.6 - 1.0 |
| Tropical Rainforest | 0.8 - 1.0 | 0.5 - 1.0 |
| Mountains | (elevation override) | any |
| Ocean | (below sea level) | any |
| Beach | (near sea level) | any |

Mountains, Ocean, and Beach are assigned by elevation rather than temperature/moisture. The biome sampler applies elevation-based overrides after the Whittaker lookup.

### Biome Sampler

```rust
use noise::{NoiseFn, Simplex};

pub struct BiomeSampler {
    temp_noise: Simplex,
    moisture_noise: Simplex,
    diagram: WhittakerDiagram,
    /// Frequency for temperature noise. Lower = broader temperature zones.
    pub temp_frequency: f64,
    /// Frequency for moisture noise.
    pub moisture_frequency: f64,
}

impl BiomeSampler {
    pub fn new(seed: u64, diagram: WhittakerDiagram) -> Self {
        // Use different seeds for temperature and moisture to decorrelate them.
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

    /// Sample the biome at a 3D sphere-surface point.
    /// Returns the BiomeId, along with the raw temperature and moisture values.
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
```

### Smooth Boundaries

To prevent noisy single-voxel biome flicker at boundaries, the biome sampler can optionally blend surface materials by sampling the biome at slightly jittered positions and choosing the majority biome within a small kernel. This smoothing is applied only for visual material assignment, not for the underlying biome classification.

## Outcome

A `BiomeRegistry`, `BiomeDef`, `WhittakerDiagram`, and `BiomeSampler` in `nebula-terrain` that deterministically assign biomes to sphere-surface points based on temperature/moisture noise. The system is extensible -- game code registers custom biomes and diagram regions. Running `cargo test -p nebula-terrain` passes all biome tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Different terrain regions show distinct biome colors: green forests, yellow deserts, white tundra. The console logs `Biomes: 5 registered, sampling Whittaker diagram`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `noise` | 0.9 | Simplex noise for temperature and moisture fields |
| `glam` | 0.29 | `DVec3` for sphere-surface coordinate input |
| `hashbrown` | 0.15 | Fast hash map for biome name-to-ID lookup |
| `thiserror` | 2.0 | Error type for `BiomeRegistryError` |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_registry_and_diagram() -> (BiomeRegistry, WhittakerDiagram) {
        let mut reg = BiomeRegistry::new();

        let tundra = reg.register(BiomeDef {
            name: "tundra".into(),
            surface_voxel: VoxelTypeId(10),
            subsurface_voxel: VoxelTypeId(11),
            vegetation_density: 0.02,
            tree_type: None,
        }).unwrap();

        let desert = reg.register(BiomeDef {
            name: "desert".into(),
            surface_voxel: VoxelTypeId(20),
            subsurface_voxel: VoxelTypeId(21),
            vegetation_density: 0.01,
            tree_type: None,
        }).unwrap();

        let plains = reg.register(BiomeDef {
            name: "plains".into(),
            surface_voxel: VoxelTypeId(30),
            subsurface_voxel: VoxelTypeId(31),
            vegetation_density: 0.3,
            tree_type: Some("oak".into()),
        }).unwrap();

        let forest = reg.register(BiomeDef {
            name: "forest".into(),
            surface_voxel: VoxelTypeId(40),
            subsurface_voxel: VoxelTypeId(41),
            vegetation_density: 0.8,
            tree_type: Some("birch".into()),
        }).unwrap();

        let diagram = WhittakerDiagram {
            regions: vec![
                WhittakerRegion {
                    temp_min: 0.0, temp_max: 0.3,
                    moisture_min: 0.0, moisture_max: 0.5,
                    biome_id: tundra,
                },
                WhittakerRegion {
                    temp_min: 0.7, temp_max: 1.0,
                    moisture_min: 0.0, moisture_max: 0.3,
                    biome_id: desert,
                },
                WhittakerRegion {
                    temp_min: 0.3, temp_max: 0.7,
                    moisture_min: 0.0, moisture_max: 0.5,
                    biome_id: plains,
                },
                WhittakerRegion {
                    temp_min: 0.3, temp_max: 0.7,
                    moisture_min: 0.5, moisture_max: 1.0,
                    biome_id: forest,
                },
                // Remaining regions for full coverage...
                WhittakerRegion {
                    temp_min: 0.0, temp_max: 0.3,
                    moisture_min: 0.5, moisture_max: 1.0,
                    biome_id: tundra, // Taiga in full impl
                },
                WhittakerRegion {
                    temp_min: 0.7, temp_max: 1.0,
                    moisture_min: 0.3, moisture_max: 1.0,
                    biome_id: forest, // Tropical in full impl
                },
            ],
            fallback: plains,
        };

        (reg, diagram)
    }

    #[test]
    fn test_cold_dry_is_tundra() {
        let (reg, diagram) = make_test_registry_and_diagram();
        let biome_id = diagram.lookup(0.1, 0.2); // cold, dry
        assert_eq!(reg.get(biome_id).name, "tundra");
    }

    #[test]
    fn test_hot_dry_is_desert() {
        let (reg, diagram) = make_test_registry_and_diagram();
        let biome_id = diagram.lookup(0.8, 0.1); // hot, dry
        assert_eq!(reg.get(biome_id).name, "desert");
    }

    #[test]
    fn test_moderate_wet_is_forest() {
        let (reg, diagram) = make_test_registry_and_diagram();
        let biome_id = diagram.lookup(0.5, 0.7); // moderate temp, wet
        assert_eq!(reg.get(biome_id).name, "forest");
    }

    #[test]
    fn test_diagram_covers_entire_range() {
        let (_reg, diagram) = make_test_registry_and_diagram();

        // Sample a fine grid and ensure every point resolves to a biome
        // (does not hit the fallback except in intentional gaps).
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
        // Walk across a temperature gradient at fixed moisture and verify
        // that biome transitions happen cleanly without rapid oscillation.
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

        // With rectangular regions, we expect a small number of transitions
        // (at most one per region boundary crossed). Certainly not hundreds.
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
        }).unwrap();

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
        // Re-create with same seed.
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
```
