// PBR Voxel Shader — Cook-Torrance BRDF with per-vertex material IDs.

const PI: f32 = 3.14159265359;

// --- Bind Group 0: Camera ---

struct CameraUniform {
    view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

// --- Bind Group 1: Light ---

struct LightUniform {
    sun_direction: vec4<f32>,
    sun_color: vec4<f32>,
    ambient_color: vec4<f32>,
    sun_view_proj: mat4x4<f32>,
};

@group(1) @binding(0)
var<uniform> light: LightUniform;

// --- Bind Group 2: Material Atlas + Buffer ---

struct MaterialGpuData {
    albedo: vec4<f32>,
    emissive_rgb_intensity: vec4<f32>,
    metallic: f32,
    roughness: f32,
    normal_strength: f32,
    opacity: f32,
};

struct AnimationGpuData {
    uv_offset: vec2<f32>,
    _padding: vec2<f32>,
};

@group(2) @binding(0) var atlas_texture: texture_2d<f32>;
@group(2) @binding(1) var atlas_sampler: sampler;
@group(2) @binding(2) var<storage, read> materials: array<MaterialGpuData>;
@group(2) @binding(3) var<storage, read> anim_data: array<AnimationGpuData>;

// --- Bind Group 3: Shadow Map ---

@group(3) @binding(0) var shadow_map: texture_depth_2d;
@group(3) @binding(1) var shadow_sampler: sampler_comparison;

// --- Vertex I/O ---

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

// --- PBR BRDF ---

fn distribution_ggx(n_dot_h: f32, roughness: f32) -> f32 {
    let a = roughness * roughness;
    let a2 = a * a;
    let denom = n_dot_h * n_dot_h * (a2 - 1.0) + 1.0;
    return a2 / (PI * denom * denom);
}

fn geometry_schlick_ggx(n_dot_v: f32, roughness: f32) -> f32 {
    let r = roughness + 1.0;
    let k = (r * r) / 8.0;
    return n_dot_v / (n_dot_v * (1.0 - k) + k);
}

fn geometry_smith(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    return geometry_schlick_ggx(n_dot_v, roughness) * geometry_schlick_ggx(n_dot_l, roughness);
}

fn fresnel_schlick(cos_theta: f32, f0: vec3<f32>) -> vec3<f32> {
    return f0 + (1.0 - f0) * pow(clamp(1.0 - cos_theta, 0.0, 1.0), 5.0);
}

// --- Shadow ---

fn shadow_factor(world_pos: vec3<f32>) -> f32 {
    let light_space = light.sun_view_proj * vec4<f32>(world_pos, 1.0);
    let proj = light_space.xyz / light_space.w;
    let shadow_uv = proj.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);

    if shadow_uv.x < 0.0 || shadow_uv.x > 1.0 || shadow_uv.y < 0.0 || shadow_uv.y > 1.0 {
        return 1.0;
    }

    return textureSampleCompare(shadow_map, shadow_sampler, shadow_uv, proj.z);
}

// --- Fragment ---

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let mat = materials[in.material_id];

    let anim = anim_data[in.material_id];
    let animated_uv = in.uv + anim.uv_offset;
    let tex_color = textureSample(atlas_texture, atlas_sampler, animated_uv);
    let albedo = tex_color.rgb * mat.albedo.rgb;
    let metallic = mat.metallic;
    let roughness = max(mat.roughness, 0.04);

    let n = normalize(in.normal);
    let v = normalize(camera.camera_pos.xyz - in.world_pos);
    let l = normalize(-light.sun_direction.xyz);
    let h = normalize(v + l);

    let n_dot_l = max(dot(n, l), 0.0);
    let n_dot_v = max(dot(n, v), 0.0);
    let n_dot_h = max(dot(n, h), 0.0);
    let h_dot_v = max(dot(h, v), 0.0);

    let f0 = mix(vec3<f32>(0.04), albedo, metallic);

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

    let ambient = light.ambient_color.rgb * light.ambient_color.w * albedo * in.ao;

    let emissive = mat.emissive_rgb_intensity.rgb * mat.emissive_rgb_intensity.w;

    let color = lo + ambient + emissive;

    return vec4<f32>(color, mat.opacity);
}
