//! Basic unlit rendering pipeline for colored geometry.

use bytemuck::{Pod, Zeroable};
use std::num::NonZeroU64;

use crate::buffer::{MeshBuffer, VertexPositionColor};

/// Uniform buffer for camera view-projection matrix.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4], // 64 bytes, mat4x4
}

/// Basic unlit rendering pipeline for colored geometry.
pub struct UnlitPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
}

impl UnlitPipeline {
    /// Create a new unlit pipeline.
    pub fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        surface_format: wgpu::TextureFormat,
        depth_format: Option<wgpu::TextureFormat>,
    ) -> Self {
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera-bind-group-layout"),
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
            label: Some("unlit-pipeline-layout"),
            bind_group_layouts: &[&camera_bind_group_layout],
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
            label: Some("unlit-pipeline"),
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
                cull_mode: Some(wgpu::Face::Back),
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
        }
    }
}

/// Draw unlit geometry with the given pipeline and camera bind group.
pub fn draw_unlit<'a>(
    render_pass: &mut wgpu::RenderPass<'a>,
    pipeline: &UnlitPipeline,
    camera_bind_group: &'a wgpu::BindGroup,
    mesh: &'a MeshBuffer,
) {
    render_pass.set_pipeline(&pipeline.pipeline);
    render_pass.set_bind_group(0, camera_bind_group, &[]);
    mesh.bind(render_pass);
    mesh.draw(render_pass);
}

/// The WGSL source code for the unlit shader.
pub const UNLIT_SHADER_SOURCE: &str = r#"
struct CameraUniform {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_device() -> wgpu::Device {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::default(),
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })
                .await
                .expect("Failed to find adapter");

            let (device, _queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                    experimental_features: Default::default(),
                    trace: Default::default(),
                })
                .await
                .expect("Failed to create device");

            device
        })
    }

    fn create_test_shader(device: &wgpu::Device, source: &str) -> wgpu::ShaderModule {
        device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("test-shader"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        })
    }

    fn create_test_unlit_pipeline(device: &wgpu::Device) -> UnlitPipeline {
        let shader = create_test_shader(device, UNLIT_SHADER_SOURCE);
        UnlitPipeline::new(
            device,
            &shader,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            Some(wgpu::TextureFormat::Depth32Float),
        )
    }

    #[test]
    fn test_pipeline_creation_succeeds() {
        let device = create_test_device();
        let shader = create_test_shader(&device, UNLIT_SHADER_SOURCE);
        let _pipeline = UnlitPipeline::new(
            &device,
            &shader,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            Some(wgpu::TextureFormat::Depth32Float),
        );
        // Pipeline creation should not panic — reaching this line is success.
    }

    #[test]
    fn test_vertex_buffer_layout_matches_shader() {
        let layout = VertexPositionColor::layout();
        // The shader expects location(0) = vec3<f32> and location(1) = vec4<f32>
        assert_eq!(layout.attributes.len(), 2);

        // location(0): position, offset 0, Float32x3
        assert_eq!(layout.attributes[0].shader_location, 0);
        assert_eq!(layout.attributes[0].offset, 0);
        assert_eq!(layout.attributes[0].format, wgpu::VertexFormat::Float32x3);

        // location(1): color, offset 12, Float32x4
        assert_eq!(layout.attributes[1].shader_location, 1);
        assert_eq!(layout.attributes[1].offset, 12);
        assert_eq!(layout.attributes[1].format, wgpu::VertexFormat::Float32x4);
    }

    #[test]
    fn test_pipeline_uses_correct_entry_points() {
        // The shader module must contain entry points named "vs_main" and "fs_main".
        // This is validated at pipeline creation time — if the entry points are
        // wrong, create_render_pipeline panics. The fact that pipeline creation
        // succeeds in test_pipeline_creation_succeeds confirms correct entry points.
        //
        // Additionally, verify the shader source contains the expected entry point names.
        assert!(UNLIT_SHADER_SOURCE.contains("fn vs_main"));
        assert!(UNLIT_SHADER_SOURCE.contains("fn fs_main"));
    }

    #[test]
    fn test_camera_uniform_size() {
        // The CameraUniform must be exactly 64 bytes (one mat4x4<f32>).
        assert_eq!(std::mem::size_of::<CameraUniform>(), 64);
    }

    #[test]
    fn test_camera_bind_group_layout_has_one_entry() {
        let device = create_test_device();
        let pipeline = create_test_unlit_pipeline(&device);
        // The bind group layout should have exactly one entry at binding 0.
        // Verified by successfully creating a bind group with a single buffer.
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test-camera"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });
        let _bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("test"),
            layout: &pipeline.camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });
        // If create_bind_group does not panic, the layout is correct.
    }

    #[test]
    fn test_pipeline_without_depth() {
        let device = create_test_device();
        let shader = create_test_shader(&device, UNLIT_SHADER_SOURCE);
        // Creating a pipeline without depth should also succeed
        let _pipeline = UnlitPipeline::new(
            &device,
            &shader,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            None, // no depth
        );
        // Pipeline creation should succeed.
    }
}
