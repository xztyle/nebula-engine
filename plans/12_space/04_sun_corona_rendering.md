# Sun/Corona Rendering

## Problem

The nearest star (the system's sun) is not just another point in the starfield -- it is an intensely bright disk that dominates the sky and drives the lighting for the entire scene. Rendering it as a starfield point would be visually wrong: a sun is a resolved disk with a visible corona (the outer atmosphere of radiant, streaming plasma). The sun must be rendered as a bright billboard that always faces the camera, with an animated corona effect using noise-based radial rays. The disk must emit HDR values far exceeding 1.0 to drive the bloom system (story 02). Sun color must vary by star type: yellow for Sol-like G-type stars, blue-white for hot O/B-type stars, and red for cool M-type stars. The angular size of the sun disk must decrease with distance following real optics (angular_diameter = physical_diameter / distance).

## Solution

Implement a `SunRenderer` in the `nebula-space` crate that renders the nearest star as a camera-facing billboard with a multi-layered disk-and-corona shader. The sun is a special-case entity in the ECS that is identified by its proximity to the camera and rendered with a dedicated pipeline after the skybox but before scene geometry.

### Star Type and Sun Properties

```rust
/// Spectral classification of a star, determining its color and temperature.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StarType {
    /// O-type: blue, very hot (30,000-50,000K). Rare, extremely luminous.
    O,
    /// B-type: blue-white (10,000-30,000K). Hot and bright.
    B,
    /// A-type: white (7,500-10,000K).
    A,
    /// F-type: yellow-white (6,000-7,500K).
    F,
    /// G-type: yellow (5,200-6,000K). Sol-like.
    G,
    /// K-type: orange (3,700-5,200K).
    K,
    /// M-type: red (2,400-3,700K). Cool, most common.
    M,
}

impl StarType {
    /// Returns the characteristic linear RGB color for this star type.
    pub fn color(&self) -> [f32; 3] {
        match self {
            StarType::O => [0.6, 0.7, 1.0],
            StarType::B => [0.7, 0.8, 1.0],
            StarType::A => [0.9, 0.9, 1.0],
            StarType::F => [1.0, 1.0, 0.9],
            StarType::G => [1.0, 0.95, 0.8],
            StarType::K => [1.0, 0.8, 0.5],
            StarType::M => [1.0, 0.5, 0.3],
        }
    }

    /// Returns the approximate effective temperature in Kelvin.
    pub fn temperature_k(&self) -> f32 {
        match self {
            StarType::O => 40000.0,
            StarType::B => 20000.0,
            StarType::A => 8750.0,
            StarType::F => 6750.0,
            StarType::G => 5600.0,
            StarType::K => 4450.0,
            StarType::M => 3050.0,
        }
    }
}

/// Properties of the sun (nearest star) for rendering.
#[derive(Clone, Debug)]
pub struct SunProperties {
    /// Direction from the camera to the sun in local f32 space (unit vector).
    pub direction: glam::Vec3,
    /// Physical diameter in engine units (e.g., Sol = 1,392,700 km).
    pub physical_diameter: f64,
    /// Distance from the camera in engine units.
    pub distance: f64,
    /// Star spectral type.
    pub star_type: StarType,
    /// Base luminosity multiplier (1.0 = Sol-like).
    pub luminosity: f32,
}

impl SunProperties {
    /// Compute the angular diameter in radians as seen from the camera.
    pub fn angular_diameter(&self) -> f32 {
        (self.physical_diameter / self.distance) as f32
    }

    /// Compute the HDR brightness value for the sun disk center.
    /// This should be far above 1.0 to drive the bloom system.
    pub fn hdr_brightness(&self) -> f32 {
        // Base HDR value scaled by luminosity. Even at minimum, the sun
        // should be much brighter than 1.0 to trigger bloom.
        self.luminosity * 50.0
    }
}
```

### Billboard Geometry

The sun is rendered as a camera-facing quad (billboard). The quad size is determined by the angular diameter plus extra margin for the corona:

```rust
pub struct SunRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl SunRenderer {
    /// Update the billboard to face the camera and match the sun's current properties.
    pub fn update(
        &self,
        queue: &wgpu::Queue,
        camera: &Camera,
        sun: &SunProperties,
        time: f32,
    ) {
        let angular_radius = sun.angular_diameter() * 0.5;
        // Corona extends 3x the disk radius.
        let corona_radius = angular_radius * 3.0;

        // Billboard axes: camera's right and up vectors.
        let right = camera.right() * corona_radius;
        let up = camera.up() * corona_radius;

        // Billboard center: position the quad along the sun direction at a fixed
        // distance in the near-far range (far enough to be behind scene geometry
        // is handled by depth, but the sun should be "behind" everything).
        let center = sun.direction; // Unit vector; rendered at skybox depth.

        let vertices = [
            SunVertex { position: (center - right - up).into(), uv: [-1.0, -1.0] },
            SunVertex { position: (center + right - up).into(), uv: [ 1.0, -1.0] },
            SunVertex { position: (center + right + up).into(), uv: [ 1.0,  1.0] },
            SunVertex { position: (center - right + up).into(), uv: [-1.0,  1.0] },
        ];

        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));

        let uniforms = SunUniforms {
            color: sun.star_type.color(),
            brightness: sun.hdr_brightness(),
            disk_radius_uv: angular_radius / corona_radius,
            time,
            _padding: [0.0; 2],
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SunVertex {
    position: [f32; 3],
    uv: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct SunUniforms {
    color: [f32; 3],
    brightness: f32,
    disk_radius_uv: f32,
    time: f32,
    _padding: [f32; 2],
}
```

### Corona Shader

The sun shader renders a bright central disk with animated corona rays:

```wgsl
struct SunUniforms {
    color: vec3<f32>,
    brightness: f32,
    disk_radius_uv: f32,
    time: f32,
};

@group(0) @binding(0)
var<uniform> sun: SunUniforms;

// Simple 2D hash for procedural noise.
fn hash21(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453);
}

// Procedural noise for corona rays.
fn corona_noise(uv: vec2<f32>, time: f32) -> f32 {
    let angle = atan2(uv.y, uv.x);
    let radius = length(uv);

    // Radial rays: high-frequency angular variation.
    let ray_count = 24.0;
    let ray = sin(angle * ray_count + time * 0.5) * 0.5 + 0.5;
    let ray2 = sin(angle * ray_count * 1.7 - time * 0.3) * 0.5 + 0.5;

    // Combine ray patterns with radius-based falloff.
    let combined = mix(ray, ray2, 0.5);
    let falloff = 1.0 / (radius * radius + 0.01);

    return combined * falloff * 0.05;
}

@fragment
fn fs_sun(in: SunVertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let dist = length(uv);

    // Central disk: bright, solid color.
    let disk_edge = smoothstep(sun.disk_radius_uv, sun.disk_radius_uv * 0.9, dist);
    let disk = disk_edge * sun.brightness;

    // Corona: animated radial glow beyond the disk edge.
    let corona = corona_noise(uv, sun.time) * sun.brightness * 0.5;

    // Radial gradient falloff for the overall glow.
    let glow = exp(-dist * dist * 4.0) * sun.brightness * 0.3;

    let total_brightness = disk + corona + glow;
    let final_color = sun.color * total_brightness;

    // Alpha: fully opaque at center, fading to zero at corona edge.
    let alpha = clamp(total_brightness / sun.brightness, 0.0, 1.0);

    return vec4<f32>(final_color, alpha);
}
```

The shader output is in HDR space (values >> 1.0), which will be picked up by the bloom pass to produce the characteristic solar glow.

### Depth Handling

The sun billboard is rendered with depth write disabled. The vertex shader writes `clip_position.z = 0.0` (maximum depth in reverse-Z), placing the sun behind all scene geometry but in front of the skybox cubemap (which is rendered at exactly z=0.0 with `LessEqual`). Alternatively, the sun can be rendered as part of the skybox pass itself.

## Outcome

A `SunRenderer`, `SunProperties`, and `StarType` enum in `nebula-space` that render the nearest star as a bright, animated billboard with disk and corona effects. The sun drives the bloom system through HDR output values. Running `cargo test -p nebula-space` passes all sun rendering tests. The sun integrates with the skybox pipeline and the bloom post-processing from story 02.

## Demo Integration

**Demo crate:** `nebula-demo`

The sun renders as a bright disk with animated corona tendrils. Looking toward the sun produces intense bloom. The sun's color and size reflect its star type.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Render pipeline, billboard geometry, shader execution |
| `bytemuck` | `1.21` | Vertex and uniform buffer serialization |
| `glam` | `0.29` | Vec3 for billboard positioning, camera orientation |

The sun renderer lives in the `nebula-space` crate. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};

    #[test]
    fn test_sun_disk_faces_camera_from_all_angles() {
        // The billboard right and up vectors should always be perpendicular
        // to the camera's forward direction, regardless of camera rotation.
        let test_rotations = [
            Quat::IDENTITY,
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            Quat::from_rotation_x(std::f32::consts::FRAC_PI_4),
            Quat::from_rotation_z(std::f32::consts::PI),
            Quat::from_euler(glam::EulerRot::YXZ, 1.0, 0.5, 0.3),
        ];

        for (i, rotation) in test_rotations.iter().enumerate() {
            let camera = Camera {
                rotation: *rotation,
                ..Camera::default()
            };
            let right = camera.right();
            let up = camera.up();
            let forward = camera.forward();

            // Right and up should be perpendicular to forward.
            assert!(
                right.dot(forward).abs() < 1e-5,
                "Rotation {i}: billboard right is not perpendicular to forward"
            );
            assert!(
                up.dot(forward).abs() < 1e-5,
                "Rotation {i}: billboard up is not perpendicular to forward"
            );
        }
    }

    #[test]
    fn test_corona_animates_over_time() {
        // The corona noise function should produce different values at different times
        // for the same UV coordinate.
        let uv = [0.5_f32, 0.3];
        let uv_vec = glam::Vec2::new(uv[0], uv[1]);

        // Simulate the corona noise at two different times.
        let angle = uv_vec.y.atan2(uv_vec.x);
        let ray_count = 24.0_f32;

        let ray_t0 = (angle * ray_count + 0.0 * 0.5).sin() * 0.5 + 0.5;
        let ray_t1 = (angle * ray_count + 5.0 * 0.5).sin() * 0.5 + 0.5;

        assert!(
            (ray_t0 - ray_t1).abs() > 0.001,
            "Corona should animate: value at t=0 ({ray_t0}) vs t=5 ({ray_t1})"
        );
    }

    #[test]
    fn test_sun_brightness_drives_bloom() {
        let sun = SunProperties {
            direction: Vec3::new(0.0, 0.5, -0.866),
            physical_diameter: 1_392_700.0,
            distance: 149_597_870.0,
            star_type: StarType::G,
            luminosity: 1.0,
        };

        let brightness = sun.hdr_brightness();
        assert!(
            brightness > 10.0,
            "Sun HDR brightness ({brightness}) should far exceed 1.0 to drive bloom"
        );
    }

    #[test]
    fn test_sun_color_matches_star_type() {
        // G-type (Sol-like) should be yellowish: R > B.
        let g_color = StarType::G.color();
        assert!(
            g_color[0] > g_color[2],
            "G-type star should be yellow (R > B): {:?}",
            g_color
        );

        // O-type should be bluish: B > R.
        let o_color = StarType::O.color();
        assert!(
            o_color[2] > o_color[0],
            "O-type star should be blue (B > R): {:?}",
            o_color
        );

        // M-type should be reddish: R >> B.
        let m_color = StarType::M.color();
        assert!(
            m_color[0] > m_color[2] * 2.0,
            "M-type star should be red (R >> B): {:?}",
            m_color
        );
    }

    #[test]
    fn test_sun_angular_size_decreases_with_distance() {
        let sun_near = SunProperties {
            direction: Vec3::Z,
            physical_diameter: 1_392_700.0,
            distance: 100_000_000.0,
            star_type: StarType::G,
            luminosity: 1.0,
        };
        let sun_far = SunProperties {
            direction: Vec3::Z,
            physical_diameter: 1_392_700.0,
            distance: 500_000_000.0,
            star_type: StarType::G,
            luminosity: 1.0,
        };

        let angular_near = sun_near.angular_diameter();
        let angular_far = sun_far.angular_diameter();

        assert!(
            angular_near > angular_far,
            "Sun at closer distance should have larger angular diameter: {angular_near} vs {angular_far}"
        );
        // At 5x distance, angular diameter should be 5x smaller.
        let ratio = angular_near / angular_far;
        assert!(
            (ratio - 5.0).abs() < 0.01,
            "Angular diameter ratio should be ~5.0 for 5x distance ratio, got {ratio}"
        );
    }

    #[test]
    fn test_star_type_temperatures_are_ordered() {
        let types = [
            StarType::M,
            StarType::K,
            StarType::G,
            StarType::F,
            StarType::A,
            StarType::B,
            StarType::O,
        ];
        for window in types.windows(2) {
            assert!(
                window[0].temperature_k() < window[1].temperature_k(),
                "{:?} ({} K) should be cooler than {:?} ({} K)",
                window[0],
                window[0].temperature_k(),
                window[1],
                window[1].temperature_k()
            );
        }
    }

    #[test]
    fn test_sun_uniforms_size_is_gpu_aligned() {
        // GPU uniform buffers typically require 16-byte alignment.
        let size = std::mem::size_of::<SunUniforms>();
        assert_eq!(
            size % 16,
            0,
            "SunUniforms size ({size} bytes) must be 16-byte aligned for GPU uniform buffers"
        );
    }

    #[test]
    fn test_angular_diameter_formula() {
        let sun = SunProperties {
            direction: Vec3::Z,
            physical_diameter: 100.0,
            distance: 1000.0,
            star_type: StarType::G,
            luminosity: 1.0,
        };
        let angular = sun.angular_diameter();
        // angular_diameter = diameter / distance = 100 / 1000 = 0.1 radians
        assert!(
            (angular - 0.1).abs() < 1e-6,
            "Angular diameter should be 0.1 rad, got {angular}"
        );
    }
}
```
