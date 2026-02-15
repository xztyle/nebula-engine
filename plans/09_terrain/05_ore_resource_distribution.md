# Ore/Resource Distribution

## Problem

A voxel planet needs underground resources -- iron, copper, gold, diamond, and other ores -- for crafting, progression, and economic systems. These resources must be distributed realistically: common ores (iron, copper) appear at shallow depths in generous veins, while rare ores (diamond, emerald) appear only deep underground in small pockets. The distribution must be deterministic (same seed produces same ore placement), configurable per ore type, and efficient enough to evaluate during chunk generation without becoming a bottleneck. Ores must never appear above the terrain surface, and deeper ores should be genuinely rarer to reward exploration.

## Solution

Implement an `OreDistributor` in the `nebula-terrain` crate that uses per-ore-type 3D noise fields to place ore veins at configured depth ranges and densities. Each ore type has its own `OreDistribution` definition with depth bounds, noise threshold, and vein scale. During chunk generation, the distributor is queried for each subsurface voxel to determine if it should be replaced with an ore type.

### Ore Distribution Configuration

```rust
use crate::voxel::VoxelTypeId;

/// Configuration for a single ore type's underground distribution.
#[derive(Clone, Debug)]
pub struct OreDistribution {
    /// The voxel type ID for this ore (e.g., iron_ore, gold_ore).
    pub ore_type: VoxelTypeId,
    /// Human-readable name for logging/debugging.
    pub name: String,
    /// Minimum depth below the terrain surface where this ore can appear.
    /// In engine units. E.g., 0 = can appear immediately below surface.
    pub min_depth: f64,
    /// Maximum depth below the terrain surface where this ore can appear.
    pub max_depth: f64,
    /// Noise threshold for ore placement. Voxels where noise > threshold
    /// become ore. Higher threshold = rarer ore. Range: [0.0, 1.0].
    pub noise_threshold: f64,
    /// Spatial scale of ore veins. Controls the noise frequency.
    /// Smaller values = larger veins, larger values = smaller veins.
    /// Default: 0.1.
    pub vein_scale: f64,
    /// Seed offset to decorrelate this ore's noise from other ore types.
    /// Automatically derived from the ore name hash if not specified.
    pub seed_offset: u64,
}
```

### Ore Distributor

```rust
use noise::{NoiseFn, Simplex};

pub struct OreDistributor {
    /// World seed.
    seed: u64,
    /// All registered ore distributions, sorted by priority (common first).
    ores: Vec<OreDistributionRuntime>,
}

/// Runtime data for one ore type, including the pre-initialized noise function.
struct OreDistributionRuntime {
    config: OreDistribution,
    noise: Simplex,
}

impl OreDistributor {
    pub fn new(seed: u64, ore_configs: Vec<OreDistribution>) -> Self {
        let ores = ore_configs
            .into_iter()
            .map(|config| {
                let ore_seed = seed.wrapping_add(config.seed_offset);
                let noise = Simplex::new(ore_seed as u32);
                OreDistributionRuntime { config, noise }
            })
            .collect();

        Self { seed, ores }
    }

    /// Query which ore type (if any) should replace the default stone voxel
    /// at the given position.
    ///
    /// # Arguments
    /// - `voxel_pos`: The 3D world-space position of the voxel.
    /// - `surface_height`: The terrain surface height at this column.
    ///
    /// # Returns
    /// `Some(VoxelTypeId)` if the voxel should be an ore, `None` if it stays stone.
    pub fn sample_ore(
        &self,
        voxel_pos: glam::DVec3,
        surface_height: f64,
    ) -> Option<VoxelTypeId> {
        let voxel_height = voxel_pos.length();

        // Only place ores below the surface.
        if voxel_height >= surface_height {
            return None;
        }

        let depth = surface_height - voxel_height;

        // Check each ore type. First match wins (priority ordering).
        for ore_rt in &self.ores {
            let cfg = &ore_rt.config;

            // Check depth range.
            if depth < cfg.min_depth || depth > cfg.max_depth {
                continue;
            }

            // Sample 3D noise at the voxel position, scaled by vein_scale.
            let noise_val = ore_rt.noise.get([
                voxel_pos.x * cfg.vein_scale,
                voxel_pos.y * cfg.vein_scale,
                voxel_pos.z * cfg.vein_scale,
            ]);

            // Normalize from [-1, 1] to [0, 1].
            let normalized = (noise_val + 1.0) * 0.5;

            if normalized > cfg.noise_threshold {
                return Some(cfg.ore_type);
            }
        }

        None
    }

    /// Count the number of registered ore types.
    pub fn ore_count(&self) -> usize {
        self.ores.len()
    }
}
```

### Default Ore Configuration

A typical planet defines ores with increasing rarity at greater depths:

| Ore | Min Depth | Max Depth | Threshold | Vein Scale | Relative Rarity |
|-----|-----------|-----------|-----------|------------|-----------------|
| Coal | 3 | 200 | 0.75 | 0.08 | Common |
| Iron | 5 | 300 | 0.80 | 0.10 | Common |
| Copper | 5 | 250 | 0.82 | 0.10 | Moderate |
| Gold | 30 | 400 | 0.90 | 0.15 | Rare |
| Diamond | 80 | 500 | 0.95 | 0.20 | Very rare |
| Emerald | 60 | 450 | 0.94 | 0.25 | Very rare |

```rust
pub fn default_ore_distributions() -> Vec<OreDistribution> {
    vec![
        OreDistribution {
            ore_type: VoxelTypeId(100),
            name: "coal".into(),
            min_depth: 3.0,
            max_depth: 200.0,
            noise_threshold: 0.75,
            vein_scale: 0.08,
            seed_offset: 0x0001,
        },
        OreDistribution {
            ore_type: VoxelTypeId(101),
            name: "iron".into(),
            min_depth: 5.0,
            max_depth: 300.0,
            noise_threshold: 0.80,
            vein_scale: 0.10,
            seed_offset: 0x0002,
        },
        OreDistribution {
            ore_type: VoxelTypeId(102),
            name: "copper".into(),
            min_depth: 5.0,
            max_depth: 250.0,
            noise_threshold: 0.82,
            vein_scale: 0.10,
            seed_offset: 0x0003,
        },
        OreDistribution {
            ore_type: VoxelTypeId(103),
            name: "gold".into(),
            min_depth: 30.0,
            max_depth: 400.0,
            noise_threshold: 0.90,
            vein_scale: 0.15,
            seed_offset: 0x0004,
        },
        OreDistribution {
            ore_type: VoxelTypeId(104),
            name: "diamond".into(),
            min_depth: 80.0,
            max_depth: 500.0,
            noise_threshold: 0.95,
            vein_scale: 0.20,
            seed_offset: 0x0005,
        },
    ]
}
```

### Depth-Based Rarity

Deeper ores are rarer through two complementary mechanisms:

1. **Higher noise threshold**: Diamond at 0.95 means only the top 5% of noise values produce diamond, versus coal at 0.75 (top 25%).
2. **Smaller vein scale**: Higher `vein_scale` values produce smaller noise features, so individual veins are physically smaller.

## Outcome

An `OreDistributor` struct and `OreDistribution` config in `nebula-terrain` that deterministically places ore veins in the subsurface volume. Each ore type has configurable depth range, rarity, and vein size. Running `cargo test -p nebula-terrain` passes all ore distribution tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Ore veins appear as colored patches within underground terrain. The debug overlay shows ore type distribution and density.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `noise` | 0.9 | 3D simplex noise for ore vein field generation |
| `glam` | 0.29 | `DVec3` for voxel position coordinates |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::DVec3;

    const PLANET_RADIUS: f64 = 6_371_000.0;

    fn test_distributor() -> OreDistributor {
        OreDistributor::new(42, default_ore_distributions())
    }

    #[test]
    fn test_ores_appear_within_depth_range() {
        let dist = test_distributor();
        let surface_height = PLANET_RADIUS + 100.0;

        // Sample many points at various depths and verify any ores found
        // are within their configured depth range.
        for x in 0..50 {
            for z in 0..50 {
                for depth_idx in 0..100 {
                    let depth = (depth_idx as f64) * 5.0 + 1.0;
                    let voxel_pos = DVec3::new(
                        surface_height - depth,
                        x as f64 * 3.0,
                        z as f64 * 3.0,
                    );
                    if let Some(ore_id) = dist.sample_ore(voxel_pos, surface_height) {
                        // Find which ore config matches this ID.
                        let ores = default_ore_distributions();
                        let cfg = ores.iter().find(|o| o.ore_type == ore_id).unwrap();
                        assert!(
                            depth >= cfg.min_depth && depth <= cfg.max_depth,
                            "Ore '{}' found at depth {depth}, but allowed range is [{}, {}]",
                            cfg.name, cfg.min_depth, cfg.max_depth
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_deeper_ores_are_rarer() {
        let dist = test_distributor();
        let surface_height = PLANET_RADIUS + 600.0; // enough depth for all ores

        let mut coal_count = 0u64;
        let mut diamond_count = 0u64;
        let samples = 50_000;

        for i in 0..samples {
            let x = (i as f64) * 0.7;
            let z = (i as f64) * 1.3;

            // Sample at coal's typical depth (shallow).
            let shallow_pos = DVec3::new(surface_height - 50.0, x, z);
            if let Some(id) = dist.sample_ore(shallow_pos, surface_height) {
                if id == VoxelTypeId(100) {
                    coal_count += 1;
                }
            }

            // Sample at diamond's typical depth (deep).
            let deep_pos = DVec3::new(surface_height - 200.0, x, z);
            if let Some(id) = dist.sample_ore(deep_pos, surface_height) {
                if id == VoxelTypeId(104) {
                    diamond_count += 1;
                }
            }
        }

        assert!(
            coal_count > diamond_count,
            "Coal (threshold=0.75) should be more common than diamond (threshold=0.95): \
             coal={coal_count}, diamond={diamond_count}"
        );
    }

    #[test]
    fn test_different_seeds_produce_different_distributions() {
        let surface_height = PLANET_RADIUS + 200.0;
        let dist_a = OreDistributor::new(1, default_ore_distributions());
        let dist_b = OreDistributor::new(9999, default_ore_distributions());

        let mut differences = 0;
        for i in 0..1000 {
            let voxel_pos = DVec3::new(
                surface_height - 50.0,
                i as f64 * 2.1,
                i as f64 * 0.7,
            );
            let a = dist_a.sample_ore(voxel_pos, surface_height);
            let b = dist_b.sample_ore(voxel_pos, surface_height);
            if a != b {
                differences += 1;
            }
        }

        assert!(
            differences > 0,
            "Different seeds should produce at least some different ore placements"
        );
    }

    #[test]
    fn test_ore_density_matches_configured_threshold() {
        // Lower threshold = more ore. Compare two configs for the same ore type.
        let surface_height = PLANET_RADIUS + 200.0;

        let ore_common = OreDistribution {
            ore_type: VoxelTypeId(200),
            name: "test_common".into(),
            min_depth: 1.0,
            max_depth: 500.0,
            noise_threshold: 0.5, // 50% of noise values produce ore
            vein_scale: 0.1,
            seed_offset: 0x100,
        };
        let ore_rare = OreDistribution {
            ore_type: VoxelTypeId(201),
            name: "test_rare".into(),
            min_depth: 1.0,
            max_depth: 500.0,
            noise_threshold: 0.95, // 5% of noise values produce ore
            vein_scale: 0.1,
            seed_offset: 0x100, // Same seed offset for fair comparison
        };

        let dist_common = OreDistributor::new(42, vec![ore_common]);
        let dist_rare = OreDistributor::new(42, vec![ore_rare]);

        let mut count_common = 0;
        let mut count_rare = 0;
        for i in 0..10_000 {
            let voxel_pos = DVec3::new(
                surface_height - 50.0,
                i as f64 * 0.5,
                i as f64 * 0.3,
            );
            if dist_common.sample_ore(voxel_pos, surface_height).is_some() {
                count_common += 1;
            }
            if dist_rare.sample_ore(voxel_pos, surface_height).is_some() {
                count_rare += 1;
            }
        }

        assert!(
            count_common > count_rare,
            "Lower threshold should produce more ore: common={count_common}, rare={count_rare}"
        );
    }

    #[test]
    fn test_no_ores_above_surface() {
        let dist = test_distributor();
        let surface_height = PLANET_RADIUS + 100.0;

        for i in 0..1000 {
            let above = surface_height + (i as f64) + 1.0;
            let voxel_pos = DVec3::new(above, i as f64, i as f64 * 0.5);
            assert!(
                dist.sample_ore(voxel_pos, surface_height).is_none(),
                "No ores should appear above the terrain surface (height={above})"
            );
        }
    }

    #[test]
    fn test_deterministic_with_same_seed() {
        let dist_a = OreDistributor::new(77, default_ore_distributions());
        let dist_b = OreDistributor::new(77, default_ore_distributions());
        let surface_height = PLANET_RADIUS + 300.0;

        for i in 0..500 {
            let voxel_pos = DVec3::new(
                surface_height - 100.0,
                i as f64 * 1.1,
                i as f64 * 2.3,
            );
            assert_eq!(
                dist_a.sample_ore(voxel_pos, surface_height),
                dist_b.sample_ore(voxel_pos, surface_height),
                "Same seed must produce identical ore placement at index {i}"
            );
        }
    }
}
```
