//! GPU pipeline for orbital planet rendering.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use nebula_render::DepthBuffer;

use super::mesh::OrbitalMesh;

/// WGSL source for the orbital planet shader.
pub const ORBITAL_SHADER_SOURCE: &str = include_str!("orbital.wgsl");

/// GPU uniform for orbital planet rendering.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PlanetUniform {
    /// Model matrix (translation × rotation × scale).
    pub model: [[f32; 4]; 4],
    /// Normalized sun direction in world space.
    pub sun_direction: [f32; 3],
    /// Planet radius (used for reference, baked into model matrix).
    pub planet_radius: f32,
}

/// Compute the model matrix for the orbital sphere.
///
/// Includes translation to planet center, Y-axis rotation for planet spin,
/// and uniform scale to planet radius.
pub fn orbital_model_matrix(planet_center: Vec3, planet_radius: f32, rotation_angle: f32) -> Mat4 {
    Mat4::from_translation(planet_center)
        * Mat4::from_rotation_y(rotation_angle)
        * Mat4::from_scale(Vec3::splat(planet_radius))
}

/// Vertex layout for orbital mesh: position (vec3), normal (vec3), uv (vec2).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct OrbitalVertex {
    /// Position on unit sphere.
    pub position: [f32; 3],
    /// Normal (same as position for unit sphere).
    pub normal: [f32; 3],
    /// Equirectangular UV.
    pub uv: [f32; 2],
}

impl OrbitalVertex {
    /// Vertex buffer layout for the orbital shader.
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<OrbitalVertex>() as wgpu::BufferAddress,
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
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 24,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        }
    }
}

/// Render pipeline for orbital planet sphere.
pub struct OrbitalPipeline {
    /// The wgpu render pipeline.
    pub pipeline: wgpu::RenderPipeline,
    /// Camera bind group layout (group 0).
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
    /// Planet bind group layout (group 1): texture + sampler + uniform.
    pub planet_bind_group_layout: wgpu::BindGroupLayout,
}

impl OrbitalPipeline {
    /// Create the orbital render pipeline.
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("orbital-shader"),
            source: wgpu::ShaderSource::Wgsl(ORBITAL_SHADER_SOURCE.into()),
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("orbital-camera-bgl"),
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

        let planet_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("orbital-planet-bgl"),
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
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: std::num::NonZeroU64::new(std::mem::size_of::<
                                PlanetUniform,
                            >(
                            )
                                as u64),
                        },
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("orbital-pipeline-layout"),
            bind_group_layouts: &[&camera_bind_group_layout, &planet_bind_group_layout],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("orbital-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_orbital"),
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
                depth_compare: wgpu::CompareFunction::GreaterEqual, // reverse-Z
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_orbital"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None, // opaque
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
            planet_bind_group_layout,
        }
    }
}

/// High-level orbital planet renderer. Owns GPU resources for rendering
/// the planet as a textured sphere from orbit.
pub struct OrbitalRenderer {
    /// The render pipeline.
    pub pipeline: OrbitalPipeline,
    /// Vertex buffer for the icosphere.
    pub vertex_buffer: wgpu::Buffer,
    /// Index buffer for the icosphere.
    pub index_buffer: wgpu::Buffer,
    /// Number of indices.
    pub index_count: u32,
    /// Planet uniform buffer.
    pub planet_uniform_buffer: wgpu::Buffer,
    /// Camera uniform buffer.
    pub camera_buffer: wgpu::Buffer,
    /// Camera bind group.
    pub camera_bind_group: wgpu::BindGroup,
    /// Planet bind group (texture + sampler + uniform).
    pub planet_bind_group: wgpu::BindGroup,
    /// Planet radius.
    pub planet_radius: f32,
}

impl OrbitalRenderer {
    /// Create a new orbital renderer with the given mesh and terrain texture.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        mesh: &OrbitalMesh,
        terrain_pixels: &[[u8; 4]],
        tex_width: u32,
        tex_height: u32,
        planet_radius: f32,
    ) -> Self {
        use wgpu::util::DeviceExt;

        let pipeline = OrbitalPipeline::new(device, surface_format);

        // Build vertex data
        let vertices: Vec<OrbitalVertex> = (0..mesh.positions.len())
            .map(|i| OrbitalVertex {
                position: mesh.positions[i].to_array(),
                normal: mesh.normals[i].to_array(),
                uv: mesh.uvs[i],
            })
            .collect();

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("orbital-vertices"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("orbital-indices"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Terrain texture
        let tex_data: Vec<u8> = terrain_pixels
            .iter()
            .flat_map(|p| p.iter().copied())
            .collect();
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("orbital-terrain-texture"),
                size: wgpu::Extent3d {
                    width: tex_width,
                    height: tex_height,
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
            &tex_data,
        );
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("orbital-terrain-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Planet uniform buffer
        let planet_uniform = PlanetUniform {
            model: Mat4::IDENTITY.to_cols_array_2d(),
            sun_direction: [0.0, 1.0, 0.0],
            planet_radius,
        };
        let planet_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("orbital-planet-uniform"),
            contents: bytemuck::cast_slice(&[planet_uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Camera buffer
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("orbital-camera-uniform"),
            contents: &[0u8; 64],
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("orbital-camera-bg"),
            layout: &pipeline.camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        let planet_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("orbital-planet-bg"),
            layout: &pipeline.planet_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: planet_uniform_buffer.as_entire_binding(),
                },
            ],
        });

        Self {
            pipeline,
            vertex_buffer,
            index_buffer,
            index_count: mesh.indices.len() as u32,
            planet_uniform_buffer,
            camera_buffer,
            camera_bind_group,
            planet_bind_group,
            planet_radius,
        }
    }

    /// Update the camera and planet uniforms for the current frame.
    pub fn update(
        &self,
        queue: &wgpu::Queue,
        view_proj: Mat4,
        planet_center: Vec3,
        sun_direction: Vec3,
        rotation_angle: f32,
    ) {
        use nebula_render::CameraUniform;

        let camera_uniform = CameraUniform {
            view_proj: view_proj.to_cols_array_2d(),
            camera_pos: [0.0, 0.0, 0.0, 0.0],
        };
        queue.write_buffer(
            &self.camera_buffer,
            0,
            bytemuck::cast_slice(&[camera_uniform]),
        );

        let model = orbital_model_matrix(planet_center, self.planet_radius, rotation_angle);
        let planet_uniform = PlanetUniform {
            model: model.to_cols_array_2d(),
            sun_direction: sun_direction.normalize().to_array(),
            planet_radius: self.planet_radius,
        };
        queue.write_buffer(
            &self.planet_uniform_buffer,
            0,
            bytemuck::cast_slice(&[planet_uniform]),
        );
    }

    /// Render the orbital planet sphere.
    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_pipeline(&self.pipeline.pipeline);
        render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.planet_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orbital_model_matrix_rotation() {
        let t0 = orbital_model_matrix(Vec3::ZERO, 1.0, 0.0);
        let t1 = orbital_model_matrix(Vec3::ZERO, 1.0, 0.5);
        let t2 = orbital_model_matrix(Vec3::ZERO, 1.0, 1.0);

        assert_ne!(t0, t1, "Model matrix should change with rotation");
        assert_ne!(t1, t2, "Model matrix should change with rotation");

        let equator_point = glam::Vec4::new(1.0, 0.0, 0.0, 1.0);
        let p0 = t0 * equator_point;
        let p1 = t1 * equator_point;
        let diff = (p0 - p1).length();
        assert!(
            diff > 0.01,
            "Equator point should move with rotation, diff = {diff}"
        );
    }

    #[test]
    fn test_planet_uniform_size_alignment() {
        assert_eq!(std::mem::size_of::<PlanetUniform>() % 16, 0);
    }

    #[test]
    fn test_planet_size_decreases_with_distance() {
        let planet_radius = 200.0_f32;
        let distances = [
            planet_radius * 2.0,
            planet_radius * 5.0,
            planet_radius * 20.0,
        ];
        let mut prev_angular_size = f32::MAX;

        for &dist in &distances {
            let angular_size = 2.0 * (planet_radius / dist).asin();
            assert!(
                angular_size < prev_angular_size,
                "Planet should appear smaller at distance {dist}"
            );
            prev_angular_size = angular_size;
        }
    }
}
