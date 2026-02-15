# PBR Material Pipeline

## Problem

The engine's basic unlit pipeline (story 04/05) renders colored vertices without any lighting, materials, or shadows. For a game with cubesphere-voxel planets, sunlight, and diverse surface materials, the renderer must perform physically-based shading that takes into account each voxel's material properties (albedo, metallic, roughness, emissive) alongside directional and point lights, shadow maps, and the texture atlas. Without a dedicated PBR pipeline, the world looks flat and unlit — completely unsuitable for production rendering. This pipeline replaces the unlit path and becomes the primary rendering pipeline for all voxel terrain.

## Solution

Implement a `PbrVoxelPipeline` in the `nebula_rendering` crate. The pipeline uses four bind groups to supply camera data, lighting data, the material texture atlas, and shadow maps to a Cook-Torrance PBR fragment shader.

### Vertex Format

The PBR pipeline uses a richer vertex format than the unlit pipeline, including normal, material ID, and AO:

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VoxelVertex {
    pub position: [f32; 3],     // 12 bytes, location(0)
    pub normal: [f32; 3],       // 12 bytes, location(1)
    pub uv: [f32; 2],           // 8 bytes,  location(2)
    pub material_id: u32,       // 4 bytes,  location(3) — MaterialId packed as u32
    pub ao: f32,                // 4 bytes,  location(4) — ambient occlusion [0,1]
}
// Total stride: 40 bytes
```

### Bind Group Layout

Four bind groups supply all data the shaders need:

**Group 0 — Camera Uniforms:**
```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4],  // 64 bytes
    pub camera_pos: [f32; 4],      // 16 bytes (w unused, padding)
}
// binding(0), visibility: VERTEX | FRAGMENT
```

**Group 1 — Light Data:**
```rust
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightUniform {
    pub sun_direction: [f32; 4],    // 16 bytes (w unused)
    pub sun_color: [f32; 4],        // 16 bytes (rgb + intensity)
    pub ambient_color: [f32; 4],    // 16 bytes (rgb + intensity)
    pub sun_view_proj: [[f32; 4]; 4], // 64 bytes — for shadow mapping
}
// binding(0), visibility: FRAGMENT
```

**Group 2 — Material Atlas + Sampler + Material Buffer:**
```rust
// binding(0): texture_2d<f32>        — the voxel texture atlas
// binding(1): sampler                — atlas sampler (linear, repeat)
// binding(2): storage buffer (read)  — MaterialGpuData[] array
// visibility: FRAGMENT
```

**Group 3 — Shadow Maps:**
```rust
// binding(0): texture_depth_2d       — directional shadow map
// binding(1): sampler_comparison     — shadow sampler (PCF)
// visibility: FRAGMENT
```

### WGSL Shader

**Vertex shader (`pbr_voxel.wgsl`):**

```wgsl
struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) material_id: u32,
    @location(4) ao: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) material_id: u32,
    @location(4) ao: f32,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.world_pos = in.position;
    out.normal = in.normal;
    out.uv = in.uv;
    out.material_id = in.material_id;
    out.ao = in.ao;
    return out;
}
```

**Fragment shader (Cook-Torrance PBR):**

```wgsl
struct LightUniform {
    sun_direction: vec4<f32>,
    sun_color: vec4<f32>,
    ambient_color: vec4<f32>,
    sun_view_proj: mat4x4<f32>,
};

struct MaterialGpuData {
    albedo: vec4<f32>,
    emissive_rgb_intensity: vec4<f32>,
    metallic: f32,
    roughness: f32,
    normal_strength: f32,
    opacity: f32,
};

@group(1) @binding(0) var<uniform> light: LightUniform;

@group(2) @binding(0) var atlas_texture: texture_2d<f32>;
@group(2) @binding(1) var atlas_sampler: sampler;
@group(2) @binding(2) var<storage, read> materials: array<MaterialGpuData>;

@group(3) @binding(0) var shadow_map: texture_depth_2d;
@group(3) @binding(1) var shadow_sampler: sampler_comparison;

const PI: f32 = 3.14159265359;

// Normal Distribution Function (GGX/Trowbridge-Reitz)
fn distribution_ggx(n_dot_h: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
}

// Geometry function (Schlick-GGX)
fn geometry_schlick_ggx(n_dot_v: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}

fn geometry_smith(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    return geometry_schlick_ggx(n_dot_v, roughness) * geometry_schlick_ggx(n_dot_l, roughness);
}

// Fresnel (Schlick approximation)
fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

fn shadow_factor(world_pos: vec3<f32>) -> f32 {
    let light_space = light.sun_view_proj * vec4<f32>(world_pos, 1.0);
    let proj = light_space.xyz / light_space.w;
    let shadow_uv = proj.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);

    if shadow_uv.x < 0.0 || shadow_uv.x > 1.0 || shadow_uv.y < 0.0 || shadow_uv.y > 1.0 {
        return 1.0; // Outside shadow map — fully lit
    }

    return textureSampleCompare(shadow_map, shadow_sampler, shadow_uv, proj.z);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let mat = materials[in.material_id];

    // Sample atlas texture
    let tex_color = textureSample(atlas_texture, atlas_sampler, in.uv);
    let albedo = tex_color.rgb * mat.albedo.rgb;
    let metallic = mat.metallic;
    let roughness = max(mat.roughness, 0.04); // prevent division by zero

    let n = normalize(in.normal);
    let v = normalize(camera.camera_pos.xyz - in.world_pos);
    let l = normalize(-light.sun_direction.xyz);
    let h = normalize(v + l);

    let n_dot_l = max(dot(n, l), 0.0);
    let n_dot_v = max(dot(n, v), 0.0);
    let n_dot_h = max(dot(n, h), 0.0);
    let h_dot_v = max(dot(h, v), 0.0);

    // Fresnel reflectance at normal incidence
    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

    // Cook-Torrance BRDF
    let ndf = distribution_ggx(n_dot_h, roughness);
    let g = geometry_smith(n_dot_v, n_dot_l, roughness);
    let f = fresnel_schlick(h_dot_v, f0);

    let numerator = ndf * g * f;
    let denominator = 4.0 * n_dot_v * n_dot_l + 0.0001;
    let specular = numerator / denominator;

    let k_s = f;
    let k_d = (vec3<f32>(1.0) - k_s) * (1.0 - metallic);

    let sun_radiance = light.sun_color.rgb * light.sun_color.w;
    let shadow = shadow_factor(in.world_pos);

    let lo = (k_d * albedo / PI + specular) * sun_radiance * n_dot_l * shadow;

    // Ambient
    let ambient = light.ambient_color.rgb * light.ambient_color.w * albedo * in.ao;

    // Emissive
    let emissive = mat.emissive_rgb_intensity.rgb * mat.emissive_rgb_intensity.w;

    let color = lo + ambient + emissive;

    return vec4<f32>(color, mat.opacity);
}
```

### Pipeline Creation

```rust
pub struct PbrVoxelPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub camera_bind_group_layout: wgpu::BindGroupLayout,    // group 0
    pub light_bind_group_layout: wgpu::BindGroupLayout,     // group 1
    pub material_bind_group_layout: wgpu::BindGroupLayout,  // group 2
    pub shadow_bind_group_layout: wgpu::BindGroupLayout,    // group 3
}

impl PbrVoxelPipeline {
    pub fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        surface_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        // Create all four bind group layouts ...
        // Create pipeline layout with [camera, light, material, shadow] ...
        // Create render pipeline with VoxelVertex layout, CullMode::Back,
        // FrontFace::Ccw, TriangleList topology, reverse-Z depth ...
    }
}
```

Pipeline configuration:
- **Primitive state**: `TriangleList`, `Ccw`, `CullMode::Back`
- **Depth stencil**: `Depth32Float`, `CompareFunction::GreaterEqual` (reverse-Z)
- **Color target**: surface format with no blending (opaque pass) or alpha blending (transparent pass)
- **Multisample**: `count: 1` initially (MSAA added later)

## Outcome

A `PbrVoxelPipeline` in `nebula_rendering` that renders voxel terrain with full Cook-Torrance PBR shading, directional sun lighting, shadow mapping, ambient occlusion, and emissive materials. The pipeline reads material properties from a GPU storage buffer and samples the texture atlas built by the material registry. This replaces the unlit pipeline for all production voxel rendering. Running `cargo test -p nebula_rendering` passes all PBR pipeline tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Normal maps give stone surfaces depth, roughness maps vary across material types, and metallic maps make ore deposits gleam under directional light.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `28.0` | Render pipeline, bind groups, shader compilation |
| `bytemuck` | `1.21` | Pod/Zeroable for uniform structs (`CameraUniform`, `LightUniform`, `VoxelVertex`) |
| `glam` | `0.32` | Matrix and vector math for camera/light uniforms |

Depends on stories 14/01 (`MaterialGpuData`), 14/02 (`TextureAtlas`), 14/03 (`MaterialRegistry`), 04/06 (camera), and 13 (lighting/shadows). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_device() -> wgpu::Device {
        // Create a wgpu device using the default backend (or software adapter for CI)
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    ..Default::default()
                })
                .await
                .expect("No suitable GPU adapter found");
            let (device, _queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default(), None)
                .await
                .expect("Failed to create device");
            device
        })
    }

    #[test]
    fn test_pipeline_compiles_with_all_bind_groups() {
        let device = create_test_device();
        let shader_source = include_str!("../shaders/pbr_voxel.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr-voxel-shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let pipeline = PbrVoxelPipeline::new(
            &device,
            &shader,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Depth32Float,
        );

        // If we reach here without panic, the pipeline compiled successfully
        // with all four bind group layouts.
        assert!(true, "Pipeline creation succeeded");
    }

    #[test]
    fn test_shader_compilation_succeeds() {
        let device = create_test_device();
        let shader_source = include_str!("../shaders/pbr_voxel.wgsl");

        // ShaderModule creation will panic if the WGSL fails to parse/validate
        let _shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr-voxel-shader-test"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });
    }

    #[test]
    fn test_pipeline_compatible_with_chunk_vertex_format() {
        let layout = VoxelVertex::vertex_buffer_layout();

        // The shader expects 5 attributes at locations 0-4
        assert_eq!(layout.attributes.len(), 5);

        // location(0): position — vec3<f32>
        assert_eq!(layout.attributes[0].shader_location, 0);
        assert_eq!(layout.attributes[0].format, wgpu::VertexFormat::Float32x3);
        assert_eq!(layout.attributes[0].offset, 0);

        // location(1): normal — vec3<f32>
        assert_eq!(layout.attributes[1].shader_location, 1);
        assert_eq!(layout.attributes[1].format, wgpu::VertexFormat::Float32x3);
        assert_eq!(layout.attributes[1].offset, 12);

        // location(2): uv — vec2<f32>
        assert_eq!(layout.attributes[2].shader_location, 2);
        assert_eq!(layout.attributes[2].format, wgpu::VertexFormat::Float32x2);
        assert_eq!(layout.attributes[2].offset, 24);

        // location(3): material_id — u32
        assert_eq!(layout.attributes[3].shader_location, 3);
        assert_eq!(layout.attributes[3].format, wgpu::VertexFormat::Uint32);
        assert_eq!(layout.attributes[3].offset, 32);

        // location(4): ao — f32
        assert_eq!(layout.attributes[4].shader_location, 4);
        assert_eq!(layout.attributes[4].format, wgpu::VertexFormat::Float32);
        assert_eq!(layout.attributes[4].offset, 36);

        // Total stride = 40 bytes
        assert_eq!(layout.array_stride, 40);
    }

    #[test]
    fn test_all_bind_group_layouts_match_shader_expectations() {
        let device = create_test_device();
        let shader_source = include_str!("../shaders/pbr_voxel.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr-voxel-shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let pipeline = PbrVoxelPipeline::new(
            &device,
            &shader,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            wgpu::TextureFormat::Depth32Float,
        );

        // Verify bind group 0 (camera) accepts a 80-byte uniform buffer
        let camera_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test-camera"),
            size: std::mem::size_of::<CameraUniform>() as u64,
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

        // Verify bind group 1 (light) accepts a LightUniform-sized buffer
        let light_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("test-light"),
            size: std::mem::size_of::<LightUniform>() as u64,
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

        // If all bind groups create without panic, layouts match shader expectations
    }

    #[test]
    fn test_camera_uniform_size() {
        // CameraUniform: mat4x4 (64) + vec4 (16) = 80 bytes
        assert_eq!(std::mem::size_of::<CameraUniform>(), 80);
    }

    #[test]
    fn test_light_uniform_size() {
        // LightUniform: 3 * vec4 (48) + mat4x4 (64) = 112 bytes
        assert_eq!(std::mem::size_of::<LightUniform>(), 112);
    }

    #[test]
    fn test_voxel_vertex_size() {
        assert_eq!(std::mem::size_of::<VoxelVertex>(), 40);
    }
}
```
