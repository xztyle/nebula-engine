//! Voxel texture atlas: packs individual tile textures into a single GPU-friendly atlas.
//!
//! The atlas arranges square tiles in a grid layout, computes UV coordinates for each tile,
//! and generates a full mipmap chain for distance rendering without aliasing.

use std::collections::HashMap;
use std::path::Path;

use glam::Vec2;
use serde::{Deserialize, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// AtlasError
// ---------------------------------------------------------------------------

/// Errors returned during atlas construction.
#[derive(Debug, Error)]
pub enum AtlasError {
    /// The atlas is full — no more tile slots available.
    #[error("atlas is full (max {max} tiles)")]
    AtlasFull {
        /// Maximum number of tiles the atlas can hold.
        max: u32,
    },

    /// Failed to load an image file.
    #[error("image load error: {0}")]
    ImageLoad(#[from] image::ImageError),

    /// Configuration validation error.
    #[error("invalid atlas config: {0}")]
    InvalidConfig(String),
}

// ---------------------------------------------------------------------------
// AtlasConfig
// ---------------------------------------------------------------------------

/// Configuration for atlas construction.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AtlasConfig {
    /// Width and height of the atlas texture in pixels.
    /// Must be a power of 2. Supported: 256, 512, 1024, 2048, or 4096.
    pub atlas_size: u32,
    /// Width and height of each individual tile in pixels.
    /// Must be a power of 2 and must evenly divide `atlas_size`.
    pub tile_size: u32,
}

impl AtlasConfig {
    /// Returns the number of tiles that fit in one row of the atlas.
    pub fn tiles_per_row(&self) -> u32 {
        self.atlas_size / self.tile_size
    }

    /// Returns the maximum number of tiles the atlas can hold.
    pub fn max_tiles(&self) -> u32 {
        let per_row = self.tiles_per_row();
        per_row * per_row
    }

    /// Validates that both sizes are powers of two and tile divides atlas evenly.
    pub fn validate(&self) -> Result<(), AtlasError> {
        if !self.atlas_size.is_power_of_two() {
            return Err(AtlasError::InvalidConfig(format!(
                "atlas_size {} is not a power of 2",
                self.atlas_size
            )));
        }
        if !self.tile_size.is_power_of_two() {
            return Err(AtlasError::InvalidConfig(format!(
                "tile_size {} is not a power of 2",
                self.tile_size
            )));
        }
        if !self.atlas_size.is_multiple_of(self.tile_size) {
            return Err(AtlasError::InvalidConfig(format!(
                "tile_size {} does not evenly divide atlas_size {}",
                self.tile_size, self.atlas_size
            )));
        }
        if self.tile_size > self.atlas_size {
            return Err(AtlasError::InvalidConfig(
                "tile_size must be <= atlas_size".to_string(),
            ));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// VoxelTextures
// ---------------------------------------------------------------------------

/// Describes which atlas tiles a voxel type uses.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum VoxelTextures {
    /// All six faces use the same texture.
    Uniform {
        /// Texture name (key into atlas tile map).
        texture: String,
    },
    /// Top, side, and bottom are distinct.
    TopSideBottom {
        /// Texture for the +Y face.
        top: String,
        /// Texture for the ±X and ±Z faces.
        side: String,
        /// Texture for the -Y face.
        bottom: String,
    },
}

// ---------------------------------------------------------------------------
// TextureAtlas
// ---------------------------------------------------------------------------

/// A completed texture atlas with mipmap chain and tile UV lookup.
pub struct TextureAtlas {
    /// Atlas configuration (sizes).
    pub config: AtlasConfig,
    /// Mip level 0 is the full-resolution atlas. Subsequent levels are half-size.
    pub mip_chain: Vec<image::RgbaImage>,
    /// Maps tile name → tile index.
    pub tile_map: HashMap<String, u32>,
}

impl TextureAtlas {
    /// Returns `(uv_min, uv_max)` for the given tile index.
    ///
    /// Both values are in `[0.0, 1.0]` normalized texture coordinates.
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

    /// Returns `(uv_min, uv_max)` with a half-pixel inset to prevent bleeding.
    pub fn tile_uvs_inset(&self, tile_index: u32) -> (Vec2, Vec2) {
        let (uv_min, uv_max) = self.tile_uvs(tile_index);
        let half_pixel = 0.5 / self.config.atlas_size as f32;
        (
            Vec2::new(uv_min.x + half_pixel, uv_min.y + half_pixel),
            Vec2::new(uv_max.x - half_pixel, uv_max.y - half_pixel),
        )
    }

    /// Looks up a tile index by name.
    pub fn tile_index(&self, name: &str) -> Option<u32> {
        self.tile_map.get(name).copied()
    }

    /// Returns the number of tiles in the atlas.
    pub fn tile_count(&self) -> u32 {
        self.tile_map.len() as u32
    }

    /// Returns the number of mip levels.
    pub fn mip_level_count(&self) -> u32 {
        self.mip_chain.len() as u32
    }
}

// ---------------------------------------------------------------------------
// AtlasBuilder
// ---------------------------------------------------------------------------

/// Builds a [`TextureAtlas`] by accumulating tile images into a grid.
pub struct AtlasBuilder {
    config: AtlasConfig,
    /// Maps texture name → tile index in the atlas.
    tile_map: HashMap<String, u32>,
    /// The atlas image being assembled.
    atlas_image: image::RgbaImage,
    /// Next free tile slot.
    next_slot: u32,
}

impl AtlasBuilder {
    /// Creates a new builder with the given configuration.
    pub fn new(config: AtlasConfig) -> Self {
        let atlas_image = image::RgbaImage::new(config.atlas_size, config.atlas_size);
        Self {
            tile_map: HashMap::new(),
            next_slot: 0,
            atlas_image,
            config,
        }
    }

    /// Loads a texture file from disk and places it in the next free atlas slot.
    ///
    /// Returns the tile index. If a texture with the same name was already added,
    /// returns the existing index (deduplication).
    pub fn add_texture(&mut self, name: &str, path: &Path) -> Result<u32, AtlasError> {
        if let Some(&idx) = self.tile_map.get(name) {
            return Ok(idx);
        }

        let img = image::open(path)?
            .resize_exact(
                self.config.tile_size,
                self.config.tile_size,
                image::imageops::FilterType::Lanczos3,
            )
            .to_rgba8();

        self.add_texture_from_image(name, &img)
    }

    /// Adds an in-memory RGBA image as a tile.
    ///
    /// The image is resized to `tile_size × tile_size` if it doesn't match.
    /// Returns the tile index, or the existing index if deduplicated.
    pub fn add_texture_from_image(
        &mut self,
        name: &str,
        img: &image::RgbaImage,
    ) -> Result<u32, AtlasError> {
        if let Some(&idx) = self.tile_map.get(name) {
            return Ok(idx);
        }
        if self.next_slot >= self.config.max_tiles() {
            return Err(AtlasError::AtlasFull {
                max: self.config.max_tiles(),
            });
        }

        // Resize if dimensions don't match tile_size.
        let tile = if img.width() != self.config.tile_size || img.height() != self.config.tile_size
        {
            image::imageops::resize(
                img,
                self.config.tile_size,
                self.config.tile_size,
                image::imageops::FilterType::Lanczos3,
            )
        } else {
            img.clone()
        };

        let tiles_per_row = self.config.tiles_per_row();
        let col = self.next_slot % tiles_per_row;
        let row = self.next_slot / tiles_per_row;
        let px = col * self.config.tile_size;
        let py = row * self.config.tile_size;

        image::imageops::overlay(&mut self.atlas_image, &tile, px as i64, py as i64);

        let idx = self.next_slot;
        self.tile_map.insert(name.to_string(), idx);
        self.next_slot += 1;

        Ok(idx)
    }

    /// Returns how many tile slots have been used.
    pub fn used_slots(&self) -> u32 {
        self.next_slot
    }

    /// Returns a reference to the tile name → index map.
    pub fn tile_map(&self) -> &HashMap<String, u32> {
        &self.tile_map
    }

    /// Returns a reference to the atlas configuration.
    pub fn config(&self) -> &AtlasConfig {
        &self.config
    }

    /// Finalizes the atlas: generates the mipmap chain and returns a [`TextureAtlas`].
    pub fn build(self) -> TextureAtlas {
        let mip_count = (self.config.atlas_size as f32).log2() as usize + 1;
        let mut mip_chain = Vec::with_capacity(mip_count);
        mip_chain.push(self.atlas_image);

        for level in 1..mip_count {
            let prev = &mip_chain[level - 1];
            let w = (prev.width() / 2).max(1);
            let h = (prev.height() / 2).max(1);
            let downscaled =
                image::imageops::resize(prev, w, h, image::imageops::FilterType::Triangle);
            mip_chain.push(downscaled);
        }

        TextureAtlas {
            config: self.config,
            mip_chain,
            tile_map: self.tile_map,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
        let config = AtlasConfig {
            atlas_size: 4096,
            tile_size: 32,
        };
        assert!(config.atlas_size.is_power_of_two());
        assert!(config.tile_size.is_power_of_two());

        let config2 = AtlasConfig {
            atlas_size: 2048,
            tile_size: 16,
        };
        assert!(config2.atlas_size.is_power_of_two());
        assert!(config2.tile_size.is_power_of_two());
    }

    #[test]
    fn test_all_voxel_textures_fit_in_atlas() {
        let config = test_config();
        let max_tiles = config.max_tiles();
        // 256 / 16 = 16 tiles per row, 16 * 16 = 256 tiles max
        assert_eq!(max_tiles, 256);

        let mut builder = AtlasBuilder::new(config);
        for i in 0..256 {
            let name = format!("tile_{i}");
            let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([128, 128, 128, 255]));
            let result = builder.add_texture_from_image(&name, &img);
            assert!(result.is_ok(), "Tile {i} should fit in atlas");
        }

        // One more should fail
        let overflow_img = image::RgbaImage::from_pixel(16, 16, image::Rgba([0, 0, 0, 255]));
        let result = builder.add_texture_from_image("overflow", &overflow_img);
        assert!(matches!(result, Err(AtlasError::AtlasFull { .. })));
    }

    #[test]
    fn test_uv_coordinates_within_unit_range() {
        let config = test_config();
        let atlas = AtlasBuilder::new(config).build();
        let max_tiles = atlas.config.max_tiles();

        for tile_idx in 0..max_tiles {
            let (uv_min, uv_max) = atlas.tile_uvs(tile_idx);
            assert!(
                uv_min.x >= 0.0 && uv_min.x <= 1.0,
                "UV min x out of range for tile {tile_idx}: {}",
                uv_min.x
            );
            assert!(
                uv_min.y >= 0.0 && uv_min.y <= 1.0,
                "UV min y out of range for tile {tile_idx}: {}",
                uv_min.y
            );
            assert!(
                uv_max.x >= 0.0 && uv_max.x <= 1.0,
                "UV max x out of range for tile {tile_idx}: {}",
                uv_max.x
            );
            assert!(
                uv_max.y >= 0.0 && uv_max.y <= 1.0,
                "UV max y out of range for tile {tile_idx}: {}",
                uv_max.y
            );
            assert!(uv_min.x < uv_max.x);
            assert!(uv_min.y < uv_max.y);
        }
    }

    #[test]
    fn test_atlas_can_be_uploaded_to_gpu() {
        let config = AtlasConfig {
            atlas_size: 256,
            tile_size: 16,
        };
        let atlas = AtlasBuilder::new(config).build();

        assert!(!atlas.mip_chain.is_empty());
        assert_eq!(atlas.mip_chain[0].width(), 256);
        assert_eq!(atlas.mip_chain[0].height(), 256);
        let expected_bytes = 256 * 256 * 4;
        assert_eq!(atlas.mip_chain[0].as_raw().len(), expected_bytes);
    }

    #[test]
    fn test_mipmap_chain_is_complete() {
        let config = AtlasConfig {
            atlas_size: 256,
            tile_size: 16,
        };
        let atlas = AtlasBuilder::new(config).build();

        // 256 -> log2(256) + 1 = 9 levels (256, 128, 64, 32, 16, 8, 4, 2, 1)
        let expected_levels = (256f32).log2() as usize + 1;
        assert_eq!(atlas.mip_chain.len(), expected_levels);

        for i in 1..atlas.mip_chain.len() {
            let prev = &atlas.mip_chain[i - 1];
            let curr = &atlas.mip_chain[i];
            assert_eq!(curr.width(), prev.width() / 2);
            assert_eq!(curr.height(), prev.height() / 2);
        }

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

        assert_eq!(idx_a, idx_b);
    }

    #[test]
    fn test_tile_uvs_do_not_overlap() {
        let config = test_config();
        let atlas = AtlasBuilder::new(config).build();
        let tiles_per_row = atlas.config.tiles_per_row();

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

    #[test]
    fn test_config_validation() {
        let valid = AtlasConfig {
            atlas_size: 256,
            tile_size: 16,
        };
        assert!(valid.validate().is_ok());

        let bad_atlas = AtlasConfig {
            atlas_size: 300,
            tile_size: 16,
        };
        assert!(bad_atlas.validate().is_err());

        let bad_tile = AtlasConfig {
            atlas_size: 256,
            tile_size: 17,
        };
        assert!(bad_tile.validate().is_err());
    }

    #[test]
    fn test_tile_uvs_inset() {
        let config = test_config();
        let atlas = AtlasBuilder::new(config).build();
        let (uv_min, uv_max) = atlas.tile_uvs(0);
        let (inset_min, inset_max) = atlas.tile_uvs_inset(0);

        assert!(inset_min.x > uv_min.x);
        assert!(inset_min.y > uv_min.y);
        assert!(inset_max.x < uv_max.x);
        assert!(inset_max.y < uv_max.y);
    }

    #[test]
    fn test_voxel_textures_serde() {
        let uniform = VoxelTextures::Uniform {
            texture: "stone".to_string(),
        };
        let json = serde_json::to_string(&uniform).unwrap();
        let _: VoxelTextures = serde_json::from_str(&json).unwrap();

        let tsb = VoxelTextures::TopSideBottom {
            top: "grass_top".to_string(),
            side: "grass_side".to_string(),
            bottom: "dirt".to_string(),
        };
        let json = serde_json::to_string(&tsb).unwrap();
        let _: VoxelTextures = serde_json::from_str(&json).unwrap();
    }
}
