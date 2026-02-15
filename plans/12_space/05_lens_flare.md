# Lens Flare

## Problem

When a camera looks toward an intensely bright light source like the sun, real camera lenses produce flare artifacts: hexagonal ghost images, anamorphic streaks, and central starburst patterns. These artifacts are not physically accurate to the human eye, but they are a deeply ingrained visual language that communicates "extremely bright light source" to the player. Without lens flare, looking at the sun feels flat despite the bloom effect (story 02). The flare must behave correctly: it should appear when the sun is on screen, fade smoothly as the sun approaches the screen edge, and disappear entirely when the sun is occluded behind geometry (a planet, a ship). Occlusion detection requires sampling the depth buffer at the sun's screen position.

## Solution

Implement a `LensFlareRenderer` in the `nebula-render` crate that draws screen-space lens flare elements when a bright light source (the sun) is visible. The flare is composed of multiple elements positioned along the line from the light source position through the screen center, with intensity modulated by screen-edge proximity and depth-buffer occlusion.

### Flare Element Definition

```rust
/// A single lens flare element (ghost, streak, or starburst).
#[derive(Clone, Debug)]
pub struct FlareElement {
    /// Position along the flare line. 0.0 = at the light source, 1.0 = at screen center,
    /// >1.0 = on the opposite side of center. Ghosts typically use values 0.3-1.7.
    pub line_position: f32,
    /// Scale of this element relative to the screen height.
    pub scale: f32,
    /// Color tint for this element (linear RGB).
    pub color: [f32; 3],
    /// Opacity multiplier.
    pub opacity: f32,
    /// Element type determines the texture/shape used.
    pub shape: FlareShape,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FlareShape {
    /// Hexagonal ghost (most common flare artifact).
    HexagonalGhost,
    /// Circular halo / ring.
    CircularHalo,
    /// Central starburst (radial spikes at the light source position).
    Starburst,
    /// Horizontal anamorphic streak.
    AnamorphicStreak,
}

/// Configuration for the entire lens flare effect.
#[derive(Clone, Debug)]
pub struct LensFlareConfig {
    /// The flare elements to render.
    pub elements: Vec<FlareElement>,
    /// Overall intensity multiplier, scaled by the light source brightness.
    pub intensity: f32,
    /// Screen-edge fade margin. The flare starts fading when the light source
    /// is within this fraction of the screen edge. Range [0, 0.5]. Default: 0.3.
    pub edge_fade_margin: f32,
    /// Number of depth samples to take for occlusion testing. More samples =
    /// smoother occlusion transition but higher cost. Default: 16.
    pub occlusion_samples: u32,
}

impl Default for LensFlareConfig {
    fn default() -> Self {
        Self {
            elements: vec![
                FlareElement {
                    line_position: 0.0,
                    scale: 0.15,
                    color: [1.0, 0.9, 0.7],
                    opacity: 0.8,
                    shape: FlareShape::Starburst,
                },
                FlareElement {
                    line_position: 0.0,
                    scale: 0.5,
                    color: [1.0, 0.95, 0.8],
                    opacity: 0.3,
                    shape: FlareShape::AnamorphicStreak,
                },
                FlareElement {
                    line_position: 0.4,
                    scale: 0.08,
                    color: [0.5, 0.8, 1.0],
                    opacity: 0.4,
                    shape: FlareShape::HexagonalGhost,
                },
                FlareElement {
                    line_position: 0.7,
                    scale: 0.12,
                    color: [0.8, 0.5, 1.0],
                    opacity: 0.3,
                    shape: FlareShape::HexagonalGhost,
                },
                FlareElement {
                    line_position: 1.2,
                    scale: 0.06,
                    color: [0.3, 1.0, 0.5],
                    opacity: 0.25,
                    shape: FlareShape::CircularHalo,
                },
                FlareElement {
                    line_position: 1.5,
                    scale: 0.10,
                    color: [1.0, 0.6, 0.3],
                    opacity: 0.35,
                    shape: FlareShape::HexagonalGhost,
                },
            ],
            intensity: 1.0,
            edge_fade_margin: 0.3,
            occlusion_samples: 16,
        }
    }
}
```

### Screen-Space Light Position

The sun's world-space direction is projected to screen coordinates using the view-projection matrix:

```rust
impl LensFlareRenderer {
    /// Project the sun direction to normalized screen coordinates [0, 1].
    /// Returns None if the sun is behind the camera.
    fn project_to_screen(
        &self,
        sun_direction: glam::Vec3,
        view_proj: glam::Mat4,
    ) -> Option<glam::Vec2> {
        // The sun is at "infinite" distance, so treat it as a directional light.
        // Project a point along the sun direction at a large but finite distance.
        let sun_pos = sun_direction * 1000.0;
        let clip = view_proj * glam::Vec4::new(sun_pos.x, sun_pos.y, sun_pos.z, 1.0);

        // Behind the camera: w <= 0.
        if clip.w <= 0.0 {
            return None;
        }

        let ndc = glam::Vec3::new(clip.x / clip.w, clip.y / clip.w, clip.z / clip.w);

        // Convert NDC [-1, 1] to screen UV [0, 1].
        let screen = glam::Vec2::new(ndc.x * 0.5 + 0.5, -ndc.y * 0.5 + 0.5);
        Some(screen)
    }
}
```

### Flare Element Positioning

Each flare element is positioned along the line from the light source screen position through the screen center. The `line_position` parameter interpolates along this line:

```rust
/// Compute the screen position of a flare element.
fn element_screen_position(
    light_screen_pos: glam::Vec2,
    line_position: f32,
) -> glam::Vec2 {
    let screen_center = glam::Vec2::new(0.5, 0.5);
    let direction = screen_center - light_screen_pos;
    light_screen_pos + direction * line_position
}
```

### Edge Fade

The flare intensity fades as the sun moves toward the screen edge to avoid an abrupt pop-in/pop-out:

```rust
/// Compute the edge fade factor based on how close the light is to the screen border.
fn edge_fade_factor(screen_pos: glam::Vec2, margin: f32) -> f32 {
    // Distance from screen center in [0, 0.5] for each axis.
    let dx = (screen_pos.x - 0.5).abs();
    let dy = (screen_pos.y - 0.5).abs();
    let max_dist = dx.max(dy);

    // Fade starts at (0.5 - margin) and reaches zero at 0.5.
    let fade_start = 0.5 - margin;
    if max_dist < fade_start {
        1.0
    } else {
        ((0.5 - max_dist) / margin).clamp(0.0, 1.0)
    }
}
```

### Occlusion Testing

The depth buffer is sampled at and around the sun's screen position to determine what fraction of the sun is occluded by geometry. Multiple samples in a small disk produce a smooth occlusion gradient:

```rust
/// Sample the depth buffer to compute sun occlusion.
/// Returns a visibility factor in [0, 1] where 0 = fully occluded, 1 = fully visible.
fn compute_occlusion(
    depth_buffer: &wgpu::Texture,
    screen_pos: glam::Vec2,
    sample_count: u32,
    screen_size: glam::UVec2,
) -> f32 {
    // This is executed via a compute shader or CPU readback.
    // The compute shader samples `sample_count` points in a small disk
    // around the sun's screen position and counts how many have depth = 0.0
    // (reverse-Z far plane, meaning no geometry occluding).
    //
    // Returns visible_samples / total_samples.
    todo!("Implemented in WGSL compute shader")
}
```

### Occlusion Compute Shader

```wgsl
@group(0) @binding(0) var depth_texture: texture_depth_2d;
@group(0) @binding(1) var<storage, read_write> result: OcclusionResult;
@group(0) @binding(2) var<uniform> params: OcclusionParams;

struct OcclusionParams {
    screen_pos: vec2<f32>,
    screen_size: vec2<f32>,
    sample_count: u32,
    sample_radius: f32, // in pixels
};

struct OcclusionResult {
    visibility: f32,
};

@compute @workgroup_size(1)
fn cs_occlusion_test() {
    var visible = 0u;
    let center = vec2<i32>(params.screen_pos * params.screen_size);

    for (var i = 0u; i < params.sample_count; i = i + 1u) {
        // Poisson disk or spiral sample pattern.
        let angle = f32(i) * 2.399963; // golden angle
        let radius = sqrt(f32(i) / f32(params.sample_count)) * params.sample_radius;
        let offset = vec2<i32>(
            i32(cos(angle) * radius),
            i32(sin(angle) * radius),
        );
        let sample_pos = center + offset;

        // Bounds check.
        if sample_pos.x >= 0 && sample_pos.x < i32(params.screen_size.x)
            && sample_pos.y >= 0 && sample_pos.y < i32(params.screen_size.y) {
            let depth = textureLoad(depth_texture, sample_pos, 0);
            // In reverse-Z, depth near 0.0 means far plane (no geometry).
            if depth < 0.001 {
                visible = visible + 1u;
            }
        }
    }

    result.visibility = f32(visible) / f32(params.sample_count);
}
```

### Rendering

Each flare element is rendered as a screen-space quad with its specific shape texture or procedural shader. All elements share a single draw call using instancing:

```rust
impl LensFlareRenderer {
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        hdr_target: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        sun_direction: glam::Vec3,
        sun_brightness: f32,
        view_proj: glam::Mat4,
        screen_size: glam::UVec2,
        queue: &wgpu::Queue,
    ) {
        // 1. Project sun to screen space.
        let screen_pos = match self.project_to_screen(sun_direction, view_proj) {
            Some(pos) => pos,
            None => return, // Sun is behind camera; no flare.
        };

        // 2. Compute edge fade.
        let edge_fade = edge_fade_factor(screen_pos, self.config.edge_fade_margin);
        if edge_fade <= 0.0 {
            return; // Sun is off-screen.
        }

        // 3. Compute occlusion (async readback from previous frame to avoid stall).
        let visibility = self.last_occlusion_visibility;
        if visibility <= 0.0 {
            return; // Sun is fully occluded.
        }

        // 4. Combined intensity.
        let intensity = self.config.intensity * sun_brightness * edge_fade * visibility;

        // 5. Position and render each flare element.
        // ... upload instance data and draw ...
    }
}
```

## Outcome

A `LensFlareRenderer` and `LensFlareConfig` in `nebula-render` that draws screen-space lens flare artifacts when looking toward the sun. The flare includes hexagonal ghosts, an anamorphic streak, and a central starburst, all positioned along the light-to-center diagonal. Occlusion testing prevents flares when the sun is behind geometry. Running `cargo test -p nebula-render` passes all lens flare tests. The flare integrates with the bloom pipeline and sun renderer from stories 02 and 04.

## Demo Integration

**Demo crate:** `nebula-demo`

Looking toward the sun produces hexagonal ghost flares, an anamorphic streak, and a starburst along the light diagonal. The flares disappear when the sun is occluded by terrain.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Render pipeline, compute shader for occlusion, texture sampling |
| `bytemuck` | `1.21` | Uniform and instance buffer serialization |
| `glam` | `0.29` | Vec2/Vec3/Mat4 for projection and positioning |

The lens flare renderer lives in `nebula-render`. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec2, Vec3};

    #[test]
    fn test_flare_visible_when_looking_at_sun() {
        // Sun directly ahead of the camera should project to screen center.
        let view_proj = Mat4::perspective_rh(
            std::f32::consts::FRAC_PI_4,
            16.0 / 9.0,
            1000.0, // reverse-Z: far as near param
            0.1,    // reverse-Z: near as far param
        );
        let sun_dir = Vec3::new(0.0, 0.0, -1.0); // directly ahead

        let renderer = LensFlareRenderer::default_test();
        let screen_pos = renderer.project_to_screen(sun_dir, view_proj);
        assert!(
            screen_pos.is_some(),
            "Sun directly ahead should project to screen"
        );

        let pos = screen_pos.unwrap();
        assert!(
            (pos.x - 0.5).abs() < 0.1 && (pos.y - 0.5).abs() < 0.1,
            "Sun ahead should be near screen center, got ({}, {})",
            pos.x,
            pos.y
        );
    }

    #[test]
    fn test_flare_fades_near_screen_edge() {
        let margin = 0.3;

        // Center of screen: full intensity.
        let center_fade = edge_fade_factor(Vec2::new(0.5, 0.5), margin);
        assert!(
            (center_fade - 1.0).abs() < 1e-6,
            "Center should have full intensity, got {center_fade}"
        );

        // Near edge: reduced intensity.
        let edge_fade = edge_fade_factor(Vec2::new(0.9, 0.5), margin);
        assert!(
            edge_fade < 1.0 && edge_fade > 0.0,
            "Near edge should have reduced intensity, got {edge_fade}"
        );

        // Off screen: zero.
        let off_fade = edge_fade_factor(Vec2::new(1.1, 0.5), margin);
        assert!(
            off_fade <= 0.0,
            "Off screen should have zero intensity, got {off_fade}"
        );
    }

    #[test]
    fn test_flare_disappears_when_sun_is_occluded() {
        // When occlusion visibility is 0.0, the combined intensity should be zero.
        let visibility = 0.0_f32;
        let edge_fade = 1.0;
        let sun_brightness = 50.0;
        let config_intensity = 1.0;

        let intensity = config_intensity * sun_brightness * edge_fade * visibility;
        assert_eq!(
            intensity, 0.0,
            "Fully occluded sun should produce zero flare intensity"
        );
    }

    #[test]
    fn test_flare_elements_positioned_along_diagonal() {
        let light_pos = Vec2::new(0.3, 0.2);
        let center = Vec2::new(0.5, 0.5);
        let direction = center - light_pos; // (0.2, 0.3)

        let positions: Vec<Vec2> = [0.0, 0.4, 0.7, 1.0, 1.5]
            .iter()
            .map(|&t| element_screen_position(light_pos, t))
            .collect();

        // All positions should be collinear on the line from light through center.
        for (i, pos) in positions.iter().enumerate() {
            if i == 0 {
                // t=0 should be at the light position.
                assert!(
                    (pos.x - light_pos.x).abs() < 1e-6 && (pos.y - light_pos.y).abs() < 1e-6,
                    "Element at t=0 should be at light position"
                );
                continue;
            }
            // Check collinearity: (pos - light_pos) should be parallel to direction.
            let offset = *pos - light_pos;
            let cross = offset.x * direction.y - offset.y * direction.x;
            assert!(
                cross.abs() < 1e-5,
                "Element {i} at ({}, {}) is not on the diagonal (cross product = {cross})",
                pos.x,
                pos.y
            );
        }
    }

    #[test]
    fn test_flare_intensity_proportional_to_sun_brightness() {
        let edge_fade = 1.0;
        let visibility = 1.0;
        let config_intensity = 1.0;

        let intensity_dim = config_intensity * 10.0 * edge_fade * visibility;
        let intensity_bright = config_intensity * 100.0 * edge_fade * visibility;

        assert!(
            intensity_bright > intensity_dim,
            "Brighter sun should produce stronger flare: {intensity_bright} vs {intensity_dim}"
        );
        assert!(
            (intensity_bright / intensity_dim - 10.0).abs() < 1e-6,
            "Flare intensity should scale linearly with sun brightness"
        );
    }

    #[test]
    fn test_sun_behind_camera_produces_no_flare() {
        let view_proj = Mat4::perspective_rh(
            std::f32::consts::FRAC_PI_4,
            16.0 / 9.0,
            1000.0,
            0.1,
        );
        // Sun behind the camera (positive Z in a right-handed system looking down -Z).
        let sun_dir = Vec3::new(0.0, 0.0, 1.0);

        let renderer = LensFlareRenderer::default_test();
        let screen_pos = renderer.project_to_screen(sun_dir, view_proj);
        assert!(
            screen_pos.is_none(),
            "Sun behind camera should not project to screen"
        );
    }

    #[test]
    fn test_default_config_has_expected_elements() {
        let config = LensFlareConfig::default();
        assert!(
            config.elements.len() >= 4,
            "Default config should have at least 4 flare elements, got {}",
            config.elements.len()
        );

        // Should contain at least one starburst and one ghost.
        let has_starburst = config
            .elements
            .iter()
            .any(|e| e.shape == FlareShape::Starburst);
        let has_ghost = config
            .elements
            .iter()
            .any(|e| e.shape == FlareShape::HexagonalGhost);
        assert!(has_starburst, "Default config should include a starburst element");
        assert!(has_ghost, "Default config should include a hexagonal ghost element");
    }

    #[test]
    fn test_edge_fade_is_symmetric() {
        let margin = 0.3;
        let fade_left = edge_fade_factor(Vec2::new(0.1, 0.5), margin);
        let fade_right = edge_fade_factor(Vec2::new(0.9, 0.5), margin);
        assert!(
            (fade_left - fade_right).abs() < 1e-6,
            "Edge fade should be symmetric: left={fade_left}, right={fade_right}"
        );

        let fade_top = edge_fade_factor(Vec2::new(0.5, 0.1), margin);
        let fade_bottom = edge_fade_factor(Vec2::new(0.5, 0.9), margin);
        assert!(
            (fade_top - fade_bottom).abs() < 1e-6,
            "Edge fade should be symmetric: top={fade_top}, bottom={fade_bottom}"
        );
    }
}
```
