//! Canonical `wgpu::VertexBufferLayout` for chunk mesh rendering.
//!
//! All chunk mesh render pipelines (opaque, transparent, water, debug wireframe)
//! reference [`CHUNK_VERTEX_LAYOUT`] to avoid layout drift bugs.
//!
//! ## Attribute Packing
//!
//! | Location | Offset | Format   | Fields                          |
//! |----------|--------|----------|---------------------------------|
//! | 0        | 0      | Uint8x4  | position xyz + normal index     |
//! | 1        | 4      | Uint8x4  | ao + pad + material_id (2 bytes)|
//! | 2        | 8      | Uint8x4  | uv xy + pad                    |

use std::mem;

use wgpu::{VertexAttribute, VertexBufferLayout, VertexFormat, VertexStepMode};

use crate::packed::ChunkVertex;

/// Vertex attributes for the chunk mesh format.
///
/// Three 4-byte `Uint8x4` attributes covering all 12 bytes of [`ChunkVertex`].
pub const CHUNK_VERTEX_ATTRIBUTES: [VertexAttribute; 3] = [
    // Attribute 0: position (xyz) + normal (w), packed as 4× u8
    VertexAttribute {
        format: VertexFormat::Uint8x4,
        offset: 0,
        shader_location: 0,
    },
    // Attribute 1: ao + padding + material_id, packed as 4× u8
    VertexAttribute {
        format: VertexFormat::Uint8x4,
        offset: 4,
        shader_location: 1,
    },
    // Attribute 2: uv + padding, packed as 4× u8
    VertexAttribute {
        format: VertexFormat::Uint8x4,
        offset: 8,
        shader_location: 2,
    },
];

/// The vertex buffer layout for all chunk mesh render pipelines.
///
/// Uses [`CHUNK_VERTEX_ATTRIBUTES`] with a 12-byte stride matching [`ChunkVertex`].
pub const CHUNK_VERTEX_LAYOUT: VertexBufferLayout<'static> = VertexBufferLayout {
    array_stride: mem::size_of::<ChunkVertex>() as u64,
    step_mode: VertexStepMode::Vertex,
    attributes: &CHUNK_VERTEX_ATTRIBUTES,
};

/// Return the chunk vertex buffer layout as an owned value.
///
/// Equivalent to [`CHUNK_VERTEX_LAYOUT`] but convenient when a `'static`
/// lifetime is awkward to thread through.
pub fn chunk_vertex_buffer_layout() -> VertexBufferLayout<'static> {
    CHUNK_VERTEX_LAYOUT
}

// ---------------------------------------------------------------------------
// Compile-time validation
// ---------------------------------------------------------------------------

/// Stride must match `ChunkVertex` size.
const _: () = assert!(
    mem::size_of::<ChunkVertex>() == 12,
    "ChunkVertex size changed — update CHUNK_VERTEX_LAYOUT"
);

/// Attribute offsets must be correct.
const _: () = assert!(CHUNK_VERTEX_ATTRIBUTES[0].offset == 0);
const _: () = assert!(CHUNK_VERTEX_ATTRIBUTES[1].offset == 4);
const _: () = assert!(CHUNK_VERTEX_ATTRIBUTES[2].offset == 8);

/// Last attribute must fit within the stride.
const _: () = assert!(
    CHUNK_VERTEX_ATTRIBUTES[2].offset + 4 <= mem::size_of::<ChunkVertex>() as u64,
    "Last attribute exceeds vertex stride"
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;
    use wgpu::VertexFormat;

    #[test]
    fn test_layout_stride_matches_vertex_struct_size() {
        assert_eq!(
            CHUNK_VERTEX_LAYOUT.array_stride,
            mem::size_of::<ChunkVertex>() as u64,
        );
    }

    #[test]
    fn test_all_attributes_have_correct_offsets() {
        let stride = CHUNK_VERTEX_LAYOUT.array_stride;
        assert_eq!(CHUNK_VERTEX_ATTRIBUTES[0].offset, 0);
        assert_eq!(CHUNK_VERTEX_ATTRIBUTES[1].offset, 4);
        assert_eq!(CHUNK_VERTEX_ATTRIBUTES[2].offset, 8);

        for (i, attr) in CHUNK_VERTEX_ATTRIBUTES.iter().enumerate() {
            let size = vertex_format_size(attr.format);
            assert!(
                attr.offset + size <= stride,
                "Attribute {i} at offset {} with size {size} exceeds stride {stride}",
                attr.offset,
            );
        }
    }

    #[test]
    fn test_attribute_formats_match_shader_expectations() {
        assert_eq!(CHUNK_VERTEX_ATTRIBUTES[0].format, VertexFormat::Uint8x4);
        assert_eq!(CHUNK_VERTEX_ATTRIBUTES[1].format, VertexFormat::Uint8x4);
        assert_eq!(CHUNK_VERTEX_ATTRIBUTES[2].format, VertexFormat::Uint8x4);
    }

    #[test]
    fn test_shader_locations_are_sequential() {
        for (i, attr) in CHUNK_VERTEX_ATTRIBUTES.iter().enumerate() {
            assert_eq!(attr.shader_location, i as u32);
        }
    }

    #[test]
    fn test_helper_returns_same_layout() {
        let layout = chunk_vertex_buffer_layout();
        assert_eq!(layout.array_stride, CHUNK_VERTEX_LAYOUT.array_stride);
        assert_eq!(
            layout.attributes.len(),
            CHUNK_VERTEX_LAYOUT.attributes.len()
        );
    }

    #[test]
    fn test_layout_is_valid_for_wgpu_pipeline() {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            force_fallback_adapter: true,
            ..Default::default()
        }));

        let Ok(adapter) = adapter else {
            // No adapter available (headless CI without GPU) — skip.
            return;
        };

        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
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

        let shader: wgpu::ShaderModule =
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("test_chunk_shader"),
                source: wgpu::ShaderSource::Wgsl(shader_source.into()),
            });

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
            multiview_mask: None,
            cache: None,
        });
    }

    /// Helper: byte size of a `VertexFormat`.
    fn vertex_format_size(format: VertexFormat) -> u64 {
        match format {
            VertexFormat::Uint8x2
            | VertexFormat::Sint8x2
            | VertexFormat::Unorm8x2
            | VertexFormat::Snorm8x2 => 2,
            VertexFormat::Uint8x4
            | VertexFormat::Sint8x4
            | VertexFormat::Unorm8x4
            | VertexFormat::Snorm8x4 => 4,
            VertexFormat::Uint16x2
            | VertexFormat::Sint16x2
            | VertexFormat::Unorm16x2
            | VertexFormat::Snorm16x2
            | VertexFormat::Float16x2 => 4,
            VertexFormat::Uint16x4
            | VertexFormat::Sint16x4
            | VertexFormat::Unorm16x4
            | VertexFormat::Snorm16x4
            | VertexFormat::Float16x4 => 8,
            VertexFormat::Float32 | VertexFormat::Uint32 | VertexFormat::Sint32 => 4,
            VertexFormat::Float32x2 | VertexFormat::Uint32x2 | VertexFormat::Sint32x2 => 8,
            VertexFormat::Float32x3 | VertexFormat::Uint32x3 | VertexFormat::Sint32x3 => 12,
            VertexFormat::Float32x4 | VertexFormat::Uint32x4 | VertexFormat::Sint32x4 => 16,
            VertexFormat::Float64 => 8,
            VertexFormat::Float64x2 => 16,
            VertexFormat::Float64x3 => 24,
            VertexFormat::Float64x4 => 32,
            _ => panic!("Unknown vertex format"),
        }
    }
}
