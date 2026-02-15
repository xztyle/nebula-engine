# Basic Unlit Pipeline

## Problem

All the infrastructure from the previous stories — device initialization, render pass abstraction, buffer management, shader loading — needs to come together into a working render pipeline that puts pixels on screen. Without a concrete pipeline, there is no way to visually verify that the rendering stack works. The engine needs its "hello triangle" moment: a colored triangle rendered with a view-projection matrix. This first pipeline establishes the pattern that every subsequent pipeline (lit terrain, PBR materials, particles, UI) will follow. Getting the vertex format, bind group layout, shader entry points, and pipeline configuration right here prevents compounding errors in later stories.

## Solution

### Vertex Format

Use the `VertexPositionColor` type from story 03:

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VertexPositionColor {
    pub position: [f32; 3],  // 12 bytes, location(0)
    pub color: [f32; 4],     // 16 bytes, location(1)
}
// Total stride: 28 bytes
```

### WGSL Shaders

**Vertex shader** (`unlit.wgsl`):

```wgsl
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
```

The vertex shader transforms positions by a view-projection matrix and passes the vertex color through to the fragment shader. The fragment shader outputs the interpolated color directly — no lighting calculations. This is the simplest possible 3D pipeline.

### Camera Uniform Buffer

A uniform buffer holding the combined view-projection matrix:

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniform {
    pub view_proj: [[f32; 4]; 4], // 64 bytes, mat4x4
}
```

The bind group layout for the camera uniform:

```rust
let camera_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
```

### Pipeline Creation

```rust
pub struct UnlitPipeline {
    pub pipeline: wgpu::RenderPipeline,
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
}

impl UnlitPipeline {
    pub fn new(
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        surface_format: wgpu::TextureFormat,
        depth_format: Option<wgpu::TextureFormat>,
    ) -> Self { ... }
}
```

Pipeline configuration details:

- **Vertex state**: Uses `VertexPositionColor::layout()` for the single vertex buffer at slot 0.
- **Primitive state**: `PrimitiveTopology::TriangleList`, `FrontFace::Ccw`, `CullMode::Back`. Counter-clockwise winding is the default convention.
- **Fragment state**: Single color target matching `surface_format` with no blending (opaque).
- **Depth stencil state**: If `depth_format` is provided, use `CompareFunction::GreaterEqual` for reverse-Z (per story 07). If `None`, no depth testing.
- **Multisample state**: `count: 1` (no MSAA initially), `mask: !0`, `alpha_to_coverage: false`.
- **Shader entry points**: `vs_main` for vertex, `fs_main` for fragment.

### Drawing

A helper function that draws unlit geometry:

```rust
pub fn draw_unlit(
    render_pass: &mut wgpu::RenderPass,
    pipeline: &UnlitPipeline,
    camera_bind_group: &wgpu::BindGroup,
    mesh: &MeshBuffer,
) {
    render_pass.set_pipeline(&pipeline.pipeline);
    render_pass.set_bind_group(0, camera_bind_group, &[]);
    mesh.bind(render_pass);
    mesh.draw(render_pass);
}
```

### First Visual Test

Create a triangle with three differently-colored vertices:

```rust
let vertices = [
    VertexPositionColor { position: [ 0.0,  0.5, 0.0], color: [1.0, 0.0, 0.0, 1.0] }, // red top
    VertexPositionColor { position: [-0.5, -0.5, 0.0], color: [0.0, 1.0, 0.0, 1.0] }, // green left
    VertexPositionColor { position: [ 0.5, -0.5, 0.0], color: [0.0, 0.0, 1.0, 1.0] }, // blue right
];
let indices: [u16; 3] = [0, 1, 2];
```

With an identity view-projection matrix, this produces a colored triangle centered on screen. This serves as the visual smoke test for the entire rendering stack.

## Outcome

A working `UnlitPipeline` that draws colored triangles on screen using a view-projection matrix. This is the engine's first visual output. The pipeline establishes the pattern for bind group layouts, vertex buffer layouts, shader entry points, and pipeline state configuration that all subsequent pipelines will follow. The colored triangle serves as a visual regression test — if it renders correctly, the device, surface, buffers, shaders, and render pass are all working.

## Demo Integration

**Demo crate:** `nebula-demo`

THE TRIANGLE APPEARS. A single RGB triangle is drawn to the screen — red top, green left, blue right — with smooth color interpolation. This is the first rendered geometry in the demo's history.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | Render pipeline creation and draw calls |
| `bytemuck` | `1.21` | Uniform buffer serialization |
| `glam` | `0.29` | Matrix math for view-projection (used by caller, not by pipeline itself) |

Depends on stories 01 (RenderContext), 02 (render pass), 03 (MeshBuffer), and 04 (ShaderLibrary). Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_creation_succeeds() {
        let device = create_test_device();
        let shader = create_test_shader(&device, UNLIT_SHADER_SOURCE);
        let pipeline = UnlitPipeline::new(
            &device,
            &shader,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            Some(wgpu::TextureFormat::Depth32Float),
        );
        // Pipeline creation should not panic — reaching this line is success.
        assert!(!pipeline.pipeline.global_id().is_invalid());
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
    fn test_primitive_topology_is_triangle_list() {
        // The pipeline should use TriangleList topology, not TriangleStrip or Lines.
        // This is set in the PrimitiveState during pipeline creation.
        // Verified structurally by inspecting the pipeline descriptor.
        let config = UnlitPipelineConfig::default();
        assert_eq!(config.topology, wgpu::PrimitiveTopology::TriangleList);
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
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
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
        let pipeline = UnlitPipeline::new(
            &device,
            &shader,
            wgpu::TextureFormat::Bgra8UnormSrgb,
            None, // no depth
        );
        assert!(!pipeline.pipeline.global_id().is_invalid());
    }
}
```
