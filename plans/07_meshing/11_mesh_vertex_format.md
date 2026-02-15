# Mesh Vertex Format

## Problem

The wgpu rendering pipeline requires an explicit `VertexBufferLayout` that describes how vertex data is arranged in memory — which attributes exist, their byte offsets within each vertex, their data formats, and the stride between consecutive vertices. This layout must match both the CPU-side `ChunkVertex` struct (story 05) and the vertex shader's input declarations. If any of these three disagree — wrong offset, wrong format, wrong stride — the GPU reads garbage data, producing invisible meshes, corrupted geometry, or driver crashes.

Every chunk mesh render pipeline (opaque terrain, transparent terrain, water, debug wireframe) uses the same vertex format, so the layout must be defined once as a shared constant. Duplicating the layout definition across pipelines is a guaranteed source of drift bugs. Additionally, the layout must be validated against the actual `ChunkVertex` struct size at compile time to prevent silent breakage when the vertex format is modified.

## Solution

Define a canonical `wgpu::VertexBufferLayout` and attribute descriptors in the `nebula_meshing` crate as constants, with compile-time validation against the `ChunkVertex` struct.

### Vertex Attributes

The `ChunkVertex` from story 05 is 12 bytes with the following layout:

| Offset | Size | Field | wgpu Format | Shader Location |
|--------|------|-------|-------------|-----------------|
| 0 | 3 bytes | `position` `[u8; 3]` | `Uint8x4` (padded) | 0 |
| 3 | 1 byte | `normal` `u8` | (packed into position's 4th byte) | — |
| 4 | 1 byte | `ao` `u8` | `Uint8x4` (with padding) | 1 |
| 5 | 1 byte | `_pad0` | — | — |
| 6 | 2 bytes | `material_id` `u16` | `Uint16x2` (with padding) | 2 |
| 8 | 2 bytes | `uv` `[u8; 2]` | `Uint8x4` (padded) | 3 |
| 10 | 2 bytes | `_pad1` | — | — |

Because wgpu attributes must align to 4-byte boundaries and use predefined formats, the packed fields are read using the smallest compatible format and unpacked in the vertex shader. The approach groups fields into 4-byte-aligned attribute slots:

```rust
/// Attribute 0: position (3 bytes) + normal (1 byte) = 4 bytes at offset 0
///   Format: Uint8x4 → shader reads as uvec4, xyz = position, w = normal index
///
/// Attribute 1: ao (1 byte) + padding (1 byte) + material_id (2 bytes) = 4 bytes at offset 4
///   Format: Uint8x4 → shader reads as uvec4, x = ao, zw = material_id (reassembled)
///
/// Attribute 2: uv (2 bytes) + padding (2 bytes) = 4 bytes at offset 8
///   Format: Uint8x4 → shader reads as uvec4, xy = uv
```

### Constant Layout Definition

```rust
use std::mem;
use wgpu::{VertexAttribute, VertexBufferLayout, VertexFormat, VertexStepMode};

/// The vertex buffer layout for all chunk mesh render pipelines.
pub const CHUNK_VERTEX_LAYOUT: VertexBufferLayout<'static> = VertexBufferLayout {
    array_stride: mem::size_of::<ChunkVertex>() as u64, // 12
    step_mode: VertexStepMode::Vertex,
    attributes: &CHUNK_VERTEX_ATTRIBUTES,
};

/// Vertex attributes for the chunk mesh format.
pub const CHUNK_VERTEX_ATTRIBUTES: [VertexAttribute; 3] = [
    // Attribute 0: position (xyz) + normal (w), packed as 4x u8
    VertexAttribute {
        format: VertexFormat::Uint8x4,
        offset: 0,
        shader_location: 0,
    },
    // Attribute 1: ao + padding + material_id, packed as 4x u8
    VertexAttribute {
        format: VertexFormat::Uint8x4,
        offset: 4,
        shader_location: 1,
    },
    // Attribute 2: uv + padding, packed as 4x u8
    VertexAttribute {
        format: VertexFormat::Uint8x4,
        offset: 8,
        shader_location: 2,
    },
];
```

### Helper Function

```rust
/// Create a vertex buffer layout descriptor for chunk meshes.
/// This is equivalent to CHUNK_VERTEX_LAYOUT but returned as an owned value
/// for contexts where a `'static` lifetime is inconvenient.
pub fn chunk_vertex_buffer_layout() -> VertexBufferLayout<'static> {
    CHUNK_VERTEX_LAYOUT
}
```

### Compile-Time Validation

```rust
// Ensure the stride matches the actual struct size.
// If ChunkVertex changes size, this fails to compile.
const _: () = assert!(
    mem::size_of::<ChunkVertex>() == 12,
    "ChunkVertex size changed — update CHUNK_VERTEX_LAYOUT"
);

// Ensure attribute offsets don't exceed the stride.
const _: () = assert!(CHUNK_VERTEX_ATTRIBUTES[0].offset == 0);
const _: () = assert!(CHUNK_VERTEX_ATTRIBUTES[1].offset == 4);
const _: () = assert!(CHUNK_VERTEX_ATTRIBUTES[2].offset == 8);
const _: () = assert!(
    CHUNK_VERTEX_ATTRIBUTES[2].offset + 4 <= mem::size_of::<ChunkVertex>() as u64,
    "Last attribute exceeds vertex stride"
);
```

### Vertex Shader Unpacking

The corresponding WGSL vertex shader unpacks the attributes:

```wgsl
struct VertexInput {
    @location(0) pos_normal: vec4<u32>,  // xyz = position, w = normal index
    @location(1) ao_material: vec4<u32>, // x = ao, y = pad, zw = material_id
    @location(2) uv_pad: vec4<u32>,      // xy = uv, zw = pad
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    let position = vec3<f32>(f32(in.pos_normal.x), f32(in.pos_normal.y), f32(in.pos_normal.z));
    let normal_idx = in.pos_normal.w;
    let ao = f32(in.ao_material.x) * 0.25; // 0..3 → 0.0..0.75
    let brightness = 1.0 - ao;
    let material = in.ao_material.z | (in.ao_material.w << 8u);
    let uv = vec2<f32>(f32(in.uv_pad.x), f32(in.uv_pad.y));

    // Look up normal from constant table
    let normals = array<vec3<f32>, 6>(
        vec3(1.0, 0.0, 0.0),  vec3(-1.0, 0.0, 0.0),
        vec3(0.0, 1.0, 0.0),  vec3(0.0, -1.0, 0.0),
        vec3(0.0, 0.0, 1.0),  vec3(0.0, 0.0, -1.0),
    );
    let normal = normals[normal_idx];

    // ... transform and output
}
```

### Pipeline Integration

Every render pipeline that draws chunk meshes uses `CHUNK_VERTEX_LAYOUT` in its `vertex.buffers` array:

```rust
let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    vertex: wgpu::VertexState {
        module: &shader_module,
        entry_point: Some("vs_main"),
        buffers: &[CHUNK_VERTEX_LAYOUT],
        compilation_options: Default::default(),
    },
    // ...
});
```

## Outcome

The `nebula_meshing` crate exports `CHUNK_VERTEX_LAYOUT`, `CHUNK_VERTEX_ATTRIBUTES`, and `chunk_vertex_buffer_layout()`. All chunk mesh render pipelines reference this single layout definition. Compile-time assertions ensure the layout stays in sync with `ChunkVertex`. The WGSL shader unpacking is documented alongside the layout. Running `cargo test -p nebula_meshing` passes all vertex format tests.

## Demo Integration

**Demo crate:** `nebula-demo`

No visible demo change; the vertex format is finalized with position, normal, ambient occlusion, and material index packed into an efficient layout.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | `VertexBufferLayout`, `VertexAttribute`, `VertexFormat` types |
| `bytemuck` | `1.21` | `Pod` trait on `ChunkVertex` ensures safe byte reinterpretation |
| `static_assertions` | `1.1` | Compile-time struct size and offset validation |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;

    /// The layout stride must exactly match the ChunkVertex struct size.
    #[test]
    fn test_layout_stride_matches_vertex_struct_size() {
        assert_eq!(
            CHUNK_VERTEX_LAYOUT.array_stride,
            mem::size_of::<ChunkVertex>() as u64,
            "Layout stride ({}) != ChunkVertex size ({})",
            CHUNK_VERTEX_LAYOUT.array_stride,
            mem::size_of::<ChunkVertex>(),
        );
    }

    /// All attribute offsets must be within the vertex stride and correctly ordered.
    #[test]
    fn test_all_attributes_have_correct_offsets() {
        let stride = CHUNK_VERTEX_LAYOUT.array_stride;

        assert_eq!(CHUNK_VERTEX_ATTRIBUTES[0].offset, 0, "Attribute 0 offset");
        assert_eq!(CHUNK_VERTEX_ATTRIBUTES[1].offset, 4, "Attribute 1 offset");
        assert_eq!(CHUNK_VERTEX_ATTRIBUTES[2].offset, 8, "Attribute 2 offset");

        // All attributes must fit within the stride
        for (i, attr) in CHUNK_VERTEX_ATTRIBUTES.iter().enumerate() {
            let attr_size = vertex_format_size(attr.format);
            assert!(
                attr.offset + attr_size <= stride,
                "Attribute {i} at offset {} with size {attr_size} exceeds stride {stride}",
                attr.offset,
            );
        }
    }

    /// Attribute formats must match what the shader expects.
    #[test]
    fn test_attribute_formats_match_shader_expectations() {
        // Attribute 0: position + normal packed as Uint8x4
        assert_eq!(
            CHUNK_VERTEX_ATTRIBUTES[0].format,
            VertexFormat::Uint8x4,
            "Attribute 0 should be Uint8x4 (position + normal)"
        );

        // Attribute 1: ao + material packed as Uint8x4
        assert_eq!(
            CHUNK_VERTEX_ATTRIBUTES[1].format,
            VertexFormat::Uint8x4,
            "Attribute 1 should be Uint8x4 (ao + material)"
        );

        // Attribute 2: uv packed as Uint8x4
        assert_eq!(
            CHUNK_VERTEX_ATTRIBUTES[2].format,
            VertexFormat::Uint8x4,
            "Attribute 2 should be Uint8x4 (uv + padding)"
        );
    }

    /// Shader locations must be sequential starting from 0.
    #[test]
    fn test_shader_locations_are_sequential() {
        for (i, attr) in CHUNK_VERTEX_ATTRIBUTES.iter().enumerate() {
            assert_eq!(
                attr.shader_location, i as u32,
                "Attribute {i} should have shader_location {i}, got {}",
                attr.shader_location,
            );
        }
    }

    /// The layout should be valid for wgpu pipeline creation.
    /// This test creates a minimal render pipeline using the layout to verify
    /// wgpu accepts it without errors.
    #[test]
    fn test_layout_is_valid_for_wgpu_pipeline() {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            force_fallback_adapter: true,
            ..Default::default()
        }));

        let Some(adapter) = adapter else {
            // Skip test if no adapter available (headless CI without GPU)
            return;
        };

        let (device, _queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor::default(),
            None,
        ))
        .expect("failed to create device");

        let shader_source = r#"
            @vertex
            fn vs_main(
                @location(0) pos_normal: vec4<u32>,
                @location(1) ao_material: vec4<u32>,
                @location(2) uv_pad: vec4<u32>,
            ) -> @builtin(position) vec4<f32> {
                return vec4<f32>(f32(pos_normal.x), f32(pos_normal.y), f32(pos_normal.z), 1.0);
            }

            @fragment
            fn fs_main() -> @location(0) vec4<f32> {
                return vec4<f32>(1.0, 1.0, 1.0, 1.0);
            }
        "#;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("test_chunk_shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // This will panic if the vertex layout is invalid for the shader
        let _pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("test_chunk_pipeline"),
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[CHUNK_VERTEX_LAYOUT],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Bgra8UnormSrgb,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview: None,
            cache: None,
        });
        // If we reach here without panicking, the layout is valid.
    }

    /// Helper: returns the byte size of a VertexFormat.
    fn vertex_format_size(format: VertexFormat) -> u64 {
        match format {
            VertexFormat::Uint8x2 => 2,
            VertexFormat::Uint8x4 => 4,
            VertexFormat::Sint8x2 => 2,
            VertexFormat::Sint8x4 => 4,
            VertexFormat::Unorm8x2 => 2,
            VertexFormat::Unorm8x4 => 4,
            VertexFormat::Snorm8x2 => 2,
            VertexFormat::Snorm8x4 => 4,
            VertexFormat::Uint16x2 => 4,
            VertexFormat::Uint16x4 => 8,
            VertexFormat::Sint16x2 => 4,
            VertexFormat::Sint16x4 => 8,
            VertexFormat::Unorm16x2 => 4,
            VertexFormat::Unorm16x4 => 8,
            VertexFormat::Snorm16x2 => 4,
            VertexFormat::Snorm16x4 => 8,
            VertexFormat::Float16x2 => 4,
            VertexFormat::Float16x4 => 8,
            VertexFormat::Float32 => 4,
            VertexFormat::Float32x2 => 8,
            VertexFormat::Float32x3 => 12,
            VertexFormat::Float32x4 => 16,
            VertexFormat::Uint32 => 4,
            VertexFormat::Uint32x2 => 8,
            VertexFormat::Uint32x3 => 12,
            VertexFormat::Uint32x4 => 16,
            VertexFormat::Sint32 => 4,
            VertexFormat::Sint32x2 => 8,
            VertexFormat::Sint32x3 => 12,
            VertexFormat::Sint32x4 => 16,
            VertexFormat::Float64 => 8,
            VertexFormat::Float64x2 => 16,
            VertexFormat::Float64x3 => 24,
            VertexFormat::Float64x4 => 32,
            _ => panic!("Unknown vertex format"),
        }
    }
}
```
