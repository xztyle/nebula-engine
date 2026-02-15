# Planet Impostors

## Problem

A solar system contains multiple planets, moons, and other celestial bodies. When a planet is extremely distant -- millions or billions of kilometers away -- rendering it with full geometry (even the orbital sphere from story 06) is wasteful. An icosphere with 40,000 triangles is overkill for an object that occupies 3x3 pixels on screen. At interstellar distances, even the sphere mesh is more geometry than the rasterizer can meaningfully use. The engine needs a far cheaper representation: a billboard impostor. An impostor is a textured quad (2 triangles) that always faces the camera, displaying a pre-rendered snapshot of the planet. The snapshot is updated when the viewing angle changes significantly. This reduces the rendering cost of a distant planet from thousands of triangles to exactly 2, while maintaining visual fidelity at the pixel sizes where the planet is rendered.

## Solution

### Impostor Data Structure

Each distant planet has an impostor that stores a pre-rendered texture and the metadata needed to decide when to re-render:

```rust
use glam::{Vec3, Mat4};

/// An impostor representation of a distant planet.
pub struct PlanetImpostor {
    /// The rendered snapshot texture.
    pub texture: wgpu::Texture,
    pub texture_view: wgpu::TextureView,
    /// Resolution of the snapshot (square). Typically 64-256 pixels.
    pub resolution: u32,
    /// The view direction (camera-to-planet normalized) at which the snapshot was captured.
    pub captured_view_dir: Vec3,
    /// The sun direction at capture time (affects lighting/atmosphere).
    pub captured_sun_dir: Vec3,
    /// Angular threshold (radians) before the snapshot needs updating.
    pub update_threshold: f32,
    /// Whether the impostor texture needs re-rendering.
    pub dirty: bool,
}

/// Configuration for when to use impostors vs. geometry.
pub struct ImpostorConfig {
    /// Distance (in meters) beyond which the orbital sphere is replaced by an impostor.
    pub impostor_distance: f64,
    /// Transition band (in meters) for blending between sphere and impostor.
    pub transition_band: f64,
    /// Angular change (radians) in view direction that triggers a texture update.
    pub angle_threshold: f32,
    /// Impostor texture resolution in pixels (square).
    pub texture_resolution: u32,
}

impl Default for ImpostorConfig {
    fn default() -> Self {
        Self {
            impostor_distance: 1_000_000_000.0, // 1 million km
            transition_band: 100_000_000.0,      // 100k km
            angle_threshold: 0.05,               // ~2.9 degrees
            texture_resolution: 128,
        }
    }
}
```

### Snapshot Rendering

To capture the impostor texture, render the planet (orbital sphere + atmosphere) to an off-screen render target from the current camera direction:

```rust
impl PlanetImpostor {
    /// Render the planet to the impostor texture.
    ///
    /// Uses a temporary camera positioned far from the planet, looking at it,
    /// and renders the orbital sphere + atmosphere to the impostor texture.
    pub fn capture(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        planet: &PlanetState,
        view_dir: Vec3,
        sun_dir: Vec3,
        orbital_renderer: &OrbitalRenderer,
        atmosphere_renderer: &AtmosphereRenderer,
    ) {
        let mut encoder = device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor { label: Some("impostor-capture") },
        );

        // Set up a camera looking at the planet from the view direction.
        let camera_pos = planet.center_f32() - view_dir * planet.radius_f32() * 5.0;
        let vp = build_impostor_camera(camera_pos, planet.center_f32(), planet.radius_f32());

        // Render the orbital sphere to the impostor texture.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("impostor-capture-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.texture_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });

            orbital_renderer.render_to_pass(&mut pass, &vp, planet);
        }

        // Atmosphere pass over the same render target.
        atmosphere_renderer.render_to_texture(&mut encoder, &self.texture_view, &vp, planet);

        queue.submit(std::iter::once(encoder.finish()));

        self.captured_view_dir = view_dir;
        self.captured_sun_dir = sun_dir;
        self.dirty = false;
    }

    /// Check if the impostor texture needs re-rendering.
    pub fn needs_update(&self, current_view_dir: Vec3, current_sun_dir: Vec3) -> bool {
        if self.dirty {
            return true;
        }
        let view_angle_change = self.captured_view_dir.angle_between(current_view_dir);
        let sun_angle_change = self.captured_sun_dir.angle_between(current_sun_dir);
        view_angle_change > self.update_threshold || sun_angle_change > self.update_threshold
    }
}
```

### Billboard Rendering

The impostor is rendered as a camera-facing quad. The quad's size is computed from the planet's angular diameter at the current distance:

```rust
/// Compute the world-space size of the impostor quad.
///
/// The quad should subtend the same visual angle as the planet.
pub fn impostor_quad_size(planet_radius: f64, distance: f64) -> f32 {
    // Angular diameter = 2 * arcsin(radius / distance).
    // Quad half-size at distance 1 = tan(angular_radius).
    let angular_radius = (planet_radius / distance).asin();
    // Scale by distance to get world-space half-size, then multiply by 2.
    // The quad is placed at the planet's position, so we need the
    // visual size at that distance.
    (angular_radius.tan() * distance * 2.0) as f32
}

/// Generate billboard vertices for a camera-facing quad.
pub fn billboard_vertices(
    planet_center: Vec3,
    camera_right: Vec3,
    camera_up: Vec3,
    half_size: f32,
) -> [ImpostorVertex; 4] {
    let r = camera_right * half_size;
    let u = camera_up * half_size;
    [
        ImpostorVertex { position: (planet_center - r - u).into(), uv: [0.0, 1.0] },
        ImpostorVertex { position: (planet_center + r - u).into(), uv: [1.0, 1.0] },
        ImpostorVertex { position: (planet_center + r + u).into(), uv: [1.0, 0.0] },
        ImpostorVertex { position: (planet_center - r + u).into(), uv: [0.0, 0.0] },
    ]
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ImpostorVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
}

const IMPOSTOR_INDICES: [u16; 6] = [0, 1, 2, 0, 2, 3];
```

### WGSL Shader

```wgsl
@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var impostor_texture: texture_2d<f32>;
@group(1) @binding(1) var impostor_sampler: sampler;

@vertex
fn vs_impostor(in: ImpostorVertexInput) -> ImpostorVertexOutput {
    var out: ImpostorVertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_impostor(in: ImpostorVertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(impostor_texture, impostor_sampler, in.uv);
    // Discard transparent pixels (background of the impostor).
    if color.a < 0.01 {
        discard;
    }
    return color;
}
```

### Transition from Geometry to Impostor

The LOD system determines when to switch. During the transition band, both the orbital sphere and the impostor are rendered, with the sphere fading out and the impostor fading in:

```rust
pub fn select_planet_representation(
    distance: f64,
    config: &ImpostorConfig,
) -> PlanetRepresentation {
    if distance < config.impostor_distance {
        PlanetRepresentation::Geometry
    } else if distance < config.impostor_distance + config.transition_band {
        let t = ((distance - config.impostor_distance) / config.transition_band) as f32;
        PlanetRepresentation::Blending { impostor_alpha: t }
    } else {
        PlanetRepresentation::Impostor
    }
}

#[derive(Debug, Clone, Copy)]
pub enum PlanetRepresentation {
    Geometry,
    Blending { impostor_alpha: f32 },
    Impostor,
}
```

## Outcome

The `nebula-planet` crate exports `PlanetImpostor`, `ImpostorConfig`, `ImpostorVertex`, `impostor_quad_size()`, `billboard_vertices()`, and `select_planet_representation()`. Extremely distant planets render as 2-triangle billboard quads with a pre-rendered texture, reducing the per-planet cost to near zero. The impostor texture is updated when the view angle changes by more than the configured threshold. Transition from geometry to impostor is smooth with an alpha blend over the transition band.

## Demo Integration

**Demo crate:** `nebula-demo`

Extremely distant planets appear as small billboard sprites. Approaching a planet, the impostor smoothly transitions to full geometry rendering.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Off-screen render target, texture creation, billboard rendering |
| `glam` | `0.29` | Billboard orientation, angular size computation |
| `bytemuck` | `1.21` | Vertex buffer serialization |

Internal dependencies: `nebula-render`, `nebula-planet` (orbital renderer, atmosphere). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn test_impostor_replaces_geometry_at_extreme_distance() {
        let config = ImpostorConfig::default();

        // At 10 billion km, should be an impostor.
        let result = select_planet_representation(10_000_000_000_000.0, &config);
        assert!(
            matches!(result, PlanetRepresentation::Impostor),
            "Extreme distance should use impostor, got {result:?}"
        );

        // At 100 km, should be geometry.
        let result = select_planet_representation(100_000.0, &config);
        assert!(
            matches!(result, PlanetRepresentation::Geometry),
            "Close distance should use geometry, got {result:?}"
        );
    }

    #[test]
    fn test_impostor_texture_updates_on_view_change() {
        let mut impostor = PlanetImpostor {
            texture: create_test_texture(128),
            texture_view: create_test_texture_view(128),
            resolution: 128,
            captured_view_dir: Vec3::Z,
            captured_sun_dir: Vec3::Y,
            update_threshold: 0.05,
            dirty: false,
        };

        // Same view direction: no update needed.
        assert!(
            !impostor.needs_update(Vec3::Z, Vec3::Y),
            "Same view direction should not need update"
        );

        // Slightly different view: no update needed (below threshold).
        let small_change = Vec3::new(0.01, 0.0, 1.0).normalize();
        assert!(
            !impostor.needs_update(small_change, Vec3::Y),
            "Small view change should not need update"
        );

        // Large view change: update needed.
        let large_change = Vec3::new(1.0, 0.0, 1.0).normalize(); // 45 degrees
        assert!(
            impostor.needs_update(large_change, Vec3::Y),
            "Large view change should trigger update"
        );

        // Sun direction change: update needed.
        let new_sun = Vec3::new(1.0, 1.0, 0.0).normalize();
        assert!(
            impostor.needs_update(Vec3::Z, new_sun),
            "Sun direction change should trigger update"
        );
    }

    #[test]
    fn test_impostor_correctly_sized_for_distance() {
        let planet_radius = 6_371_000.0; // Earth-like, meters

        // At 10x radius distance, the planet should have a visible angular size.
        let size_near = impostor_quad_size(planet_radius, planet_radius * 10.0);
        let size_far = impostor_quad_size(planet_radius, planet_radius * 100.0);
        let size_very_far = impostor_quad_size(planet_radius, planet_radius * 1000.0);

        // Size should decrease with distance.
        assert!(
            size_near > size_far,
            "Near size ({size_near}) should be larger than far ({size_far})"
        );
        assert!(
            size_far > size_very_far,
            "Far size ({size_far}) should be larger than very far ({size_very_far})"
        );

        // At 10x radius, the angular diameter is about 2*arcsin(1/10) ≈ 0.2 rad,
        // so the quad size should be approximately 2 * tan(0.1) * 10R ≈ 12.8M meters.
        let expected_near = 2.0 * (planet_radius / (planet_radius * 10.0)).asin().tan()
            * planet_radius * 10.0;
        assert!(
            ((size_near as f64) - expected_near).abs() / expected_near < 0.01,
            "Near size {size_near} should match expected {expected_near}"
        );
    }

    #[test]
    fn test_geometry_to_impostor_transition_is_smooth() {
        let config = ImpostorConfig::default();
        let start = config.impostor_distance - config.transition_band;
        let end = config.impostor_distance + config.transition_band * 2.0;
        let steps = 100;

        let mut prev_alpha = 0.0_f32;
        for i in 0..=steps {
            let distance = start + (end - start) * (i as f64 / steps as f64);
            let rep = select_planet_representation(distance, &config);
            let alpha = match rep {
                PlanetRepresentation::Geometry => 0.0,
                PlanetRepresentation::Blending { impostor_alpha } => impostor_alpha,
                PlanetRepresentation::Impostor => 1.0,
            };

            // Alpha should be monotonically non-decreasing with distance.
            assert!(
                alpha >= prev_alpha - 1e-6,
                "Alpha decreased at distance {distance}: {prev_alpha} -> {alpha}"
            );
            prev_alpha = alpha;
        }
    }

    #[test]
    fn test_impostor_is_two_triangles() {
        // The impostor quad should consist of exactly 4 vertices and 6 indices (2 triangles).
        let vertices = billboard_vertices(
            Vec3::ZERO,
            Vec3::X,
            Vec3::Y,
            1.0,
        );
        assert_eq!(vertices.len(), 4, "Impostor should have 4 vertices");
        assert_eq!(IMPOSTOR_INDICES.len(), 6, "Impostor should have 6 indices (2 triangles)");

        // Verify the indices form two triangles.
        assert_eq!(IMPOSTOR_INDICES[0..3], [0, 1, 2]);
        assert_eq!(IMPOSTOR_INDICES[3..6], [0, 2, 3]);
    }
}
```
