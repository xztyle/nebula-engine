// PBR Voxel Shader — Cook-Torrance BRDF with material blending and triplanar projection.

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
    @location(3) material_id_a: u32,
    @location(4) material_id_b: u32,
    @location(5) blend_weight: f32,
    @location(6) ao: f32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) material_id_a: u32,
    @location(4) @interpolate(flat) material_id_b: u32,
    @location(5) blend_weight: f32,
    @location(6) ao: f32,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.world_pos = in.position;
    out.normal = in.normal;
    out.uv = in.uv;
    out.material_id_a = in.material_id_a;
    out.material_id_b = in.material_id_b;
    out.blend_weight = in.blend_weight;
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

// --- Triplanar Sampling ---

fn triplanar_sample(
    world_pos: vec3<f32>,
    normal: vec3<f32>,
    uv_offset: vec2<f32>,
    tile_scale: f32,
) -> vec4<f32> {
    // Compute blend weights from the absolute normal components
    var blend = abs(normal);
    // Sharpen the blend to reduce the transition band
    blend = pow(blend, vec3<f32>(4.0));
    // Normalize so weights sum to 1.0
    blend = blend / (blend.x + blend.y + blend.z);

    // Sample along each axis projection
    let uv_x = fract(world_pos.yz * tile_scale) + uv_offset;
    let uv_y = fract(world_pos.xz * tile_scale) + uv_offset;
    let uv_z = fract(world_pos.xy * tile_scale) + uv_offset;

    let tex_x = textureSample(atlas_texture, atlas_sampler, uv_x);
    let tex_y = textureSample(atlas_texture, atlas_sampler, uv_y);
    let tex_z = textureSample(atlas_texture, atlas_sampler, uv_z);

    return tex_x * blend.x + tex_y * blend.y + tex_z * blend.z;
}

// --- Fragment ---

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let mat_a = materials[in.material_id_a];
    let mat_b = materials[in.material_id_b];

    let anim_a = anim_data[in.material_id_a];
    let anim_b = anim_data[in.material_id_b];

    let n = normalize(in.normal);

    // Determine if triplanar projection is needed.
    // Use triplanar when the dominant normal component is < 0.7 (slope > ~45 degrees)
    let max_component = max(abs(n.x), max(abs(n.y), abs(n.z)));
    let use_triplanar = max_component < 0.7;

    var tex_a: vec4<f32>;
    var tex_b: vec4<f32>;

    if use_triplanar {
        tex_a = triplanar_sample(in.world_pos, n, anim_a.uv_offset, 1.0);
        tex_b = triplanar_sample(in.world_pos, n, anim_b.uv_offset, 1.0);
    } else {
        let animated_uv_a = in.uv + anim_a.uv_offset;
        let animated_uv_b = in.uv + anim_b.uv_offset;
        tex_a = textureSample(atlas_texture, atlas_sampler, animated_uv_a);
        tex_b = textureSample(atlas_texture, atlas_sampler, animated_uv_b);
    }

    // Blend between material A and material B
    let w = in.blend_weight;
    let albedo = mix(tex_a.rgb * mat_a.albedo.rgb, tex_b.rgb * mat_b.albedo.rgb, w);
    let metallic = mix(mat_a.metallic, mat_b.metallic, w);
    let roughness = max(mix(mat_a.roughness, mat_b.roughness, w), 0.04);
    let emissive_a = mat_a.emissive_rgb_intensity.rgb * mat_a.emissive_rgb_intensity.w;
    let emissive_b = mat_b.emissive_rgb_intensity.rgb * mat_b.emissive_rgb_intensity.w;
    let emissive = mix(emissive_a, emissive_b, w);
    let opacity = mix(mat_a.opacity, mat_b.opacity, w);

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

    let color = lo + ambient + emissive;

    return vec4<f32>(color, opacity);
}
