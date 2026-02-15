# Cascaded Shadow Maps

## Problem

Without shadows, a directional light produces uniform illumination on all surfaces facing the sun, regardless of whether other geometry is between them and the light. This makes the world look unconvincing — buildings do not cast shadows, caves have fully-lit interiors from the entrance, and terrain features lose all depth. The engine needs shadow mapping for the primary directional light (the sun). A single shadow map cannot cover the enormous range of distances needed in a voxel engine — nearby voxels need sub-meter shadow resolution, while terrain kilometers away still needs shadows to look correct. Cascaded Shadow Maps (CSM) solve this by splitting the view frustum into multiple depth slices, each with its own shadow map covering a progressively larger area at lower resolution.

## Solution

### Cascade Configuration

Use 4 cascades with configurable far-plane boundaries:

```rust
pub struct CascadedShadowConfig {
    /// Number of cascades (1-4). Default: 4.
    pub cascade_count: u32,
    /// Far distance of each cascade in meters.
    /// cascade_far[0] < cascade_far[1] < ... < cascade_far[cascade_count-1].
    pub cascade_far: [f32; 4],
    /// Shadow map resolution (width = height) per cascade. Default: 2048.
    pub resolution: u32,
    /// Depth bias to mitigate shadow acne (constant term).
    pub depth_bias_constant: f32,
    /// Depth bias slope factor.
    pub depth_bias_slope: f32,
    /// Normal offset bias in texels.
    pub normal_bias_texels: f32,
}

impl Default for CascadedShadowConfig {
    fn default() -> Self {
        Self {
            cascade_count: 4,
            cascade_far: [32.0, 128.0, 512.0, 2048.0],
            resolution: 2048,
            depth_bias_constant: 1.25,
            depth_bias_slope: 1.75,
            normal_bias_texels: 0.5,
        }
    }
}
```

### Shadow Map Textures

Create a 2D texture array with `cascade_count` layers, each `resolution x resolution`, using `TextureFormat::Depth32Float`:

```rust
pub struct CascadedShadowMaps {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    /// One view per cascade layer for rendering into.
    pub cascade_views: Vec<wgpu::TextureView>,
    pub sampler: wgpu::Sampler,
    /// Light-space view-projection matrix per cascade.
    pub light_matrices: [glam::Mat4; 4],
    pub config: CascadedShadowConfig,
}
```

The sampler uses `CompareFunction::GreaterEqual` (reverse-Z) for hardware-accelerated percentage-closer filtering.

### Light-Space Matrices

For each cascade, compute a tight orthographic projection from the sun's perspective that encloses the corresponding frustum slice:

```rust
pub fn compute_cascade_matrix(
    light_dir: glam::Vec3,
    camera_view_proj_inv: glam::Mat4,
    near: f32,
    far: f32,
) -> glam::Mat4 {
    // 1. Compute the 8 corners of the frustum slice in world space.
    // 2. Transform corners into light space (view from light direction).
    // 3. Compute the AABB of the corners in light space.
    // 4. Build an orthographic projection that encloses the AABB.
    // 5. Snap to texel grid to prevent shadow swimming on camera movement.
    // ...
}
```

Texel snapping: round the orthographic projection's min/max to the nearest shadow-map texel boundary, so small camera movements do not cause shadow edges to shimmer.

### Shadow Render Pass

For each cascade, render the scene from the light's perspective using a depth-only pipeline (no fragment output, no color attachment):

```rust
pub fn render_shadow_cascades(
    encoder: &mut wgpu::CommandEncoder,
    shadow_maps: &CascadedShadowMaps,
    shadow_pipeline: &wgpu::RenderPipeline,
    scene_meshes: &[MeshDrawCall],
) {
    for i in 0..shadow_maps.config.cascade_count as usize {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(&format!("shadow-cascade-{i}")),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &shadow_maps.cascade_views[i],
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0), // reverse-Z: clear to 0
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
        // Bind light matrix for cascade i, draw scene geometry.
    }
}
```

### Fragment Shader Sampling

In the main fragment shader, determine which cascade to sample based on fragment depth:

```wgsl
struct ShadowUniforms {
    light_matrices: array<mat4x4<f32>, 4>,
    cascade_far: vec4<f32>,
    cascade_count: u32,
};

fn shadow_factor(world_pos: vec3<f32>, view_depth: f32) -> f32 {
    // Select cascade based on view-space depth.
    var cascade_idx = 0u;
    for (var i = 0u; i < shadow_uniforms.cascade_count; i++) {
        if view_depth < shadow_uniforms.cascade_far[i] {
            cascade_idx = i;
            break;
        }
    }

    // Project into light space for the selected cascade.
    let light_pos = shadow_uniforms.light_matrices[cascade_idx] * vec4<f32>(world_pos, 1.0);
    let shadow_coord = light_pos.xyz / light_pos.w;
    let uv = shadow_coord.xy * 0.5 + 0.5;

    // Sample shadow map with hardware PCF via comparison sampler.
    let shadow = textureSampleCompare(
        shadow_map_texture,
        shadow_sampler,
        uv,
        cascade_idx,      // array layer
        shadow_coord.z,   // depth comparison
    );

    return shadow;
}
```

### Cascade Blending

At cascade boundaries, blend between adjacent cascades over a small depth range (5% of the cascade's depth range) to avoid hard transitions:

```wgsl
fn blended_shadow_factor(world_pos: vec3<f32>, view_depth: f32) -> f32 {
    let s1 = shadow_for_cascade(world_pos, cascade_idx);
    let blend_start = cascade_far[cascade_idx] * 0.95;
    if view_depth > blend_start && cascade_idx + 1 < cascade_count {
        let s2 = shadow_for_cascade(world_pos, cascade_idx + 1);
        let t = (view_depth - blend_start) / (cascade_far[cascade_idx] - blend_start);
        return mix(s1, s2, t);
    }
    return s1;
}
```

## Outcome

A `CascadedShadowMaps` system in `nebula_lighting` that renders 4 shadow map cascades covering 32m, 128m, 512m, and 2048m from the camera. The fragment shader samples the correct cascade with smooth blending at boundaries. Shadow acne is mitigated with configurable depth and normal biases. Running `cargo test -p nebula_lighting` passes all CSM tests. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The directional sun now casts shadows. Mountains shadow the valleys behind them. Four shadow map cascades provide sharp shadows near and acceptable quality far.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Depth texture array, comparison sampler, depth-only render pass |
| `bytemuck` | `1.21` | Pod/Zeroable for shadow uniform buffer |
| `glam` | `0.29` | Mat4 orthographic projection, frustum corner computation |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shadow_maps_correct_resolution() {
        let config = CascadedShadowConfig::default();
        let maps = CascadedShadowMaps::new_test(&config);
        assert_eq!(maps.texture.width(), config.resolution);
        assert_eq!(maps.texture.height(), config.resolution);
        assert_eq!(maps.cascade_views.len(), config.cascade_count as usize);
    }

    #[test]
    fn test_cascade_boundaries_cover_view_frustum() {
        let config = CascadedShadowConfig::default();
        // Cascade 0 starts at camera near plane.
        assert!(config.cascade_far[0] > 0.0, "first cascade must cover near range");
        // Each subsequent cascade must extend farther.
        for i in 1..config.cascade_count as usize {
            assert!(
                config.cascade_far[i] > config.cascade_far[i - 1],
                "cascade {i} must be farther than cascade {}",
                i - 1
            );
        }
        // Last cascade should cover a substantial distance.
        assert!(
            config.cascade_far[config.cascade_count as usize - 1] >= 1000.0,
            "last cascade should cover at least 1000m"
        );
    }

    #[test]
    fn test_shadow_acne_bias_is_positive() {
        let config = CascadedShadowConfig::default();
        assert!(
            config.depth_bias_constant > 0.0,
            "depth bias constant must be positive to mitigate shadow acne"
        );
        assert!(
            config.depth_bias_slope > 0.0,
            "depth bias slope must be positive"
        );
    }

    #[test]
    fn test_peter_panning_bias_is_bounded() {
        // Bias values should be small enough to avoid visible peter-panning
        // (shadows detaching from the base of objects).
        let config = CascadedShadowConfig::default();
        assert!(
            config.depth_bias_constant < 10.0,
            "depth bias constant too large, will cause peter-panning"
        );
        assert!(
            config.normal_bias_texels < 4.0,
            "normal bias too large, will cause peter-panning"
        );
    }

    #[test]
    fn test_cascade_blending_overlap() {
        // Verify that the blending zone covers the expected range.
        let config = CascadedShadowConfig::default();
        let blend_fraction = 0.05; // 5% of cascade depth
        for i in 0..config.cascade_count as usize - 1 {
            let blend_start = config.cascade_far[i] * (1.0 - blend_fraction);
            let blend_end = config.cascade_far[i];
            assert!(
                blend_end > blend_start,
                "blend zone for cascade {i} must have positive width"
            );
            // Blend zone should be at least 1 meter wide for smooth transitions.
            assert!(
                (blend_end - blend_start) >= 1.0,
                "blend zone for cascade {i} is too narrow: {}m",
                blend_end - blend_start
            );
        }
    }

    #[test]
    fn test_light_matrix_is_valid() {
        let light_dir = glam::Vec3::new(0.3, -1.0, 0.2).normalize();
        let camera_inv = glam::Mat4::IDENTITY;
        let matrix = compute_cascade_matrix(light_dir, camera_inv, 0.1, 32.0);
        // The matrix should not contain NaN or infinity.
        for col in 0..4 {
            for row in 0..4 {
                let val = matrix.col(col)[row];
                assert!(val.is_finite(), "light matrix element [{col}][{row}] is not finite: {val}");
            }
        }
        // The matrix should not be identity (a real projection transforms coordinates).
        assert_ne!(matrix, glam::Mat4::IDENTITY, "light matrix should not be identity");
    }
}
```
