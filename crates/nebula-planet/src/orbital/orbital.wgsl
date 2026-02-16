// Orbital planet rendering shader.
// Renders a textured sphere with Lambert diffuse lighting.

struct CameraUniform {
    view_proj: mat4x4<f32>,
};

struct PlanetUniform {
    model: mat4x4<f32>,
    sun_direction: vec3<f32>,
    planet_radius: f32,
    blend_alpha: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;

@group(1) @binding(0) var terrain_texture: texture_2d<f32>;
@group(1) @binding(1) var terrain_sampler: sampler;
@group(1) @binding(2) var<uniform> planet: PlanetUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) world_position: vec3<f32>,
};

@vertex
fn vs_orbital(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world_pos = planet.model * vec4<f32>(in.position, 1.0);
    out.clip_position = camera.view_proj * world_pos;
    out.uv = in.uv;
    out.world_normal = normalize((planet.model * vec4<f32>(in.normal, 0.0)).xyz);
    out.world_position = world_pos.xyz;
    return out;
}

@fragment
fn fs_orbital(in: VertexOutput) -> @location(0) vec4<f32> {
    let terrain_color = textureSample(terrain_texture, terrain_sampler, in.uv).rgb;

    // Lambert diffuse lighting
    let ndotl = max(dot(in.world_normal, planet.sun_direction), 0.0);
    let ambient = vec3<f32>(0.08, 0.08, 0.12);
    let lit_color = terrain_color * (ambient + ndotl * vec3<f32>(1.0, 0.98, 0.92));

    return vec4<f32>(lit_color, planet.blend_alpha);
}
