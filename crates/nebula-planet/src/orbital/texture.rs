//! Terrain color texture generation for orbital planet rendering.
//!
//! Samples the terrain heightmap and biome system across the planet surface
//! to produce an equirectangular color texture.

use glam::DVec3;
use nebula_terrain::{BiomeSampler, HeightmapParams, TerrainHeightConfig, TerrainHeightSampler};

/// Generate a 2D equirectangular texture representing the planet's terrain colors.
///
/// Resolution is typically 512×256 or 1024×512 (longitude × latitude).
/// Each pixel is an RGBA color derived from the biome and elevation at that point.
pub fn generate_terrain_color_texture(
    terrain: &TerrainHeightSampler,
    biome_sampler: &BiomeSampler,
    width: u32,
    height: u32,
) -> Vec<[u8; 4]> {
    let config = terrain.config();
    let height_range = config.max_height - config.min_height;
    let mut pixels = Vec::with_capacity((width * height) as usize);

    for y in 0..height {
        let latitude = std::f64::consts::PI * (0.5 - y as f64 / height as f64);
        for x in 0..width {
            let longitude = std::f64::consts::TAU * (x as f64 / width as f64 - 0.5);

            let sphere_point = DVec3::new(
                latitude.cos() * longitude.cos(),
                latitude.sin(),
                latitude.cos() * longitude.sin(),
            );

            let height_val = terrain.sample_height(sphere_point);
            let (_biome_id, temperature, moisture) = biome_sampler.sample(sphere_point);

            let color = terrain_color(height_val, height_range, config, temperature, moisture);
            pixels.push(color);
        }
    }

    pixels
}

/// Map terrain height + biome parameters to an RGBA color.
///
/// Uses simple heuristics to produce recognizable orbital colors:
/// - Deep water: dark blue
/// - Shallow water: light blue
/// - Low + warm + wet: green (forest)
/// - Low + warm + dry: yellow-brown (desert)
/// - High elevation: grey-brown (mountains)
/// - Very high / cold: white (snow/ice)
fn terrain_color(
    height: f64,
    height_range: f64,
    config: &TerrainHeightConfig,
    temperature: f64,
    moisture: f64,
) -> [u8; 4] {
    let sea_level = config.sea_level;

    // Underwater
    if height < sea_level {
        let depth_frac = ((sea_level - height) / (height_range * 0.3)).clamp(0.0, 1.0);
        let r = lerp(70.0, 20.0, depth_frac);
        let g = lerp(130.0, 40.0, depth_frac);
        let b = lerp(180.0, 100.0, depth_frac);
        return [r as u8, g as u8, b as u8, 255];
    }

    // Normalized height above sea level
    let h_frac = ((height - sea_level) / height_range.abs().max(1.0)).clamp(0.0, 1.0);

    // Snow/ice at high elevation or very cold
    if h_frac > 0.6 || temperature < 0.15 {
        let snow_blend = if temperature < 0.15 {
            1.0
        } else {
            ((h_frac - 0.6) / 0.2).clamp(0.0, 1.0)
        };
        let r = lerp(160.0, 240.0, snow_blend);
        let g = lerp(140.0, 240.0, snow_blend);
        let b = lerp(130.0, 250.0, snow_blend);
        return [r as u8, g as u8, b as u8, 255];
    }

    // Mountain rock at medium-high elevation
    if h_frac > 0.35 {
        let rock_blend = ((h_frac - 0.35) / 0.25).clamp(0.0, 1.0);
        let r = lerp(100.0, 140.0, rock_blend);
        let g = lerp(90.0, 120.0, rock_blend);
        let b = lerp(70.0, 100.0, rock_blend);
        return [r as u8, g as u8, b as u8, 255];
    }

    // Low elevation: biome-dependent
    if moisture > 0.5 && temperature > 0.4 {
        // Forest (green)
        let g_val = lerp(120.0, 80.0, h_frac / 0.35);
        return [40, g_val as u8, 30, 255];
    }

    if moisture < 0.3 && temperature > 0.6 {
        // Desert (sandy)
        return [194, 178, 128, 255];
    }

    // Plains / grassland
    let g_val = lerp(150.0, 110.0, h_frac / 0.35);
    [80, g_val as u8, 50, 255]
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

/// Helper to create default terrain + biome samplers for orbital texture generation.
pub fn create_default_samplers(
    seed: u64,
    planet_radius: f64,
) -> (TerrainHeightSampler, BiomeSampler) {
    use nebula_terrain::{BiomeDef, BiomeRegistry, WhittakerDiagram, WhittakerRegion};
    use nebula_voxel::VoxelTypeId;

    // Scale frequency inversely to planet radius so features are visible
    let base_freq = 1.0 / (planet_radius * 0.5);
    let heightmap_params = HeightmapParams {
        seed,
        base_frequency: base_freq,
        ..Default::default()
    };
    let config = TerrainHeightConfig {
        planet_radius,
        ..Default::default()
    };
    let terrain = TerrainHeightSampler::new(heightmap_params, config);

    let mut reg = BiomeRegistry::new();
    let tundra = reg
        .register(BiomeDef {
            name: "tundra".into(),
            surface_voxel: VoxelTypeId(10),
            subsurface_voxel: VoxelTypeId(11),
            vegetation_density: 0.02,
            tree_type: None,
        })
        .expect("register tundra");
    let desert = reg
        .register(BiomeDef {
            name: "desert".into(),
            surface_voxel: VoxelTypeId(20),
            subsurface_voxel: VoxelTypeId(21),
            vegetation_density: 0.01,
            tree_type: None,
        })
        .expect("register desert");
    let plains = reg
        .register(BiomeDef {
            name: "plains".into(),
            surface_voxel: VoxelTypeId(30),
            subsurface_voxel: VoxelTypeId(31),
            vegetation_density: 0.3,
            tree_type: Some("oak".into()),
        })
        .expect("register plains");
    let forest = reg
        .register(BiomeDef {
            name: "forest".into(),
            surface_voxel: VoxelTypeId(40),
            subsurface_voxel: VoxelTypeId(41),
            vegetation_density: 0.8,
            tree_type: Some("birch".into()),
        })
        .expect("register forest");

    let diagram = WhittakerDiagram {
        regions: vec![
            WhittakerRegion {
                temp_min: 0.0,
                temp_max: 0.3,
                moisture_min: 0.0,
                moisture_max: 1.0,
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
                temp_min: 0.7,
                temp_max: 1.0,
                moisture_min: 0.3,
                moisture_max: 1.0,
                biome_id: forest,
            },
        ],
        fallback: plains,
    };

    let mut biome_sampler = BiomeSampler::new(seed, diagram);
    // Scale biome noise to planet size
    let biome_freq = 1.0 / (planet_radius * 0.3);
    biome_sampler.temp_frequency = biome_freq;
    biome_sampler.moisture_frequency = biome_freq * 1.4;
    (terrain, biome_sampler)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_terrain_texture_correct_size() {
        let (terrain, biome) = create_default_samplers(42, 200.0);
        let texture = generate_terrain_color_texture(&terrain, &biome, 64, 32);
        assert_eq!(texture.len(), 64 * 32);
    }

    #[test]
    fn test_terrain_texture_has_color_variety() {
        let (terrain, biome) = create_default_samplers(42, 200.0);
        let texture = generate_terrain_color_texture(&terrain, &biome, 256, 128);

        let unique: std::collections::HashSet<[u8; 4]> = texture.iter().copied().collect();
        assert!(
            unique.len() > 10,
            "Terrain texture should have color variety, got {} unique colors",
            unique.len()
        );
    }

    #[test]
    fn test_terrain_texture_all_opaque() {
        let (terrain, biome) = create_default_samplers(42, 200.0);
        let texture = generate_terrain_color_texture(&terrain, &biome, 64, 32);
        for (i, pixel) in texture.iter().enumerate() {
            assert_eq!(
                pixel[3], 255,
                "Pixel {i} has unexpected alpha: {}",
                pixel[3]
            );
        }
    }
}
