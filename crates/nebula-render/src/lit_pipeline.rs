//! Lit rendering pipeline with directional light N·L shading and cascaded shadows.
//!
//! Uses [`VertexPositionColor`] geometry plus a directional light uniform
//! at `@group(1) @binding(0)`. Normals are computed from world position
//! (assuming sphere centered at origin), making this ideal for planet terrain.
//!
//! Shadow maps are bound at `@group(2)` with a depth texture array, comparison
//! sampler, and shadow uniform buffer.

use std::num::NonZeroU64;

use crate::buffer::{MeshBuffer, VertexPositionColor};

/// Lit rendering pipeline: camera at group 0, light at group 1, shadows at group 2.
pub struct LitPipeline {
    /// The underlying wgpu render pipeline.
    pub pipeline: wgpu::RenderPipeline,
    /// Camera uniform bind group layout (group 0).
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
    /// Directional light uniform bind group layout (group 1).
    pub light_bind_group_layout: wgpu::BindGroupLayout,
    /// Shadow map bind group layout (group 2).
    pub shadow_bind_group_layout: wgpu::BindGroupLayout,
}

impl LitPipeline {
    /// Create a new lit pipeline with shadow support.
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
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(32), // DirectionalLightUniform
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(64),
                        },
                        count: None,
                    },
                ],
            });

        let shadow_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("lit-shadow-bgl"),
                entries: &[
                    // binding 0: shadow uniform buffer
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: NonZeroU64::new(288), // ShadowUniform
                        },
                        count: None,
                    },
                    // binding 1: shadow depth texture array
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Depth,
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // binding 2: comparison sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                        count: None,
                    },
                ],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lit-pipeline-layout"),
            bind_group_layouts: &[
                &camera_bind_group_layout,
                &light_bind_group_layout,
                &shadow_bind_group_layout,
            ],
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
            shadow_bind_group_layout,
        }
    }
}

/// Draw lit geometry with camera, light, and shadow bind groups.
pub fn draw_lit<'a>(
    render_pass: &mut wgpu::RenderPass<'a>,
    pipeline: &LitPipeline,
    camera_bind_group: &'a wgpu::BindGroup,
    light_bind_group: &'a wgpu::BindGroup,
    shadow_bind_group: &'a wgpu::BindGroup,
    mesh: &'a MeshBuffer,
) {
    render_pass.set_pipeline(&pipeline.pipeline);
    render_pass.set_bind_group(0, camera_bind_group, &[]);
    render_pass.set_bind_group(1, light_bind_group, &[]);
    render_pass.set_bind_group(2, shadow_bind_group, &[]);
    mesh.bind(render_pass);
    mesh.draw(render_pass);
}

/// WGSL shader source for lit planet rendering with cascaded shadow maps.
///
/// Computes normal from world position (assumes sphere at origin).
/// Applies N·L diffuse lighting plus a small ambient term.
/// Samples the correct shadow cascade with smooth blending at boundaries.
pub const LIT_SHADER_SOURCE: &str = r#"
struct CameraUniform {
    view_proj: mat4x4<f32>,
};

struct DirectionalLight {
    direction_intensity: vec4<f32>,
    color_padding: vec4<f32>,
};

struct PointLightData {
    position_radius: vec4<f32>,
    color_intensity: vec4<f32>,
    _padding: vec4<f32>,
};

struct PointLightBuffer {
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    lights: array<PointLightData>,
};

struct ShadowUniforms {
    light_matrices: array<mat4x4<f32>, 4>,
    cascade_far: vec4<f32>,
    cascade_count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var<uniform> sun: DirectionalLight;

@group(1) @binding(1)
var<storage, read> point_lights: PointLightBuffer;

@group(2) @binding(0)
var<uniform> shadow_uniforms: ShadowUniforms;

@group(2) @binding(1)
var shadow_map_texture: texture_depth_2d_array;

@group(2) @binding(2)
var shadow_sampler: sampler_comparison;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world_position: vec3<f32>,
};

fn point_light_attenuation(dist: f32, radius: f32) -> f32 {
    if dist >= radius {
        return 0.0;
    }
    let inv_sq = 1.0 / (dist * dist + 1.0);
    let ratio = dist / radius;
    let t = max(1.0 - ratio * ratio, 0.0);
    let window = t * t;
    return inv_sq * window;
}

fn point_light_contribution(
    frag_pos: vec3<f32>,
    normal: vec3<f32>,
) -> vec3<f32> {
    var total = vec3<f32>(0.0);
    let count = point_lights.count;
    for (var i = 0u; i < count; i++) {
        let light = point_lights.lights[i];
        let to_light = light.position_radius.xyz - frag_pos;
        let dist = length(to_light);
        let radius = light.position_radius.w;
        if dist >= radius { continue; }
        let atten = point_light_attenuation(dist, radius);
        let n_dot_l = max(dot(normal, normalize(to_light)), 0.0);
        total += light.color_intensity.xyz * light.color_intensity.w * atten * n_dot_l;
    }
    return total;
}

fn shadow_for_cascade(world_pos: vec3<f32>, cascade_idx: i32) -> f32 {
    let light_pos = shadow_uniforms.light_matrices[cascade_idx] * vec4<f32>(world_pos, 1.0);
    let shadow_coord = light_pos.xyz / light_pos.w;
    // Map from NDC [-1,1] to UV [0,1]. Y is flipped for texture coords.
    let uv = vec2<f32>(shadow_coord.x * 0.5 + 0.5, -shadow_coord.y * 0.5 + 0.5);

    // Clamp: if outside [0,1] treat as fully lit.
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return 1.0;
    }

    // Sample with hardware PCF via comparison sampler.
    // Reverse-Z: shadow_coord.z is depth, comparison is GreaterEqual.
    return textureSampleCompareLevel(
        shadow_map_texture,
        shadow_sampler,
        uv,
        cascade_idx,
        shadow_coord.z,
    );
}

fn blended_shadow_factor(world_pos: vec3<f32>, view_depth: f32) -> f32 {
    if shadow_uniforms.cascade_count == 0u { return 1.0; }

    // Select cascade based on view-space depth.
    var cascade_idx = i32(shadow_uniforms.cascade_count) - 1;
    for (var i = 0; i < i32(shadow_uniforms.cascade_count); i++) {
        if view_depth < shadow_uniforms.cascade_far[i] {
            cascade_idx = i;
            break;
        }
    }

    let s1 = shadow_for_cascade(world_pos, cascade_idx);

    // Blend at cascade boundary.
    let blend_start = shadow_uniforms.cascade_far[cascade_idx] * 0.95;
    if view_depth > blend_start && cascade_idx + 1 < i32(shadow_uniforms.cascade_count) {
        let s2 = shadow_for_cascade(world_pos, cascade_idx + 1);
        let t = (view_depth - blend_start) / (shadow_uniforms.cascade_far[cascade_idx] - blend_start);
        return mix(s1, s2, t);
    }

    return s1;
}

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

    // Approximate view depth as distance from origin (camera orbits around origin).
    let view_depth = length(in.world_position);

    // Shadow factor from cascaded shadow maps.
    let shadow = blended_shadow_factor(in.world_position, view_depth);

    // N dot L: negate sun direction because it points FROM the light.
    let n_dot_l = max(dot(normal, -sun.direction_intensity.xyz), 0.0);

    // Directional (sun) diffuse contribution, modulated by shadow.
    let diffuse = sun.color_padding.xyz * sun.direction_intensity.w * n_dot_l * shadow;

    // Point light contribution (not shadowed for now).
    let point = point_light_contribution(in.world_position, normal);

    // Small ambient term so shadowed areas aren't pure black.
    let ambient = vec3<f32>(0.08, 0.08, 0.12);

    let lit_color = in.color.rgb * (diffuse + point + ambient);
    return vec4<f32>(lit_color, in.color.a);
}
"#;
