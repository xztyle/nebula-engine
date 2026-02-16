//! Packed mesh data structures optimized for GPU bandwidth.
//!
//! [`ChunkVertex`] is a 12-byte packed vertex format that reduces GPU memory
//! usage by 3x compared to the unpacked [`super::MeshVertex`] format.

use crate::face_direction::FaceDirection;

/// A single vertex in a chunk mesh, packed to 12 bytes for efficient GPU upload.
///
/// Layout (12 bytes total):
///   - `[0..3]`  position `[u8; 3]` — XYZ in chunk-local coords (0..=32)
///   - `[3]`     normal `u8` — face direction index (0..=5)
///   - `[4]`     ao `u8` — ambient occlusion level (0..=3)
///   - `[5]`     padding
///   - `[6..8]`  material_id `u16` — voxel type / material index (little-endian)
///   - `[8..10]` uv `[u8; 2]` — texture coordinates (0..=32)
///   - `[10..12]` padding
#[repr(C, packed)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ChunkVertex {
    /// Position relative to chunk origin (each component 0..=32).
    pub position: [u8; 3],
    /// Face direction index (0..=5). See [`FaceDirection`].
    pub normal: u8,
    /// Ambient occlusion level (0..=3).
    pub ao: u8,
    /// Reserved, set to 0.
    pub _pad0: u8,
    /// Voxel type / material index.
    pub material_id: u16,
    /// Texture coordinates (each component 0..=32, tiles across merged quads).
    pub uv: [u8; 2],
    /// Reserved, aligns struct to 12 bytes.
    pub _pad1: u16,
}

static_assertions::assert_eq_size!(ChunkVertex, [u8; 12]);

impl ChunkVertex {
    /// Construct a packed vertex from meshing output.
    pub fn new(pos: [u8; 3], direction: FaceDirection, ao: u8, material: u16, uv: [u8; 2]) -> Self {
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
    ///
    /// Returns `None` if the stored value is out of range.
    pub fn face_direction(&self) -> Option<FaceDirection> {
        FaceDirection::from_u8(self.normal)
    }

    /// Decode the position as `[f32; 3]` for rendering or debugging.
    pub fn position_f32(&self) -> [f32; 3] {
        [
            self.position[0] as f32,
            self.position[1] as f32,
            self.position[2] as f32,
        ]
    }
}

/// A packed chunk mesh containing vertex and index buffers ready for GPU upload.
///
/// Uses [`ChunkVertex`] (12 bytes each) for bandwidth-efficient rendering.
pub struct PackedChunkMesh {
    /// Packed vertex buffer.
    pub vertices: Vec<ChunkVertex>,
    /// Index buffer (triangles, 3 indices per triangle).
    pub indices: Vec<u32>,
}

impl PackedChunkMesh {
    /// Creates an empty packed mesh.
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    /// Add a quad (4 vertices, 6 indices).
    ///
    /// If `flip` is true, the triangulation diagonal is flipped for
    /// AO-aware rendering.
    pub fn push_quad(&mut self, verts: [ChunkVertex; 4], flip: bool) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&verts);

        if flip {
            self.indices.extend_from_slice(&[
                base + 1,
                base + 2,
                base + 3,
                base,
                base + 1,
                base + 3,
            ]);
        } else {
            self.indices
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    /// Returns `true` if the mesh contains no vertices.
    pub fn is_empty(&self) -> bool {
        self.vertices.is_empty()
    }

    /// Returns the number of triangles in the mesh.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Returns the size of the vertex buffer in bytes.
    pub fn vertex_buffer_bytes(&self) -> usize {
        self.vertices.len() * std::mem::size_of::<ChunkVertex>()
    }

    /// Returns the size of the index buffer in bytes.
    pub fn index_buffer_bytes(&self) -> usize {
        self.indices.len() * std::mem::size_of::<u32>()
    }

    /// Returns the vertex data as a byte slice for GPU upload (zero-copy).
    pub fn vertex_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.vertices)
    }

    /// Returns the index data as a byte slice for GPU upload (zero-copy).
    pub fn index_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.indices)
    }
}

impl Default for PackedChunkMesh {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem;

    #[test]
    fn test_vertex_size_is_12_bytes() {
        assert_eq!(mem::size_of::<ChunkVertex>(), 12);
    }

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

    #[test]
    fn test_normal_encodes_six_directions_in_one_byte() {
        for dir in FaceDirection::ALL {
            let v = ChunkVertex::new([0, 0, 0], dir, 0, 0, [0, 0]);
            assert_eq!(v.normal, dir as u8);
            assert_eq!(v.face_direction(), Some(dir));
        }
        assert!(FaceDirection::from_u8(6).is_none());
    }

    #[test]
    fn test_mesh_indices_are_valid() {
        let mut mesh = PackedChunkMesh::new();
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
            assert!(
                idx < vertex_count,
                "Index {idx} >= vertex count {vertex_count}"
            );
        }
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices.len(), 6);
    }

    #[test]
    fn test_flipped_quad_indices_are_valid() {
        let mut mesh = PackedChunkMesh::new();
        mesh.push_quad(
            [
                ChunkVertex::new([0, 0, 0], FaceDirection::PosY, 0, 1, [0, 0]),
                ChunkVertex::new([1, 0, 0], FaceDirection::PosY, 1, 1, [1, 0]),
                ChunkVertex::new([1, 0, 1], FaceDirection::PosY, 0, 1, [1, 1]),
                ChunkVertex::new([0, 0, 1], FaceDirection::PosY, 1, 1, [0, 1]),
            ],
            true,
        );

        let vertex_count = mesh.vertices.len() as u32;
        for &idx in &mesh.indices {
            assert!(idx < vertex_count);
        }
        assert_eq!(mesh.indices, vec![1, 2, 3, 0, 1, 3]);
    }

    #[test]
    fn test_vertex_is_pod() {
        let v = ChunkVertex::new([1, 2, 3], FaceDirection::NegZ, 2, 42, [5, 6]);
        let bytes: &[u8] = bytemuck::bytes_of(&v);
        assert_eq!(bytes.len(), 12);
    }

    #[test]
    fn test_empty_mesh_stats() {
        let mesh = PackedChunkMesh::new();
        assert!(mesh.is_empty());
        assert_eq!(mesh.triangle_count(), 0);
        assert_eq!(mesh.vertex_buffer_bytes(), 0);
        assert_eq!(mesh.index_buffer_bytes(), 0);
    }

    #[test]
    fn test_vertex_bytes_zero_copy() {
        let mut mesh = PackedChunkMesh::new();
        mesh.push_quad(
            [
                ChunkVertex::new([0, 0, 0], FaceDirection::PosX, 0, 1, [0, 0]),
                ChunkVertex::new([1, 0, 0], FaceDirection::PosX, 0, 1, [1, 0]),
                ChunkVertex::new([1, 0, 1], FaceDirection::PosX, 0, 1, [1, 1]),
                ChunkVertex::new([0, 0, 1], FaceDirection::PosX, 0, 1, [0, 1]),
            ],
            false,
        );
        assert_eq!(mesh.vertex_bytes().len(), 4 * 12);
        assert_eq!(mesh.index_bytes().len(), 6 * 4);
    }
}
