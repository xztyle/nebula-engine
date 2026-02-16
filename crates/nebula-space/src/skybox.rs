//! Skybox renderer: draws a cubemap starfield behind all scene geometry.
//!
//! Uses a fullscreen triangle with inverse view-projection to sample a cubemap texture.

use bytemuck::{Pod, Zeroable};

use crate::StarfieldCubemap;

/// Uniform buffer for the skybox: inverse view-projection matrix.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct SkyboxUniform {
    /// Inverse view-projection matrix (rotation only, no translation).
    pub inv_view_proj: [[f32; 4]; 4],
}

/// WGSL shader source for the skybox pass.
pub const SKYBOX_SHADER_SOURCE: &str = r#"
struct SkyboxUniform {
    inv_view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> skybox: SkyboxUniform;

@group(1) @binding(0)
var skybox_texture: texture_cube<f32>;
@group(1) @binding(1)
var skybox_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) view_dir: vec3<f32>,
};

@vertex
fn vs_skybox(@builtin(vertex_index) idx: u32) -> VertexOutput {
    // Fullscreen triangle
    let uv = vec2<f32>(f32((idx << 1u) & 2u), f32(idx & 2u));
    let ndc = uv * 2.0 - 1.0;

    // Use far plane (z=1) for direction reconstruction
    let clip_far = vec4<f32>(ndc.x, ndc.y, 1.0, 1.0);
    let world = skybox.inv_view_proj * clip_far;
    let view_dir = normalize(world.xyz / world.w);

    var out: VertexOutput;
    out.position = vec4<f32>(ndc.x, ndc.y, 0.0, 1.0);
    out.view_dir = view_dir;
    return out;
}

@fragment
fn fs_skybox(in: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(skybox_texture, skybox_sampler, in.view_dir);
    // HDR output: bright stars exceed 1.0 for bloom extraction.
    // No clamping — values above bloom threshold (1.0) produce glow.
    return vec4<f32>(color.rgb * 8.0, 1.0);
}
"#;

/// GPU skybox renderer that draws a cubemap starfield.
pub struct SkyboxRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    cubemap_bind_group: wgpu::BindGroup,
}

impl SkyboxRenderer {
    /// Create a new skybox renderer, uploading the cubemap to the GPU.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        cubemap: &StarfieldCubemap,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("skybox-shader"),
            source: wgpu::ShaderSource::Wgsl(SKYBOX_SHADER_SOURCE.into()),
        });

        // Uniform bind group layout (group 0)
        let uniform_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skybox-uniform-bgl"),
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

        // Cubemap bind group layout (group 1)
        let cubemap_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("skybox-cubemap-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
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
            label: Some("skybox-pipeline-layout"),
            bind_group_layouts: &[&uniform_bgl, &cubemap_bgl],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("skybox-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_skybox"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None, // No depth — skybox rendered first, behind everything
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_skybox"),
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

        // Upload cubemap texture
        let face_size = cubemap.face_size;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("starfield-cubemap"),
            size: wgpu::Extent3d {
                width: face_size,
                height: face_size,
                depth_or_array_layers: 6,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let rgba8_faces = cubemap.to_rgba8();
        for (i, face_data) in rgba8_faces.iter().enumerate() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: i as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                face_data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(face_size * 4),
                    rows_per_image: Some(face_size),
                },
                wgpu::Extent3d {
                    width: face_size,
                    height: face_size,
                    depth_or_array_layers: 1,
                },
            );
        }

        let cubemap_view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("skybox-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let cubemap_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skybox-cubemap-bg"),
            layout: &cubemap_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&cubemap_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        // Uniform buffer
        use wgpu::util::DeviceExt;
        let identity = glam::Mat4::IDENTITY;
        let uniform = SkyboxUniform {
            inv_view_proj: identity.to_cols_array_2d(),
        };
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("skybox-uniform"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skybox-uniform-bg"),
            layout: &uniform_bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        // Count non-black pixels for debug verification
        let non_black: usize = rgba8_faces
            .iter()
            .map(|face| {
                face.chunks(4)
                    .filter(|px| px[0] > 0 || px[1] > 0 || px[2] > 0)
                    .count()
            })
            .sum();
        log::info!(
            "Skybox renderer initialized: {}x{} cubemap, 6 faces, {} non-black pixels",
            face_size,
            face_size,
            non_black
        );

        Self {
            pipeline,
            uniform_buffer,
            uniform_bind_group,
            cubemap_bind_group,
        }
    }

    /// Update the skybox uniform with a new inverse view-projection matrix.
    ///
    /// The matrix should be rotation-only (strip translation from view matrix)
    /// so the skybox appears at infinite distance.
    pub fn update(&self, queue: &wgpu::Queue, inv_view_proj: glam::Mat4) {
        let uniform = SkyboxUniform {
            inv_view_proj: inv_view_proj.to_cols_array_2d(),
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));
    }

    /// Render the skybox. Should be the first pass (before scene geometry).
    pub fn render<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_bind_group(1, &self.cubemap_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
