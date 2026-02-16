// Ocean surface shader: animated waves, Fresnel reflections, depth-based coloring.

struct CameraUniform {
    view_proj: mat4x4<f32>,
};

struct OceanParams {
    deep_color: vec3<f32>,
    color_depth: f32,
    shallow_color: vec3<f32>,
    wave_amplitude: f32,
    wave_frequency: f32,
    wave_speed: f32,
    fresnel_f0: f32,
    time: f32,
    sun_direction: vec3<f32>,
    ocean_radius: f32,
    camera_position: vec3<f32>,
    _padding: f32,
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<uniform> ocean: OceanParams;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
};

@vertex
fn vs_ocean(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Base position on the ocean sphere.
    let normal = normalize(in.position);
    let sphere_pos = normal * ocean.ocean_radius;

    // Animated wave displacement along the surface normal.
    let wave1 = sin(
        dot(in.position.xz, vec2<f32>(1.0, 0.0)) * ocean.wave_frequency
        + ocean.time * ocean.wave_speed
    ) * ocean.wave_amplitude;
    let wave2 = sin(
        dot(in.position.xz, vec2<f32>(0.7, 0.7)) * ocean.wave_frequency * 1.3
        + ocean.time * ocean.wave_speed * 0.8
    ) * ocean.wave_amplitude * 0.5;

    let displaced = sphere_pos + normal * (wave1 + wave2);

    out.world_position = displaced;
    out.clip_position = camera.view_proj * vec4<f32>(displaced, 1.0);
    out.world_normal = normal;
    return out;
}

@fragment
fn fs_ocean(in: VertexOutput) -> @location(0) vec4<f32> {
    // Depth-based color blending: use distance from ocean center as proxy for water depth.
    let surface_dist = length(in.world_position);
    let water_depth = ocean.ocean_radius - surface_dist + ocean.ocean_radius * 0.05;
    let depth_factor = clamp(water_depth / ocean.color_depth, 0.0, 1.0);
    let water_color = mix(
        vec3<f32>(ocean.shallow_color[0], ocean.shallow_color[1], ocean.shallow_color[2]),
        vec3<f32>(ocean.deep_color[0], ocean.deep_color[1], ocean.deep_color[2]),
        depth_factor,
    );

    // Fresnel effect.
    let view_dir = normalize(ocean.camera_position - in.world_position);
    let ndotv = max(dot(in.world_normal, view_dir), 0.0);
    let fresnel = ocean.fresnel_f0 + (1.0 - ocean.fresnel_f0) * pow(1.0 - ndotv, 5.0);

    // Diffuse lighting.
    let sun_dir = vec3<f32>(ocean.sun_direction[0], ocean.sun_direction[1], ocean.sun_direction[2]);
    let ndotl = max(dot(in.world_normal, sun_dir), 0.0);
    let diffuse = water_color * (ndotl * 0.8 + 0.2); // ambient minimum

    // Specular highlight (Blinn-Phong).
    let half_vec = normalize(view_dir + sun_dir);
    let spec = pow(max(dot(in.world_normal, half_vec), 0.0), 256.0);
    let specular = vec3<f32>(1.0) * spec * fresnel;

    let final_color = diffuse * (1.0 - fresnel) + specular;
    return vec4<f32>(final_color, 0.85 + fresnel * 0.15);
}
