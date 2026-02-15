# Emissive Materials

## Problem

Certain voxel types in the world — lava, glowing ore, lanterns, redstone-like circuits, bioluminescent plants — emit light. They need to glow visually on screen (producing HDR values that feed into the bloom post-process) and also serve as actual light sources in the voxel light propagation system (story 04). Without emissive materials, the only way to light a cave would be explicit point-light entities, creating a disconnect between what the player sees (a glowing block) and what the lighting system knows (nothing). The engine needs to unify the concept of "a block that glows" across both the material/rendering pipeline and the voxel light propagation system, storing emissive color and intensity as part of the material definition.

## Solution

### Material Extension

Extend the PBR material (story 05) with emissive properties:

```rust
/// Extended PBR material with emissive support.
#[derive(Clone, Debug)]
pub struct PbrMaterial {
    pub albedo: glam::Vec3,
    pub metallic: f32,
    pub roughness: f32,
    pub ao: f32,
    /// Emissive color in linear RGB. Zero for non-emissive materials.
    pub emissive_color: glam::Vec3,
    /// Emissive intensity multiplier. Values > 1.0 produce HDR output for bloom.
    pub emissive_intensity: f32,
}

impl PbrMaterial {
    pub fn is_emissive(&self) -> bool {
        self.emissive_intensity > 0.0 && self.emissive_color.length_squared() > 0.0
    }

    /// Total emissive contribution = color * intensity.
    pub fn emissive_output(&self) -> glam::Vec3 {
        self.emissive_color * self.emissive_intensity
    }
}
```

### Emissive Voxel Types in the Registry

The `VoxelTypeDef` (from `nebula_voxel` story 01) already has a `light_emission: u8` field (0-15). Emissive materials connect this to the rendering:

```rust
/// Emissive voxel type definitions.
pub fn register_emissive_types(registry: &mut VoxelTypeRegistry) {
    registry.register(VoxelTypeDef {
        name: "lava".into(),
        solid: true,
        transparency: Transparency::Opaque,
        material_index: 10,
        light_emission: 15, // maximum brightness
    }).unwrap();

    registry.register(VoxelTypeDef {
        name: "glowstone".into(),
        solid: true,
        transparency: Transparency::Opaque,
        material_index: 11,
        light_emission: 14,
    }).unwrap();

    registry.register(VoxelTypeDef {
        name: "lantern".into(),
        solid: true,
        transparency: Transparency::Opaque,
        material_index: 12,
        light_emission: 12,
    }).unwrap();
}
```

### GPU Material Buffer

The emissive properties are packed into the per-material GPU buffer:

```rust
/// GPU-side material data, 48 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PbrMaterialGpu {
    /// xyz = albedo, w = metallic.
    pub albedo_metallic: [f32; 4],
    /// x = roughness, y = ao, z = unused, w = unused.
    pub roughness_ao: [f32; 4],
    /// xyz = emissive_color * emissive_intensity (pre-multiplied), w = unused.
    pub emissive: [f32; 4],
}
```

### Fragment Shader: Emissive Addition

Emissive light is added after all PBR shading, so it appears as self-illumination regardless of incoming light:

```wgsl
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // ... PBR lighting calculations from story 05 ...
    var color = pbr_result + ambient;

    // Add emissive output. This can produce values > 1.0 (HDR).
    color += material.emissive.xyz;

    return vec4<f32>(color, 1.0);
}
```

Values exceeding 1.0 are intentional — the HDR render target preserves them, and the bloom post-process (a later story) detects bright fragments to create the glow effect.

### Bridge to Voxel Light Propagation

When a chunk is loaded or a voxel is placed, the engine queries the voxel type registry for `light_emission`. If non-zero, the voxel is registered as a light source for the flood-fill propagation (story 04):

```rust
pub fn collect_emissive_sources(
    voxels: &ChunkVoxels,
    registry: &VoxelTypeRegistry,
) -> Vec<(u32, u32, u32, u8)> {
    let mut sources = Vec::new();
    for x in 0..32 {
        for y in 0..32 {
            for z in 0..32 {
                let id = voxels.get(x, y, z);
                let def = registry.get(id);
                if def.light_emission > 0 {
                    sources.push((x, y, z, def.light_emission));
                }
            }
        }
    }
    sources
}
```

This bridges the material system (what the block looks like) with the light propagation system (how the block affects its surroundings).

## Outcome

`PbrMaterial` is extended with `emissive_color` and `emissive_intensity` fields. Emissive voxel types feed into both the fragment shader (HDR glow) and the voxel light propagation BFS (block lighting). The GPU material buffer includes pre-multiplied emissive output. Running `cargo test -p nebula_lighting` passes all emissive material tests. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Lava voxels deep in caves glow orange-red, casting light on surrounding surfaces. They are visible in total darkness without external illumination.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Material storage buffer |
| `bytemuck` | `1.21` | Pod/Zeroable for GPU material struct |
| `glam` | `0.29` | Vec3 for emissive color math |
| `nebula_voxel` | workspace | `VoxelTypeRegistry`, `VoxelTypeDef`, `ChunkVoxels` |

Depends on story 04 (voxel light propagation) and story 05 (PBR shading).

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn lava_material() -> PbrMaterial {
        PbrMaterial {
            albedo: glam::Vec3::new(0.8, 0.2, 0.0),
            metallic: 0.0,
            roughness: 0.9,
            ao: 1.0,
            emissive_color: glam::Vec3::new(1.0, 0.3, 0.0),
            emissive_intensity: 5.0,
        }
    }

    fn stone_material() -> PbrMaterial {
        PbrMaterial {
            albedo: glam::Vec3::new(0.5, 0.5, 0.5),
            metallic: 0.0,
            roughness: 0.8,
            ao: 1.0,
            emissive_color: glam::Vec3::ZERO,
            emissive_intensity: 0.0,
        }
    }

    #[test]
    fn test_emissive_material_outputs_hdr_values() {
        let mat = lava_material();
        let output = mat.emissive_output();
        // emissive_color * intensity = (1.0, 0.3, 0.0) * 5.0 = (5.0, 1.5, 0.0)
        assert!(output.x > 1.0, "emissive R ({}) should exceed 1.0 for HDR", output.x);
        assert!(output.y > 1.0, "emissive G ({}) should exceed 1.0 for HDR", output.y);
        assert!((output.x - 5.0).abs() < 1e-6);
        assert!((output.y - 1.5).abs() < 1e-6);
    }

    #[test]
    fn test_non_emissive_material_has_zero_emissive() {
        let mat = stone_material();
        let output = mat.emissive_output();
        assert_eq!(output, glam::Vec3::ZERO, "non-emissive material should output zero");
        assert!(!mat.is_emissive());
    }

    #[test]
    fn test_emissive_voxel_acts_as_light_source_in_propagation() {
        let mut registry = VoxelTypeRegistry::new();
        registry.register(VoxelTypeDef {
            name: "glowstone".into(),
            solid: true,
            transparency: Transparency::Opaque,
            material_index: 1,
            light_emission: 14,
        }).unwrap();

        let glow_id = registry.lookup_by_name("glowstone").unwrap();
        let mut voxels = ChunkVoxels::new_air();
        voxels.set(16, 16, 16, glow_id);

        let sources = collect_emissive_sources(&voxels, &registry);
        assert_eq!(sources.len(), 1, "should find exactly one emissive source");
        assert_eq!(sources[0], (16, 16, 16, 14));
    }

    #[test]
    fn test_emissive_color_matches_material_definition() {
        let mat = lava_material();
        let gpu = PbrMaterialGpu::from_material(&mat);
        // Pre-multiplied emissive in GPU struct should match emissive_output().
        let expected = mat.emissive_output();
        assert!((gpu.emissive[0] - expected.x).abs() < 1e-6);
        assert!((gpu.emissive[1] - expected.y).abs() < 1e-6);
        assert!((gpu.emissive[2] - expected.z).abs() < 1e-6);
    }

    #[test]
    fn test_bloom_responds_to_emissive_surfaces() {
        // Bloom threshold is typically 1.0. Emissive surfaces with output > 1.0
        // should pass the bloom threshold.
        let mat = lava_material();
        let output = mat.emissive_output();
        let bloom_threshold = 1.0;
        let max_channel = output.x.max(output.y).max(output.z);
        assert!(
            max_channel > bloom_threshold,
            "emissive output max channel ({max_channel}) should exceed bloom threshold ({bloom_threshold})"
        );

        // Non-emissive material should NOT trigger bloom from emissive alone.
        let stone_output = stone_material().emissive_output();
        let stone_max = stone_output.x.max(stone_output.y).max(stone_output.z);
        assert!(
            stone_max <= bloom_threshold,
            "non-emissive output ({stone_max}) should not exceed bloom threshold"
        );
    }

    #[test]
    fn test_gpu_material_struct_size() {
        assert_eq!(std::mem::size_of::<PbrMaterialGpu>(), 48);
    }
}
```
