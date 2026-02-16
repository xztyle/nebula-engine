//! Terrain debug visualization renderers: heightmap, biome, cave, and ore.

use glam::DVec3;
use nebula_cubesphere::{CubeFace, FaceCoord, face_coord_to_sphere_everitt};
use nebula_voxel::VoxelTypeId;

use super::image::DebugImage;
use crate::biome::{BiomeId, BiomeRegistry, BiomeSampler};
use crate::cave::CaveCarver;
use crate::heightmap::HeightmapSampler;
use crate::ore::OreDistributor;
use crate::terrain_height::TerrainHeightConfig;

/// Generate a debug heightmap image for a region of a cube face.
///
/// Samples the heightmap noise at each pixel and maps the value to a
/// color-coded elevation: deep blue (ocean) → green (plains) → brown
/// (mountains) → white (snow peaks).
pub fn render_heightmap_debug(
    sampler: &HeightmapSampler,
    config: &TerrainHeightConfig,
    width: u32,
    height: u32,
    face: CubeFace,
    region: (f64, f64, f64, f64),
) -> DebugImage {
    let mut image = DebugImage::new(width, height);
    let max_amp = sampler.max_amplitude();

    for py in 0..height {
        for px in 0..width {
            let u = region.0 + (px as f64 / width as f64) * (region.2 - region.0);
            let v = region.1 + (py as f64 / height as f64) * (region.3 - region.1);

            let fc = FaceCoord::new(face, u, v);
            let sphere_pt = face_coord_to_sphere_everitt(&fc);
            let raw = sampler.sample_3d(sphere_pt);

            // Normalize to [0, 1].
            let normalized = if max_amp > 0.0 {
                ((raw / max_amp) + 1.0) * 0.5
            } else {
                0.5
            };

            let (r, g, b) = height_to_color(normalized, config);
            image.set_pixel(px, py, r, g, b, 255);
        }
    }

    image
}

/// Map a normalized height `[0, 1]` to an RGB color.
///
/// Color bands: deep ocean → shallow ocean → beach → plains → mountains → snow.
pub fn height_to_color(normalized: f64, config: &TerrainHeightConfig) -> (u8, u8, u8) {
    let range = config.max_height - config.min_height;
    let sea_level_normalized = if range > 0.0 {
        (0.0 - config.min_height) / range
    } else {
        0.5
    };

    if normalized < sea_level_normalized * 0.5 {
        // Deep ocean: dark blue
        (0, 0, 128)
    } else if normalized < sea_level_normalized {
        // Shallow ocean: blue
        (30, 80, 200)
    } else if normalized < sea_level_normalized + 0.02 {
        // Beach: sandy yellow
        (220, 200, 130)
    } else if normalized < 0.55 {
        // Plains/lowlands: green
        let t =
            ((normalized - sea_level_normalized) / (0.55 - sea_level_normalized)).clamp(0.0, 1.0);
        (
            (30.0 + t * 80.0) as u8,
            (160.0 - t * 40.0) as u8,
            (30.0 + t * 20.0) as u8,
        )
    } else if normalized < 0.75 {
        // Mountains: brown
        let t = (normalized - 0.55) / 0.2;
        (
            (110.0 + t * 40.0) as u8,
            (120.0 - t * 50.0) as u8,
            (50.0 + t * 20.0) as u8,
        )
    } else {
        // Snow peaks: white
        let t = ((normalized - 0.75) / 0.25).min(1.0);
        let base = 150.0 + t * 105.0;
        (base as u8, base as u8, base as u8)
    }
}

/// Return an RGB color for a given biome ID based on its name.
pub fn biome_color(biome_id: BiomeId, registry: &BiomeRegistry) -> (u8, u8, u8) {
    let name = &registry.get(biome_id).name;
    match name.as_str() {
        "ocean" => (20, 50, 180),
        "beach" => (230, 210, 140),
        "plains" => (100, 180, 60),
        "forest" => (30, 120, 30),
        "desert" => (220, 190, 80),
        "tundra" => (200, 210, 220),
        "taiga" => (40, 90, 60),
        "mountains" => (130, 110, 90),
        "tropical_rainforest" => (10, 80, 20),
        "savanna" => (180, 170, 60),
        _ => (128, 128, 128),
    }
}

/// Generate a debug biome map image for a region of a cube face.
pub fn render_biome_debug(
    biome_sampler: &BiomeSampler,
    registry: &BiomeRegistry,
    width: u32,
    height: u32,
    face: CubeFace,
    region: (f64, f64, f64, f64),
) -> DebugImage {
    let mut image = DebugImage::new(width, height);

    for py in 0..height {
        for px in 0..width {
            let u = region.0 + (px as f64 / width as f64) * (region.2 - region.0);
            let v = region.1 + (py as f64 / height as f64) * (region.3 - region.1);

            let fc = FaceCoord::new(face, u, v);
            let sphere_pt = face_coord_to_sphere_everitt(&fc);
            let (biome_id, _, _) = biome_sampler.sample(sphere_pt);
            let (r, g, b) = biome_color(biome_id, registry);
            image.set_pixel(px, py, r, g, b, 255);
        }
    }

    image
}

/// Parameters for a 2D slice through 3D space.
#[derive(Clone, Debug)]
pub struct SliceParams {
    /// Center of the slice in world space.
    pub origin: DVec3,
    /// Horizontal axis direction of the slice.
    pub u_axis: DVec3,
    /// Vertical axis direction of the slice.
    pub v_axis: DVec3,
    /// Total extent of the slice (width/height in world units).
    pub extent: f64,
}

/// Generate a debug cave cross-section at a slice through the 3D cave noise.
///
/// The slice is defined by an origin point and two axis vectors (U and V),
/// spanning `extent` units in each direction from the origin.
pub fn render_cave_cross_section(
    carver: &CaveCarver,
    width: u32,
    height: u32,
    slice: &SliceParams,
    surface_height: f64,
    sea_level: f64,
) -> DebugImage {
    let mut image = DebugImage::new(width, height);

    for py in 0..height {
        for px in 0..width {
            let u = (px as f64 / width as f64 - 0.5) * slice.extent;
            let v = (py as f64 / height as f64 - 0.5) * slice.extent;
            let world_pos = slice.origin + slice.u_axis * u + slice.v_axis * v;

            if carver.is_cave(world_pos, surface_height, sea_level) {
                // Cave: dark
                image.set_pixel(px, py, 20, 20, 30, 255);
            } else {
                // Solid: brown/stone
                image.set_pixel(px, py, 140, 120, 90, 255);
            }
        }
    }

    image
}

/// Generate a debug ore distribution heatmap at a slice through the world.
pub fn render_ore_heatmap(
    distributor: &OreDistributor,
    width: u32,
    height: u32,
    slice: &SliceParams,
    surface_height: f64,
) -> DebugImage {
    let mut image = DebugImage::new(width, height);

    for py in 0..height {
        for px in 0..width {
            let u = (px as f64 / width as f64 - 0.5) * slice.extent;
            let v = (py as f64 / height as f64 - 0.5) * slice.extent;
            let world_pos = slice.origin + slice.u_axis * u + slice.v_axis * v;

            if let Some(ore_id) = distributor.sample_ore(world_pos, surface_height) {
                let (r, g, b) = ore_type_color(ore_id);
                image.set_pixel(px, py, r, g, b, 255);
            } else {
                // Stone: neutral gray
                image.set_pixel(px, py, 100, 100, 100, 255);
            }
        }
    }

    image
}

/// Map an ore [`VoxelTypeId`] to an RGB color for heatmap visualization.
fn ore_type_color(id: VoxelTypeId) -> (u8, u8, u8) {
    match id.0 {
        100 => (50, 50, 50),    // Coal: dark gray
        101 => (180, 100, 60),  // Iron: rust brown
        102 => (200, 130, 50),  // Copper: orange
        103 => (255, 215, 0),   // Gold: gold
        104 => (100, 200, 255), // Diamond: light blue
        _ => (200, 200, 200),   // Unknown: light gray
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::biome::{BiomeDef, WhittakerDiagram, WhittakerRegion};
    use crate::cave::CaveConfig;
    use crate::heightmap::HeightmapParams;
    use crate::ore::default_ore_distributions;

    #[test]
    fn test_heightmap_image_has_correct_dimensions() {
        let sampler = HeightmapSampler::new(HeightmapParams {
            seed: 42,
            ..Default::default()
        });
        let config = TerrainHeightConfig::default();

        let image = render_heightmap_debug(
            &sampler,
            &config,
            64,
            64,
            CubeFace::PosX,
            (0.0, 0.0, 1.0, 1.0),
        );

        assert_eq!(image.dimensions(), (64, 64));
        assert_eq!(image.pixels.len(), 64 * 64 * 4);
    }

    #[test]
    fn test_biome_map_uses_correct_colors() {
        let mut reg = BiomeRegistry::new();
        let ocean_id = reg
            .register(BiomeDef {
                name: "ocean".into(),
                surface_voxel: VoxelTypeId(1),
                subsurface_voxel: VoxelTypeId(2),
                vegetation_density: 0.0,
                tree_type: None,
            })
            .unwrap();

        let (r, g, b) = biome_color(ocean_id, &reg);
        assert_eq!((r, g, b), (20, 50, 180), "Ocean should be blue");

        let desert_id = reg
            .register(BiomeDef {
                name: "desert".into(),
                surface_voxel: VoxelTypeId(3),
                subsurface_voxel: VoxelTypeId(4),
                vegetation_density: 0.0,
                tree_type: None,
            })
            .unwrap();

        let (r, g, b) = biome_color(desert_id, &reg);
        assert_eq!((r, g, b), (220, 190, 80), "Desert should be sandy yellow");
    }

    #[test]
    fn test_debug_views_update_when_parameters_change() {
        let config = TerrainHeightConfig::default();

        let sampler_a = HeightmapSampler::new(HeightmapParams {
            seed: 1,
            ..Default::default()
        });
        let sampler_b = HeightmapSampler::new(HeightmapParams {
            seed: 2,
            ..Default::default()
        });

        let image_a = render_heightmap_debug(
            &sampler_a,
            &config,
            32,
            32,
            CubeFace::PosX,
            (0.0, 0.0, 1.0, 1.0),
        );
        let image_b = render_heightmap_debug(
            &sampler_b,
            &config,
            32,
            32,
            CubeFace::PosX,
            (0.0, 0.0, 1.0, 1.0),
        );

        assert_ne!(
            image_a.pixels, image_b.pixels,
            "Different seeds should produce different debug images"
        );
    }

    #[test]
    fn test_height_to_color_covers_full_range() {
        let config = TerrainHeightConfig::default();

        for i in 0..=100 {
            let normalized = i as f64 / 100.0;
            let (r, g, b) = height_to_color(normalized, &config);
            // Just ensure it doesn't panic and produces valid u8 values.
            let _ = (r, g, b);
        }
    }

    #[test]
    fn test_heightmap_has_multiple_colors() {
        let sampler = HeightmapSampler::new(HeightmapParams {
            seed: 42,
            ..Default::default()
        });
        let config = TerrainHeightConfig::default();

        let image = render_heightmap_debug(
            &sampler,
            &config,
            64,
            64,
            CubeFace::PosX,
            (0.0, 0.0, 1.0, 1.0),
        );

        assert!(
            image.unique_color_count() > 1,
            "Heightmap should have multiple colors, got {}",
            image.unique_color_count()
        );
    }

    #[test]
    fn test_cave_cross_section_has_two_states() {
        let carver = CaveCarver::new(CaveConfig {
            seed: 42,
            threshold: 0.0,
            ..Default::default()
        });

        let planet_radius = 6_371_000.0_f64;
        let surface_height = planet_radius + 200.0;
        let sea_level = planet_radius;

        let slice = SliceParams {
            origin: DVec3::new(surface_height - 50.0, 0.0, 0.0),
            u_axis: DVec3::Y,
            v_axis: DVec3::Z,
            extent: 500.0,
        };
        let image = render_cave_cross_section(&carver, 64, 64, &slice, surface_height, sea_level);

        // Should have at least cave (dark) and solid (brown) colors.
        assert!(
            image.unique_color_count() >= 2,
            "Cave cross-section should have at least 2 colors"
        );
    }

    #[test]
    fn test_ore_heatmap_dimensions() {
        let dist = OreDistributor::new(42, default_ore_distributions());
        let planet_radius = 6_371_000.0_f64;
        let surface_height = planet_radius + 200.0;

        let slice = SliceParams {
            origin: DVec3::new(surface_height - 50.0, 0.0, 0.0),
            u_axis: DVec3::Y,
            v_axis: DVec3::Z,
            extent: 200.0,
        };
        let image = render_ore_heatmap(&dist, 32, 32, &slice, surface_height);

        assert_eq!(image.dimensions(), (32, 32));
    }

    #[test]
    fn test_biome_debug_has_correct_dimensions() {
        let mut reg = BiomeRegistry::new();
        let plains = reg
            .register(BiomeDef {
                name: "plains".into(),
                surface_voxel: VoxelTypeId(1),
                subsurface_voxel: VoxelTypeId(2),
                vegetation_density: 0.3,
                tree_type: None,
            })
            .unwrap();

        let diagram = WhittakerDiagram {
            regions: vec![WhittakerRegion {
                temp_min: 0.0,
                temp_max: 1.0,
                moisture_min: 0.0,
                moisture_max: 1.0,
                biome_id: plains,
            }],
            fallback: plains,
        };

        let sampler = BiomeSampler::new(42, diagram);

        let image =
            render_biome_debug(&sampler, &reg, 48, 48, CubeFace::PosZ, (0.0, 0.0, 1.0, 1.0));

        assert_eq!(image.dimensions(), (48, 48));
        assert_eq!(image.pixels.len(), 48 * 48 * 4);
    }
}
