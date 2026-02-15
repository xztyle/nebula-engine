# Material Registry

## Problem

Material definitions (story 01) and atlas UV coordinates (story 02) exist as separate data structures. The rendering pipeline, meshing system, and ECS all need a single, unified lookup point that maps a `MaterialId` to both the material's PBR properties and the atlas UV rectangles for each face. Without a registry, every system would need to hold references to both the `MaterialDef` collection and the `TextureAtlas`, duplicating lookup logic and making it easy to get out of sync. The registry must also be data-driven — artists and designers define materials in a RON configuration file, not in Rust code — so the material set can be changed without recompiling the engine.

## Solution

Implement a `MaterialRegistry` in the `nebula_materials` crate that loads material definitions from a RON file, builds the texture atlas, and provides O(1) lookups by `MaterialId`.

### RON Configuration Format

Materials are defined in a `materials.ron` file:

```ron
MaterialManifest(
    atlas: AtlasConfig(
        atlas_size: 4096,
        tile_size: 32,
    ),
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
            textures: Uniform(texture: "textures/stone.png"),
        ),
        (
            name: "grass",
            albedo: (0.3, 0.7, 0.2, 1.0),
            metallic: 0.0,
            roughness: 0.9,
            emissive_color: (0.0, 0.0, 0.0),
            emissive_intensity: 0.0,
            normal_strength: 1.0,
            opacity: 1.0,
            textures: TopSideBottom(
                top: "textures/grass_top.png",
                side: "textures/grass_side.png",
                bottom: "textures/dirt.png",
            ),
        ),
        (
            name: "lava",
            albedo: (1.0, 0.3, 0.0, 1.0),
            metallic: 0.0,
            roughness: 0.3,
            emissive_color: (1.0, 0.4, 0.0),
            emissive_intensity: 5.0,
            normal_strength: 0.5,
            opacity: 1.0,
            textures: Uniform(texture: "textures/lava_01.png"),
        ),
    ],
)
```

### RON Deserialization

```rust
#[derive(serde::Deserialize)]
pub struct MaterialManifest {
    pub atlas: AtlasConfig,
    pub materials: Vec<MaterialEntry>,
}

#[derive(serde::Deserialize)]
pub struct MaterialEntry {
    pub name: String,
    pub albedo: (f32, f32, f32, f32),
    pub metallic: f32,
    pub roughness: f32,
    pub emissive_color: (f32, f32, f32),
    pub emissive_intensity: f32,
    pub normal_strength: f32,
    pub opacity: f32,
    pub textures: VoxelTextures,
}
```

### Registry Structure

```rust
/// Face direction for UV lookups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Top,
    Bottom,
    North,
    South,
    East,
    West,
}

/// Per-material atlas UV data for each face.
#[derive(Clone, Debug)]
pub struct MaterialUVs {
    /// UV rectangle (min, max) for each face.
    /// For Uniform textures, all six faces share the same UVs.
    pub top: (Vec2, Vec2),
    pub side: (Vec2, Vec2),
    pub bottom: (Vec2, Vec2),
}

pub struct MaterialRegistry {
    /// Dense array: index == MaterialId.0
    materials: Vec<MaterialDef>,
    /// Parallel array: atlas UV rectangles per material.
    uvs: Vec<MaterialUVs>,
    /// Reverse lookup: name -> MaterialId.
    name_to_id: HashMap<String, MaterialId>,
    /// The built texture atlas.
    atlas: TextureAtlas,
}
```

### API

```rust
impl MaterialRegistry {
    /// Load the registry from a RON manifest file.
    /// Builds the texture atlas and validates all materials.
    pub fn from_ron(path: &Path, texture_base_dir: &Path) -> Result<Self, RegistryError> {
        let contents = std::fs::read_to_string(path)?;
        let manifest: MaterialManifest = ron::from_str(&contents)?;
        // ... build atlas, register materials, compute UVs ...
    }

    /// Returns the PBR properties for a material.
    /// If the ID is out of range, returns the fallback material at ID 0.
    pub fn get(&self, id: MaterialId) -> &MaterialDef {
        self.materials.get(id.0 as usize)
            .unwrap_or(&self.materials[0])
    }

    /// Returns the atlas UV rectangle for a specific face of a material.
    /// If the ID is out of range, returns the fallback (pink checkerboard) UVs.
    pub fn atlas_uvs(&self, id: MaterialId, face: Face) -> (Vec2, Vec2) {
        let uvs = self.uvs.get(id.0 as usize)
            .unwrap_or(&self.uvs[0]);
        match face {
            Face::Top => uvs.top,
            Face::Bottom => uvs.bottom,
            _ => uvs.side,
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

    /// Access the underlying texture atlas for GPU upload.
    pub fn atlas(&self) -> &TextureAtlas {
        &self.atlas
    }
}
```

### Fallback Material

`MaterialId(0)` is always the fallback material — a pink checkerboard pattern that is immediately visible as "missing texture." It is registered automatically before processing the RON manifest:

```rust
fn register_fallback(builder: &mut AtlasBuilder) -> (MaterialDef, MaterialUVs) {
    let fallback_def = MaterialDef {
        name: "fallback".to_string(),
        albedo: [1.0, 0.0, 1.0, 1.0], // magenta
        metallic: 0.0,
        roughness: 1.0,
        emissive_color: [0.0, 0.0, 0.0],
        emissive_intensity: 0.0,
        normal_strength: 0.0,
        opacity: 1.0,
    };

    // Generate a 32x32 pink/black checkerboard texture
    let checkerboard = generate_checkerboard(32, [255, 0, 255, 255], [0, 0, 0, 255]);
    let tile_idx = builder.add_texture_from_image("__fallback__", &checkerboard).unwrap();
    let uvs = builder.tile_uvs(tile_idx);

    (fallback_def, MaterialUVs { top: uvs, side: uvs, bottom: uvs })
}
```

### ECS Integration

The registry is inserted as a shared resource in the ECS world, wrapped in `Arc<MaterialRegistry>`:

```rust
world.insert_resource(Arc::new(
    MaterialRegistry::from_ron(
        &asset_path("materials.ron"),
        &asset_path(""),
    )?
));
```

Systems access it via `Res<Arc<MaterialRegistry>>` and can query materials without any synchronization overhead since the registry is immutable after startup.

## Outcome

A `MaterialRegistry` struct in `nebula_materials` that loads all material definitions and textures from a RON file, builds a unified texture atlas, and provides O(1) lookup of PBR properties and atlas UVs by `MaterialId`. A pink checkerboard fallback is always available at ID 0. The registry is shared as an immutable ECS resource. Running `cargo test -p nebula_materials` passes all registry tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Materials are looked up by name in the registry. Adding a new material requires only a registry entry and a texture — the system is data-driven.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `ron` | `0.12` | Deserialize `MaterialManifest` from `.ron` configuration files |
| `serde` | `1.0` with `derive` | Deserialization of manifest structs |
| `image` | `0.25` | Texture loading via `AtlasBuilder` (transitive from story 02) |
| `glam` | `0.32` | `Vec2` for UV coordinates |
| `wgpu` | `28.0` | Atlas GPU upload (transitive from story 02) |
| `thiserror` | `2.0` | `RegistryError` derivation |

Depends on stories 01 (`MaterialDef`, `MaterialId`) and 02 (`TextureAtlas`, `AtlasBuilder`). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

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
        )"#.to_string()
    }

    fn create_test_registry() -> MaterialRegistry {
        let ron_str = sample_ron();
        // Create temp texture files and parse the RON
        MaterialRegistry::from_ron_str(&ron_str, &create_test_textures()).unwrap()
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
        // ID 0 = fallback, ID 1 = stone, ID 2 = dirt
        assert_eq!(registry.get(MaterialId(0)).name, "fallback");
        assert_eq!(registry.get(MaterialId(1)).name, "stone");
        assert_eq!(registry.get(MaterialId(2)).name, "dirt");
    }

    #[test]
    fn test_atlas_uvs_are_valid() {
        let registry = create_test_registry();
        for id in 0..registry.len() as u16 {
            for face in [Face::Top, Face::Bottom, Face::North, Face::South, Face::East, Face::West] {
                let (uv_min, uv_max) = registry.atlas_uvs(MaterialId(id), face);
                assert!(uv_min.x >= 0.0 && uv_min.x <= 1.0,
                    "UV min x out of range for material {id}, face {face:?}");
                assert!(uv_min.y >= 0.0 && uv_min.y <= 1.0,
                    "UV min y out of range for material {id}, face {face:?}");
                assert!(uv_max.x >= 0.0 && uv_max.x <= 1.0,
                    "UV max x out of range for material {id}, face {face:?}");
                assert!(uv_max.y >= 0.0 && uv_max.y <= 1.0,
                    "UV max y out of range for material {id}, face {face:?}");
                assert!(uv_min.x < uv_max.x);
                assert!(uv_min.y < uv_max.y);
            }
        }
    }

    #[test]
    fn test_missing_material_returns_fallback() {
        let registry = create_test_registry();
        // Request an ID that does not exist
        let mat = registry.get(MaterialId(9999));
        assert_eq!(mat.name, "fallback");
        assert_eq!(mat.albedo, [1.0, 0.0, 1.0, 1.0]); // magenta
    }

    #[test]
    fn test_registry_is_read_only_at_runtime() {
        let registry = create_test_registry();
        // The registry exposes only &self methods — no &mut self methods exist
        // after construction. Verify immutability by confirming the public API
        // returns shared references only.
        let _ref: &MaterialDef = registry.get(MaterialId(1));
        let _uvs: (Vec2, Vec2) = registry.atlas_uvs(MaterialId(1), Face::Top);
        let _name: Option<MaterialId> = registry.lookup_by_name("stone");
        // No mutation methods are available — this is enforced at compile time.
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
        // The fallback should be a highly visible material (magenta)
        assert_eq!(fallback.albedo[0], 1.0); // R
        assert_eq!(fallback.albedo[1], 0.0); // G
        assert_eq!(fallback.albedo[2], 1.0); // B
    }
}
```
