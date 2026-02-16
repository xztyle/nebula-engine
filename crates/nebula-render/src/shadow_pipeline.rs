//! Depth-only render pipeline for shadow map generation.
//!
//! Renders scene geometry from the light's perspective into a depth texture
//! array, one layer per cascade. No color output — only depth writes.

use std::num::NonZeroU64;

use crate::buffer::VertexPositionColor;

/// WGSL shader source for shadow depth-only rendering.
///
/// Uses a per-cascade light-space matrix uniform to project vertices.
pub const SHADOW_SHADER_SOURCE: &str = r#"
struct LightMatrix {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> light: LightMatrix;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

@vertex
fn vs_shadow(in: VertexInput) -> @builtin(position) vec4<f32> {
    return light.view_proj * vec4<f32>(in.position, 1.0);
}
"#;

/// Depth-only pipeline for rendering shadow maps.
pub struct ShadowPipeline {
    /// The underlying wgpu render pipeline.
    pub pipeline: wgpu::RenderPipeline,
    /// Light matrix uniform bind group layout (group 0).
    pub light_bind_group_layout: wgpu::BindGroupLayout,
}

impl ShadowPipeline {
    /// Create a new shadow depth-only pipeline.
    pub fn new(device: &wgpu::Device, shader: &wgpu::ShaderModule) -> Self {
        let light_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("shadow-light-bgl"),
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow-pipeline-layout"),
            bind_group_layouts: &[&light_bind_group_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow-depth-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_shadow"),
                buffers: &[VertexPositionColor::layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Front), // front-face culling reduces acne
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::GreaterEqual, // reverse-Z
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 1.75,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: None, // depth-only — no fragment output
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            light_bind_group_layout,
        }
    }
}

/// Render shadow cascades into the depth texture array.
pub fn render_shadow_cascades(
    encoder: &mut wgpu::CommandEncoder,
    shadow_pipeline: &ShadowPipeline,
    cascade_views: &[wgpu::TextureView],
    cascade_bind_groups: &[wgpu::BindGroup],
    mesh: &crate::buffer::MeshBuffer,
) {
    let count = cascade_views.len().min(cascade_bind_groups.len());
    for i in 0..count {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shadow-cascade"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &cascade_views[i],
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0), // reverse-Z: clear to 0
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&shadow_pipeline.pipeline);
        pass.set_bind_group(0, &cascade_bind_groups[i], &[]);
        mesh.bind(&mut pass);
        mesh.draw(&mut pass);
    }
}
