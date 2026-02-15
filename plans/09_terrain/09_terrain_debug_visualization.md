# Terrain Debug Visualization

## Problem

Terrain generation involves multiple layered noise functions (heightmap, temperature, moisture, caves, ores), each with configurable parameters (octaves, frequency, threshold). When terrain looks wrong -- flat where it should be mountainous, caves breaching the ocean floor, ores appearing at the wrong depth -- the developer needs to see the raw noise outputs visually. Without debug overlays, diagnosing terrain generation issues requires reading raw voxel data or scattering print statements, both of which are slow and impractical for spatial data. The engine needs real-time 2D visualizations of the terrain generation pipeline: heightmaps as color-coded elevation images, biome maps with biome-specific colors, cave cross-sections, and ore distribution heatmaps, all accessible as toggleable egui debug windows.

## Solution

Implement a `TerrainDebugOverlay` system in the `nebula-debug` crate (or a `debug` module within `nebula-terrain`) that renders terrain generation data as 2D textures displayed in egui windows. Each overlay samples the corresponding noise function at a grid of points, maps the values to colors, and presents the result as an interactive debug panel.

### Debug Image Generation

```rust
/// A 2D debug image represented as a flat array of RGBA pixels.
#[derive(Clone, Debug)]
pub struct DebugImage {
    pub width: u32,
    pub height: u32,
    /// Pixel data in row-major RGBA format. Length = width * height * 4.
    pub pixels: Vec<u8>,
}

impl DebugImage {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; (width * height * 4) as usize],
        }
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, r: u8, g: u8, b: u8, a: u8) {
        let idx = ((y * self.width + x) * 4) as usize;
        self.pixels[idx] = r;
        self.pixels[idx + 1] = g;
        self.pixels[idx + 2] = b;
        self.pixels[idx + 3] = a;
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}
```

### Heightmap Visualization

Renders the heightmap noise as a color-coded elevation map. Low elevations are deep blue (ocean), sea level is cyan, plains are green, mountains are brown, and peaks are white:

```rust
use crate::heightmap::HeightmapSampler;

/// Generate a debug heightmap image for a region of the cube face.
pub fn render_heightmap_debug(
    sampler: &HeightmapSampler,
    config: &TerrainHeightConfig,
    width: u32,
    height: u32,
    face: CubeFace,
    /// UV range on the cube face: (u_min, v_min, u_max, v_max).
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
            let normalized = ((raw / max_amp) + 1.0) * 0.5;

            let (r, g, b) = height_to_color(normalized, config);
            image.set_pixel(px, py, r, g, b, 255);
        }
    }

    image
}

/// Map a normalized height [0, 1] to an RGB color.
fn height_to_color(normalized: f64, config: &TerrainHeightConfig) -> (u8, u8, u8) {
    let sea_level_normalized =
        (0.0 - config.min_height) / (config.max_height - config.min_height);

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
        let t = (normalized - sea_level_normalized) / (0.55 - sea_level_normalized);
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
```

### Biome Map Visualization

```rust
use crate::biome::{BiomeSampler, BiomeRegistry, BiomeId};

/// Color palette for biome debug visualization.
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
        _ => (128, 128, 128), // Unknown biome: gray
    }
}

/// Generate a debug biome map image.
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
```

### Cave Cross-Section Visualization

Renders a horizontal or vertical slice through the 3D cave noise field:

```rust
use crate::caves::CaveCarver;

/// Generate a debug cave cross-section at a fixed depth below the surface.
pub fn render_cave_cross_section(
    carver: &CaveCarver,
    width: u32,
    height: u32,
    slice_origin: glam::DVec3,
    slice_u_axis: glam::DVec3,
    slice_v_axis: glam::DVec3,
    extent: f64,
    surface_height: f64,
    sea_level: f64,
) -> DebugImage {
    let mut image = DebugImage::new(width, height);

    for py in 0..height {
        for px in 0..width {
            let u = (px as f64 / width as f64 - 0.5) * extent;
            let v = (py as f64 / height as f64 - 0.5) * extent;
            let world_pos = slice_origin + slice_u_axis * u + slice_v_axis * v;

            if carver.is_cave(world_pos, surface_height, sea_level) {
                // Cave: dark/black
                image.set_pixel(px, py, 20, 20, 30, 255);
            } else {
                // Solid: brown/stone color
                image.set_pixel(px, py, 140, 120, 90, 255);
            }
        }
    }

    image
}
```

### Ore Distribution Heatmap

```rust
use crate::ore::OreDistributor;

/// Generate a debug ore distribution heatmap at a fixed depth.
pub fn render_ore_heatmap(
    distributor: &OreDistributor,
    width: u32,
    height: u32,
    slice_origin: glam::DVec3,
    slice_u_axis: glam::DVec3,
    slice_v_axis: glam::DVec3,
    extent: f64,
    surface_height: f64,
) -> DebugImage {
    let mut image = DebugImage::new(width, height);

    // Color map for ore types by VoxelTypeId.
    let ore_color = |id: VoxelTypeId| -> (u8, u8, u8) {
        match id.0 {
            100 => (50, 50, 50),     // Coal: dark gray
            101 => (180, 100, 60),   // Iron: rust brown
            102 => (200, 130, 50),   // Copper: orange
            103 => (255, 215, 0),    // Gold: gold
            104 => (100, 200, 255),  // Diamond: light blue
            _ => (200, 200, 200),    // Unknown: light gray
        }
    };

    for py in 0..height {
        for px in 0..width {
            let u = (px as f64 / width as f64 - 0.5) * extent;
            let v = (py as f64 / height as f64 - 0.5) * extent;
            let world_pos = slice_origin + slice_u_axis * u + slice_v_axis * v;

            if let Some(ore_id) = distributor.sample_ore(world_pos, surface_height) {
                let (r, g, b) = ore_color(ore_id);
                image.set_pixel(px, py, r, g, b, 255);
            } else {
                // Stone: neutral gray
                image.set_pixel(px, py, 100, 100, 100, 255);
            }
        }
    }

    image
}
```

### Egui Integration

The debug overlays are presented in egui windows that can be toggled on/off via the debug menu:

```rust
use egui::{Context, TextureHandle, ColorImage, TextureOptions};

/// State for all terrain debug overlays.
pub struct TerrainDebugState {
    pub show_heightmap: bool,
    pub show_biome_map: bool,
    pub show_cave_section: bool,
    pub show_ore_heatmap: bool,
    heightmap_texture: Option<TextureHandle>,
    biome_texture: Option<TextureHandle>,
    cave_texture: Option<TextureHandle>,
    ore_texture: Option<TextureHandle>,
    /// Whether the textures need regeneration (parameters changed).
    dirty: bool,
}

impl TerrainDebugState {
    pub fn new() -> Self {
        Self {
            show_heightmap: false,
            show_biome_map: false,
            show_cave_section: false,
            show_ore_heatmap: false,
            heightmap_texture: None,
            biome_texture: None,
            cave_texture: None,
            ore_texture: None,
            dirty: true,
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn any_visible(&self) -> bool {
        self.show_heightmap || self.show_biome_map
            || self.show_cave_section || self.show_ore_heatmap
    }
}

/// Render the terrain debug UI. Called each frame from the debug panel system.
pub fn render_terrain_debug_ui(
    ctx: &Context,
    state: &mut TerrainDebugState,
    // ... sampler references for lazy regeneration ...
) {
    egui::Window::new("Terrain Debug")
        .open(&mut true)
        .show(ctx, |ui| {
            ui.checkbox(&mut state.show_heightmap, "Heightmap");
            ui.checkbox(&mut state.show_biome_map, "Biome Map");
            ui.checkbox(&mut state.show_cave_section, "Cave Cross-Section");
            ui.checkbox(&mut state.show_ore_heatmap, "Ore Heatmap");

            if state.show_heightmap {
                if let Some(tex) = &state.heightmap_texture {
                    ui.image((tex.id(), tex.size_vec2()));
                }
            }
            if state.show_biome_map {
                if let Some(tex) = &state.biome_texture {
                    ui.image((tex.id(), tex.size_vec2()));
                }
            }
            if state.show_cave_section {
                if let Some(tex) = &state.cave_texture {
                    ui.image((tex.id(), tex.size_vec2()));
                }
            }
            if state.show_ore_heatmap {
                if let Some(tex) = &state.ore_texture {
                    ui.image((tex.id(), tex.size_vec2()));
                }
            }
        });
}

/// Convert a DebugImage to an egui ColorImage for texture upload.
pub fn debug_image_to_egui(image: &DebugImage) -> ColorImage {
    ColorImage::from_rgba_unmultiplied(
        [image.width as usize, image.height as usize],
        &image.pixels,
    )
}
```

## Outcome

A `TerrainDebugState`, `DebugImage`, and rendering functions for heightmap, biome, cave, and ore visualizations in `nebula-terrain` (or `nebula-debug`). Debug overlays are toggleable egui windows that display real-time 2D slices of the terrain generation pipeline. Running `cargo test -p nebula-terrain` passes all debug visualization tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Pressing F3 toggles a debug overlay showing heightmap, biome, cave density, and ore distribution as color-coded 2D maps in egui windows.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `egui` | 0.31 | Immediate-mode UI for debug windows and image display |
| `glam` | 0.29 | `DVec3` for slice origin and axis vectors |

All terrain noise crates (`noise`, `rand`, etc.) are indirect dependencies via the sampler types. The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_debug_image_correct_dimensions() {
        let image = DebugImage::new(256, 128);
        assert_eq!(image.dimensions(), (256, 128));
        assert_eq!(image.pixels.len(), 256 * 128 * 4);
    }

    #[test]
    fn test_debug_image_set_pixel() {
        let mut image = DebugImage::new(10, 10);
        image.set_pixel(3, 5, 255, 128, 64, 255);

        let idx = ((5 * 10 + 3) * 4) as usize;
        assert_eq!(image.pixels[idx], 255);
        assert_eq!(image.pixels[idx + 1], 128);
        assert_eq!(image.pixels[idx + 2], 64);
        assert_eq!(image.pixels[idx + 3], 255);
    }

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
        // Verify that known biome IDs produce the expected colors.
        let mut reg = BiomeRegistry::new();
        let ocean_id = reg.register(BiomeDef {
            name: "ocean".into(),
            surface_voxel: VoxelTypeId(1),
            subsurface_voxel: VoxelTypeId(2),
            vegetation_density: 0.0,
            tree_type: None,
        }).unwrap();

        let (r, g, b) = biome_color(ocean_id, &reg);
        assert_eq!((r, g, b), (20, 50, 180), "Ocean should be blue");

        let desert_id = reg.register(BiomeDef {
            name: "desert".into(),
            surface_voxel: VoxelTypeId(3),
            subsurface_voxel: VoxelTypeId(4),
            vegetation_density: 0.0,
            tree_type: None,
        }).unwrap();

        let (r, g, b) = biome_color(desert_id, &reg);
        assert_eq!((r, g, b), (220, 190, 80), "Desert should be sandy yellow");
    }

    #[test]
    fn test_debug_views_update_when_parameters_change() {
        // Generate two heightmap images with different seeds and verify they differ.
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
            &sampler_a, &config, 32, 32, CubeFace::PosX, (0.0, 0.0, 1.0, 1.0),
        );
        let image_b = render_heightmap_debug(
            &sampler_b, &config, 32, 32, CubeFace::PosX, (0.0, 0.0, 1.0, 1.0),
        );

        assert_ne!(
            image_a.pixels, image_b.pixels,
            "Different seeds should produce different debug images"
        );
    }

    #[test]
    fn test_overlays_can_be_toggled() {
        let mut state = TerrainDebugState::new();

        assert!(!state.show_heightmap);
        assert!(!state.show_biome_map);
        assert!(!state.any_visible());

        state.show_heightmap = true;
        assert!(state.any_visible());

        state.show_heightmap = false;
        assert!(!state.any_visible());

        state.show_biome_map = true;
        state.show_cave_section = true;
        state.show_ore_heatmap = true;
        assert!(state.any_visible());

        state.show_biome_map = false;
        state.show_cave_section = false;
        state.show_ore_heatmap = false;
        assert!(!state.any_visible());
    }

    #[test]
    fn test_height_to_color_covers_full_range() {
        let config = TerrainHeightConfig::default();

        // Sample the color function at many points in [0, 1] and ensure
        // it never panics and always produces valid RGB.
        for i in 0..=100 {
            let normalized = i as f64 / 100.0;
            let (r, g, b) = height_to_color(normalized, &config);
            // RGB values are u8, so they're inherently in [0, 255].
            // Just ensure the function doesn't panic.
            let _ = (r, g, b);
        }
    }

    #[test]
    fn test_debug_image_to_egui_conversion() {
        let mut image = DebugImage::new(4, 4);
        image.set_pixel(0, 0, 255, 0, 0, 255);
        image.set_pixel(3, 3, 0, 255, 0, 255);

        let color_image = debug_image_to_egui(&image);
        assert_eq!(color_image.size, [4, 4]);
        assert_eq!(color_image.pixels.len(), 16); // 4x4 = 16 pixels
    }
}
```
