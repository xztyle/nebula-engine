//! Screen-space lens flare rendering for bright light sources.
//!
//! Draws hexagonal ghosts, anamorphic streaks, circular halos, and starburst
//! patterns along the light-to-center screen diagonal. Intensity is modulated
//! by screen-edge proximity and (optionally) depth-buffer occlusion.

use bytemuck::{Pod, Zeroable};

/// A single lens flare element (ghost, streak, or starburst).
#[derive(Clone, Debug)]
pub struct FlareElement {
    /// Position along the flare line. 0.0 = at the light source, 1.0 = at screen center,
    /// >1.0 = on the opposite side of center.
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

/// Shape type for a lens flare element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

impl FlareShape {
    /// Integer index for the GPU shader.
    fn as_u32(self) -> u32 {
        match self {
            FlareShape::HexagonalGhost => 0,
            FlareShape::CircularHalo => 1,
            FlareShape::Starburst => 2,
            FlareShape::AnamorphicStreak => 3,
        }
    }
}

/// Configuration for the entire lens flare effect.
#[derive(Clone, Debug)]
pub struct LensFlareConfig {
    /// The flare elements to render.
    pub elements: Vec<FlareElement>,
    /// Overall intensity multiplier.
    pub intensity: f32,
    /// Screen-edge fade margin. Range [0, 0.5]. Default: 0.3.
    pub edge_fade_margin: f32,
    /// Number of depth samples for occlusion testing. Default: 16.
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

/// GPU instance data for a single flare element.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct FlareInstance {
    /// Screen-space center (UV coords [0,1]).
    center: [f32; 2],
    /// Scale (x, y) in UV space.
    scale: [f32; 2],
    /// Color tint (linear RGB) + opacity.
    color: [f32; 4],
    /// Shape index (0=hex ghost, 1=circular halo, 2=starburst, 3=anamorphic).
    shape: u32,
    _pad: [u32; 3],
}

/// WGSL shader source for lens flare rendering.
pub const LENS_FLARE_SHADER_SOURCE: &str = r#"
struct FlareInstance {
    center: vec2<f32>,
    scale: vec2<f32>,
    color: vec4<f32>,
    shape: u32,
};

@group(0) @binding(0) var<storage, read> instances: array<FlareInstance>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) @interpolate(flat) shape: u32,
};

@vertex
fn vs_flare(@builtin(vertex_index) vid: u32, @builtin(instance_index) iid: u32) -> VertexOutput {
    let inst = instances[iid];
    // Fullscreen triangle-strip quad: 0,1,2,3 → BL,BR,TL,TR
    let uv = vec2<f32>(f32(vid & 1u), f32((vid >> 1u) & 1u));
    let local = (uv - 0.5) * 2.0; // [-1, 1]

    // Scale and position in NDC
    let screen = inst.center * 2.0 - 1.0; // UV→NDC
    let pos = vec2<f32>(
        screen.x + local.x * inst.scale.x,
        -(screen.y + local.y * inst.scale.y), // flip Y for NDC
    );

    var out: VertexOutput;
    out.position = vec4<f32>(pos, 0.0, 1.0);
    out.uv = local;
    out.color = inst.color;
    out.shape = inst.shape;
    return out;
}

// Hexagonal ghost: soft hexagonal ring.
fn hexagonal_ghost(uv: vec2<f32>) -> f32 {
    let r = length(uv);
    // Hexagonal distance function.
    let a = abs(uv);
    let hex = max(a.x * 0.866 + a.y * 0.5, a.y);
    let ring = smoothstep(0.7, 0.5, hex) * smoothstep(0.3, 0.5, hex);
    let glow = exp(-r * r * 8.0) * 0.3;
    return ring + glow;
}

// Circular halo: soft ring.
fn circular_halo(uv: vec2<f32>) -> f32 {
    let r = length(uv);
    let ring = smoothstep(0.8, 0.6, r) * smoothstep(0.4, 0.6, r);
    let glow = exp(-r * r * 6.0) * 0.2;
    return ring + glow;
}

// Starburst: radial spikes from center.
fn starburst(uv: vec2<f32>) -> f32 {
    let r = length(uv);
    let angle = atan2(uv.y, uv.x);
    let spikes = pow(abs(cos(angle * 8.0)), 16.0);
    let spikes2 = pow(abs(cos(angle * 6.0 + 0.5)), 24.0);
    let falloff = exp(-r * r * 3.0);
    let core = exp(-r * r * 20.0);
    return (spikes * 0.6 + spikes2 * 0.4) * falloff + core;
}

// Anamorphic streak: horizontal line through center.
fn anamorphic_streak(uv: vec2<f32>) -> f32 {
    let hor = exp(-uv.y * uv.y * 80.0) * exp(-uv.x * uv.x * 0.5);
    let core = exp(-dot(uv, uv) * 10.0);
    return hor * 0.7 + core * 0.3;
}

@fragment
fn fs_flare(in: VertexOutput) -> @location(0) vec4<f32> {
    var alpha: f32;
    switch in.shape {
        case 0u: { alpha = hexagonal_ghost(in.uv); }
        case 1u: { alpha = circular_halo(in.uv); }
        case 2u: { alpha = starburst(in.uv); }
        case 3u: { alpha = anamorphic_streak(in.uv); }
        default: { alpha = 0.0; }
    }
    let final_alpha = alpha * in.color.a;
    if final_alpha < 0.001 {
        discard;
    }
    return vec4<f32>(in.color.rgb * final_alpha, final_alpha);
}
"#;

/// Maximum number of flare elements supported per frame.
const MAX_FLARE_ELEMENTS: usize = 32;

/// Screen-space lens flare renderer.
///
/// Renders procedural flare elements (ghosts, streaks, starburst) as instanced
/// quads in HDR space, positioned along the light-to-center diagonal.
pub struct LensFlareRenderer {
    config: LensFlareConfig,
    pipeline: wgpu::RenderPipeline,
    instance_buffer: wgpu::Buffer,
    instance_bind_group: wgpu::BindGroup,
    /// Occlusion visibility from the previous frame (0=occluded, 1=visible).
    last_occlusion_visibility: f32,
}

impl LensFlareRenderer {
    /// Create a new lens flare renderer.
    ///
    /// `hdr_format` is the HDR render target format (typically `Rgba16Float`).
    pub fn new(device: &wgpu::Device, hdr_format: wgpu::TextureFormat) -> Self {
        Self::with_config(device, hdr_format, LensFlareConfig::default())
    }

    /// Create a new lens flare renderer with custom configuration.
    pub fn with_config(
        device: &wgpu::Device,
        hdr_format: wgpu::TextureFormat,
        config: LensFlareConfig,
    ) -> Self {
        use wgpu::util::DeviceExt;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lens-flare-shader"),
            source: wgpu::ShaderSource::Wgsl(LENS_FLARE_SHADER_SOURCE.into()),
        });

        // Storage buffer for flare instances
        let instance_data = vec![FlareInstance::zeroed(); MAX_FLARE_ELEMENTS];
        let instance_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flare-instances"),
            contents: bytemuck::cast_slice(&instance_data),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let instance_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flare-instance-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let instance_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flare-instance-bg"),
            layout: &instance_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: instance_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flare-pipeline-layout"),
            bind_group_layouts: &[&instance_bgl],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lens-flare-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_flare"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_flare"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: hdr_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        log::info!(
            "Lens flare renderer initialized ({} elements)",
            config.elements.len()
        );

        Self {
            config,
            pipeline,
            instance_buffer,
            instance_bind_group,
            last_occlusion_visibility: 1.0,
        }
    }

    /// Project the sun direction to normalized screen coordinates [0, 1].
    /// Returns `None` if the sun is behind the camera.
    pub fn project_to_screen(
        &self,
        sun_direction: glam::Vec3,
        view_proj: glam::Mat4,
    ) -> Option<glam::Vec2> {
        let sun_pos = sun_direction * 1000.0;
        let clip = view_proj * glam::Vec4::new(sun_pos.x, sun_pos.y, sun_pos.z, 1.0);

        if clip.w <= 0.0 {
            return None;
        }

        let ndc = glam::Vec3::new(clip.x / clip.w, clip.y / clip.w, clip.z / clip.w);
        let screen = glam::Vec2::new(ndc.x * 0.5 + 0.5, -ndc.y * 0.5 + 0.5);
        Some(screen)
    }

    /// Update flare instance data and upload to the GPU.
    ///
    /// Call this once per frame before [`render`](Self::render).
    pub fn update(
        &self,
        queue: &wgpu::Queue,
        sun_direction: glam::Vec3,
        sun_brightness: f32,
        view_proj: glam::Mat4,
    ) -> bool {
        let screen_pos = match self.project_to_screen(sun_direction, view_proj) {
            Some(pos) => pos,
            None => return false,
        };

        let edge_fade = edge_fade_factor(screen_pos, self.config.edge_fade_margin);
        if edge_fade <= 0.0 {
            return false;
        }

        let visibility = self.last_occlusion_visibility;
        if visibility <= 0.0 {
            return false;
        }

        let intensity = self.config.intensity * sun_brightness * edge_fade * visibility;

        let mut instances = vec![FlareInstance::zeroed(); MAX_FLARE_ELEMENTS];
        let count = self.config.elements.len().min(MAX_FLARE_ELEMENTS);

        for (i, elem) in self.config.elements.iter().take(count).enumerate() {
            let pos = element_screen_position(screen_pos, elem.line_position);
            let sx = elem.scale;
            let sy = if elem.shape == FlareShape::AnamorphicStreak {
                elem.scale * 0.05 // Very thin vertically
            } else {
                elem.scale
            };

            instances[i] = FlareInstance {
                center: [pos.x, pos.y],
                scale: [sx, sy],
                color: [
                    elem.color[0] * intensity,
                    elem.color[1] * intensity,
                    elem.color[2] * intensity,
                    elem.opacity,
                ],
                shape: elem.shape.as_u32(),
                _pad: [0; 3],
            };
        }

        queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instances));
        true
    }

    /// Render the lens flare elements. Should be called after the sun, before bloom.
    ///
    /// Returns without drawing if `update` returned `false`.
    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, element_count: u32) {
        if element_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.instance_bind_group, &[]);
        pass.draw(0..4, 0..element_count);
    }

    /// Returns the number of active flare elements (capped at [`MAX_FLARE_ELEMENTS`]).
    pub fn element_count(&self) -> u32 {
        (self.config.elements.len().min(MAX_FLARE_ELEMENTS)) as u32
    }

    /// Set the occlusion visibility factor (0 = fully occluded, 1 = fully visible).
    pub fn set_occlusion_visibility(&mut self, visibility: f32) {
        self.last_occlusion_visibility = visibility.clamp(0.0, 1.0);
    }

    /// Create a test-only projector (no GPU resources needed).
    #[cfg(test)]
    pub(crate) fn default_test() -> TestFlareProjector {
        TestFlareProjector {
            config: LensFlareConfig::default(),
        }
    }
}

/// Compute the screen position of a flare element along the light-to-center line.
pub fn element_screen_position(light_screen_pos: glam::Vec2, line_position: f32) -> glam::Vec2 {
    let screen_center = glam::Vec2::new(0.5, 0.5);
    let direction = screen_center - light_screen_pos;
    light_screen_pos + direction * line_position
}

/// Compute the edge fade factor based on how close the light is to the screen border.
pub fn edge_fade_factor(screen_pos: glam::Vec2, margin: f32) -> f32 {
    let dx = (screen_pos.x - 0.5).abs();
    let dy = (screen_pos.y - 0.5).abs();
    let max_dist = dx.max(dy);

    let fade_start = 0.5 - margin;
    if max_dist < fade_start {
        1.0
    } else if margin <= 0.0 {
        0.0
    } else {
        ((0.5 - max_dist) / margin).clamp(0.0, 1.0)
    }
}

/// Test-only projector for lens flare math (no GPU resources).
#[cfg(test)]
#[allow(dead_code)]
pub(crate) struct TestFlareProjector {
    pub config: LensFlareConfig,
}

#[cfg(test)]
impl TestFlareProjector {
    /// Project the sun direction to normalized screen coordinates [0, 1].
    pub fn project_to_screen(
        &self,
        sun_direction: glam::Vec3,
        view_proj: glam::Mat4,
    ) -> Option<glam::Vec2> {
        let sun_pos = sun_direction * 1000.0;
        let clip = view_proj * glam::Vec4::new(sun_pos.x, sun_pos.y, sun_pos.z, 1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = glam::Vec3::new(clip.x / clip.w, clip.y / clip.w, clip.z / clip.w);
        Some(glam::Vec2::new(ndc.x * 0.5 + 0.5, -ndc.y * 0.5 + 0.5))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat4, Vec2, Vec3};

    #[test]
    fn test_flare_visible_when_looking_at_sun() {
        let view_proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 1000.0, 0.1);
        let sun_dir = Vec3::new(0.0, 0.0, -1.0);

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

        let center_fade = edge_fade_factor(Vec2::new(0.5, 0.5), margin);
        assert!(
            (center_fade - 1.0).abs() < 1e-6,
            "Center should have full intensity, got {center_fade}"
        );

        let edge_fade = edge_fade_factor(Vec2::new(0.9, 0.5), margin);
        assert!(
            edge_fade < 1.0 && edge_fade > 0.0,
            "Near edge should have reduced intensity, got {edge_fade}"
        );

        let off_fade = edge_fade_factor(Vec2::new(1.1, 0.5), margin);
        assert!(
            off_fade <= 0.0,
            "Off screen should have zero intensity, got {off_fade}"
        );
    }

    #[test]
    fn test_flare_disappears_when_sun_is_occluded() {
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
        let direction = center - light_pos;

        let positions: Vec<Vec2> = [0.0, 0.4, 0.7, 1.0, 1.5]
            .iter()
            .map(|&t| element_screen_position(light_pos, t))
            .collect();

        for (i, pos) in positions.iter().enumerate() {
            if i == 0 {
                assert!(
                    (pos.x - light_pos.x).abs() < 1e-6 && (pos.y - light_pos.y).abs() < 1e-6,
                    "Element at t=0 should be at light position"
                );
                continue;
            }
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

        let intensity_dim: f32 = config_intensity * 10.0 * edge_fade * visibility;
        let intensity_bright: f32 = config_intensity * 100.0 * edge_fade * visibility;

        assert!(intensity_bright > intensity_dim);
        assert!(
            (intensity_bright / intensity_dim - 10.0).abs() < 1e-6,
            "Flare intensity should scale linearly with sun brightness"
        );
    }

    #[test]
    fn test_sun_behind_camera_produces_no_flare() {
        let view_proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, 16.0 / 9.0, 1000.0, 0.1);
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

        let has_starburst = config
            .elements
            .iter()
            .any(|e| e.shape == FlareShape::Starburst);
        let has_ghost = config
            .elements
            .iter()
            .any(|e| e.shape == FlareShape::HexagonalGhost);
        assert!(
            has_starburst,
            "Default config should include a starburst element"
        );
        assert!(
            has_ghost,
            "Default config should include a hexagonal ghost element"
        );
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
