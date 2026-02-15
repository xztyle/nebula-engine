# Wireframe Rendering

## Problem

When building a voxel engine with greedy meshing, LOD stitching, and cubesphere geometry, understanding the actual mesh topology is essential for debugging. Developers need to see:

- **Whether greedy meshing is combining faces correctly** — A single flat wall should merge into one large quad, not remain as hundreds of individual voxel faces. Without wireframe, there is no way to verify this visually.
- **Whether LOD seams produce T-junctions** — When adjacent chunks are at different LOD levels, the stitching algorithm must produce matching edge vertices. T-junctions cause visible cracks. Wireframe rendering makes these immediately obvious.
- **Whether cubesphere subdivision is uniform** — The cube-to-sphere projection can cause triangle density to vary across the planet surface. Wireframe shows exactly where triangles are dense or sparse.
- **Whether chunk boundaries align** — Adjacent chunk meshes must share edge vertices exactly. Wireframe rendering reveals any gaps or overlapping edges at chunk boundaries.

Without wireframe, these issues manifest as subtle visual artifacts (cracks, z-fighting, LOD popping) that are hard to diagnose from the solid rendered view alone.

## Solution

### Primary Approach: wgpu PolygonMode::Line

The preferred implementation uses wgpu's built-in polygon mode to render triangles as lines. This requires the `NON_FILL_POLYGON_MODE` feature (`wgpu::Features::NON_FILL_POLYGON_MODE`), which is supported on most desktop Vulkan and Metal backends but not on WebGPU or some mobile GPUs.

Create a second render pipeline identical to the main terrain/mesh pipeline but with the polygon mode set to `Line`:

```rust
pub fn create_wireframe_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    target_format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("wireframe_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[vertex_buffer_layout()],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_wireframe"),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            polygon_mode: wgpu::PolygonMode::Line, // <-- wireframe
            cull_mode: None, // Show back faces in wireframe
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: Default::default(),
            bias: wgpu::DepthBiasState {
                constant: -2, // Bias toward camera to render on top of solid
                slope_scale: -1.0,
                clamp: 0.0,
            },
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}
```

The wireframe fragment shader outputs a constant color (or a color derived from chunk ID):

```wgsl
@fragment
fn fs_wireframe(@builtin(primitive_index) prim_idx: u32) -> @location(0) vec4<f32> {
    // Default: white lines with slight transparency
    return vec4<f32>(1.0, 1.0, 1.0, 0.7);
}
```

### Fallback: Geometry-Based Line Generation

When `NON_FILL_POLYGON_MODE` is not available, the fallback generates actual line geometry from the triangle mesh. For each triangle, extract the three edges and deduplicate shared edges:

```rust
use std::collections::HashSet;

pub fn generate_wireframe_lines(
    indices: &[u32],
    vertices: &[Vertex],
) -> (Vec<WireframeVertex>, Vec<u32>) {
    let mut edges: HashSet<(u32, u32)> = HashSet::new();
    let mut line_vertices = Vec::new();
    let mut line_indices = Vec::new();

    for tri in indices.chunks(3) {
        let [a, b, c] = [tri[0], tri[1], tri[2]];
        for (i, j) in [(a, b), (b, c), (c, a)] {
            let edge = if i < j { (i, j) } else { (j, i) };
            if edges.insert(edge) {
                let idx = line_vertices.len() as u32;
                line_vertices.push(WireframeVertex {
                    position: vertices[i as usize].position,
                });
                line_vertices.push(WireframeVertex {
                    position: vertices[j as usize].position,
                });
                line_indices.push(idx);
                line_indices.push(idx + 1);
            }
        }
    }

    (line_vertices, line_indices)
}
```

This fallback uses `PrimitiveTopology::LineList` and a simple pipeline that does not require `NON_FILL_POLYGON_MODE`. The trade-off is additional memory for the line buffers and CPU time to generate them, but this only happens when wireframe is active.

### Chunk Color-Coding

In color-coded mode, each chunk's wireframe is tinted using a deterministic color derived from the chunk position:

```rust
pub fn chunk_wireframe_color(chunk_pos: &ChunkPos) -> [f32; 4] {
    let hash = (chunk_pos.x.wrapping_mul(73856093)
        ^ chunk_pos.y.wrapping_mul(19349663)
        ^ chunk_pos.z.wrapping_mul(83492791)) as u32;
    let r = ((hash >> 0) & 0xFF) as f32 / 255.0 * 0.7 + 0.3;
    let g = ((hash >> 8) & 0xFF) as f32 / 255.0 * 0.7 + 0.3;
    let b = ((hash >> 16) & 0xFF) as f32 / 255.0 * 0.7 + 0.3;
    [r, g, b, 0.8]
}
```

The color is passed to the wireframe shader via a per-draw push constant or a small uniform buffer.

### Toggle

F4 toggles wireframe rendering. A `WireframeState` resource tracks the current mode:

```rust
pub enum WireframeMode {
    Off,
    White,
    ChunkColored,
}

pub struct WireframeState {
    pub mode: WireframeMode,
    pub use_native_polygon_mode: bool, // Set at init based on feature support
}
```

Pressing F4 cycles through `Off -> White -> ChunkColored -> Off`.

### Render Pass Integration

When wireframe is active, the wireframe pass runs immediately after the solid geometry pass within the same render pass, using the same depth buffer (with `depth_compare: LessEqual` and a depth bias so lines render on top). This ensures wireframe lines overlay the solid geometry correctly without z-fighting.

## Outcome

Pressing F4 toggles a wireframe overlay that renders all triangle edges on top of the solid geometry. The overlay supports two modes: uniform white lines for general topology inspection, and chunk-colored lines for identifying chunk boundaries and LOD transitions. On GPUs that support `PolygonMode::Line`, the implementation uses native wireframe rendering with near-zero overhead. On unsupported platforms, a geometry-based fallback generates line primitives from the triangle mesh. The implementation lives in `crates/nebula-debug/src/wireframe.rs` and integrates with the render pipeline in `nebula-render`.

## Demo Integration

**Demo crate:** `nebula-demo`

A toggle renders all geometry as wireframe lines. The terrain's triangle structure, LOD transitions, and mesh efficiency are all visible for debugging.

## Crates & Dependencies

- **`wgpu = "28.0"`** — Render pipeline creation with `PolygonMode::Line`, depth bias configuration, `Features::NON_FILL_POLYGON_MODE` detection, and the `LineList` topology for the fallback path.
- **`egui = "0.31"`** — Not directly used for rendering, but the wireframe mode indicator ("Wireframe: ON") can be shown in the debug overlay panel.
- **`tracing = "0.1"`** — Logging wireframe toggle events, feature support detection results, and fallback activation.

## Unit Tests

- **`test_wireframe_shows_triangle_edges`** — Create a simple mesh with 2 triangles (a quad: 4 vertices, 6 indices). Run `generate_wireframe_lines` on it. Assert the output contains exactly 5 unique edges (4 outer edges + 1 diagonal), producing 10 line vertices and 10 line indices.

- **`test_wireframe_toggles_on_off`** — Create a `WireframeState` with `mode: Off`. Simulate F4 presses and assert the mode cycles: `Off -> White -> ChunkColored -> Off -> White`. Verify each transition is correct.

- **`test_wireframe_renders_over_solid_geometry`** — Assert that the wireframe pipeline's depth stencil state has `depth_compare: LessEqual` and a negative depth bias (`constant < 0`). This ensures wireframe lines render in front of the solid surface they overlay, preventing z-fighting.

- **`test_wireframe_works_with_chunk_meshes`** — Create a chunk-sized mesh (e.g., a 4x4x4 region of solid voxels meshed with greedy meshing). Run `generate_wireframe_lines` on it. Assert the result has more than 0 edges. Assert no duplicate edges exist (each unique edge appears exactly once).

- **`test_fallback_when_polygon_mode_unsupported`** — Create a `WireframeState` with `use_native_polygon_mode: false`. Assert the system selects the geometry-based fallback path. Create a triangle mesh, run the fallback, and verify it produces valid `LineList` geometry with the correct number of edges.

- **`test_chunk_color_deterministic`** — Call `chunk_wireframe_color` with the same `ChunkPos` twice. Assert the returned colors are identical. Call with two different positions and assert the colors differ (with high probability). Assert all color components are in the range `[0.3, 1.0]` to ensure visibility.

- **`test_degenerate_mesh_produces_no_crash`** — Run `generate_wireframe_lines` on an empty mesh (0 indices, 0 vertices). Assert the output is empty but no panic occurs. Run on a single triangle (3 indices) and assert exactly 3 edges are produced.
