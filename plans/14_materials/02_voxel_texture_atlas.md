# Voxel Texture Atlas

## Problem

Each voxel type in the engine can have distinct textures for its top, side, and bottom faces (grass has a green top, brown side, and dirt bottom). If every texture were a separate GPU texture object, the renderer would need to rebind textures constantly during chunk rendering — causing thousands of draw calls per frame and destroying GPU performance. Voxel engines solve this by packing all face textures into a single large texture atlas, allowing the entire visible world to be rendered with a single texture bind. The atlas must be built at startup, provide correct UV coordinates for each face, and include a full mipmap chain to avoid aliasing at distance.

## Solution

Implement a `TextureAtlas` builder in the `nebula_materials` crate that loads individual texture files, packs them into a power-of-2 atlas texture, computes UV rectangles for each entry, and generates mipmaps.

### Atlas Layout

Each individual voxel texture is a square tile of uniform size (default 16x16 or 32x32 pixels). The atlas arranges tiles in a simple grid:

```rust
/// Configuration for atlas construction.
pub struct AtlasConfig {
    /// Width and height of the atlas texture in pixels.
    /// Must be a power of 2. Supported: 2048 or 4096.
    pub atlas_size: u32,
    /// Width and height of each individual tile in pixels.
    /// Must be a power of 2 and must evenly divide atlas_size.
    pub tile_size: u32,
}

impl AtlasConfig {
    pub fn tiles_per_row(&self) -> u32 {
        self.atlas_size / self.tile_size
    }

    pub fn max_tiles(&self) -> u32 {
        let per_row = self.tiles_per_row();
        per_row * per_row
    }
}
```

For a 4096x4096 atlas with 32x32 tiles, the maximum capacity is 16,384 tiles — more than enough for even the most asset-rich voxel game.

### Texture Entry

Each voxel type can reference up to three texture tiles (top, side, bottom), or a single tile used for all faces:

```rust
/// Describes which atlas tiles a voxel type uses.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum VoxelTextures {
    /// All six faces use the same texture.
    Uniform { texture: String },
    /// Top, side, and bottom are distinct.
    TopSideBottom {
        top: String,
        side: String,
        bottom: String,
    },
}
```

### Atlas Builder

```rust
pub struct AtlasBuilder {
    config: AtlasConfig,
    /// Maps texture filename -> tile index in the atlas.
    tile_map: HashMap<String, u32>,
    /// The atlas image being assembled.
    atlas_image: image::RgbaImage,
    /// Next free tile slot.
    next_slot: u32,
}

impl AtlasBuilder {
    pub fn new(config: AtlasConfig) -> Self {
        Self {
            atlas_image: image::RgbaImage::new(config.atlas_size, config.atlas_size),
            tile_map: HashMap::new(),
            next_slot: 0,
            config,
        }
    }

    /// Loads a texture file and places it in the next free atlas slot.
    /// Returns the tile index. If the texture has already been loaded,
    /// returns the existing index (deduplication).
    pub fn add_texture(&mut self, name: &str, path: &Path) -> Result<u32, AtlasError> {
        if let Some(&idx) = self.tile_map.get(name) {
            return Ok(idx);
        }
        if self.next_slot >= self.config.max_tiles() {
            return Err(AtlasError::AtlasFull);
        }

        let img = image::open(path)?
            .resize_exact(
                self.config.tile_size,
                self.config.tile_size,
                image::imageops::FilterType::Lanczos3,
            )
            .to_rgba8();

        let tiles_per_row = self.config.tiles_per_row();
        let col = self.next_slot % tiles_per_row;
        let row = self.next_slot / tiles_per_row;
        let px = col * self.config.tile_size;
        let py = row * self.config.tile_size;

        image::imageops::overlay(&mut self.atlas_image, &img, px as i64, py as i64);

        self.tile_map.insert(name.to_string(), self.next_slot);
        self.next_slot += 1;

        Ok(self.next_slot - 1)
    }

    /// Finalize the atlas: generate mipmaps and return the completed TextureAtlas.
    pub fn build(self) -> TextureAtlas { ... }
}
```

### UV Coordinate Calculation

Given a tile index, the UV rectangle is computed as:

```rust
impl TextureAtlas {
    /// Returns (uv_min, uv_max) for the given tile index.
    /// Both values are in [0.0, 1.0] normalized texture coordinates.
    pub fn tile_uvs(&self, tile_index: u32) -> (Vec2, Vec2) {
        let tiles_per_row = self.config.tiles_per_row();
        let col = tile_index % tiles_per_row;
        let row = tile_index / tiles_per_row;
        let tile_uv_size = self.config.tile_size as f32 / self.config.atlas_size as f32;

        let u_min = col as f32 * tile_uv_size;
        let v_min = row as f32 * tile_uv_size;

        (
            Vec2::new(u_min, v_min),
            Vec2::new(u_min + tile_uv_size, v_min + tile_uv_size),
        )
    }
}
```

A half-pixel inset can be applied to prevent texture bleeding at tile boundaries during mipmap sampling.

### Mipmap Generation

Mipmaps are generated by successively halving the atlas image using a box filter. The number of mip levels is `log2(atlas_size) + 1`. Each level is stored as a separate `RgbaImage` in a `Vec<image::RgbaImage>`:

```rust
pub struct TextureAtlas {
    pub config: AtlasConfig,
    /// Mip level 0 is the full-resolution atlas. Subsequent levels are half-size.
    pub mip_chain: Vec<image::RgbaImage>,
    /// Maps tile name -> tile index.
    pub tile_map: HashMap<String, u32>,
}
```

### GPU Upload

The atlas is uploaded to the GPU as a `wgpu::Texture` with `TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST` and the full mipmap chain:

```rust
impl TextureAtlas {
    pub fn upload_to_gpu(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> wgpu::Texture {
        let mip_count = self.mip_chain.len() as u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("voxel-atlas"),
            size: wgpu::Extent3d {
                width: self.config.atlas_size,
                height: self.config.atlas_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: mip_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        for (level, mip) in self.mip_chain.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                mip.as_raw(),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * mip.width()),
                    rows_per_image: Some(mip.height()),
                },
                wgpu::Extent3d {
                    width: mip.width(),
                    height: mip.height(),
                    depth_or_array_layers: 1,
                },
            );
        }

        texture
    }
}
```

## Outcome

A `TextureAtlas` type in `nebula_materials` that packs all voxel face textures into a single power-of-2 atlas image with a complete mipmap chain. The atlas builder loads individual texture files via the `image` crate, deduplicates them, and arranges them in a grid. UV coordinates are available for each tile. The atlas can be uploaded to a `wgpu::Texture` in a single call. Running `cargo test -p nebula_materials` passes all atlas tests.

## Demo Integration

**Demo crate:** `nebula-demo`

A texture atlas packs all voxel face textures into a single large GPU texture. Each voxel face samples from the correct atlas region. A debug overlay shows the atlas layout.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `image` | `0.25` | Load PNG/JPEG texture files, resize, composite into atlas, generate mipmaps |
| `wgpu` | `28.0` | GPU texture creation and upload |
| `glam` | `0.32` | `Vec2` for UV coordinates |
| `serde` | `1.0` with `derive` | Serialize `VoxelTextures` and `AtlasConfig` for RON asset files |
| `thiserror` | `2.0` | `AtlasError` derivation |

Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> AtlasConfig {
        AtlasConfig {
            atlas_size: 256,
            tile_size: 16,
        }
    }

    #[test]
    fn test_atlas_texture_has_power_of_2_dimensions() {
        let config = AtlasConfig { atlas_size: 4096, tile_size: 32 };
        assert!(config.atlas_size.is_power_of_two());
        assert!(config.tile_size.is_power_of_two());

        let config2 = AtlasConfig { atlas_size: 2048, tile_size: 16 };
        assert!(config2.atlas_size.is_power_of_two());
        assert!(config2.tile_size.is_power_of_two());
    }

    #[test]
    fn test_all_voxel_textures_fit_in_atlas() {
        let config = test_config();
        let max_tiles = config.max_tiles();
        // 256 / 16 = 16 tiles per row, 16 * 16 = 256 tiles max
        assert_eq!(max_tiles, 256);

        // Simulate adding up to capacity
        let mut builder = AtlasBuilder::new(config);
        for i in 0..256 {
            let name = format!("tile_{i}");
            // Create a small in-memory test image
            let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([128, 128, 128, 255]));
            let result = builder.add_texture_from_image(&name, &img);
            assert!(result.is_ok(), "Tile {i} should fit in atlas");
        }

        // One more should fail
        let overflow_img = image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 0, 0, 255]));
        let result = builder.add_texture_from_image("overflow", &overflow_img);
        assert!(matches!(result, Err(AtlasError::AtlasFull)));
    }

    #[test]
    fn test_uv_coordinates_within_unit_range() {
        let config = test_config();
        let atlas = AtlasBuilder::new(config).build();
        let max_tiles = atlas.config.max_tiles();

        for tile_idx in 0..max_tiles {
            let (uv_min, uv_max) = atlas.tile_uvs(tile_idx);
            assert!(uv_min.x >= 0.0 && uv_min.x <= 1.0,
                "UV min x out of range for tile {tile_idx}: {}", uv_min.x);
            assert!(uv_min.y >= 0.0 && uv_min.y <= 1.0,
                "UV min y out of range for tile {tile_idx}: {}", uv_min.y);
            assert!(uv_max.x >= 0.0 && uv_max.x <= 1.0,
                "UV max x out of range for tile {tile_idx}: {}", uv_max.x);
            assert!(uv_max.y >= 0.0 && uv_max.y <= 1.0,
                "UV max y out of range for tile {tile_idx}: {}", uv_max.y);
            assert!(uv_min.x < uv_max.x);
            assert!(uv_min.y < uv_max.y);
        }
    }

    #[test]
    fn test_atlas_can_be_uploaded_to_gpu() {
        // Verify the atlas produces valid wgpu texture descriptor parameters.
        let config = AtlasConfig { atlas_size: 256, tile_size: 16 };
        let atlas = AtlasBuilder::new(config).build();

        // Mip chain should exist with at least 1 level
        assert!(!atlas.mip_chain.is_empty());
        // Level 0 should match atlas_size
        assert_eq!(atlas.mip_chain[0].width(), 256);
        assert_eq!(atlas.mip_chain[0].height(), 256);
        // The texture format Rgba8UnormSrgb is 4 bytes per pixel
        let expected_bytes = 256 * 256 * 4;
        assert_eq!(atlas.mip_chain[0].as_raw().len(), expected_bytes as usize);
    }

    #[test]
    fn test_mipmap_chain_is_complete() {
        let config = AtlasConfig { atlas_size: 256, tile_size: 16 };
        let atlas = AtlasBuilder::new(config).build();

        // For a 256x256 atlas, the mip chain should have log2(256) + 1 = 9 levels
        // (256, 128, 64, 32, 16, 8, 4, 2, 1)
        let expected_levels = (256f32).log2() as usize + 1;
        assert_eq!(atlas.mip_chain.len(), expected_levels);

        // Each level should be exactly half the previous
        for i in 1..atlas.mip_chain.len() {
            let prev = &atlas.mip_chain[i - 1];
            let curr = &atlas.mip_chain[i];
            assert_eq!(curr.width(), prev.width() / 2);
            assert_eq!(curr.height(), prev.height() / 2);
        }

        // Final level should be 1x1
        let last = atlas.mip_chain.last().unwrap();
        assert_eq!(last.width(), 1);
        assert_eq!(last.height(), 1);
    }

    #[test]
    fn test_duplicate_texture_is_deduplicated() {
        let config = test_config();
        let mut builder = AtlasBuilder::new(config);
        let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([255, 0, 0, 255]));

        let idx_a = builder.add_texture_from_image("red_tile", &img).unwrap();
        let idx_b = builder.add_texture_from_image("red_tile", &img).unwrap();

        // Same name should return the same tile index, not allocate a second slot
        assert_eq!(idx_a, idx_b);
    }

    #[test]
    fn test_tile_uvs_do_not_overlap() {
        let config = test_config();
        let atlas = AtlasBuilder::new(config).build();
        let tiles_per_row = config.tiles_per_row();

        // Check that adjacent tiles in the same row have non-overlapping UVs
        for row in 0..tiles_per_row {
            for col in 0..(tiles_per_row - 1) {
                let idx_left = row * tiles_per_row + col;
                let idx_right = row * tiles_per_row + col + 1;
                let (_, left_max) = atlas.tile_uvs(idx_left);
                let (right_min, _) = atlas.tile_uvs(idx_right);
                assert!(
                    (left_max.x - right_min.x).abs() < f32::EPSILON,
                    "Adjacent tiles should share an edge, not overlap"
                );
            }
        }
    }
}
```
