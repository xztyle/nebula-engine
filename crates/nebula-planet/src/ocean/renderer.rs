//! GPU-side ocean renderer: shader, pipeline, and per-frame rendering.

use super::{OceanParams, OceanUniform};
use crate::orbital::{OrbitalMesh, OrbitalVertex};
use glam::Vec3;
use nebula_render::DepthBuffer;

/// WGSL source for the ocean shader.
pub const OCEAN_SHADER_SOURCE: &str = include_str!("ocean.wgsl");

/// Ocean surface renderer. Renders a smooth sphere with animated waves,
/// Fresnel reflections, and depth-dependent coloring.
pub struct OceanRenderer {
    /// The render pipeline for the ocean pass.
    pub pipeline: wgpu::RenderPipeline,
    /// Bind group layout for the camera (group 0).
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
    /// Bind group layout for ocean uniforms (group 1).
    pub ocean_bind_group_layout: wgpu::BindGroupLayout,
    /// Ocean parameters.
    pub params: OceanParams,
    /// GPU uniform buffer for ocean params.
    pub uniform_buffer: wgpu::Buffer,
    /// Camera uniform buffer.
    pub camera_buffer: wgpu::Buffer,
    /// Camera bind group.
    pub camera_bind_group: wgpu::BindGroup,
    /// Ocean bind group.
    pub ocean_bind_group: wgpu::BindGroup,
    /// Vertex buffer for the ocean sphere.
    pub vertex_buffer: wgpu::Buffer,
    /// Index buffer for the ocean sphere.
    pub index_buffer: wgpu::Buffer,
    /// Number of indices.
    pub index_count: u32,
    /// Planet radius used for computing ocean radius.
    pub planet_radius: f32,
    /// Accumulated time for wave animation.
    pub time: f32,
}

impl OceanRenderer {
    /// Create a new ocean renderer.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        mesh: &OrbitalMesh,
        params: OceanParams,
        planet_radius: f32,
    ) -> Self {
        use wgpu::util::DeviceExt;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ocean-shader"),
            source: wgpu::ShaderSource::Wgsl(OCEAN_SHADER_SOURCE.into()),
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ocean-camera-bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(80),
                    },
                    count: None,
                }],
            });

        let ocean_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ocean-bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: std::num::NonZeroU64::new(
                            std::mem::size_of::<OceanUniform>() as u64,
                        ),
                    },
                    count: None,
                }],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ocean-pipeline-layout"),
            bind_group_layouts: &[&camera_bind_group_layout, &ocean_bind_group_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ocean-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_ocean"),
                buffers: &[OrbitalVertex::layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DepthBuffer::FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::GreaterEqual,
                stencil: wgpu::StencilState::default(),
                bias: Self::depth_bias_state(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_ocean"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
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

        // Build vertex data (reuse OrbitalVertex layout)
        let vertices: Vec<OrbitalVertex> = (0..mesh.positions.len())
            .map(|i| OrbitalVertex {
                position: mesh.positions[i].to_array(),
                normal: mesh.normals[i].to_array(),
                uv: mesh.uvs[i],
            })
            .collect();

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ocean-vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ocean-indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ocean-uniform"),
            size: std::mem::size_of::<OceanUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ocean-camera-uniform"),
            contents: &[0u8; 80],
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ocean-camera-bg"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let ocean_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ocean-bg"),
            layout: &ocean_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        Self {
            pipeline,
            camera_bind_group_layout,
            ocean_bind_group_layout,
            params,
            uniform_buffer,
            camera_buffer,
            camera_bind_group,
            ocean_bind_group,
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
            planet_radius,
            time: 0.0,
        }
    }

    /// Depth bias state to prevent z-fighting at shorelines.
    /// Negative constant pushes ocean slightly behind terrain in reverse-Z.
    pub fn depth_bias_state() -> wgpu::DepthBiasState {
        wgpu::DepthBiasState {
            constant: -2,
            slope_scale: -1.0,
            clamp: 0.0,
        }
    }

    /// Update the uniform buffer with current frame state.
    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        view_proj: glam::Mat4,
        sun_direction: Vec3,
        camera_position: Vec3,
        dt: f32,
    ) {
        self.time += dt;

        let ocean_radius = self.planet_radius + self.params.sea_level as f32;
        let uniform = OceanUniform::from_params(
            &self.params,
            sun_direction,
            camera_position,
            ocean_radius,
            self.time,
        );
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));

        let camera_uniform = nebula_render::CameraUniform {
            view_proj: view_proj.to_cols_array_2d(),
            camera_pos: [0.0, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[camera_uniform]),
        );
    }

    /// Render the ocean sphere.
    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.ocean_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}
