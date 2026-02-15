# Point Lights

## Problem

A voxel world needs local light sources — torches on cave walls, lanterns in villages, glowing lava pools, bioluminescent fungi underground. A single directional light (the sun) cannot illuminate enclosed spaces or create the warm, localized pools of light that make environments feel alive. The engine needs point lights with position, color, intensity, and a finite attenuation radius. Because a planet-scale world can contain thousands of placed light sources, the system must cap the number of visible lights per frame, cull lights that are outside the camera frustum or beyond a distance threshold, and pack them into a GPU-side storage buffer that the fragment shader iterates over. Getting the attenuation model right (inverse-square with a smooth cutoff) is essential for physically plausible results that integrate cleanly with the PBR pipeline.

## Solution

### Data Structures

```rust
/// CPU-side point light descriptor.
#[derive(Clone, Debug)]
pub struct PointLight {
    /// Position in world-local coordinates (relative to the current origin chunk).
    pub position: glam::Vec3,
    /// Linear RGB color.
    pub color: glam::Vec3,
    /// Luminous intensity. Higher values push the light further.
    pub intensity: f32,
    /// Maximum radius of effect. Beyond this distance, contribution is zero.
    /// Used for culling and to bound the shader loop.
    pub radius: f32,
}
```

### Attenuation Model

The light follows a physically-based inverse-square falloff with a smooth windowing function to reach exactly zero at the cutoff radius:

```rust
/// Compute attenuation at a given distance from a point light.
/// Returns a value in [0.0, 1.0].
pub fn attenuation(distance: f32, radius: f32) -> f32 {
    if distance >= radius {
        return 0.0;
    }
    // Inverse-square falloff.
    let inv_sq = 1.0 / (distance * distance + 1.0); // +1.0 prevents singularity at d=0
    // Smooth windowing: (1 - (d/r)^2)^2 ensures C1 continuity at the boundary.
    let window = {
        let ratio = distance / radius;
        let t = (1.0 - ratio * ratio).max(0.0);
        t * t
    };
    inv_sq * window
}
```

At distance 0, `inv_sq = 1.0` and `window = 1.0`, giving full intensity. At distance >= radius, the window evaluates to 0. The `+1.0` in the denominator prevents division-by-zero without noticeably affecting falloff at practical distances.

### GPU Buffer Layout

Point lights are packed into a storage buffer (not uniform, since the count is dynamic):

```rust
/// Per-light GPU data, 48 bytes, std430-compatible.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PointLightGpu {
    /// xyz = position (view space), w = radius.
    pub position_radius: [f32; 4],
    /// xyz = color (linear RGB), w = intensity.
    pub color_intensity: [f32; 4],
    /// Padding to align to 16 bytes per row (already aligned).
    pub _padding: [f32; 4],
}

/// Header at the start of the light buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct PointLightHeader {
    pub count: u32,
    pub _pad: [u32; 3],
}
```

The maximum buffer holds 1 header (16 bytes) + 256 lights * 48 bytes = 12,304 bytes.

### Light Culling

Each frame, before uploading lights to the GPU:

1. **Distance cull**: Discard lights whose position is farther than `radius + camera_far_plane` from the camera.
2. **Frustum cull**: Test each light's bounding sphere (center = position, radius = attenuation radius) against the 6 frustum planes. Lights fully outside are discarded.
3. **Sort by distance**: Closest lights first, so the 256-light cap keeps the most visually important lights.
4. **Truncate**: If more than 256 lights survive culling, keep only the nearest 256.

```rust
pub struct PointLightManager {
    /// All registered point lights in the scene.
    all_lights: Vec<PointLight>,
    /// Scratch buffer reused each frame for culled/sorted lights.
    visible_lights: Vec<PointLightGpu>,
    /// GPU storage buffer.
    gpu_buffer: wgpu::Buffer,
}

impl PointLightManager {
    pub const MAX_VISIBLE_LIGHTS: usize = 256;

    pub fn cull_and_upload(
        &mut self,
        camera_pos: glam::Vec3,
        frustum: &Frustum,
        queue: &wgpu::Queue,
    ) {
        self.visible_lights.clear();
        // ... culling, sorting, truncation ...
        // Write header + light data to gpu_buffer via queue.write_buffer().
    }
}
```

### WGSL Shader Loop

```wgsl
struct PointLightData {
    position_radius: vec4<f32>,
    color_intensity: vec4<f32>,
    _padding: vec4<f32>,
};

struct PointLightBuffer {
    count: u32,
    _pad: vec3<u32>,
    lights: array<PointLightData>,
};

@group(1) @binding(1)
var<storage, read> point_lights: PointLightBuffer;

fn point_light_contribution(
    frag_pos: vec3<f32>,
    normal: vec3<f32>,
) -> vec3<f32> {
    var total = vec3<f32>(0.0);
    for (var i = 0u; i < point_lights.count; i++) {
        let light = point_lights.lights[i];
        let to_light = light.position_radius.xyz - frag_pos;
        let dist = length(to_light);
        let radius = light.position_radius.w;
        if dist >= radius { continue; }
        let atten = attenuation(dist, radius);
        let n_dot_l = max(dot(normal, normalize(to_light)), 0.0);
        total += light.color_intensity.xyz * light.color_intensity.w * atten * n_dot_l;
    }
    return total;
}
```

## Outcome

A `PointLight` type, `PointLightManager`, and `PointLightGpu` buffer representation in `nebula_lighting`. The manager culls, sorts, and uploads up to 256 visible lights per frame into a storage buffer. The fragment shader loops over active lights applying inverse-square attenuation. Running `cargo test -p nebula_lighting` passes all point light tests. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Glowing point lights are placed at intervals on the terrain surface. Each casts a warm pool of orange light in a radius, illuminating nearby voxels.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Storage buffer creation and upload |
| `bytemuck` | `1.21` | Pod/Zeroable derives for GPU structs |
| `glam` | `0.29` | Vec3 for position, distance, and frustum math |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_light_at_distance_zero_has_full_intensity() {
        let atten = attenuation(0.0, 10.0);
        // At distance 0: inv_sq = 1/(0+1) = 1.0, window = (1-0)^2 = 1.0.
        assert!((atten - 1.0).abs() < 1e-6, "attenuation at d=0 should be 1.0, got {atten}");
    }

    #[test]
    fn test_light_beyond_radius_has_zero_contribution() {
        let atten_at = attenuation(10.0, 10.0);
        let atten_beyond = attenuation(15.0, 10.0);
        assert_eq!(atten_at, 0.0, "attenuation at exactly radius should be 0.0");
        assert_eq!(atten_beyond, 0.0, "attenuation beyond radius should be 0.0");
    }

    #[test]
    fn test_attenuation_follows_inverse_square() {
        // At moderate distances (well within radius), attenuation should roughly
        // follow 1/d^2. Doubling the distance should reduce intensity by ~4x.
        let radius = 100.0;
        let a1 = attenuation(5.0, radius);
        let a2 = attenuation(10.0, radius);
        let ratio = a1 / a2;
        // Expect ratio near 4.0 (exact value affected by windowing and +1 term).
        assert!(ratio > 3.0 && ratio < 5.0,
            "doubling distance should roughly quarter intensity, got ratio {ratio}");
    }

    #[test]
    fn test_light_culling_reduces_active_count() {
        let mut manager = PointLightManager::new_test();
        // Add 10 lights: 5 close, 5 very far.
        for i in 0..5 {
            manager.add(PointLight {
                position: glam::Vec3::new(i as f32, 0.0, 0.0),
                color: glam::Vec3::ONE,
                intensity: 1.0,
                radius: 50.0,
            });
        }
        for i in 0..5 {
            manager.add(PointLight {
                position: glam::Vec3::new(10_000.0 + i as f32, 0.0, 0.0),
                color: glam::Vec3::ONE,
                intensity: 1.0,
                radius: 5.0, // tiny radius, far away
            });
        }
        let visible = manager.cull_to_list(
            glam::Vec3::ZERO,
            &Frustum::infinite(), // no frustum rejection for this test
        );
        // Only the 5 close lights should survive distance culling.
        assert_eq!(visible.len(), 5, "far lights should be culled");
    }

    #[test]
    fn test_max_256_lights_enforced() {
        let mut manager = PointLightManager::new_test();
        for i in 0..400 {
            manager.add(PointLight {
                position: glam::Vec3::new(i as f32 * 0.1, 0.0, 0.0),
                color: glam::Vec3::ONE,
                intensity: 1.0,
                radius: 1000.0,
            });
        }
        let visible = manager.cull_to_list(glam::Vec3::ZERO, &Frustum::infinite());
        assert!(
            visible.len() <= PointLightManager::MAX_VISIBLE_LIGHTS,
            "visible lights must not exceed {}, got {}",
            PointLightManager::MAX_VISIBLE_LIGHTS,
            visible.len()
        );
    }

    #[test]
    fn test_gpu_struct_size() {
        assert_eq!(std::mem::size_of::<PointLightGpu>(), 48);
        assert_eq!(std::mem::size_of::<PointLightHeader>(), 16);
    }
}
```
