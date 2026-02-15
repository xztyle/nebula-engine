# Mesh Data Structures

## Problem

The meshing pipeline (face detection, greedy meshing, AO) produces geometry data that must be efficiently stored in memory, uploaded to the GPU, and rendered. A naive approach — using full `f32` positions, `f32` normals, and `f32` UVs per vertex — wastes bandwidth. A chunk mesh vertex carrying `(f32 x3 position, f32 x3 normal, f32 x2 UV, u32 material)` consumes 36 bytes per vertex. With thousands of visible chunks, each containing hundreds to thousands of vertices, this adds up to hundreds of megabytes of GPU memory and excessive bandwidth during vertex fetch.

Voxel mesh vertices have constrained ranges that enable aggressive packing: positions are integers in `[0, 32]` (3 bytes), normals are one of 6 axis-aligned directions (1 byte), AO is 0-3 (1 byte), UVs are integers tiling across merged quads (2 bytes), and material is a registry index (2 bytes). A packed vertex should fit in 12 bytes or less — a 3x reduction.

## Solution

Define `ChunkVertex` and `ChunkMesh` in the `nebula_meshing` crate, with a tightly packed vertex format optimized for GPU bandwidth.

### ChunkVertex — Packed Layout

```rust
/// A single vertex in a chunk mesh. Packed to 12 bytes.
///
/// Layout (12 bytes total):
///   [0]  position_x: u8   — X position relative to chunk origin (0..=32)
///   [1]  position_y: u8   — Y position relative to chunk origin (0..=32)
///   [2]  position_z: u8   — Z position relative to chunk origin (0..=32)
///   [3]  normal:     u8   — Face direction index (0..=5), encodes one of 6 axis-aligned normals
///   [4]  ao:         u8   — Ambient occlusion level (0..=3)
///   [5]  _padding:   u8   — Reserved, set to 0
///   [6..8]  material_id: u16 — Voxel type / material index (little-endian)
///   [8]  uv_u:       u8   — U texture coordinate (0..=32, tiles across merged quad)
///   [9]  uv_v:       u8   — V texture coordinate (0..=32, tiles across merged quad)
///   [10..12] _padding2: u16 — Reserved, aligns struct to 12 bytes
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ChunkVertex {
    pub position: [u8; 3],
    pub normal: u8,
    pub ao: u8,
    pub _pad0: u8,
    pub material_id: u16,
    pub uv: [u8; 2],
    pub _pad1: u16,
}

static_assertions::assert_eq_size!(ChunkVertex, [u8; 12]);
```

### Vertex Encoding/Decoding

```rust
impl ChunkVertex {
    /// Construct a vertex from meshing output.
    pub fn new(
        pos: [u8; 3],
        direction: FaceDirection,
        ao: u8,
        material: u16,
        uv: [u8; 2],
    ) -> Self {
        debug_assert!(pos[0] <= 32 && pos[1] <= 32 && pos[2] <= 32);
        debug_assert!(ao <= 3);
        Self {
            position: pos,
            normal: direction as u8,
            ao,
            _pad0: 0,
            material_id: material,
            uv,
            _pad1: 0,
        }
    }

    /// Decode the face direction from the packed normal byte.
    pub fn face_direction(&self) -> FaceDirection {
        FaceDirection::from_u8(self.normal)
            .expect("invalid normal index in vertex data")
    }

    /// Decode the position as [f32; 3] for rendering or debugging.
    pub fn position_f32(&self) -> [f32; 3] {
        [
            self.position[0] as f32,
            self.position[1] as f32,
            self.position[2] as f32,
        ]
    }
}
```

### Normal Encoding

Only 6 normals are possible for voxel faces. They are encoded as a single `u8` index:

| Index | Direction | Normal Vector |
|-------|-----------|---------------|
| 0 | +X | (1, 0, 0) |
| 1 | -X | (-1, 0, 0) |
| 2 | +Y | (0, 1, 0) |
| 3 | -Y | (0, -1, 0) |
| 4 | +Z | (0, 0, 1) |
| 5 | -Z | (0, 0, -1) |

The vertex shader reconstructs the full normal vector from this index using a constant lookup table.

### ChunkMesh

```rust
/// The complete mesh output from the meshing pipeline for one chunk.
pub struct ChunkMesh {
    pub vertices: Vec<ChunkVertex>,
    pub indices: Vec<u32>,
}

impl ChunkMesh {
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Add a quad (4 vertices, 6 indices). Optionally flip the diagonal
    /// for AO-aware triangulation.
    pub fn push_quad(&mut self, verts: [ChunkVertex; 4], flip: bool) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&verts);

        if flip {
            // Triangles: (1,2,3) and (0,1,3)
            self.indices.extend_from_slice(&[
                base + 1, base + 2, base + 3,
                base + 0, base + 1, base + 3,
            ]);
        } else {
            // Triangles: (0,1,2) and (0,2,3)
            self.indices.extend_from_slice(&[
                base + 0, base + 1, base + 2,
                base + 0, base + 2, base + 3,
            ]);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn vertex_buffer_bytes(&self) -> usize {
        self.vertices.len() * std::mem::size_of::<ChunkVertex>()
    }

    pub fn index_buffer_bytes(&self) -> usize {
        self.indices.len() * std::mem::size_of::<u32>()
    }
}
```

### Design Rationale

- **12 bytes per vertex** vs 36 bytes naive: 3x bandwidth reduction. For a chunk with 2,000 vertices, the buffer is 24 KB instead of 72 KB.
- **`repr(C, packed)`** ensures the in-memory layout matches the GPU vertex buffer layout exactly. No padding surprises.
- **`bytemuck::Pod`** allows zero-copy casting of `&[ChunkVertex]` to `&[u8]` for GPU upload.
- **`u32` indices** rather than `u16` to support chunks with more than 65,536 vertices (rare but possible with complex geometry).
- **Position range 0..=32**: 33 values needed (a face at the far edge of a 32-wide chunk has position 32). This fits in `u8` (0-255).

## Outcome

The `nebula_meshing` crate exports `ChunkVertex` (12 bytes, packed, `Pod`-compatible) and `ChunkMesh` (vertex + index buffers). All meshing stages produce `ChunkMesh` instances. The GPU upload stage (story 06) casts the vertex slice directly to bytes with zero copy. Running `cargo test -p nebula_meshing` passes all data structure tests.

## Demo Integration

**Demo crate:** `nebula-demo`

No visible demo change; the mesh data structures are formalized into the canonical `MeshData` format used by all subsequent meshing and rendering.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `bytemuck` | `1.21` | Safe transmute for `Pod` and `Zeroable` derives — zero-copy GPU upload |
| `static_assertions` | `1.1` | Compile-time check that `ChunkVertex` is exactly 12 bytes |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;

    /// ChunkVertex must be exactly 12 bytes. This is critical for GPU buffer layout.
    #[test]
    fn test_vertex_size_is_12_bytes() {
        assert_eq!(mem::size_of::<ChunkVertex>(), 12);
    }

    /// Position packing round-trips correctly for all valid values (0..=32).
    #[test]
    fn test_position_packs_and_unpacks_correctly() {
        for x in 0..=32u8 {
            for &y in &[0u8, 16, 32] {
                for &z in &[0u8, 16, 32] {
                    let v = ChunkVertex::new([x, y, z], FaceDirection::PosX, 0, 0, [0, 0]);
                    let pos = v.position_f32();
                    assert_eq!(pos[0], x as f32);
                    assert_eq!(pos[1], y as f32);
                    assert_eq!(pos[2], z as f32);
                }
            }
        }
    }

    /// Normal byte encodes all 6 axis-aligned directions and decodes back.
    #[test]
    fn test_normal_encodes_six_directions_in_one_byte() {
        for dir in FaceDirection::ALL {
            let v = ChunkVertex::new([0, 0, 0], dir, 0, 0, [0, 0]);
            assert_eq!(v.normal, dir as u8);
            assert_eq!(v.face_direction(), dir);
        }
        // Only values 0..=5 are valid
        assert!(FaceDirection::from_u8(6).is_none());
    }

    /// All mesh indices must be less than the vertex count.
    #[test]
    fn test_mesh_indices_are_valid() {
        let mut mesh = ChunkMesh::new();
        let v = ChunkVertex::new([0, 0, 0], FaceDirection::PosY, 0, 1, [0, 0]);
        mesh.push_quad(
            [
                ChunkVertex::new([0, 0, 0], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([1, 0, 0], FaceDirection::PosY, 0, 1, [1, 0]),
                ChunkVertex::new([1, 0, 1], FaceDirection::PosY, 0, 1, [1, 1]),
                ChunkVertex::new([0, 0, 1], FaceDirection::PosY, 0, 1, [0, 1]),
            ],
            false,
        );

        let vertex_count = mesh.vertices.len() as u32;
        for &idx in &mesh.indices {
            assert!(idx < vertex_count, "Index {idx} >= vertex count {vertex_count}");
        }
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices.len(), 6);
    }

    /// Flipped quad produces valid indices with different winding.
    #[test]
    fn test_flipped_quad_indices_are_valid() {
        let mut mesh = ChunkMesh::new();
        mesh.push_quad(
            [
                ChunkVertex::new([0, 0, 0], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([1, 0, 0], FaceDirection::PosY, 1, 1, [1, 0]),
                ChunkVertex::new([1, 0, 1], FaceDirection::PosY, 0, 1, [1, 1]),
                ChunkVertex::new([0, 0, 1], FaceDirection::PosY, 1, 1, [0, 1]),
            ],
            true, // flip for AO
        );

        let vertex_count = mesh.vertices.len() as u32;
        for &idx in &mesh.indices {
            assert!(idx < vertex_count);
        }
        // Flipped indices should be different from non-flipped
        assert_eq!(mesh.indices, vec![1, 2, 3, 0, 1, 3]);
    }

    /// ChunkVertex is Pod-compatible (can be safely cast to bytes).
    #[test]
    fn test_vertex_is_pod() {
        let v = ChunkVertex::new([1, 2, 3], FaceDirection::NegZ, 2, 42, [5, 6]);
        let bytes: &[u8] = bytemuck::bytes_of(&v);
        assert_eq!(bytes.len(), 12);
    }

    /// Empty mesh reports correct statistics.
    #[test]
    fn test_empty_mesh_stats() {
        let mesh = ChunkMesh::new();
        assert!(mesh.is_empty());
        assert_eq!(mesh.triangle_count(), 0);
        assert_eq!(mesh.vertex_buffer_bytes(), 0);
        assert_eq!(mesh.index_buffer_bytes(), 0);
    }
}
```
