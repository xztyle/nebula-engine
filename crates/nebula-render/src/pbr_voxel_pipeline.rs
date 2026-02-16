//! PBR voxel rendering pipeline with Cook-Torrance BRDF.
//!
//! Uses [`VoxelVertex`] geometry (position, normal, UV, material ID, AO) and
//! four bind groups: camera (group 0), light (group 1), material atlas + buffer
//! (group 2), and shadow maps (group 3).

use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};

use crate::buffer::{MeshBuffer, VoxelVertex};

/// Camera uniform: view-projection matrix and world-space position.
///
/// Bound at group 0, binding 0. Visible to vertex and fragment stages.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PbrCameraUniform {
    /// View-projection matrix (64 bytes).
    pub view_proj: [[f32; 4]; 4],
    /// Camera position (xyz), w unused.
    pub camera_pos: [f32; 4],
}

/// Directional sun light uniform for PBR shading.
///
/// Bound at group 1, binding 0. Visible to fragment stage.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PbrLightUniform {
    /// Sun direction (xyz), w unused.
    pub sun_direction: [f32; 4],
    /// Sun color (rgb) and intensity (w).
    pub sun_color: [f32; 4],
    /// Ambient color (rgb) and intensity (w).
    pub ambient_color: [f32; 4],
    /// Light-space view-projection for shadow mapping.
    pub sun_view_proj: [[f32; 4]; 4],
}

/// PBR voxel pipeline: renders voxel terrain with Cook-Torrance shading.
///
/// Bind groups:
/// - Group 0: camera uniform
/// - Group 1: light uniform
/// - Group 2: material atlas texture + sampler + material storage buffer
/// - Group 3: shadow depth texture + comparison sampler
pub struct PbrVoxelPipeline {
    /// The compiled render pipeline.
    pub pipeline: wgpu::RenderPipeline,
    /// Camera uniform bind group layout (group 0).
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
    /// Light uniform bind group layout (group 1).
    pub light_bind_group_layout: wgpu::BindGroupLayout,
    /// Material atlas + storage buffer bind group layout (group 2).
    pub material_bind_group_layout: wgpu::BindGroupLayout,
    /// Shadow map bind group layout (group 3).
    pub shadow_bind_group_layout: wgpu::BindGroupLayout,
}

impl PbrVoxelPipeline {
    /// Create the PBR voxel pipeline with all four bind group layouts.
    pub fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        surface_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        // Group 0: Camera uniform
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pbr-voxel-camera-bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<PbrCameraUniform>() as u64
                        ),
                    },
                    count: None,
                }],
            });

        // Group 1: Light uniform
        let light_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pbr-voxel-light-bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<PbrLightUniform>() as u64
                        ),
                    },
                    count: None,
                }],
            });

        // Group 2: Material atlas texture + sampler + storage buffer
        let material_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pbr-voxel-material-bgl"),
                entries: &[
                    // binding 0: atlas texture
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
                    // binding 1: atlas sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    // binding 2: MaterialGpuData[] storage buffer
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(48), // one MaterialGpuData
                        },
                        count: None,
                    },
                ],
            });

        // Group 3: Shadow map
        let shadow_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("pbr-voxel-shadow-bgl"),
                entries: &[
                    // binding 0: shadow depth texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // binding 1: comparison sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pbr-voxel-pipeline-layout"),
            bind_group_layouts: &[
                &camera_bind_group_layout,
                &light_bind_group_layout,
                &material_bind_group_layout,
                &shadow_bind_group_layout,
            ],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pbr-voxel-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                buffers: &[VoxelVertex::vertex_buffer_layout()],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::GreaterEqual, // reverse-Z
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
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
            material_bind_group_layout,
            shadow_bind_group_layout,
        }
    }
}

/// Draw PBR voxel geometry with all four bind groups.
pub fn draw_pbr_voxel<'a>(
    render_pass: &mut wgpu::RenderPass<'a>,
    pipeline: &'a PbrVoxelPipeline,
    camera_bind_group: &'a wgpu::BindGroup,
    light_bind_group: &'a wgpu::BindGroup,
    material_bind_group: &'a wgpu::BindGroup,
    shadow_bind_group: &'a wgpu::BindGroup,
    mesh: &'a MeshBuffer,
) {
    render_pass.set_pipeline(&pipeline.pipeline);
    render_pass.set_bind_group(0, camera_bind_group, &[]);
    render_pass.set_bind_group(1, light_bind_group, &[]);
    render_pass.set_bind_group(2, material_bind_group, &[]);
    render_pass.set_bind_group(3, shadow_bind_group, &[]);
    mesh.bind(render_pass);
    mesh.draw(render_pass);
}

/// WGSL shader source for PBR voxel rendering with Cook-Torrance BRDF.
pub const PBR_VOXEL_SHADER_SOURCE: &str = include_str!("pbr_voxel.wgsl");

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::VoxelVertex;

    fn create_test_device() -> Option<wgpu::Device> {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    ..Default::default()
                })
                .await
                .ok()?;
            let (device, _queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .ok()?;
            Some(device)
        })
    }

    #[test]
    fn test_camera_uniform_size() {
        assert_eq!(std::mem::size_of::<PbrCameraUniform>(), 80);
    }

    #[test]
    fn test_light_uniform_size() {
        assert_eq!(std::mem::size_of::<PbrLightUniform>(), 112);
    }

    #[test]
    fn test_voxel_vertex_size() {
        assert_eq!(std::mem::size_of::<VoxelVertex>(), 40);
    }

    #[test]
    fn test_pipeline_compatible_with_chunk_vertex_format() {
        let layout = VoxelVertex::vertex_buffer_layout();
        assert_eq!(layout.attributes.len(), 5);
        assert_eq!(layout.attributes[0].shader_location, 0);
        assert_eq!(layout.attributes[0].format, wgpu::VertexFormat::Float32x3);
        assert_eq!(layout.attributes[0].offset, 0);
        assert_eq!(layout.attributes[1].shader_location, 1);
        assert_eq!(layout.attributes[1].format, wgpu::VertexFormat::Float32x3);
        assert_eq!(layout.attributes[1].offset, 12);
        assert_eq!(layout.attributes[2].shader_location, 2);
        assert_eq!(layout.attributes[2].format, wgpu::VertexFormat::Float32x2);
        assert_eq!(layout.attributes[2].offset, 24);
        assert_eq!(layout.attributes[3].shader_location, 3);
        assert_eq!(layout.attributes[3].format, wgpu::VertexFormat::Uint32);
        assert_eq!(layout.attributes[3].offset, 32);
        assert_eq!(layout.attributes[4].shader_location, 4);
        assert_eq!(layout.attributes[4].format, wgpu::VertexFormat::Float32);
        assert_eq!(layout.attributes[4].offset, 36);
        assert_eq!(layout.array_stride, 40);
    }

    #[test]
    fn test_shader_compilation_succeeds() {
        let Some(device) = create_test_device() else {
            return;
        };
        let _shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr-voxel-shader-test"),
            source: wgpu::ShaderSource::Wgsl(PBR_VOXEL_SHADER_SOURCE.into()),
        });
    }

    #[test]
    fn test_pipeline_compiles_with_all_bind_groups() {
        let Some(device) = create_test_device() else {
            return;
        };
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr-voxel-shader"),
            source: wgpu::ShaderSource::Wgsl(PBR_VOXEL_SHADER_SOURCE.into()),
        });
        let _pipeline = PbrVoxelPipeline::new(
            &device,
            &shader,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Depth32Float,
        );
    }

    #[test]
    fn test_all_bind_group_layouts_match_shader_expectations() {
        let Some(device) = create_test_device() else {
            return;
        };
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr-voxel-shader"),
            source: wgpu::ShaderSource::Wgsl(PBR_VOXEL_SHADER_SOURCE.into()),
        });
        let pipeline = PbrVoxelPipeline::new(
            &device,
            &shader,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Depth32Float,
        );

        // Verify camera bind group accepts correct buffer
        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test-camera"),
            size: std::mem::size_of::<PbrCameraUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let _camera_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test-camera-bg"),
            layout: &pipeline.camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            }],
        });

        // Verify light bind group accepts correct buffer
        let light_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test-light"),
            size: std::mem::size_of::<PbrLightUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let _light_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test-light-bg"),
            layout: &pipeline.light_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: light_buf.as_entire_binding(),
            }],
        });
    }
}
