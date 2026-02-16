//! Material registry: unified lookup for PBR properties and atlas UVs by [`MaterialId`].
//!
//! Loads material definitions from a RON manifest file, builds the texture atlas,
//! and provides O(1) lookups.

use std::collections::HashMap;
use std::path::Path;

use glam::Vec2;
use thiserror::Error;

use crate::atlas::{AtlasBuilder, AtlasConfig, AtlasError, TextureAtlas, VoxelTextures};
use crate::material::{MaterialDef, MaterialId};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors returned during registry construction.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// I/O error reading the manifest file.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// RON deserialization error.
    #[error("ron parse error: {0}")]
    Ron(#[from] ron::error::SpannedError),

    /// Atlas construction error.
    #[error("atlas error: {0}")]
    Atlas(#[from] AtlasError),

    /// Material validation error.
    #[error("material error: {0}")]
    Material(#[from] crate::material::MaterialError),

    /// Duplicate material name.
    #[error("duplicate material name: {0}")]
    DuplicateName(String),

    /// Texture file not found.
    #[error("texture not found: {0}")]
    TextureNotFound(String),
}

// ---------------------------------------------------------------------------
// RON manifest types
// ---------------------------------------------------------------------------

/// Top-level RON manifest for material definitions.
#[derive(serde::Deserialize)]
pub struct MaterialManifest {
    /// Atlas configuration (sizes).
    pub atlas: AtlasConfig,
    /// Material entries.
    pub materials: Vec<MaterialEntry>,
}

/// A single material entry in the RON manifest.
#[derive(serde::Deserialize)]
pub struct MaterialEntry {
    /// Human-readable name.
    pub name: String,
    /// Base color RGBA.
    pub albedo: (f32, f32, f32, f32),
    /// Metallic factor.
    pub metallic: f32,
    /// Roughness factor.
    pub roughness: f32,
    /// Emissive color RGB.
    pub emissive_color: (f32, f32, f32),
    /// Emissive intensity multiplier.
    pub emissive_intensity: f32,
    /// Normal map strength.
    pub normal_strength: f32,
    /// Opacity.
    pub opacity: f32,
    /// Texture assignment for faces.
    pub textures: VoxelTextures,
}

// ---------------------------------------------------------------------------
// Face
// ---------------------------------------------------------------------------

/// Face direction for UV lookups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// +Y face.
    Top,
    /// -Y face.
    Bottom,
    /// +Z face.
    North,
    /// -Z face.
    South,
    /// +X face.
    East,
    /// -X face.
    West,
}

// ---------------------------------------------------------------------------
// MaterialUVs
// ---------------------------------------------------------------------------

/// Per-material atlas UV data for each face group.
#[derive(Clone, Debug)]
pub struct MaterialUVs {
    /// UV rectangle (min, max) for the top face.
    pub top: (Vec2, Vec2),
    /// UV rectangle (min, max) for the four side faces.
    pub side: (Vec2, Vec2),
    /// UV rectangle (min, max) for the bottom face.
    pub bottom: (Vec2, Vec2),
}

// ---------------------------------------------------------------------------
// MaterialRegistry
// ---------------------------------------------------------------------------

/// Unified registry mapping [`MaterialId`] to PBR properties and atlas UVs.
///
/// `MaterialId(0)` is always the fallback (magenta checkerboard).
/// The registry is immutable after construction.
pub struct MaterialRegistry {
    /// Dense array: index == `MaterialId.0`.
    materials: Vec<MaterialDef>,
    /// Parallel array: atlas UV rectangles per material.
    uvs: Vec<MaterialUVs>,
    /// Reverse lookup: name → `MaterialId`.
    name_to_id: HashMap<String, MaterialId>,
    /// The built texture atlas.
    atlas: TextureAtlas,
}

impl MaterialRegistry {
    /// Load the registry from a RON manifest file on disk.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] on I/O, parse, or validation failures.
    pub fn from_ron(path: &Path, texture_base_dir: &Path) -> Result<Self, RegistryError> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_ron_str(&contents, texture_base_dir)
    }

    /// Load the registry from a RON string. Texture paths are resolved
    /// relative to `texture_base_dir`.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] on parse or validation failures.
    pub fn from_ron_str(ron_str: &str, texture_base_dir: &Path) -> Result<Self, RegistryError> {
        let manifest: MaterialManifest = ron::from_str(ron_str)?;
        manifest.atlas.validate()?;

        let mut builder = AtlasBuilder::new(manifest.atlas);
        let mut materials = Vec::with_capacity(manifest.materials.len() + 1);
        let mut uvs = Vec::with_capacity(manifest.materials.len() + 1);
        let mut name_to_id = HashMap::new();

        // Register fallback at ID 0
        let (fallback_def, fallback_uvs) = register_fallback(&mut builder)?;
        materials.push(fallback_def);
        uvs.push(fallback_uvs);
        name_to_id.insert("fallback".to_string(), MaterialId(0));

        // Register user materials
        for entry in manifest.materials {
            if name_to_id.contains_key(&entry.name) {
                return Err(RegistryError::DuplicateName(entry.name));
            }

            let id = MaterialId(materials.len() as u16);

            let mat_def = MaterialDef {
                name: entry.name.clone(),
                albedo: [
                    entry.albedo.0,
                    entry.albedo.1,
                    entry.albedo.2,
                    entry.albedo.3,
                ],
                metallic: entry.metallic,
                roughness: entry.roughness,
                emissive_color: [
                    entry.emissive_color.0,
                    entry.emissive_color.1,
                    entry.emissive_color.2,
                ],
                emissive_intensity: entry.emissive_intensity,
                normal_strength: entry.normal_strength,
                opacity: entry.opacity,
            }
            .validated()?;

            let mat_uvs = add_textures_to_atlas(&mut builder, &entry.textures, texture_base_dir)?;

            name_to_id.insert(entry.name, id);
            materials.push(mat_def);
            uvs.push(mat_uvs);
        }

        let atlas = builder.build();
        Ok(Self {
            materials,
            uvs,
            name_to_id,
            atlas,
        })
    }

    /// Returns the PBR properties for a material.
    ///
    /// If the ID is out of range, returns the fallback material at ID 0.
    pub fn get(&self, id: MaterialId) -> &MaterialDef {
        self.materials
            .get(id.0 as usize)
            .unwrap_or(&self.materials[0])
    }

    /// Returns the atlas UV rectangle for a specific face of a material.
    ///
    /// If the ID is out of range, returns the fallback UVs.
    pub fn atlas_uvs(&self, id: MaterialId, face: Face) -> (Vec2, Vec2) {
        let mat_uvs = self.uvs.get(id.0 as usize).unwrap_or(&self.uvs[0]);
        match face {
            Face::Top => mat_uvs.top,
            Face::Bottom => mat_uvs.bottom,
            Face::North | Face::South | Face::East | Face::West => mat_uvs.side,
        }
    }

    /// Look up a material ID by name.
    pub fn lookup_by_name(&self, name: &str) -> Option<MaterialId> {
        self.name_to_id.get(name).copied()
    }

    /// Number of registered materials (including the fallback at ID 0).
    pub fn len(&self) -> usize {
        self.materials.len()
    }

    /// Returns `true` if the registry contains no user materials (only fallback).
    pub fn is_empty(&self) -> bool {
        self.materials.len() <= 1
    }

    /// Access the underlying texture atlas for GPU upload.
    pub fn atlas(&self) -> &TextureAtlas {
        &self.atlas
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generates a checkerboard RGBA image of the given size.
fn generate_checkerboard(size: u32, color_a: [u8; 4], color_b: [u8; 4]) -> image::RgbaImage {
    let cell = (size / 4).max(1);
    image::RgbaImage::from_fn(size, size, |x, y| {
        let checker = ((x / cell) + (y / cell)).is_multiple_of(2);
        if checker {
            image::Rgba(color_a)
        } else {
            image::Rgba(color_b)
        }
    })
}

/// Registers the fallback material (magenta checkerboard) at tile slot 0.
fn register_fallback(
    builder: &mut AtlasBuilder,
) -> Result<(MaterialDef, MaterialUVs), RegistryError> {
    let fallback_def = MaterialDef {
        name: "fallback".to_string(),
        albedo: [1.0, 0.0, 1.0, 1.0],
        metallic: 0.0,
        roughness: 1.0,
        emissive_color: [0.0, 0.0, 0.0],
        emissive_intensity: 0.0,
        normal_strength: 0.0,
        opacity: 1.0,
    };

    let checkerboard = generate_checkerboard(32, [255, 0, 255, 255], [0, 0, 0, 255]);
    let tile_idx = builder.add_texture_from_image("__fallback__", &checkerboard)?;
    let uv = builder.tile_uvs(tile_idx);

    Ok((
        fallback_def,
        MaterialUVs {
            top: uv,
            side: uv,
            bottom: uv,
        },
    ))
}

/// Resolves texture paths and adds them to the atlas, returning [`MaterialUVs`].
fn add_textures_to_atlas(
    builder: &mut AtlasBuilder,
    textures: &VoxelTextures,
    base_dir: &Path,
) -> Result<MaterialUVs, RegistryError> {
    match textures {
        VoxelTextures::Uniform { texture } => {
            let idx = load_or_generate_tile(builder, texture, base_dir)?;
            let uv = builder.tile_uvs(idx);
            Ok(MaterialUVs {
                top: uv,
                side: uv,
                bottom: uv,
            })
        }
        VoxelTextures::TopSideBottom { top, side, bottom } => {
            let top_idx = load_or_generate_tile(builder, top, base_dir)?;
            let side_idx = load_or_generate_tile(builder, side, base_dir)?;
            let bottom_idx = load_or_generate_tile(builder, bottom, base_dir)?;
            Ok(MaterialUVs {
                top: builder.tile_uvs(top_idx),
                side: builder.tile_uvs(side_idx),
                bottom: builder.tile_uvs(bottom_idx),
            })
        }
    }
}

/// Loads a texture file from disk, or generates a solid-color placeholder if not found.
fn load_or_generate_tile(
    builder: &mut AtlasBuilder,
    name: &str,
    base_dir: &Path,
) -> Result<u32, RegistryError> {
    // Check if already added (deduplication in AtlasBuilder)
    if let Some(idx) = builder.tile_index(name) {
        return Ok(idx);
    }

    let path = base_dir.join(name);
    if path.exists() {
        Ok(builder.add_texture(&path.to_string_lossy(), &path)?)
    } else {
        // Generate a solid-color placeholder so the engine doesn't crash
        // on missing textures during development.
        let hash = name
            .bytes()
            .fold(0u32, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u32));
        let r = ((hash >> 16) & 0xFF) as u8;
        let g = ((hash >> 8) & 0xFF) as u8;
        let b = (hash & 0xFF) as u8;
        let placeholder = image::RgbaImage::from_pixel(32, 32, image::Rgba([r, g, b, 255]));
        Ok(builder.add_texture_from_image(name, &placeholder)?)
    }
}

// ---------------------------------------------------------------------------
// AtlasBuilder extension — tile_index and tile_uvs helpers
// ---------------------------------------------------------------------------

/// Extension trait giving `AtlasBuilder` UV lookup before `build()`.
trait AtlasBuilderExt {
    /// Look up a tile index by name.
    fn tile_index(&self, name: &str) -> Option<u32>;
    /// Get UV coords for a tile index.
    fn tile_uvs(&self, tile_index: u32) -> (Vec2, Vec2);
}

impl AtlasBuilderExt for AtlasBuilder {
    fn tile_index(&self, name: &str) -> Option<u32> {
        self.tile_map().get(name).copied()
    }

    fn tile_uvs(&self, tile_index: u32) -> (Vec2, Vec2) {
        let tiles_per_row = self.config().tiles_per_row();
        let col = tile_index % tiles_per_row;
        let row = tile_index / tiles_per_row;
        let tile_uv_size = self.config().tile_size as f32 / self.config().atlas_size as f32;
        let u_min = col as f32 * tile_uv_size;
        let v_min = row as f32 * tile_uv_size;
        (
            Vec2::new(u_min, v_min),
            Vec2::new(u_min + tile_uv_size, v_min + tile_uv_size),
        )
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_ron() -> String {
        r#"MaterialManifest(
            atlas: AtlasConfig(atlas_size: 256, tile_size: 16),
            materials: [
                (
                    name: "stone",
                    albedo: (0.5, 0.5, 0.5, 1.0),
                    metallic: 0.0,
                    roughness: 0.85,
                    emissive_color: (0.0, 0.0, 0.0),
                    emissive_intensity: 0.0,
                    normal_strength: 1.0,
                    opacity: 1.0,
                    textures: Uniform(texture: "stone.png"),
                ),
                (
                    name: "dirt",
                    albedo: (0.6, 0.4, 0.2, 1.0),
                    metallic: 0.0,
                    roughness: 0.95,
                    emissive_color: (0.0, 0.0, 0.0),
                    emissive_intensity: 0.0,
                    normal_strength: 1.0,
                    opacity: 1.0,
                    textures: Uniform(texture: "dirt.png"),
                ),
            ],
        )"#
        .to_string()
    }

    fn create_test_textures() -> TempDir {
        let dir = TempDir::new().unwrap();
        // Create small PNG textures
        let stone = image::RgbaImage::from_pixel(16, 16, image::Rgba([128, 128, 128, 255]));
        stone.save(dir.path().join("stone.png")).unwrap();
        let dirt = image::RgbaImage::from_pixel(16, 16, image::Rgba([139, 90, 43, 255]));
        dirt.save(dir.path().join("dirt.png")).unwrap();
        dir
    }

    fn create_test_registry() -> MaterialRegistry {
        let dir = create_test_textures();
        MaterialRegistry::from_ron_str(&sample_ron(), dir.path()).unwrap()
    }

    #[test]
    fn test_registry_loads_from_ron() {
        let registry = create_test_registry();
        // Fallback (ID 0) + stone (ID 1) + dirt (ID 2) = 3 materials
        assert_eq!(registry.len(), 3);
    }

    #[test]
    fn test_all_material_ids_are_sequential() {
        let registry = create_test_registry();
        assert_eq!(registry.get(MaterialId(0)).name, "fallback");
        assert_eq!(registry.get(MaterialId(1)).name, "stone");
        assert_eq!(registry.get(MaterialId(2)).name, "dirt");
    }

    #[test]
    fn test_atlas_uvs_are_valid() {
        let registry = create_test_registry();
        for id in 0..registry.len() as u16 {
            for face in [
                Face::Top,
                Face::Bottom,
                Face::North,
                Face::South,
                Face::East,
                Face::West,
            ] {
                let (uv_min, uv_max) = registry.atlas_uvs(MaterialId(id), face);
                assert!(
                    uv_min.x >= 0.0 && uv_min.x <= 1.0,
                    "UV min x out of range for material {id}, face {face:?}"
                );
                assert!(
                    uv_min.y >= 0.0 && uv_min.y <= 1.0,
                    "UV min y out of range for material {id}, face {face:?}"
                );
                assert!(
                    uv_max.x >= 0.0 && uv_max.x <= 1.0,
                    "UV max x out of range for material {id}, face {face:?}"
                );
                assert!(
                    uv_max.y >= 0.0 && uv_max.y <= 1.0,
                    "UV max y out of range for material {id}, face {face:?}"
                );
                assert!(uv_min.x < uv_max.x);
                assert!(uv_min.y < uv_max.y);
            }
        }
    }

    #[test]
    fn test_missing_material_returns_fallback() {
        let registry = create_test_registry();
        let mat = registry.get(MaterialId(9999));
        assert_eq!(mat.name, "fallback");
        assert_eq!(mat.albedo, [1.0, 0.0, 1.0, 1.0]);
    }

    #[test]
    fn test_registry_is_read_only_at_runtime() {
        let registry = create_test_registry();
        let _ref: &MaterialDef = registry.get(MaterialId(1));
        let _uvs: (Vec2, Vec2) = registry.atlas_uvs(MaterialId(1), Face::Top);
        let _name: Option<MaterialId> = registry.lookup_by_name("stone");
    }

    #[test]
    fn test_lookup_by_name() {
        let registry = create_test_registry();
        assert_eq!(registry.lookup_by_name("stone"), Some(MaterialId(1)));
        assert_eq!(registry.lookup_by_name("dirt"), Some(MaterialId(2)));
        assert_eq!(registry.lookup_by_name("nonexistent"), None);
    }

    #[test]
    fn test_fallback_is_always_id_zero() {
        let registry = create_test_registry();
        let fallback = registry.get(MaterialId(0));
        assert_eq!(fallback.name, "fallback");
        assert_eq!(fallback.albedo[0], 1.0);
        assert_eq!(fallback.albedo[1], 0.0);
        assert_eq!(fallback.albedo[2], 1.0);
    }

    #[test]
    fn test_is_empty() {
        let registry = create_test_registry();
        assert!(!registry.is_empty());
    }

    #[test]
    fn test_duplicate_name_rejected() {
        let ron = r#"MaterialManifest(
            atlas: AtlasConfig(atlas_size: 256, tile_size: 16),
            materials: [
                (
                    name: "stone",
                    albedo: (0.5, 0.5, 0.5, 1.0),
                    metallic: 0.0, roughness: 0.85,
                    emissive_color: (0.0, 0.0, 0.0), emissive_intensity: 0.0,
                    normal_strength: 1.0, opacity: 1.0,
                    textures: Uniform(texture: "stone.png"),
                ),
                (
                    name: "stone",
                    albedo: (0.5, 0.5, 0.5, 1.0),
                    metallic: 0.0, roughness: 0.85,
                    emissive_color: (0.0, 0.0, 0.0), emissive_intensity: 0.0,
                    normal_strength: 1.0, opacity: 1.0,
                    textures: Uniform(texture: "stone.png"),
                ),
            ],
        )"#;
        let dir = create_test_textures();
        let result = MaterialRegistry::from_ron_str(ron, dir.path());
        assert!(matches!(result, Err(RegistryError::DuplicateName(_))));
    }
}
