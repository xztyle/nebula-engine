# Vertex & Index Buffer Management

## Problem

Every renderable object — voxel chunk meshes, debug wireframes, UI quads, particle billboards — requires vertex and index data uploaded to GPU buffers. Without a centralized abstraction, each system independently creates wgpu buffers with inconsistent usage flags, forgets to specify `COPY_DST` for dynamic updates, mismatches vertex layouts between buffer creation and pipeline expectations, or leaks GPU memory by losing buffer handles. The voxel meshing system alone will produce thousands of chunk meshes, each requiring a vertex buffer and an index buffer. A clean buffer management layer is essential for correctness and for future optimizations like buffer sub-allocation and pooling.

## Solution

### MeshBuffer

A `MeshBuffer` struct that bundles everything needed to issue a draw call for a single mesh:

```rust
pub struct MeshBuffer {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub index_count: u32,
    pub index_format: wgpu::IndexFormat,
}
```

The `index_format` field tracks whether indices are `u16` or `u32`. Chunk meshes with fewer than 65,536 vertices use `u16` to halve index buffer size — a meaningful optimization when thousands of chunks are loaded.

`MeshBuffer` provides a convenience method for binding during a render pass:

```rust
impl MeshBuffer {
    pub fn bind<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>) {
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), self.index_format);
    }

    pub fn draw(&self, render_pass: &mut wgpu::RenderPass) {
        render_pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}
```

### BufferAllocator

A `BufferAllocator` that creates GPU buffers from CPU-side data:

```rust
pub struct BufferAllocator<'a> {
    device: &'a wgpu::Device,
}

impl<'a> BufferAllocator<'a> {
    pub fn new(device: &'a wgpu::Device) -> Self {
        Self { device }
    }

    pub fn create_mesh(
        &self,
        label: &str,
        vertices: &[u8],
        indices: IndexData,
    ) -> MeshBuffer { ... }

    pub fn create_vertex_buffer(&self, label: &str, data: &[u8]) -> wgpu::Buffer { ... }

    pub fn create_index_buffer_u16(&self, label: &str, data: &[u16]) -> wgpu::Buffer { ... }

    pub fn create_index_buffer_u32(&self, label: &str, data: &[u32]) -> wgpu::Buffer { ... }
}
```

### IndexData Enum

```rust
pub enum IndexData<'a> {
    U16(&'a [u16]),
    U32(&'a [u32]),
}

impl IndexData<'_> {
    pub fn format(&self) -> wgpu::IndexFormat { ... }
    pub fn count(&self) -> u32 { ... }
    pub fn as_bytes(&self) -> &[u8] { ... }
}
```

### Buffer Creation Details

All buffers are created with `wgpu::util::DeviceExt::create_buffer_init`:

```rust
device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
    label: Some(label),
    contents: data,
    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
})
```

- Vertex buffers get `VERTEX | COPY_DST` usage flags. The `COPY_DST` flag enables future dynamic updates (e.g., streaming LOD transitions) without recreating the buffer.
- Index buffers get `INDEX | COPY_DST` usage flags.
- The vertex data is accepted as `&[u8]` (raw bytes) rather than a typed slice. This keeps the allocator generic — the vertex layout is defined by the pipeline, not the buffer. Callers use `bytemuck::cast_slice` to convert their typed vertex arrays to byte slices.

### Empty Meshes

When vertex or index data is empty (zero-length slices), the allocator still creates buffers but with size 0. The `index_count` is 0, so `draw_indexed(0..0, ...)` is a no-op. This avoids special-casing empty meshes throughout the rendering code.

### Vertex Layout Types

Define standard vertex types used throughout the engine:

```rust
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VertexPositionColor {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct VertexPositionNormalUv {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}
```

Each vertex type provides a `fn layout() -> wgpu::VertexBufferLayout<'static>` method that describes its memory layout for pipeline creation.

## Outcome

A `BufferAllocator` and `MeshBuffer` pair that any rendering subsystem can use to upload geometry to the GPU and bind it for drawing. The voxel meshing system creates `MeshBuffer` instances for each chunk. The debug overlay creates `MeshBuffer` instances for wireframe geometry. The abstraction handles `u16`/`u32` index format selection, buffer usage flags, and empty mesh edge cases.

## Demo Integration

**Demo crate:** `nebula-demo`

Triangle vertex data is uploaded to a GPU buffer. No visible output yet — the pipeline to draw it doesn't exist — but the buffer management is exercised and logged.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `wgpu` | `24.0` | GPU buffer creation and management |
| `bytemuck` | `1.21` | Safe casting between typed vertex arrays and `&[u8]` byte slices |

`bytemuck` is essential for zero-copy conversion of vertex structs to byte slices. The `Pod` and `Zeroable` derive macros ensure the vertex types are safe to transmute. Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mesh_buffer_creation_u16() {
        let device = create_test_device();
        let allocator = BufferAllocator::new(&device);

        let vertices: &[VertexPositionColor] = &[
            VertexPositionColor { position: [0.0, 0.0, 0.0], color: [1.0; 4] },
            VertexPositionColor { position: [1.0, 0.0, 0.0], color: [1.0; 4] },
            VertexPositionColor { position: [0.0, 1.0, 0.0], color: [1.0; 4] },
        ];
        let indices: &[u16] = &[0, 1, 2];

        let mesh = allocator.create_mesh(
            "test-triangle",
            bytemuck::cast_slice(vertices),
            IndexData::U16(indices),
        );

        assert_eq!(mesh.index_count, 3);
        assert_eq!(mesh.index_format, wgpu::IndexFormat::Uint16);
    }

    #[test]
    fn test_mesh_buffer_creation_u32() {
        let device = create_test_device();
        let allocator = BufferAllocator::new(&device);

        let vertices = vec![0u8; 128]; // raw vertex data
        let indices: &[u32] = &[0, 1, 2, 2, 3, 0];

        let mesh = allocator.create_mesh(
            "test-quad",
            &vertices,
            IndexData::U32(indices),
        );

        assert_eq!(mesh.index_count, 6);
        assert_eq!(mesh.index_format, wgpu::IndexFormat::Uint32);
    }

    #[test]
    fn test_index_count_matches_input() {
        let device = create_test_device();
        let allocator = BufferAllocator::new(&device);
        let indices: &[u16] = &[0, 1, 2, 3, 4, 5, 6, 7, 8]; // 3 triangles

        let mesh = allocator.create_mesh(
            "test",
            &[0u8; 64],
            IndexData::U16(indices),
        );

        assert_eq!(mesh.index_count, 9);
    }

    #[test]
    fn test_u16_vs_u32_format_selection() {
        let u16_data = IndexData::U16(&[0, 1, 2]);
        let u32_data = IndexData::U32(&[0, 1, 2]);

        assert_eq!(u16_data.format(), wgpu::IndexFormat::Uint16);
        assert_eq!(u32_data.format(), wgpu::IndexFormat::Uint32);
    }

    #[test]
    fn test_empty_mesh_creates_zero_index_count() {
        let device = create_test_device();
        let allocator = BufferAllocator::new(&device);

        let mesh = allocator.create_mesh(
            "empty",
            &[],
            IndexData::U16(&[]),
        );

        assert_eq!(mesh.index_count, 0);
    }

    #[test]
    fn test_index_data_as_bytes() {
        let indices_u16: &[u16] = &[0, 1, 2];
        let data = IndexData::U16(indices_u16);
        assert_eq!(data.as_bytes().len(), 6); // 3 × 2 bytes

        let indices_u32: &[u32] = &[0, 1, 2];
        let data = IndexData::U32(indices_u32);
        assert_eq!(data.as_bytes().len(), 12); // 3 × 4 bytes
    }

    #[test]
    fn test_vertex_position_color_layout() {
        let layout = VertexPositionColor::layout();
        // position (f32×3) + color (f32×4) = 28 bytes stride
        assert_eq!(layout.array_stride, 28);
        assert_eq!(layout.attributes.len(), 2);
    }

    #[test]
    fn test_vertex_position_normal_uv_layout() {
        let layout = VertexPositionNormalUv::layout();
        // position (f32×3) + normal (f32×3) + uv (f32×2) = 32 bytes stride
        assert_eq!(layout.array_stride, 32);
        assert_eq!(layout.attributes.len(), 3);
    }
}
```
