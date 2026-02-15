# Ocean Rendering

## Problem

Planets with liquid water need visible oceans. Without an ocean surface, terrain below sea level appears as colored dirt or rock extending into valleys and basins, which looks completely wrong. An ocean is not just a flat blue plane -- it has animated waves, Fresnel reflections (reflectivity increases at glancing angles), and depth-dependent coloring (shallow tropical turquoise fading to deep oceanic blue-black). The ocean surface must only render where terrain is below the configured sea level; above sea level, the terrain is exposed normally. At the shoreline where ocean meets terrain, z-fighting is a critical risk because two surfaces (ocean and terrain) share nearly the same depth. This story adds a water rendering pass that handles all of these requirements.

## Solution

### Ocean Geometry

The ocean surface is a sphere at `planet_radius + sea_level`, where `sea_level` is a configurable height offset (e.g., 0.0 for default, positive for high-water scenarios). Unlike the terrain mesh, the ocean sphere is smooth (no voxel displacement). It uses the same icosphere mesh generation as the orbital renderer (story 06) but at higher local resolution near the camera, managed by the LOD system:

```rust
/// Ocean configuration for a planet.
#[derive(Clone, Debug)]
pub struct OceanParams {
    /// Sea level offset from planet surface radius, in meters.
    pub sea_level: f64,
    /// Deep ocean color (linear RGB). Default: dark blue.
    pub deep_color: [f32; 3],
    /// Shallow water color (linear RGB). Default: turquoise.
    pub shallow_color: [f32; 3],
    /// Depth at which water transitions from shallow to deep color, in meters.
    pub color_depth: f32,
    /// Wave amplitude in meters.
    pub wave_amplitude: f32,
    /// Wave frequency (cycles per meter).
    pub wave_frequency: f32,
    /// Wave speed (meters per second).
    pub wave_speed: f32,
    /// Fresnel reflectance at normal incidence (F0). Water is ~0.02.
    pub fresnel_f0: f32,
}

impl Default for OceanParams {
    fn default() -> Self {
        Self {
            sea_level: 0.0,
            deep_color: [0.01, 0.03, 0.15],
            shallow_color: [0.0, 0.5, 0.6],
            color_depth: 50.0,
            wave_amplitude: 0.5,
            wave_frequency: 0.05,
            wave_speed: 2.0,
            fresnel_f0: 0.02,
        }
    }
}
```

### GPU Uniform

```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct OceanUniform {
    pub deep_color: [f32; 3],
    pub color_depth: f32,
    pub shallow_color: [f32; 3],
    pub wave_amplitude: f32,
    pub wave_frequency: f32,
    pub wave_speed: f32,
    pub fresnel_f0: f32,
    pub time: f32,
    pub sun_direction: [f32; 3],
    pub ocean_radius: f32,
    pub camera_position: [f32; 3],
    pub _padding: f32,
}
```

### Ocean WGSL Shader

```wgsl
@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<uniform> ocean: OceanUniform;
@group(1) @binding(1) var depth_texture: texture_depth_2d;

@vertex
fn vs_ocean(in: OceanVertexInput) -> OceanVertexOutput {
    var out: OceanVertexOutput;

    // Base position on the ocean sphere.
    let sphere_pos = normalize(in.position) * ocean.ocean_radius;

    // Animated wave displacement along the surface normal.
    let normal = normalize(in.position);
    let wave1 = sin(
        dot(in.position.xz, vec2<f32>(1.0, 0.0)) * ocean.wave_frequency
        + ocean.time * ocean.wave_speed
    ) * ocean.wave_amplitude;
    let wave2 = sin(
        dot(in.position.xz, vec2<f32>(0.7, 0.7)) * ocean.wave_frequency * 1.3
        + ocean.time * ocean.wave_speed * 0.8
    ) * ocean.wave_amplitude * 0.5;

    let displaced = sphere_pos + normal * (wave1 + wave2);

    out.world_position = displaced;
    out.clip_position = camera.view_proj * vec4<f32>(displaced, 1.0);
    out.world_normal = normal;
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_ocean(in: OceanVertexOutput) -> @location(0) vec4<f32> {
    // Read terrain depth to determine water depth.
    let terrain_depth = sample_linear_depth(depth_texture, in.clip_position.xy);
    let water_depth = terrain_depth - length(in.world_position - ocean.camera_position);

    // Skip pixels where terrain is above sea level (water_depth < 0).
    if water_depth < 0.0 {
        discard;
    }

    // Depth-based color blending.
    let depth_factor = clamp(water_depth / ocean.color_depth, 0.0, 1.0);
    let water_color = mix(
        vec3<f32>(ocean.shallow_color[0], ocean.shallow_color[1], ocean.shallow_color[2]),
        vec3<f32>(ocean.deep_color[0], ocean.deep_color[1], ocean.deep_color[2]),
        depth_factor,
    );

    // Fresnel effect.
    let view_dir = normalize(ocean.camera_position - in.world_position);
    let ndotv = max(dot(in.world_normal, view_dir), 0.0);
    let fresnel = ocean.fresnel_f0 + (1.0 - ocean.fresnel_f0) * pow(1.0 - ndotv, 5.0);

    // Combine diffuse water color with specular reflection.
    let sun_dir = vec3<f32>(ocean.sun_direction[0], ocean.sun_direction[1], ocean.sun_direction[2]);
    let ndotl = max(dot(in.world_normal, sun_dir), 0.0);
    let diffuse = water_color * ndotl;

    // Specular highlight (Blinn-Phong for simplicity).
    let half_vec = normalize(view_dir + sun_dir);
    let spec = pow(max(dot(in.world_normal, half_vec), 0.0), 256.0);
    let specular = vec3<f32>(1.0) * spec * fresnel;

    let final_color = diffuse * (1.0 - fresnel) + specular;
    return vec4<f32>(final_color, 0.85 + fresnel * 0.15); // Slightly transparent
}
```

### Z-Fighting Prevention at Shoreline

At the shoreline, the ocean surface and terrain surface have nearly identical depth values. To prevent z-fighting:

1. Render terrain first with the standard depth write.
2. Render the ocean second with a small depth bias (polygon offset) pushing the ocean slightly behind equal-depth terrain.
3. The ocean fragment shader discards pixels where `water_depth < 0` (terrain above sea level).

```rust
impl OceanRenderer {
    fn depth_stencil_state() -> wgpu::DepthStencilState {
        wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::GreaterEqual, // Reverse-Z
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState {
                constant: -2,     // Push ocean slightly behind terrain
                slope_scale: -1.0,
                clamp: 0.0,
            },
        }
    }
}
```

### Render Integration

The ocean renders after terrain but before the atmosphere post-pass:

```rust
pub struct OceanRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub sphere_mesh: MeshBuffer,
    pub params: OceanParams,
}

impl OceanRenderer {
    pub fn render(
        &self,
        pass: &mut wgpu::RenderPass,
        camera_bind_group: &wgpu::BindGroup,
        ocean_bind_group: &wgpu::BindGroup,
    ) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera_bind_group, &[]);
        pass.set_bind_group(1, ocean_bind_group, &[]);
        self.sphere_mesh.bind(pass);
        self.sphere_mesh.draw(pass);
    }
}
```

Render order per frame:
1. Terrain pass (opaque, writes depth).
2. Ocean pass (reads terrain depth, writes ocean depth, depth bias applied).
3. Atmosphere pass (reads combined depth, additive blend).

## Outcome

The `nebula-planet` crate exports `OceanParams`, `OceanUniform`, and `OceanRenderer`. Oceans render at the configured sea level with animated waves, Fresnel reflections, and depth-dependent coloring. Terrain above sea level is not obscured by the ocean. Shorelines are clean with no z-fighting. The ocean is visually distinct from terrain and responds to the sun direction for specular highlights.

## Demo Integration

**Demo crate:** `nebula-demo`

Oceans fill low-lying terrain with animated blue water. Waves ripple across the surface. Shorelines are clean with no z-fighting. Sun specular highlights glint off the water.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Render pipeline, depth bias, texture sampling |
| `glam` | `0.29` | Vector math, sphere generation |
| `bytemuck` | `1.21` | Uniform buffer serialization |

Internal dependencies: `nebula-render`, `nebula-planet` (atmosphere integration, orbital sphere mesh). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn test_ocean_visible_at_sea_level() {
        let params = OceanParams::default();
        let planet_radius = 6_371_000.0_f32;
        let ocean_radius = planet_radius + params.sea_level as f32;

        // Camera on the surface looking at the horizon.
        let camera_pos = Vec3::new(0.0, planet_radius + 1.7, 0.0);
        let look_dir = Vec3::new(1.0, 0.0, 0.0);

        // Ray from camera toward horizon should intersect the ocean sphere.
        let (t_near, t_far) = ray_sphere_intersect_f32(
            camera_pos,
            look_dir,
            Vec3::ZERO,
            ocean_radius,
        );
        assert!(
            t_far > 0.0 && t_near < t_far,
            "Ocean sphere should be intersected from surface: t_near={t_near}, t_far={t_far}"
        );
    }

    #[test]
    fn test_ocean_hides_terrain_below_sea_level() {
        let params = OceanParams::default();
        let planet_radius = 1000.0_f32;
        let sea_level = params.sea_level as f32;
        let ocean_radius = planet_radius + sea_level;

        // Terrain height below sea level: should be hidden by ocean.
        let terrain_height = sea_level - 50.0; // 50m below sea level
        let terrain_surface = planet_radius + terrain_height;

        // The ocean surface is above the terrain.
        assert!(
            ocean_radius > terrain_surface,
            "Ocean ({ocean_radius}) should be above submerged terrain ({terrain_surface})"
        );

        // Water depth is positive.
        let water_depth = ocean_radius - terrain_surface;
        assert!(
            water_depth > 0.0,
            "Water depth should be positive for submerged terrain, got {water_depth}"
        );
    }

    #[test]
    fn test_waves_animate_over_time() {
        let params = OceanParams::default();

        // Evaluate wave displacement at the same position but different times.
        let position = Vec3::new(100.0, 0.0, 50.0);
        let displacements: Vec<f32> = (0..10)
            .map(|i| {
                let time = i as f32 * 0.5;
                compute_wave_displacement(&position, time, &params)
            })
            .collect();

        // Not all displacements should be the same (waves are moving).
        let all_same = displacements.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-6);
        assert!(
            !all_same,
            "Wave displacement should change over time, but got: {displacements:?}"
        );

        // All displacements should be within the configured amplitude range.
        let max_amp = params.wave_amplitude * 1.5; // Two overlapping waves
        for (i, &d) in displacements.iter().enumerate() {
            assert!(
                d.abs() <= max_amp + 0.01,
                "Displacement at time {} exceeds max amplitude: {} > {}",
                i as f32 * 0.5,
                d.abs(),
                max_amp
            );
        }
    }

    #[test]
    fn test_depth_coloring_changes() {
        let params = OceanParams::default();

        // Shallow water (depth = 1m).
        let shallow_color = compute_water_color(1.0, &params);
        // Deep water (depth = 500m).
        let deep_color = compute_water_color(500.0, &params);

        // Shallow should be closer to turquoise (shallow_color param).
        // Deep should be closer to dark blue (deep_color param).
        assert!(
            shallow_color[1] > deep_color[1], // Green channel higher in shallow
            "Shallow water should be more green/turquoise: shallow={shallow_color:?}, deep={deep_color:?}"
        );
        assert!(
            deep_color[2] >= shallow_color[2] || deep_color[0] < shallow_color[0],
            "Deep water should be darker overall: shallow={shallow_color:?}, deep={deep_color:?}"
        );
    }

    #[test]
    fn test_no_z_fighting_at_shoreline() {
        // Verify that the depth bias configuration produces a non-zero offset.
        let bias = OceanRenderer::depth_stencil_state().bias;
        assert!(
            bias.constant != 0 || bias.slope_scale != 0.0,
            "Depth bias must be non-zero to prevent z-fighting at shoreline"
        );

        // The constant bias should push the ocean behind terrain (negative for reverse-Z).
        assert!(
            bias.constant < 0,
            "Depth bias constant should be negative for reverse-Z, got {}",
            bias.constant
        );
    }
}
```
