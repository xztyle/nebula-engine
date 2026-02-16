// Impostor billboard shader for distant planets.
//
// Renders a camera-facing textured quad with alpha discard
// for the transparent background of the pre-rendered snapshot.

struct CameraUniform {
    view_proj: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;

@group(1) @binding(0) var impostor_texture: texture_2d<f32>;
@group(1) @binding(1) var impostor_sampler: sampler;

struct ImpostorVertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
};

struct ImpostorVertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_impostor(in: ImpostorVertexInput) -> ImpostorVertexOutput {
    var out: ImpostorVertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_impostor(in: ImpostorVertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(impostor_texture, impostor_sampler, in.uv);
    // Discard transparent pixels (background of the impostor snapshot).
    if color.a < 0.01 {
        discard;
    }
    return color;
}
