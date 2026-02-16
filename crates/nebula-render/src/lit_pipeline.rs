//! Lit rendering pipeline with directional light N·L shading.
//!
//! Uses [`VertexPositionColor`] geometry plus a directional light uniform
//! at `@group(1) @binding(0)`. Normals are computed from world position
//! (assuming sphere centered at origin), making this ideal for planet terrain.

use std::num::NonZeroU64;

use crate::buffer::{MeshBuffer, VertexPositionColor};

/// Lit rendering pipeline: camera at group 0, light at group 1.
pub struct LitPipeline {
    /// The underlying wgpu render pipeline.
    pub pipeline: wgpu::RenderPipeline,
    /// Camera uniform bind group layout (group 0).
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
    /// Directional light uniform bind group layout (group 1).
    pub light_bind_group_layout: wgpu::BindGroupLayout,
}

impl LitPipeline {
    /// Create a new lit pipeline.
    pub fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        surface_format: wgpu::TextureFormat,
        depth_format: Option<wgpu::TextureFormat>,
        cull_mode: Option<wgpu::Face>,
    ) -> Self {
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("lit-camera-bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(64), // mat4x4<f32>
                    },
                    count: None,
                }],
            });

        let light_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("lit-light-bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(32), // DirectionalLightUniform
                    },
                    count: None,
                }],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lit-pipeline-layout"),
            bind_group_layouts: &[&camera_bind_group_layout, &light_bind_group_layout],
            immediate_size: 0,
        });

        let depth_stencil = depth_format.map(|format| wgpu::DepthStencilState {
            format,
            depth_write_enabled: true,
            depth_compare: wgpu::CompareFunction::GreaterEqual, // reverse-Z
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lit-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                buffers: &[VertexPositionColor::layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            camera_bind_group_layout,
            light_bind_group_layout,
        }
    }
}

/// Draw lit geometry with camera and directional light bind groups.
pub fn draw_lit<'a>(
    render_pass: &mut wgpu::RenderPass<'a>,
    pipeline: &LitPipeline,
    camera_bind_group: &'a wgpu::BindGroup,
    light_bind_group: &'a wgpu::BindGroup,
    mesh: &'a MeshBuffer,
) {
    render_pass.set_pipeline(&pipeline.pipeline);
    render_pass.set_bind_group(0, camera_bind_group, &[]);
    render_pass.set_bind_group(1, light_bind_group, &[]);
    mesh.bind(render_pass);
    mesh.draw(render_pass);
}

/// WGSL shader source for lit planet rendering.
///
/// Computes normal from world position (assumes sphere at origin).
/// Applies N·L diffuse lighting plus a small ambient term.
pub const LIT_SHADER_SOURCE: &str = r#"
struct CameraUniform {
    view_proj: mat4x4<f32>,
};

struct DirectionalLight {
    direction_intensity: vec4<f32>,
    color_padding: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var<uniform> sun: DirectionalLight;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world_position: vec3<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    out.world_position = in.position;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Compute normal from world position (sphere centered at origin).
    let normal = normalize(in.world_position);

    // N dot L: negate sun direction because it points FROM the light.
    let n_dot_l = max(dot(normal, -sun.direction_intensity.xyz), 0.0);

    // Diffuse contribution.
    let diffuse = sun.color_padding.xyz * sun.direction_intensity.w * n_dot_l;

    // Small ambient term so shadowed areas aren't pure black.
    let ambient = vec3<f32>(0.08, 0.08, 0.12);

    let lit_color = in.color.rgb * (diffuse + ambient);
    return vec4<f32>(lit_color, in.color.a);
}
"#;
