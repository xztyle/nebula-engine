//! GPU pipeline and renderer for planet impostor billboards.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

/// WGSL source for the impostor billboard shader.
pub const IMPOSTOR_SHADER_SOURCE: &str = include_str!("impostor.wgsl");

/// Vertex layout for impostor billboard: position (vec3) + uv (vec2).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ImpostorVertex {
    /// World-space position of this billboard corner.
    pub position: [f32; 3],
    /// Texture coordinate.
    pub uv: [f32; 2],
}

impl ImpostorVertex {
    /// Vertex buffer layout descriptor.
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ImpostorVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 12,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

/// Render pipeline for impostor billboards.
pub struct ImpostorPipeline {
    /// The wgpu render pipeline.
    pub pipeline: wgpu::RenderPipeline,
    /// Camera bind group layout (group 0).
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
    /// Texture bind group layout (group 1): texture + sampler.
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
}

impl ImpostorPipeline {
    /// Create the impostor render pipeline.
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("impostor-shader"),
            source: wgpu::ShaderSource::Wgsl(IMPOSTOR_SHADER_SOURCE.into()),
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("impostor-camera-bgl"),
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

        let texture_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("impostor-texture-bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("impostor-pipeline-layout"),
            bind_group_layouts: &[&camera_bind_group_layout, &texture_bind_group_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("impostor-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_impostor"),
                buffers: &[ImpostorVertex::layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None, // Billboard can face either way
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: nebula_render::DepthBuffer::FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::GreaterEqual, // reverse-Z
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_impostor"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
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
            texture_bind_group_layout,
        }
    }
}

/// High-level impostor renderer. Owns GPU resources for rendering
/// a planet as a textured billboard quad.
pub struct ImpostorRenderer {
    /// The render pipeline.
    pub pipeline: ImpostorPipeline,
    /// Vertex buffer (updated each frame with billboard vertices).
    pub vertex_buffer: wgpu::Buffer,
    /// Index buffer (static, 2 triangles).
    pub index_buffer: wgpu::Buffer,
    /// Camera uniform buffer.
    pub camera_buffer: wgpu::Buffer,
    /// Camera bind group.
    pub camera_bind_group: wgpu::BindGroup,
    /// Texture bind group (impostor snapshot).
    pub texture_bind_group: wgpu::BindGroup,
}

impl ImpostorRenderer {
    /// Create a new impostor renderer with a placeholder texture.
    ///
    /// The texture is a small colored circle on transparent background,
    /// suitable for initial display before a proper snapshot is captured.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        resolution: u32,
    ) -> Self {
        let pipeline = ImpostorPipeline::new(device, surface_format);

        // Create a placeholder impostor texture (colored circle)
        let pixels = generate_placeholder_texture(resolution);
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("impostor-texture"),
                size: wgpu::Extent3d {
                    width: resolution,
                    height: resolution,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &pixels,
        );
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("impostor-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Vertex buffer (will be updated each frame)
        let dummy_verts = [ImpostorVertex {
            position: [0.0; 3],
            uv: [0.0; 2],
        }; 4];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("impostor-vertices"),
            contents: bytemuck::cast_slice(&dummy_verts),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("impostor-indices"),
            contents: bytemuck::cast_slice(&super::IMPOSTOR_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Camera buffer
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("impostor-camera-uniform"),
            contents: &[0u8; 64],
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("impostor-camera-bg"),
            layout: &pipeline.camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let texture_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("impostor-texture-bg"),
            layout: &pipeline.texture_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            camera_buffer,
            camera_bind_group,
            texture_bind_group,
        }
    }

    /// Update the billboard vertices and camera for the current frame.
    pub fn update(
        &self,
        queue: &wgpu::Queue,
        view_proj: Mat4,
        planet_center: Vec3,
        camera_right: Vec3,
        camera_up: Vec3,
        half_size: f32,
    ) {
        // Update camera uniform
        let camera_uniform = nebula_render::CameraUniform {
            view_proj: view_proj.to_cols_array_2d(),
            camera_pos: [0.0, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[camera_uniform]),
        );

        // Update billboard vertices
        let verts = super::billboard_vertices(planet_center, camera_right, camera_up, half_size);
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&verts));
    }

    /// Render the impostor billboard.
    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_pipeline(&self.pipeline.pipeline);
        render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.texture_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..6, 0, 0..1);
    }
}

/// Generate a placeholder impostor texture: a colored circle with atmosphere glow.
fn generate_placeholder_texture(resolution: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((resolution * resolution * 4) as usize);
    let center = resolution as f32 / 2.0;
    let radius = center * 0.8;
    let atmo_radius = center * 0.95;

    for y in 0..resolution {
        for x in 0..resolution {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();

            if dist < radius {
                // Planet body: earth-like blue-green
                let t = dist / radius;
                let r = (30.0 + 40.0 * t) as u8;
                let g = (80.0 + 60.0 * t) as u8;
                let b = (120.0 + 80.0 * t) as u8;
                pixels.extend_from_slice(&[r, g, b, 255]);
            } else if dist < atmo_radius {
                // Atmosphere glow
                let t = (dist - radius) / (atmo_radius - radius);
                let alpha = ((1.0 - t) * 180.0) as u8;
                pixels.extend_from_slice(&[100, 150, 220, alpha]);
            } else {
                // Transparent background
                pixels.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_impostor_vertex_size() {
        assert_eq!(std::mem::size_of::<ImpostorVertex>(), 20);
    }

    #[test]
    fn test_placeholder_texture_size() {
        let pixels = generate_placeholder_texture(64);
        assert_eq!(pixels.len(), 64 * 64 * 4);
    }

    #[test]
    fn test_placeholder_texture_has_transparency() {
        let pixels = generate_placeholder_texture(64);
        // Corners should be transparent
        let corner_alpha = pixels[3]; // pixel (0,0) alpha
        assert_eq!(corner_alpha, 0, "Corner pixel should be transparent");

        // Center should be opaque
        let center_idx = (32 * 64 + 32) * 4;
        let center_alpha = pixels[center_idx + 3];
        assert_eq!(center_alpha, 255, "Center pixel should be opaque");
    }
}
