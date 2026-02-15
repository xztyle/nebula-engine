# Atmosphere Scattering

## Problem

A planet without an atmosphere looks lifeless -- terrain geometry floating in black space. Real planets scatter sunlight through their atmosphere, producing the blue sky overhead, orange-red sunsets at the horizon, and a thin luminous halo visible from orbit. Without atmospheric scattering, the engine cannot produce believable planetary environments. The atmosphere must respond dynamically to the sun's position: blue skies when the sun is high, warm colors at dawn and dusk, and darkness at night. It must also look correct from two radically different viewpoints -- standing on the surface looking up, and orbiting above looking down at the planet's limb. This requires a physically-based scattering model, not just a skybox texture.

## Solution

### Scattering Model

Implement Rayleigh and Mie scattering using the single-scattering integral. Rayleigh scattering handles small molecules (nitrogen, oxygen) and produces the blue sky. Mie scattering handles larger aerosol particles and produces the bright haze around the sun disk. The combined model ray-marches through the atmosphere volume, accumulating scattered light and optical depth along the view ray.

### Atmosphere Parameters

```rust
/// Physical parameters defining a planet's atmosphere.
#[derive(Clone, Debug)]
pub struct AtmosphereParams {
    /// Inner radius: the planet's surface in meters.
    pub planet_radius: f32,
    /// Outer radius: top of the atmosphere in meters.
    pub atmosphere_radius: f32,
    /// Rayleigh scattering coefficients at sea level (per-wavelength, RGB).
    /// Earth-like: (5.5e-6, 13.0e-6, 22.4e-6) for red, green, blue.
    pub rayleigh_coefficients: [f32; 3],
    /// Rayleigh scale height in meters (how quickly density falls with altitude).
    /// Earth-like: ~8500m.
    pub rayleigh_scale_height: f32,
    /// Mie scattering coefficient at sea level (scalar, wavelength-independent).
    /// Earth-like: ~21e-6.
    pub mie_coefficient: f32,
    /// Mie scale height in meters. Earth-like: ~1200m.
    pub mie_scale_height: f32,
    /// Mie preferred scattering direction (g parameter for Henyey-Greenstein phase function).
    /// Earth-like: ~0.758. Range [-1, 1]; positive = forward scattering.
    pub mie_direction: f32,
    /// Sun intensity multiplier.
    pub sun_intensity: f32,
}

impl AtmosphereParams {
    pub fn earth_like(planet_radius: f32) -> Self {
        Self {
            planet_radius,
            atmosphere_radius: planet_radius * 1.025,
            rayleigh_coefficients: [5.5e-6, 13.0e-6, 22.4e-6],
            rayleigh_scale_height: 8500.0,
            mie_coefficient: 21e-6,
            mie_scale_height: 1200.0,
            mie_direction: 0.758,
            sun_intensity: 22.0,
        }
    }
}
```

### GPU Uniform Buffer

```rust
/// GPU-side atmosphere uniform buffer. Matches the WGSL struct layout.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct AtmosphereUniform {
    pub planet_center: [f32; 3],
    pub planet_radius: f32,
    pub atmosphere_radius: f32,
    pub rayleigh_coefficients: [f32; 3],
    pub rayleigh_scale_height: f32,
    pub mie_coefficient: f32,
    pub mie_scale_height: f32,
    pub mie_direction: f32,
    pub sun_direction: [f32; 3],
    pub sun_intensity: f32,
    pub camera_position: [f32; 3],
    pub _padding: f32,
}
```

### WGSL Shader

The atmosphere is rendered as a full-screen pass. For each pixel, compute the view ray from the camera, intersect it with the atmosphere sphere, and ray-march through the atmosphere volume:

```wgsl
struct AtmosphereParams {
    planet_center: vec3<f32>,
    planet_radius: f32,
    atmosphere_radius: f32,
    rayleigh_coefficients: vec3<f32>,
    rayleigh_scale_height: f32,
    mie_coefficient: f32,
    mie_scale_height: f32,
    mie_direction: f32,
    sun_direction: vec3<f32>,
    sun_intensity: f32,
    camera_position: vec3<f32>,
};

@group(0) @binding(0) var<uniform> atmo: AtmosphereParams;
@group(0) @binding(1) var depth_texture: texture_depth_2d;

const NUM_SAMPLES: i32 = 16;
const NUM_LIGHT_SAMPLES: i32 = 8;
const PI: f32 = 3.14159265359;

fn ray_sphere_intersect(origin: vec3<f32>, dir: vec3<f32>, center: vec3<f32>, radius: f32) -> vec2<f32> {
    let oc = origin - center;
    let b = dot(oc, dir);
    let c = dot(oc, oc) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return vec2<f32>(-1.0, -1.0);
    }
    let sqrt_disc = sqrt(disc);
    return vec2<f32>(-b - sqrt_disc, -b + sqrt_disc);
}

fn rayleigh_phase(cos_angle: f32) -> f32 {
    return 3.0 / (16.0 * PI) * (1.0 + cos_angle * cos_angle);
}

fn mie_phase(cos_angle: f32, g: f32) -> f32 {
    let g2 = g * g;
    let num = 3.0 * (1.0 - g2) * (1.0 + cos_angle * cos_angle);
    let denom = 8.0 * PI * (2.0 + g2) * pow(1.0 + g2 - 2.0 * g * cos_angle, 1.5);
    return num / denom;
}

@fragment
fn fs_atmosphere(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let ray_dir = compute_ray_direction(in.uv, atmo.camera_position);

    let atmo_hit = ray_sphere_intersect(
        atmo.camera_position, ray_dir, atmo.planet_center, atmo.atmosphere_radius
    );

    if atmo_hit.x > atmo_hit.y || atmo_hit.y < 0.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0); // No atmosphere intersection.
    }

    let t_start = max(atmo_hit.x, 0.0);
    let scene_depth = sample_depth(depth_texture, in.uv);
    let t_end = select(atmo_hit.y, min(scene_depth, atmo_hit.y), scene_depth > 0.0);

    let step_size = (t_end - t_start) / f32(NUM_SAMPLES);
    var total_rayleigh = vec3<f32>(0.0);
    var total_mie = vec3<f32>(0.0);
    var optical_depth_r = 0.0;
    var optical_depth_m = 0.0;

    let cos_angle = dot(ray_dir, atmo.sun_direction);
    let phase_r = rayleigh_phase(cos_angle);
    let phase_m = mie_phase(cos_angle, atmo.mie_direction);

    for (var i = 0; i < NUM_SAMPLES; i++) {
        let t = t_start + (f32(i) + 0.5) * step_size;
        let sample_pos = atmo.camera_position + ray_dir * t;
        let height = length(sample_pos - atmo.planet_center) - atmo.planet_radius;

        let density_r = exp(-height / atmo.rayleigh_scale_height) * step_size;
        let density_m = exp(-height / atmo.mie_scale_height) * step_size;

        optical_depth_r += density_r;
        optical_depth_m += density_m;

        // Light ray march toward the sun.
        let light_hit = ray_sphere_intersect(
            sample_pos, atmo.sun_direction, atmo.planet_center, atmo.atmosphere_radius
        );
        let light_step = light_hit.y / f32(NUM_LIGHT_SAMPLES);
        var light_depth_r = 0.0;
        var light_depth_m = 0.0;

        for (var j = 0; j < NUM_LIGHT_SAMPLES; j++) {
            let lt = (f32(j) + 0.5) * light_step;
            let light_pos = sample_pos + atmo.sun_direction * lt;
            let light_height = length(light_pos - atmo.planet_center) - atmo.planet_radius;
            light_depth_r += exp(-light_height / atmo.rayleigh_scale_height) * light_step;
            light_depth_m += exp(-light_height / atmo.mie_scale_height) * light_step;
        }

        let tau = atmo.rayleigh_coefficients * (optical_depth_r + light_depth_r)
                + vec3<f32>(atmo.mie_coefficient) * (optical_depth_m + light_depth_m);
        let attenuation = exp(-tau);

        total_rayleigh += density_r * attenuation;
        total_mie += density_m * attenuation;
    }

    let color = atmo.sun_intensity * (
        phase_r * atmo.rayleigh_coefficients * total_rayleigh
        + phase_m * atmo.mie_coefficient * total_mie
    );

    return vec4<f32>(color, 1.0);
}
```

### Render Integration

The atmosphere is rendered as a post-pass after terrain. A full-screen triangle is drawn, and the fragment shader samples the depth buffer to know where terrain exists. Atmosphere scattering is accumulated for the portion of the view ray between the camera and the terrain (or the atmosphere boundary if no terrain is hit). The atmosphere color is additively blended over the terrain color:

```rust
pub struct AtmosphereRenderer {
    pub pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub params: AtmosphereParams,
}

impl AtmosphereRenderer {
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        bind_group: &wgpu::BindGroup,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("atmosphere-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load, // Preserve terrain color
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None, // Read-only depth via texture binding
            ..Default::default()
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1); // Full-screen triangle
    }
}
```

The blend state for the atmosphere pipeline uses additive blending:

```rust
wgpu::BlendState {
    color: wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    },
    alpha: wgpu::BlendComponent::OVER,
}
```

## Outcome

The `nebula-planet` crate exports `AtmosphereParams`, `AtmosphereUniform`, and `AtmosphereRenderer`. From the surface, the sky appears blue overhead and warm-colored at the horizon when the sun is low. From orbit, a thin luminous halo appears around the planet's limb. The atmosphere color responds to the sun direction in real time. The shader runs as a post-pass over the terrain color buffer, reading the depth buffer to correctly composite atmosphere in front of terrain.

## Demo Integration

**Demo crate:** `nebula-demo`

The sky is blue overhead and warm-colored at the horizon. From orbit, a thin luminous halo appears around the planet's limb. The atmosphere responds to the sun direction in real time.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Render pipeline, shader execution, texture sampling |
| `bytemuck` | `1.21` | Uniform buffer serialization |
| `glam` | `0.29` | Vector math for sun direction and camera position |

Internal dependencies: `nebula-render` (RenderContext, pipeline helpers), `nebula-lighting` (sun direction). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn test_atmosphere_visible_from_surface() {
        let params = AtmosphereParams::earth_like(6_371_000.0);
        let camera_pos = Vec3::new(0.0, params.planet_radius + 1.7, 0.0); // Standing on surface
        let look_up = Vec3::Y; // Looking straight up

        let (t_near, t_far) = ray_sphere_intersect_f32(
            camera_pos,
            look_up,
            Vec3::ZERO,
            params.atmosphere_radius,
        );

        // The ray from surface upward should intersect the atmosphere.
        assert!(t_far > t_near, "Atmosphere should be intersected looking up from surface");
        assert!(t_far > 0.0, "Intersection should be in front of camera");

        // The path length through the atmosphere should be roughly
        // atmosphere_radius - planet_radius (~160 km for Earth).
        let path_length = t_far - t_near.max(0.0);
        let expected_thickness = params.atmosphere_radius - params.planet_radius;
        assert!(
            (path_length - expected_thickness).abs() / expected_thickness < 0.01,
            "Atmosphere path length {path_length} should be ~{expected_thickness}"
        );
    }

    #[test]
    fn test_atmosphere_visible_from_orbit() {
        let params = AtmosphereParams::earth_like(6_371_000.0);
        let camera_pos = Vec3::new(0.0, params.atmosphere_radius * 3.0, 0.0); // Far above
        let look_tangent = Vec3::new(1.0, -0.3, 0.0).normalize(); // Looking at planet limb

        let (t_near, t_far) = ray_sphere_intersect_f32(
            camera_pos,
            look_tangent,
            Vec3::ZERO,
            params.atmosphere_radius,
        );

        assert!(
            t_far > t_near && t_near > 0.0,
            "Atmosphere should be visible from orbit: t_near={t_near}, t_far={t_far}"
        );
    }

    #[test]
    fn test_sky_color_changes_with_sun_angle() {
        let params = AtmosphereParams::earth_like(6_371_000.0);

        // Compute scattering for sun overhead vs. sun at horizon.
        let camera_pos = Vec3::new(0.0, params.planet_radius + 1.7, 0.0);
        let look_dir = Vec3::new(1.0, 0.5, 0.0).normalize(); // Looking slightly up

        let sun_overhead = Vec3::Y;
        let sun_horizon = Vec3::new(1.0, 0.01, 0.0).normalize();

        let color_noon = compute_single_scatter(
            camera_pos, look_dir, sun_overhead, &params, 16, 8,
        );
        let color_sunset = compute_single_scatter(
            camera_pos, look_dir, sun_horizon, &params, 16, 8,
        );

        // At noon, blue channel should dominate (Rayleigh scatters blue most).
        assert!(
            color_noon[2] > color_noon[0],
            "Noon sky should be bluer: R={}, B={}",
            color_noon[0], color_noon[2]
        );

        // At sunset, red/orange should increase relative to blue (long path
        // through atmosphere attenuates blue more, leaving red).
        let noon_ratio = color_noon[0] / color_noon[2].max(1e-10);
        let sunset_ratio = color_sunset[0] / color_sunset[2].max(1e-10);
        assert!(
            sunset_ratio > noon_ratio,
            "Sunset should have higher red/blue ratio: noon={noon_ratio:.3}, sunset={sunset_ratio:.3}"
        );
    }

    #[test]
    fn test_atmosphere_fades_at_horizon() {
        let params = AtmosphereParams::earth_like(6_371_000.0);
        let camera_pos = Vec3::new(0.0, params.planet_radius + 1.7, 0.0);
        let sun_dir = Vec3::Y;

        // Compare brightness looking up (short path) vs. looking at horizon (long path).
        let color_up = compute_single_scatter(
            camera_pos, Vec3::Y, sun_dir, &params, 16, 8,
        );
        let color_horizon = compute_single_scatter(
            camera_pos, Vec3::X, sun_dir, &params, 16, 8,
        );

        let brightness_up = color_up[0] + color_up[1] + color_up[2];
        let brightness_horizon = color_horizon[0] + color_horizon[1] + color_horizon[2];

        // The horizon view passes through much more atmosphere, so it should be
        // either brighter (more scattering) or noticeably different in hue.
        // The key test is that they are NOT identical -- the angle matters.
        assert!(
            (brightness_up - brightness_horizon).abs() > brightness_up * 0.1,
            "Atmosphere should differ between up and horizon: up={brightness_up}, horizon={brightness_horizon}"
        );
    }

    #[test]
    fn test_no_artifacts_at_atmosphere_surface_boundary() {
        let params = AtmosphereParams::earth_like(6_371_000.0);
        let camera_pos = Vec3::new(0.0, params.planet_radius + 1.7, 0.0);
        let sun_dir = Vec3::Y;

        // Sample atmosphere color for rays that just graze the surface
        // vs. rays that go slightly above. There should be no discontinuity.
        let angles = [0.01_f32, 0.02, 0.03, 0.04, 0.05];
        let colors: Vec<[f32; 3]> = angles
            .iter()
            .map(|&a| {
                let dir = Vec3::new(a.cos(), a.sin(), 0.0);
                compute_single_scatter(camera_pos, dir, sun_dir, &params, 16, 8)
            })
            .collect();

        // Adjacent samples should change smoothly (no large jumps).
        for i in 1..colors.len() {
            let diff = (0..3)
                .map(|c| (colors[i][c] - colors[i - 1][c]).abs())
                .sum::<f32>();
            let avg_brightness = (0..3)
                .map(|c| colors[i][c])
                .sum::<f32>()
                .max(1e-6);
            let relative_diff = diff / avg_brightness;
            assert!(
                relative_diff < 0.5,
                "Discontinuity at boundary: angle {:.3} -> {:.3}, relative diff = {:.3}",
                angles[i - 1], angles[i], relative_diff
            );
        }
    }
}
```
