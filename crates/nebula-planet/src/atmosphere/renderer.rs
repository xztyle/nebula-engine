//! GPU-side atmosphere renderer: shader, pipeline, and full-screen pass.

use super::scatter::{AtmosphereParams, AtmosphereUniform};
use glam::Vec3;

/// WGSL source for the atmosphere full-screen shader.
pub const ATMOSPHERE_SHADER_SOURCE: &str = include_str!("atmosphere.wgsl");

/// Full-screen atmosphere post-pass renderer.
pub struct AtmosphereRenderer {
    /// The render pipeline for the atmosphere pass.
    pub pipeline: wgpu::RenderPipeline,
    /// Bind group layout for atmosphere uniforms + depth texture.
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// Atmosphere parameters.
    pub params: AtmosphereParams,
    /// GPU uniform buffer.
    pub uniform_buffer: wgpu::Buffer,
    /// Depth texture sampler.
    pub depth_sampler: wgpu::Sampler,
}

impl AtmosphereRenderer {
    /// Create a new atmosphere renderer.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        params: AtmosphereParams,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("atmosphere-shader"),
            source: wgpu::ShaderSource::Wgsl(ATMOSPHERE_SHADER_SOURCE.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("atmosphere-bind-group-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("atmosphere-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("atmosphere-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_fullscreen"),
                buffers: &[],
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
                entry_point: Some("fs_atmosphere"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
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

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("atmosphere-uniform"),
            size: std::mem::size_of::<AtmosphereUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let depth_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atmosphere-depth-sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            pipeline,
            bind_group_layout,
            params,
            uniform_buffer,
            depth_sampler,
        }
    }

    /// Create a bind group for the current depth texture view.
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atmosphere-bind-group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.depth_sampler),
                },
            ],
        })
    }

    /// Update the uniform buffer with current frame state.
    #[allow(clippy::too_many_arguments)]
    pub fn update_uniform(
        &self,
        queue: &wgpu::Queue,
        planet_center: Vec3,
        sun_direction: Vec3,
        camera_position: Vec3,
        inv_view_proj: glam::Mat4,
        near_clip: f32,
        far_clip: f32,
    ) {
        let uniform = AtmosphereUniform::from_params(
            &self.params,
            planet_center,
            sun_direction,
            camera_position,
            inv_view_proj,
            near_clip,
            far_clip,
        );
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));
    }

    /// Render the atmosphere as a full-screen additive pass.
    pub fn render<'a>(
        &'a self,
        render_pass: &mut wgpu::RenderPass<'a>,
        bind_group: &'a wgpu::BindGroup,
    ) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}
