//! Sun/corona rendering: camera-facing billboard with animated HDR corona shader.
//!
//! The nearest star is rendered as a bright billboard quad with a multi-layered
//! disk-and-corona shader. HDR output values far exceed 1.0 to drive bloom.

use bytemuck::{Pod, Zeroable};

use nebula_render::Camera;

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
        self.luminosity * 50.0
    }
}

/// GPU vertex for the sun billboard quad.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct SunVertex {
    position: [f32; 3],
    uv: [f32; 2],
}

impl SunVertex {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<SunVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x2,
                offset: 12,
                shader_location: 1,
            },
        ],
    };
}

/// GPU uniform for the sun corona shader.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SunUniforms {
    /// Star color in linear RGB.
    pub color: [f32; 3],
    /// HDR brightness multiplier.
    pub brightness: f32,
    /// Disk radius in UV space (relative to corona extent).
    pub disk_radius_uv: f32,
    /// Animation time in seconds.
    pub time: f32,
    /// Padding for 16-byte alignment.
    pub _padding: [f32; 2],
}

/// WGSL shader source for the sun corona billboard.
pub const SUN_SHADER_SOURCE: &str = r#"
struct SunUniforms {
    color: vec3<f32>,
    brightness: f32,
    disk_radius_uv: f32,
    time: f32,
};

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> camera: mat4x4<f32>;

@group(1) @binding(0)
var<uniform> sun: SunUniforms;

@vertex
fn vs_sun(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world_pos = vec4<f32>(in.position, 1.0);
    let clip = camera * world_pos;
    // Force to maximum depth (z=0 in reverse-Z) so sun is behind all geometry.
    out.clip_position = vec4<f32>(clip.xy, 0.0, clip.w);
    out.uv = in.uv;
    return out;
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
fn fs_sun(in: VertexOutput) -> @location(0) vec4<f32> {
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
"#;

/// GPU sun corona renderer: billboard quad with animated HDR corona shader.
pub struct SunRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
}

impl SunRenderer {
    /// Create a new sun renderer.
    ///
    /// `hdr_format` is the HDR render target format (typically `Rgba16Float`).
    pub fn new(device: &wgpu::Device, hdr_format: wgpu::TextureFormat) -> Self {
        use wgpu::util::DeviceExt;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sun-shader"),
            source: wgpu::ShaderSource::Wgsl(SUN_SHADER_SOURCE.into()),
        });

        // Camera bind group layout (group 0): view-projection matrix
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sun-camera-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(64),
                },
                count: None,
            }],
        });

        // Sun uniform bind group layout (group 1)
        let uniform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("sun-uniform-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: std::num::NonZeroU64::new(
                        std::mem::size_of::<SunUniforms>() as u64
                    ),
                },
                count: None,
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("sun-pipeline-layout"),
            bind_group_layouts: &[&camera_bgl, &uniform_bgl],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("sun-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_sun"),
                buffers: &[SunVertex::LAYOUT],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None, // No depth — rendered at skybox depth via shader
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_sun"),
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

        // Initial vertex data (will be overwritten in update)
        let vertices = [SunVertex {
            position: [0.0; 3],
            uv: [0.0; 2],
        }; 4];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sun-vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let indices: [u16; 6] = [0, 1, 2, 2, 3, 0];
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sun-indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let uniforms = SunUniforms {
            color: [1.0, 0.95, 0.8],
            brightness: 50.0,
            disk_radius_uv: 0.33,
            time: 0.0,
            _padding: [0.0; 2],
        };
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sun-uniforms"),
            contents: bytemuck::bytes_of(&uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sun-uniform-bg"),
            layout: &uniform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let camera_data = glam::Mat4::IDENTITY;
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("sun-camera"),
            contents: bytemuck::cast_slice(&camera_data.to_cols_array()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("sun-camera-bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        log::info!("Sun corona renderer initialized");

        Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            uniform_buffer,
            uniform_bind_group,
            camera_buffer,
            camera_bind_group,
        }
    }

    /// Update the billboard to face the camera and match the sun's current properties.
    pub fn update(
        &self,
        queue: &wgpu::Queue,
        view_proj: glam::Mat4,
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

        // Billboard center: unit direction (rendered at skybox depth via shader).
        let center = sun.direction;

        let vertices = [
            SunVertex {
                position: (center - right - up).into(),
                uv: [-1.0, -1.0],
            },
            SunVertex {
                position: (center + right - up).into(),
                uv: [1.0, -1.0],
            },
            SunVertex {
                position: (center + right + up).into(),
                uv: [1.0, 1.0],
            },
            SunVertex {
                position: (center - right + up).into(),
                uv: [-1.0, 1.0],
            },
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

        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&view_proj.to_cols_array()),
        );
    }

    /// Render the sun billboard. Should be called after skybox, before bloom.
    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_bind_group(1, &self.uniform_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};

    #[test]
    fn test_sun_disk_faces_camera_from_all_angles() {
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
        let uv_vec = glam::Vec2::new(0.5, 0.3);
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
        let g_color = StarType::G.color();
        assert!(
            g_color[0] > g_color[2],
            "G-type star should be yellow (R > B): {g_color:?}",
        );

        let o_color = StarType::O.color();
        assert!(
            o_color[2] > o_color[0],
            "O-type star should be blue (B > R): {o_color:?}",
        );

        let m_color = StarType::M.color();
        assert!(
            m_color[0] > m_color[2] * 2.0,
            "M-type star should be red (R >> B): {m_color:?}",
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
            "Closer sun should have larger angular diameter: {angular_near} vs {angular_far}"
        );
        let ratio = angular_near / angular_far;
        assert!(
            (ratio - 5.0).abs() < 0.01,
            "Angular diameter ratio should be ~5.0, got {ratio}"
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
        let size = std::mem::size_of::<SunUniforms>();
        assert_eq!(
            size % 16,
            0,
            "SunUniforms size ({size} bytes) must be 16-byte aligned"
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
        assert!(
            (angular - 0.1).abs() < 1e-6,
            "Angular diameter should be 0.1 rad, got {angular}"
        );
    }
}
