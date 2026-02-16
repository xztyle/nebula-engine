// Atmosphere scattering full-screen post-pass shader.
// Rayleigh + Mie single-scattering with ray marching.

struct AtmosphereParams {
    planet_center: vec3<f32>,
    planet_radius: f32,
    atmosphere_radius: f32,
    rayleigh_coefficients: vec3<f32>,
    rayleigh_scale_height: f32,
    mie_coefficient: f32,
    mie_scale_height: f32,
    mie_direction: f32,
    sun_direction: vec3<f32>,
    sun_intensity: f32,
    camera_position: vec3<f32>,
    _padding0: f32,
    inv_view_proj: mat4x4<f32>,
    near_clip: f32,
    far_clip: f32,
    _padding1: vec2<f32>,
};

@group(0) @binding(0) var<uniform> atmo: AtmosphereParams;
@group(0) @binding(1) var depth_tex: texture_depth_2d;
@group(0) @binding(2) var depth_sampler: sampler;

const NUM_SAMPLES: i32 = 16;
const NUM_LIGHT_SAMPLES: i32 = 8;
const PI: f32 = 3.14159265359;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) idx: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32(i32(idx & 1u) * 4 - 1);
    let y = f32(i32(idx >> 1u) * 4 - 1);
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

fn ray_sphere_intersect(
    origin: vec3<f32>, dir: vec3<f32>, center: vec3<f32>, radius: f32
) -> vec2<f32> {
    let oc = origin - center;
    let b = dot(oc, dir);
    let c = dot(oc, oc) - radius * radius;
    let disc = b * b - c;
    if disc < 0.0 {
        return vec2<f32>(-1.0, -1.0);
    }
    let sqrt_disc = sqrt(disc);
    return vec2<f32>(-b - sqrt_disc, -b + sqrt_disc);
}

fn rayleigh_phase(cos_angle: f32) -> f32 {
    return 3.0 / (16.0 * PI) * (1.0 + cos_angle * cos_angle);
}

fn mie_phase(cos_angle: f32, g: f32) -> f32 {
    let g2 = g * g;
    let num = 3.0 * (1.0 - g2) * (1.0 + cos_angle * cos_angle);
    let denom = 8.0 * PI * (2.0 + g2) * pow(1.0 + g2 - 2.0 * g * cos_angle, 1.5);
    return num / denom;
}

@fragment
fn fs_atmosphere(in: VertexOutput) -> @location(0) vec4<f32> {
    // Reconstruct view ray from UV via inverse view-projection
    let ndc = vec4<f32>(in.uv.x * 2.0 - 1.0, (1.0 - in.uv.y) * 2.0 - 1.0, 1.0, 1.0);
    let world_far = atmo.inv_view_proj * ndc;
    let world_far3 = world_far.xyz / world_far.w;
    let ray_dir = normalize(world_far3 - atmo.camera_position);

    let atmo_hit = ray_sphere_intersect(
        atmo.camera_position, ray_dir, atmo.planet_center, atmo.atmosphere_radius
    );

    if atmo_hit.x > atmo_hit.y || atmo_hit.y < 0.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let t_start = max(atmo_hit.x, 0.0);

    // Sample depth buffer to limit atmosphere to in front of terrain
    let depth_val = textureSample(depth_tex, depth_sampler, in.uv);
    var t_end = atmo_hit.y;
    if depth_val > 0.0 {
        let depth_ndc = vec4<f32>(
            in.uv.x * 2.0 - 1.0, (1.0 - in.uv.y) * 2.0 - 1.0, depth_val, 1.0
        );
        let depth_world = atmo.inv_view_proj * depth_ndc;
        let depth_world3 = depth_world.xyz / depth_world.w;
        let scene_dist = length(depth_world3 - atmo.camera_position);
        t_end = min(t_end, scene_dist);
    }

    if t_end <= t_start {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    // Check planet surface intersection
    let planet_hit = ray_sphere_intersect(
        atmo.camera_position, ray_dir, atmo.planet_center, atmo.planet_radius
    );
    if planet_hit.x > 0.0 {
        t_end = min(t_end, planet_hit.x);
    }

    let step_size = (t_end - t_start) / f32(NUM_SAMPLES);
    var total_rayleigh = vec3<f32>(0.0);
    var total_mie = vec3<f32>(0.0);
    var optical_depth_r = 0.0;
    var optical_depth_m = 0.0;

    let cos_angle = dot(ray_dir, atmo.sun_direction);
    let phase_r = rayleigh_phase(cos_angle);
    let phase_m = mie_phase(cos_angle, atmo.mie_direction);

    for (var i = 0; i < NUM_SAMPLES; i++) {
        let t = t_start + (f32(i) + 0.5) * step_size;
        let sample_pos = atmo.camera_position + ray_dir * t;
        let height = length(sample_pos - atmo.planet_center) - atmo.planet_radius;

        let density_r = exp(-height / atmo.rayleigh_scale_height) * step_size;
        let density_m = exp(-height / atmo.mie_scale_height) * step_size;

        optical_depth_r += density_r;
        optical_depth_m += density_m;

        let light_hit = ray_sphere_intersect(
            sample_pos, atmo.sun_direction, atmo.planet_center, atmo.atmosphere_radius
        );
        let light_step = light_hit.y / f32(NUM_LIGHT_SAMPLES);
        var light_depth_r = 0.0;
        var light_depth_m = 0.0;

        for (var j = 0; j < NUM_LIGHT_SAMPLES; j++) {
            let lt = (f32(j) + 0.5) * light_step;
            let light_pos = sample_pos + atmo.sun_direction * lt;
            let light_height = length(light_pos - atmo.planet_center) - atmo.planet_radius;
            light_depth_r += exp(-light_height / atmo.rayleigh_scale_height) * light_step;
            light_depth_m += exp(-light_height / atmo.mie_scale_height) * light_step;
        }

        let tau = atmo.rayleigh_coefficients * (optical_depth_r + light_depth_r)
                + vec3<f32>(atmo.mie_coefficient) * (optical_depth_m + light_depth_m);
        let attenuation = exp(-tau);

        total_rayleigh += density_r * attenuation;
        total_mie += density_m * attenuation;
    }

    let color = atmo.sun_intensity * (
        phase_r * atmo.rayleigh_coefficients * total_rayleigh
        + phase_m * atmo.mie_coefficient * total_mie
    );

    let mapped = color / (color + vec3<f32>(1.0));

    return vec4<f32>(mapped, 1.0);
}
