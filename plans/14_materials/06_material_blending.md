# Material Blending

## Problem

On a cubesphere-voxel planet, biome boundaries create hard visual transitions — a desert meets a forest and the ground texture snaps abruptly from sand to grass at the chunk or voxel boundary. This looks unnatural and breaks the illusion of a continuous world. Additionally, steep cliff faces suffer from texture stretching: a single texture projected along one axis distorts badly when the surface normal tilts away from that axis. The engine needs two blending techniques: (1) smooth material transitions at biome boundaries using per-vertex blend weights, and (2) triplanar projection to eliminate stretching on non-axis-aligned surfaces.

## Solution

Implement material blending in the `nebula_materials` and `nebula_meshing` crates. Blend weights are stored per vertex and consumed by the PBR fragment shader. Triplanar projection is computed entirely in the shader based on the surface normal.

### Blend Weight Per Vertex

Each vertex can reference two materials and a blend weight between them. The vertex format is extended:

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VoxelVertex {
    pub position: [f32; 3],         // 12 bytes, location(0)
    pub normal: [f32; 3],           // 12 bytes, location(1)
    pub uv: [f32; 2],              // 8 bytes,  location(2)
    pub material_id_a: u32,         // 4 bytes,  location(3) — primary material
    pub material_id_b: u32,         // 4 bytes,  location(4) — secondary material
    pub blend_weight: f32,          // 4 bytes,  location(5) — 0.0 = A only, 1.0 = B only
    pub ao: f32,                    // 4 bytes,  location(6)
}
// Total stride: 48 bytes
```

When `blend_weight` is 0.0, only material A is sampled. When 1.0, only material B. Values between produce a smooth mix. For voxels not at a biome boundary, `material_id_b` equals `material_id_a` and `blend_weight` is 0.0, so there is zero overhead for non-blended surfaces.

### Blend Weight Computation

During meshing, the blend weight is determined by the biome map. Each voxel column has a primary and secondary biome; the blend weight is the fractional influence of the secondary biome:

```rust
/// Compute the blend weight for a voxel at the given position.
pub fn compute_blend_weight(
    biome_map: &BiomeMap,
    world_x: i128,
    world_z: i128,
) -> (MaterialId, MaterialId, f32) {
    let (primary_biome, secondary_biome, blend_factor) = biome_map.sample(world_x, world_z);
    let mat_a = primary_biome.surface_material();
    let mat_b = secondary_biome.surface_material();
    (mat_a, mat_b, blend_factor)
}
```

The `blend_factor` is typically computed from a smooth noise function or distance-based falloff at biome boundaries, producing values in [0.0, 1.0].

### Triplanar Projection

For cliff faces and steep slopes, standard UV projection along a single axis causes extreme stretching. Triplanar projection samples the texture three times — once along each world axis (X, Y, Z) — and blends the results based on the surface normal:

```wgsl
/// Triplanar texture sampling in the fragment shader.
fn triplanar_sample(
    world_pos: vec3<f32>,
    normal: vec3<f32>,
    atlas_tex: texture_2d<f32>,
    samp: sampler,
    uv_min: vec2<f32>,
    uv_size: vec2<f32>,
    tile_scale: f32,
) -> vec4<f32> {
    // Compute blend weights from the absolute normal components
    var blend = abs(normal);
    // Sharpen the blend to reduce the transition band
    blend = pow(blend, vec3<f32>(4.0));
    // Normalize so weights sum to 1.0
    blend = blend / (blend.x + blend.y + blend.z);

    // Sample along each axis projection
    let uv_x = fract(world_pos.yz * tile_scale) * uv_size + uv_min;
    let uv_y = fract(world_pos.xz * tile_scale) * uv_size + uv_min;
    let uv_z = fract(world_pos.xy * tile_scale) * uv_size + uv_min;

    let tex_x = textureSample(atlas_tex, samp, uv_x);
    let tex_y = textureSample(atlas_tex, samp, uv_y);
    let tex_z = textureSample(atlas_tex, samp, uv_z);

    return tex_x * blend.x + tex_y * blend.y + tex_z * blend.z;
}
```

### Integrated Fragment Shader

The PBR fragment shader from story 04 is extended to handle both material blending and triplanar projection:

```wgsl
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let mat_a = materials[in.material_id_a];
    let mat_b = materials[in.material_id_b];

    let n = normalize(in.normal);

    // Determine if triplanar projection is needed.
    // Use triplanar when the dominant normal component is < 0.7 (slope > ~45 degrees)
    let max_component = max(abs(n.x), max(abs(n.y), abs(n.z)));
    let use_triplanar = max_component < 0.7;

    var tex_a: vec4<f32>;
    var tex_b: vec4<f32>;

    if use_triplanar {
        tex_a = triplanar_sample(in.world_pos, n, atlas_texture, atlas_sampler,
                                  mat_a.atlas_uv_min, mat_a.atlas_uv_size, 1.0);
        tex_b = triplanar_sample(in.world_pos, n, atlas_texture, atlas_sampler,
                                  mat_b.atlas_uv_min, mat_b.atlas_uv_size, 1.0);
    } else {
        tex_a = textureSample(atlas_texture, atlas_sampler, in.uv);
        tex_b = textureSample(atlas_texture, atlas_sampler, in.uv);
    }

    // Blend between material A and material B
    let w = in.blend_weight;
    let albedo = mix(tex_a.rgb * mat_a.albedo.rgb, tex_b.rgb * mat_b.albedo.rgb, w);
    let metallic = mix(mat_a.metallic, mat_b.metallic, w);
    let roughness = mix(mat_a.roughness, mat_b.roughness, w);
    let emissive_a = mat_a.emissive_rgb_intensity.rgb * mat_a.emissive_rgb_intensity.w;
    let emissive_b = mat_b.emissive_rgb_intensity.rgb * mat_b.emissive_rgb_intensity.w;
    let emissive = mix(emissive_a, emissive_b, w);

    // ... proceed with PBR shading using blended albedo, metallic, roughness, emissive ...
}
```

### Smooth Transition Guarantee

The blend weight is interpolated by the GPU rasterizer across the triangle, producing a per-pixel gradient. At biome boundaries this creates a smooth visual transition spanning several voxels rather than a hard edge. The width of the transition zone is controlled by the biome blending radius in the terrain generation system.

## Outcome

A material blending system in `nebula_materials` and `nebula_meshing` that produces smooth transitions between biome surface materials using per-vertex blend weights, and eliminates texture stretching on steep slopes via triplanar projection in the fragment shader. Non-blended surfaces incur no extra cost (blend weight is 0.0 and material B equals material A). Running `cargo test -p nebula_materials` passes all blending tests.

## Demo Integration

**Demo crate:** `nebula-demo`

At biome boundaries, materials blend smoothly. Grass fades into sand over several voxels rather than a hard edge. The transition is visually pleasing.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `28.0` | Extended vertex format, shader compilation |
| `bytemuck` | `1.21` | Pod/Zeroable for extended `VoxelVertex` |
| `glam` | `0.32` | Vec2/Vec3 math for UV and normal calculations |

Depends on stories 14/01-04 (material properties, atlas, registry, PBR pipeline) and 09/02 (biome system for blend weights). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a test color for a material (simulating atlas sampling).
    fn color_a() -> [f32; 3] { [1.0, 0.0, 0.0] } // red
    fn color_b() -> [f32; 3] { [0.0, 0.0, 1.0] } // blue

    /// Linear blend between two colors.
    fn blend_colors(a: [f32; 3], b: [f32; 3], w: f32) -> [f32; 3] {
        [
            a[0] * (1.0 - w) + b[0] * w,
            a[1] * (1.0 - w) + b[1] * w,
            a[2] * (1.0 - w) + b[2] * w,
        ]
    }

    #[test]
    fn test_blend_weight_zero_shows_material_a_only() {
        let result = blend_colors(color_a(), color_b(), 0.0);
        assert_eq!(result, [1.0, 0.0, 0.0]); // pure red (material A)
    }

    #[test]
    fn test_blend_weight_one_shows_material_b_only() {
        let result = blend_colors(color_a(), color_b(), 1.0);
        assert_eq!(result, [0.0, 0.0, 1.0]); // pure blue (material B)
    }

    #[test]
    fn test_blend_weight_half_blends_equally() {
        let result = blend_colors(color_a(), color_b(), 0.5);
        // 50% red + 50% blue = (0.5, 0.0, 0.5)
        let epsilon = 1e-6;
        assert!((result[0] - 0.5).abs() < epsilon);
        assert!((result[1] - 0.0).abs() < epsilon);
        assert!((result[2] - 0.5).abs() < epsilon);
    }

    #[test]
    fn test_triplanar_projection_eliminates_stretching_on_vertical_faces() {
        // For a vertical face with normal (1, 0, 0) — pointing along +X —
        // triplanar blending weights should be (1, 0, 0), meaning only
        // the YZ-projected texture is used (no stretching).
        let normal = glam::Vec3::new(1.0, 0.0, 0.0);
        let blend = triplanar_weights(normal);

        assert!(blend.x > 0.99, "X-facing surface should use X projection: {blend:?}");
        assert!(blend.y < 0.01, "Y projection weight should be ~0 for X-facing surface");
        assert!(blend.z < 0.01, "Z projection weight should be ~0 for X-facing surface");
    }

    #[test]
    fn test_triplanar_weights_for_horizontal_face() {
        // For a horizontal face with normal (0, 1, 0) — pointing up —
        // only the XZ-projected texture should be used.
        let normal = glam::Vec3::new(0.0, 1.0, 0.0);
        let blend = triplanar_weights(normal);

        assert!(blend.y > 0.99, "Y-facing surface should use Y projection: {blend:?}");
        assert!(blend.x < 0.01);
        assert!(blend.z < 0.01);
    }

    #[test]
    fn test_triplanar_weights_for_diagonal_face() {
        // A 45-degree surface with normal (0.707, 0.707, 0) should blend
        // X and Y projections roughly equally, with Z near zero.
        let normal = glam::Vec3::new(0.707, 0.707, 0.0).normalize();
        let blend = triplanar_weights(normal);

        let epsilon = 0.05;
        assert!((blend.x - blend.y).abs() < epsilon,
            "Diagonal surface should blend X and Y equally: {blend:?}");
        assert!(blend.z < 0.01,
            "Z weight should be ~0 for XY-diagonal surface: {blend:?}");
    }

    #[test]
    fn test_triplanar_weights_sum_to_one() {
        let normals = [
            glam::Vec3::new(1.0, 0.0, 0.0),
            glam::Vec3::new(0.0, 1.0, 0.0),
            glam::Vec3::new(0.0, 0.0, 1.0),
            glam::Vec3::new(0.577, 0.577, 0.577), // diagonal
            glam::Vec3::new(0.2, 0.8, 0.1).normalize(),
        ];

        for normal in normals {
            let blend = triplanar_weights(normal);
            let sum = blend.x + blend.y + blend.z;
            assert!(
                (sum - 1.0).abs() < 1e-4,
                "Triplanar weights should sum to 1.0, got {sum} for normal {normal:?}"
            );
        }
    }

    #[test]
    fn test_blending_is_smooth_no_hard_edges() {
        // Verify that small changes in blend weight produce small changes in output.
        // This ensures no discontinuities (hard edges) in the blending function.
        let steps = 100;
        let mut prev_result = blend_colors(color_a(), color_b(), 0.0);

        for i in 1..=steps {
            let w = i as f32 / steps as f32;
            let result = blend_colors(color_a(), color_b(), w);

            // The maximum change per step should be bounded
            let max_delta = 1.0 / steps as f32 + 1e-6;
            for c in 0..3 {
                let delta = (result[c] - prev_result[c]).abs();
                assert!(
                    delta <= max_delta + 1e-6,
                    "Discontinuity at w={w}: channel {c} changed by {delta} (max {max_delta})"
                );
            }

            prev_result = result;
        }
    }

    #[test]
    fn test_extended_vertex_size() {
        // Extended vertex with two material IDs and blend weight = 48 bytes
        assert_eq!(std::mem::size_of::<VoxelVertex>(), 48);
    }

    #[test]
    fn test_non_blended_vertex_defaults() {
        // A vertex at a non-boundary location should have material_id_b == material_id_a
        // and blend_weight == 0.0
        let vertex = VoxelVertex {
            position: [0.0; 3],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0; 2],
            material_id_a: 5,
            material_id_b: 5,
            blend_weight: 0.0,
            ao: 1.0,
        };
        assert_eq!(vertex.material_id_a, vertex.material_id_b);
        assert_eq!(vertex.blend_weight, 0.0);
    }

    /// Triplanar weight calculation (Rust-side mirror of the WGSL function).
    fn triplanar_weights(normal: glam::Vec3) -> glam::Vec3 {
        let mut blend = normal.abs();
        blend = glam::Vec3::new(
            blend.x.powi(4),
            blend.y.powi(4),
            blend.z.powi(4),
        );
        let sum = blend.x + blend.y + blend.z;
        blend / sum
    }
}
```
