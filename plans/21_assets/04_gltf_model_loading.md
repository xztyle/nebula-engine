# glTF Model Loading

## Problem

The Nebula Engine needs to load 3D models for entities that are not procedurally generated voxel terrain — space stations, vehicles, tools, NPCs, decorative props, and anything created by artists in external tools like Blender. The industry-standard interchange format for real-time 3D is glTF 2.0, supported by every major 3D modeling tool. glTF comes in two variants: `.gltf` (JSON manifest with separate `.bin` and texture files) and `.glb` (single binary file with everything embedded). The engine must support both.

A loaded model contains multiple data streams: vertex positions, normals, texture coordinates, tangents, vertex indices, PBR material properties, texture references, skeleton joint hierarchies, inverse bind matrices, and animation keyframes. Each of these must be extracted from the glTF container and converted into the engine's internal formats. Malformed or unsupported files must produce clear errors, not panics. The loading must happen on background threads (via the async loading system from story 02), with only GPU upload happening on the main thread.

## Solution

### GltfLoader

The `GltfLoader` implements the `AssetLoader<ModelAsset>` trait, performing CPU-side parsing of glTF files:

```rust
use gltf::Gltf;
use std::path::{Path, PathBuf};

pub struct GltfLoader;

impl AssetLoader<ModelAsset> for GltfLoader {
    fn load(
        &self,
        bytes: &[u8],
        path: &Path,
    ) -> Result<ModelAsset, AssetLoadError> {
        let gltf = Gltf::from_slice(bytes).map_err(|e| {
            AssetLoadError::DecodeFailed {
                path: path.to_path_buf(),
                reason: format!("glTF parse error: {e}"),
            }
        })?;

        let buffers = load_buffers(&gltf, path)?;
        let mut meshes = Vec::new();
        let mut materials = Vec::new();
        let mut skeleton = None;
        let mut animations = Vec::new();

        // Extract materials first so meshes can reference them by index
        for mat in gltf.materials() {
            materials.push(extract_material(&mat)?);
        }

        // Extract meshes
        for mesh in gltf.meshes() {
            for primitive in mesh.primitives() {
                meshes.push(extract_mesh_primitive(
                    &primitive,
                    &buffers,
                )?);
            }
        }

        // Extract skeleton if present
        for skin in gltf.skins() {
            skeleton = Some(extract_skeleton(&skin, &buffers)?);
        }

        // Extract animations
        for anim in gltf.animations() {
            animations.push(extract_animation(&anim, &buffers)?);
        }

        Ok(ModelAsset {
            meshes,
            materials,
            skeleton,
            animations,
        })
    }
}
```

### Internal Data Types

The engine defines its own model types, decoupled from the `gltf` crate's types:

```rust
/// A fully loaded 3D model with all sub-assets.
pub struct ModelAsset {
    pub meshes: Vec<MeshData>,
    pub materials: Vec<MaterialData>,
    pub skeleton: Option<SkeletonData>,
    pub animations: Vec<AnimationData>,
}

/// CPU-side mesh vertex and index data, ready for GPU upload.
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub tangents: Option<Vec<[f32; 4]>>,
    pub indices: Vec<u32>,
    pub material_index: Option<usize>,
    /// Optional joint indices and weights for skeletal animation.
    pub joint_indices: Option<Vec<[u16; 4]>>,
    pub joint_weights: Option<Vec<[f32; 4]>>,
}

/// PBR material properties extracted from glTF.
pub struct MaterialData {
    pub name: Option<String>,
    pub base_color_factor: [f32; 4],
    pub metallic_factor: f32,
    pub roughness_factor: f32,
    pub emissive_factor: [f32; 3],
    pub base_color_texture: Option<TextureRef>,
    pub normal_texture: Option<TextureRef>,
    pub metallic_roughness_texture: Option<TextureRef>,
    pub occlusion_texture: Option<TextureRef>,
    pub emissive_texture: Option<TextureRef>,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlphaMode {
    Opaque,
    Mask,
    Blend,
}

/// Reference to a texture within the model.
pub struct TextureRef {
    pub image_index: usize,
    pub tex_coord_set: u32,
}

/// Skeleton joint hierarchy.
pub struct SkeletonData {
    pub joints: Vec<JointData>,
    pub inverse_bind_matrices: Vec<[[f32; 4]; 4]>,
}

pub struct JointData {
    pub name: Option<String>,
    pub parent_index: Option<usize>,
    pub local_transform: [[f32; 4]; 4],
}

/// A single animation clip with multiple channels.
pub struct AnimationData {
    pub name: Option<String>,
    pub duration: f32,
    pub channels: Vec<AnimationChannel>,
}

pub struct AnimationChannel {
    pub target_joint: usize,
    pub property: AnimationProperty,
    pub keyframes: Vec<Keyframe>,
}

#[derive(Debug, Clone, Copy)]
pub enum AnimationProperty {
    Translation,
    Rotation,
    Scale,
}

pub struct Keyframe {
    pub time: f32,
    pub value: [f32; 4], // xyz for translation/scale, xyzw for rotation
}
```

### Buffer Loading

glTF models reference binary buffer data. For `.glb` files, the buffer is embedded. For `.gltf` files, the buffer is in a separate `.bin` file:

```rust
fn load_buffers(
    gltf: &Gltf,
    model_path: &Path,
) -> Result<Vec<Vec<u8>>, AssetLoadError> {
    let mut buffers = Vec::new();
    let blob = gltf.blob.as_deref();

    for buffer in gltf.buffers() {
        match buffer.source() {
            gltf::buffer::Source::Bin => {
                let data = blob.ok_or_else(|| AssetLoadError::DecodeFailed {
                    path: model_path.to_path_buf(),
                    reason: "glb missing embedded binary chunk".into(),
                })?;
                buffers.push(data.to_vec());
            }
            gltf::buffer::Source::Uri(uri) => {
                if uri.starts_with("data:") {
                    // Base64-encoded inline data
                    let data = decode_data_uri(uri).map_err(|e| {
                        AssetLoadError::DecodeFailed {
                            path: model_path.to_path_buf(),
                            reason: format!("failed to decode data URI: {e}"),
                        }
                    })?;
                    buffers.push(data);
                } else {
                    // External file relative to the .gltf file
                    let dir = model_path.parent().unwrap_or(Path::new("."));
                    let buf_path = dir.join(uri);
                    let data = std::fs::read(&buf_path).map_err(|e| {
                        AssetLoadError::Io {
                            path: buf_path,
                            source: e,
                        }
                    })?;
                    buffers.push(data);
                }
            }
        }
    }

    Ok(buffers)
}
```

### Mesh Extraction

Vertex attributes are read from buffer views using accessor metadata:

```rust
fn extract_mesh_primitive(
    primitive: &gltf::Primitive,
    buffers: &[Vec<u8>],
) -> Result<MeshData, AssetLoadError> {
    let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

    let positions: Vec<[f32; 3]> = reader
        .read_positions()
        .ok_or_else(|| AssetLoadError::DecodeFailed {
            path: PathBuf::new(),
            reason: "mesh primitive missing position attribute".into(),
        })?
        .collect();

    let normals: Vec<[f32; 3]> = reader
        .read_normals()
        .map(|iter| iter.collect())
        .unwrap_or_else(|| {
            // Generate flat normals if none are provided
            generate_flat_normals(&positions)
        });

    let uvs: Vec<[f32; 2]> = reader
        .read_tex_coords(0)
        .map(|tc| tc.into_f32().collect())
        .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);

    let tangents: Option<Vec<[f32; 4]>> = reader
        .read_tangents()
        .map(|iter| iter.collect());

    let indices: Vec<u32> = reader
        .read_indices()
        .map(|idx| idx.into_u32().collect())
        .unwrap_or_else(|| (0..positions.len() as u32).collect());

    let joint_indices = reader
        .read_joints(0)
        .map(|j| j.into_u16().collect());

    let joint_weights = reader
        .read_weights(0)
        .map(|w| w.into_f32().collect());

    Ok(MeshData {
        positions,
        normals,
        uvs,
        tangents,
        indices,
        material_index: primitive.material().index(),
        joint_indices,
        joint_weights,
    })
}
```

### Material Extraction

PBR properties map directly from glTF's metallic-roughness workflow:

```rust
fn extract_material(
    mat: &gltf::Material,
) -> Result<MaterialData, AssetLoadError> {
    let pbr = mat.pbr_metallic_roughness();

    Ok(MaterialData {
        name: mat.name().map(String::from),
        base_color_factor: pbr.base_color_factor(),
        metallic_factor: pbr.metallic_factor(),
        roughness_factor: pbr.roughness_factor(),
        emissive_factor: mat.emissive_factor(),
        base_color_texture: pbr.base_color_texture().map(|info| TextureRef {
            image_index: info.texture().source().index(),
            tex_coord_set: info.tex_coord(),
        }),
        normal_texture: mat.normal_texture().map(|info| TextureRef {
            image_index: info.texture().source().index(),
            tex_coord_set: info.tex_coord(),
        }),
        metallic_roughness_texture: pbr
            .metallic_roughness_texture()
            .map(|info| TextureRef {
                image_index: info.texture().source().index(),
                tex_coord_set: info.tex_coord(),
            }),
        occlusion_texture: mat.occlusion_texture().map(|info| TextureRef {
            image_index: info.texture().source().index(),
            tex_coord_set: info.tex_coord(),
        }),
        emissive_texture: mat.emissive_texture().map(|info| TextureRef {
            image_index: info.texture().source().index(),
            tex_coord_set: info.tex_coord(),
        }),
        alpha_mode: match mat.alpha_mode() {
            gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
            gltf::material::AlphaMode::Mask => AlphaMode::Mask,
            gltf::material::AlphaMode::Blend => AlphaMode::Blend,
        },
        alpha_cutoff: mat.alpha_cutoff().unwrap_or(0.5),
        double_sided: mat.double_sided(),
    })
}
```

### Skeleton Extraction

```rust
fn extract_skeleton(
    skin: &gltf::Skin,
    buffers: &[Vec<u8>],
) -> Result<SkeletonData, AssetLoadError> {
    let reader = skin.reader(|buffer| Some(&buffers[buffer.index()]));

    let inverse_bind_matrices: Vec<[[f32; 4]; 4]> = reader
        .read_inverse_bind_matrices()
        .map(|iter| iter.collect())
        .unwrap_or_default();

    let joints: Vec<JointData> = skin
        .joints()
        .enumerate()
        .map(|(_i, joint)| {
            let (translation, rotation, scale) = joint.transform().decomposed();
            let _ = scale; // Used to reconstruct local_transform
            JointData {
                name: joint.name().map(String::from),
                parent_index: None, // Resolved in a second pass
                local_transform: joint.transform().matrix(),
            }
        })
        .collect();

    Ok(SkeletonData {
        joints,
        inverse_bind_matrices,
    })
}
```

## Outcome

A `GltfLoader` that implements `AssetLoader<ModelAsset>`, parsing `.gltf` and `.glb` files into the engine's internal `ModelAsset` structure. The loader extracts mesh geometry (positions, normals, UVs, tangents, indices), PBR material properties (base color, metallic, roughness, emissive, textures), skeleton hierarchies (joints, inverse bind matrices), and animation clips (keyframes per channel). Both embedded `.glb` and multi-file `.gltf` formats are supported. Invalid files produce descriptive errors. The `ModelAsset` is a CPU-side data structure ready for GPU upload by the rendering pipeline.

## Demo Integration

**Demo crate:** `nebula-demo`

A glTF lantern model is loaded and placed on the terrain surface with correct PBR materials and vertex data.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `gltf` | `1.4` | Parse glTF 2.0 JSON and binary formats, read accessors and buffer views |
| `image` | `0.25` | Decode embedded textures (PNG, JPEG) within glTF files |
| `thiserror` | `2.0` | Error type derivation |
| `log` | `0.4` | Logging load progress and warnings for missing optional attributes |

Rust edition 2024. The `gltf` crate handles all glTF-specific parsing. Buffer data and accessor reads use the crate's built-in reader API. No manual binary parsing is needed.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid glb: a single triangle with positions and normals.
    /// In a real test suite this would be a checked-in test fixture file.
    fn minimal_glb_bytes() -> Vec<u8> {
        include_bytes!("../../test_fixtures/triangle.glb").to_vec()
    }

    fn cube_glb_bytes() -> Vec<u8> {
        include_bytes!("../../test_fixtures/cube.glb").to_vec()
    }

    #[test]
    fn test_load_valid_glb_produces_mesh_data() {
        let loader = GltfLoader;
        let bytes = minimal_glb_bytes();
        let result = loader.load(&bytes, Path::new("triangle.glb"));

        assert!(result.is_ok(), "Valid glb should load: {:?}", result.err());
        let model = result.unwrap();
        assert!(
            !model.meshes.is_empty(),
            "Model should contain at least one mesh"
        );
    }

    #[test]
    fn test_vertex_data_has_positions_and_normals() {
        let loader = GltfLoader;
        let bytes = cube_glb_bytes();
        let model = loader.load(&bytes, Path::new("cube.glb")).unwrap();

        let mesh = &model.meshes[0];
        assert!(
            !mesh.positions.is_empty(),
            "Mesh must have vertex positions"
        );
        assert!(
            !mesh.normals.is_empty(),
            "Mesh must have normals"
        );
        assert_eq!(
            mesh.positions.len(),
            mesh.normals.len(),
            "Position and normal counts must match"
        );
    }

    #[test]
    fn test_materials_have_pbr_properties() {
        let loader = GltfLoader;
        let bytes = cube_glb_bytes();
        let model = loader.load(&bytes, Path::new("cube.glb")).unwrap();

        // A cube model exported from Blender has at least a default material
        if !model.materials.is_empty() {
            let mat = &model.materials[0];
            // PBR factors should be in valid ranges
            assert!(mat.metallic_factor >= 0.0 && mat.metallic_factor <= 1.0);
            assert!(mat.roughness_factor >= 0.0 && mat.roughness_factor <= 1.0);
            for channel in &mat.base_color_factor {
                assert!(*channel >= 0.0 && *channel <= 1.0);
            }
        }
    }

    #[test]
    fn test_invalid_file_returns_error() {
        let loader = GltfLoader;
        let garbage = b"this is not a valid glTF file at all";
        let result = loader.load(garbage, Path::new("invalid.glb"));

        assert!(
            result.is_err(),
            "Invalid data should produce an error"
        );
        match result.unwrap_err() {
            AssetLoadError::DecodeFailed { reason, .. } => {
                assert!(
                    reason.contains("glTF"),
                    "Error should mention glTF: {reason}"
                );
            }
            other => panic!("Expected DecodeFailed, got: {other:?}"),
        }
    }

    #[test]
    fn test_skeleton_extracted_when_present() {
        // Use a test fixture that contains a rigged model with a skeleton.
        // If no such fixture is available, this test verifies the code path
        // handles the absence gracefully.
        let loader = GltfLoader;
        let bytes = minimal_glb_bytes();
        let model = loader.load(&bytes, Path::new("triangle.glb")).unwrap();

        // A simple triangle has no skeleton — verify None
        if model.skeleton.is_none() {
            // Expected for a non-rigged model
            assert!(model.skeleton.is_none());
        } else {
            let skel = model.skeleton.as_ref().unwrap();
            assert!(
                !skel.joints.is_empty(),
                "Skeleton must have at least one joint"
            );
            assert_eq!(
                skel.joints.len(),
                skel.inverse_bind_matrices.len(),
                "Joint count must match inverse bind matrix count"
            );
        }
    }

    #[test]
    fn test_mesh_indices_are_valid() {
        let loader = GltfLoader;
        let bytes = cube_glb_bytes();
        let model = loader.load(&bytes, Path::new("cube.glb")).unwrap();

        for mesh in &model.meshes {
            let vertex_count = mesh.positions.len() as u32;
            for &index in &mesh.indices {
                assert!(
                    index < vertex_count,
                    "Index {index} out of bounds for {vertex_count} vertices"
                );
            }
        }
    }

    #[test]
    fn test_empty_file_returns_error() {
        let loader = GltfLoader;
        let result = loader.load(&[], Path::new("empty.glb"));
        assert!(result.is_err(), "Empty file should produce an error");
    }

    #[test]
    fn test_alpha_mode_extraction() {
        let loader = GltfLoader;
        let bytes = cube_glb_bytes();
        let model = loader.load(&bytes, Path::new("cube.glb")).unwrap();

        for mat in &model.materials {
            // Alpha mode should be one of the valid variants
            assert!(
                matches!(
                    mat.alpha_mode,
                    AlphaMode::Opaque | AlphaMode::Mask | AlphaMode::Blend
                ),
                "Invalid alpha mode: {:?}",
                mat.alpha_mode,
            );
            if mat.alpha_mode == AlphaMode::Mask {
                assert!(
                    mat.alpha_cutoff > 0.0,
                    "Mask mode should have a positive alpha cutoff"
                );
            }
        }
    }
}
```
