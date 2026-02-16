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

/// Lit rendering pipeline: camera at group 0, light at group 1, shadows at group 2, material at group 3.
pub struct LitPipeline {
    /// The underlying wgpu render pipeline.
    pub pipeline: wgpu::RenderPipeline,
    /// Camera uniform bind group layout (group 0).
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
    /// Directional light uniform bind group layout (group 1).
    pub light_bind_group_layout: wgpu::BindGroupLayout,
    /// Shadow map bind group layout (group 2).
    pub shadow_bind_group_layout: wgpu::BindGroupLayout,
    /// PBR material uniform bind group layout (group 3).
    pub material_bind_group_layout: wgpu::BindGroupLayout,
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
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(80), // CameraUniform: mat4x4 + vec4
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

        let material_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("lit-material-bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(32), // PbrMaterialUniform
                    },
                    count: None,
                }],
            });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lit-pipeline-layout"),
            bind_group_layouts: &[
                &camera_bind_group_layout,
                &light_bind_group_layout,
                &shadow_bind_group_layout,
                &material_bind_group_layout,
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
            material_bind_group_layout,
        }
    }
}

/// Draw lit geometry with camera, light, shadow, and material bind groups.
pub fn draw_lit<'a>(
    render_pass: &mut wgpu::RenderPass<'a>,
    pipeline: &LitPipeline,
    camera_bind_group: &'a wgpu::BindGroup,
    light_bind_group: &'a wgpu::BindGroup,
    shadow_bind_group: &'a wgpu::BindGroup,
    material_bind_group: &'a wgpu::BindGroup,
    mesh: &'a MeshBuffer,
) {
    render_pass.set_pipeline(&pipeline.pipeline);
    render_pass.set_bind_group(0, camera_bind_group, &[]);
    render_pass.set_bind_group(1, light_bind_group, &[]);
    render_pass.set_bind_group(2, shadow_bind_group, &[]);
    render_pass.set_bind_group(3, material_bind_group, &[]);
    mesh.bind(render_pass);
    mesh.draw(render_pass);
}

/// WGSL shader source for lit planet rendering with PBR shading and cascaded shadow maps.
///
/// Implements Cook-Torrance BRDF with GGX distribution, Schlick Fresnel,
/// and Smith geometry terms. Material properties come from a uniform buffer
/// (group 3). Vertex color modulates the material albedo.
pub const LIT_SHADER_SOURCE: &str = r#"
const PI: f32 = 3.14159265359;

struct CameraUniform {
    view_proj: mat4x4<f32>,
    position: vec4<f32>,
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

struct PbrMaterial {
    albedo_metallic: vec4<f32>,
    roughness_ao_pad: vec4<f32>,
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

@group(3) @binding(0)
var<uniform> material: PbrMaterial;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world_position: vec3<f32>,
};

// --- PBR BRDF Functions ---

fn distribution_ggx(n_dot_h: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
}

fn geometry_schlick_ggx(n_dot: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    return n_dot / (n_dot * (1.0 - k) + k);
}

fn geometry_smith(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    return geometry_schlick_ggx(n_dot_v, roughness) * geometry_schlick_ggx(n_dot_l, roughness);
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

fn evaluate_brdf(
    light_dir: vec3<f32>,
    view_dir: vec3<f32>,
    normal: vec3<f32>,
    albedo: vec3<f32>,
    metallic: f32,
    roughness: f32,
) -> vec3<f32> {
    let half_vec = normalize(view_dir + light_dir);

    let n_dot_l = max(dot(normal, light_dir), 0.0);
    let n_dot_v = max(dot(normal, view_dir), 0.0);
    let n_dot_h = max(dot(normal, half_vec), 0.0);
    let h_dot_v = max(dot(half_vec, view_dir), 0.0);

    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

    let d = distribution_ggx(n_dot_h, roughness);
    let g = geometry_smith(n_dot_v, n_dot_l, roughness);
    let f = fresnel_schlick(h_dot_v, f0);

    let numerator = d * g * f;
    let denominator = 4.0 * n_dot_v * n_dot_l + 0.0001;
    let specular = numerator / denominator;

    let k_s = f;
    let k_d = (vec3<f32>(1.0) - k_s) * (1.0 - metallic);
    let diffuse = k_d * albedo / PI;

    return (diffuse + specular) * n_dot_l;
}

// --- Attenuation & Shadow ---

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

fn shadow_for_cascade(world_pos: vec3<f32>, cascade_idx: i32) -> f32 {
    let light_pos = shadow_uniforms.light_matrices[cascade_idx] * vec4<f32>(world_pos, 1.0);
    let shadow_coord = light_pos.xyz / light_pos.w;
    let uv = vec2<f32>(shadow_coord.x * 0.5 + 0.5, -shadow_coord.y * 0.5 + 0.5);

    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
        return 1.0;
    }

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

    var cascade_idx = i32(shadow_uniforms.cascade_count) - 1;
    for (var i = 0; i < i32(shadow_uniforms.cascade_count); i++) {
        if view_depth < shadow_uniforms.cascade_far[i] {
            cascade_idx = i;
            break;
        }
    }

    let s1 = shadow_for_cascade(world_pos, cascade_idx);

    let blend_start = shadow_uniforms.cascade_far[cascade_idx] * 0.95;
    if view_depth > blend_start && cascade_idx + 1 < i32(shadow_uniforms.cascade_count) {
        let s2 = shadow_for_cascade(world_pos, cascade_idx + 1);
        let t = (view_depth - blend_start) / (shadow_uniforms.cascade_far[cascade_idx] - blend_start);
        return mix(s1, s2, t);
    }

    return s1;
}

// --- Vertex & Fragment ---

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
    let normal = normalize(in.world_position);
    let view_depth = length(in.world_position);
    let view_dir = normalize(camera.position.xyz - in.world_position);

    // Material properties: vertex color modulates material albedo.
    let albedo = in.color.rgb * material.albedo_metallic.xyz;
    let metallic = material.albedo_metallic.w;
    let roughness = material.roughness_ao_pad.x;
    let ao = material.roughness_ao_pad.y;

    // Shadow factor.
    let shadow = blended_shadow_factor(in.world_position, view_depth);

    // Directional light (sun) PBR contribution.
    let sun_dir = -sun.direction_intensity.xyz;
    var color = evaluate_brdf(sun_dir, view_dir, normal, albedo, metallic, roughness)
              * sun.color_padding.xyz * sun.direction_intensity.w * shadow;

    // Point light PBR contributions.
    let count = point_lights.count;
    for (var i = 0u; i < count; i++) {
        let light = point_lights.lights[i];
        let to_light = light.position_radius.xyz - in.world_position;
        let dist = length(to_light);
        let radius = light.position_radius.w;
        if dist >= radius { continue; }
        let atten = point_light_attenuation(dist, radius);
        color += evaluate_brdf(normalize(to_light), view_dir, normal, albedo, metallic, roughness)
               * light.color_intensity.xyz * light.color_intensity.w * atten;
    }

    // Ambient term (simple; IBL comes later).
    let ambient = vec3<f32>(0.03) * albedo * ao;
    color += ambient;

    return vec4<f32>(color, in.color.a);
}
"#;
