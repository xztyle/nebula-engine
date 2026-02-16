//! GPU renderer for distant planet impostors using instanced billboards.

use bytemuck;
use wgpu::util::DeviceExt;

use super::{ImpostorInstance, ImpostorVertex};

/// Maximum number of simultaneous planet impostors.
const MAX_IMPOSTORS: usize = 64;

/// WGSL shader source for planet impostor billboards.
const IMPOSTOR_SHADER: &str = r#"
struct CameraUniforms {
    view_proj: mat4x4<f32>,
};

struct VertexInput {
    @location(0) quad_pos: vec2<f32>,
    // Instance attributes
    @location(2) center: vec3<f32>,
    @location(3) scale: f32,
    @location(4) color: vec3<f32>,
    @location(5) brightness: f32,
    @location(6) sun_dir_local: vec3<f32>,
    @location(7) has_atmosphere: u32,
    @location(8) atmosphere_color: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec3<f32>,
    @location(2) brightness: f32,
    @location(3) sun_dir_local: vec3<f32>,
    @location(4) @interpolate(flat) has_atmosphere: u32,
    @location(5) atmosphere_color: vec3<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniforms;

@vertex
fn vs_planet_impostor(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Build billboard in clip space: project center, then offset by quad_pos * scale.
    let clip_center = camera.view_proj * vec4<f32>(in.center, 1.0);

    // Scale in clip space (angular radius -> NDC).
    let offset = vec2<f32>(in.quad_pos.x * in.scale, in.quad_pos.y * in.scale);
    let scaled = vec4<f32>(
        clip_center.x + offset.x * clip_center.w,
        clip_center.y + offset.y * clip_center.w,
        0.0,  // z=0 in reverse-Z: behind all geometry (at far plane).
        clip_center.w
    );
    out.clip_position = scaled;
    out.uv = in.quad_pos;
    out.color = in.color;
    out.brightness = in.brightness;
    out.sun_dir_local = in.sun_dir_local;
    out.has_atmosphere = in.has_atmosphere;
    out.atmosphere_color = in.atmosphere_color;
    return out;
}

@fragment
fn fs_planet_impostor(in: VertexOutput) -> @location(0) vec4<f32> {
    let uv = in.uv;
    let dist_sq = dot(uv, uv);

    // Discard fragments outside the planet disk.
    if dist_sq > 1.0 {
        discard;
    }

    // Reconstruct normal on a unit sphere.
    let z = sqrt(1.0 - dist_sq);
    let normal = vec3<f32>(uv.x, uv.y, z);

    // Lambertian shading.
    let ndotl = max(dot(normal, in.sun_dir_local), 0.0);
    let lit_color = in.color * ndotl * in.brightness;

    // Atmospheric rim glow on the lit limb.
    var atmo = vec3<f32>(0.0);
    if in.has_atmosphere != 0u {
        let rim = 1.0 - z;
        let rim_factor = pow(rim, 2.0) * max(ndotl + 0.2, 0.0);
        atmo = in.atmosphere_color * rim_factor * 0.5;
    }

    let final_color = lit_color + atmo;
    return vec4<f32>(final_color, 1.0);
}
"#;

/// GPU renderer for distant planet impostors as instanced billboards.
pub struct PlanetImpostorRenderer {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    instance_buffer: wgpu::Buffer,
    camera_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    /// Number of active instances to draw this frame.
    instance_count: u32,
}

impl PlanetImpostorRenderer {
    /// Create a new planet impostor renderer.
    ///
    /// `hdr_format` is the render target format (typically `Rgba16Float`).
    pub fn new(device: &wgpu::Device, hdr_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("planet-impostor-shader"),
            source: wgpu::ShaderSource::Wgsl(IMPOSTOR_SHADER.into()),
        });

        // Camera bind group layout (group 0)
        let camera_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("planet-impostor-camera-bgl"),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("planet-impostor-layout"),
            bind_group_layouts: &[&camera_bgl],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("planet-impostor-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_planet_impostor"),
                buffers: &[ImpostorVertex::LAYOUT, ImpostorInstance::LAYOUT],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_planet_impostor"),
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

        // Unit quad vertices: [-1, 1] range
        let quad_verts = [
            ImpostorVertex {
                position: [-1.0, -1.0],
            },
            ImpostorVertex {
                position: [1.0, -1.0],
            },
            ImpostorVertex {
                position: [1.0, 1.0],
            },
            ImpostorVertex {
                position: [-1.0, 1.0],
            },
        ];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("planet-impostor-verts"),
            contents: bytemuck::cast_slice(&quad_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let indices: [u16; 6] = [0, 1, 2, 2, 3, 0];
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("planet-impostor-indices"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("planet-impostor-instances"),
            size: (MAX_IMPOSTORS * std::mem::size_of::<ImpostorInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let camera_data = glam::Mat4::IDENTITY;
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("planet-impostor-camera"),
            contents: bytemuck::cast_slice(&camera_data.to_cols_array()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("planet-impostor-camera-bg"),
            layout: &camera_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        log::info!("Planet impostor renderer initialized (max {MAX_IMPOSTORS} planets)");

        Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            instance_buffer,
            camera_buffer,
            camera_bind_group,
            instance_count: 0,
        }
    }

    /// Update impostor instances for the current frame.
    ///
    /// `instances` are the pre-computed impostor billboard data. The caller is
    /// responsible for computing positions from orbital elements, phase angles,
    /// and billboard-local sun directions.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        view_proj: glam::Mat4,
        instances: &[ImpostorInstance],
    ) {
        let count = instances.len().min(MAX_IMPOSTORS);
        self.instance_count = count as u32;

        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&view_proj.to_cols_array()),
        );

        if count > 0 {
            queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&instances[..count]),
            );
        }
    }

    /// Render all active planet impostors. Call after skybox, before bloom.
    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.instance_count == 0 {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.camera_bind_group, &[]);
        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..self.instance_count);
    }

    /// Returns the number of active impostors being rendered.
    pub fn active_count(&self) -> u32 {
        self.instance_count
    }
}
