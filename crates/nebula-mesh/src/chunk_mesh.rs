//! Chunk mesh data structure holding vertices and indices produced by meshing algorithms.

use crate::face_direction::FaceDirection;
use nebula_voxel::VoxelTypeId;

/// A single vertex in a chunk mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshVertex {
    /// Position in chunk-local coordinates.
    pub position: [f32; 3],
    /// Face normal.
    pub normal: [f32; 3],
    /// Texture coordinates (tiled across merged quads).
    pub uv: [f32; 2],
    /// Voxel type for material/texture lookup.
    pub voxel_type: VoxelTypeId,
}

/// Metadata for a single merged quad, used for analysis and debugging.
#[derive(Clone, Copy, Debug)]
pub struct QuadInfo {
    /// Which face direction this quad belongs to.
    pub direction: FaceDirection,
}

/// The mesh output of a chunk meshing pass.
///
/// Contains interleaved vertex data and triangle indices ready for GPU upload.
pub struct ChunkMesh {
    /// Vertex buffer.
    pub vertices: Vec<MeshVertex>,
    /// Index buffer (triangles, 3 indices per triangle).
    pub indices: Vec<u32>,
    /// One entry per emitted quad, for debugging / statistics.
    pub quads: Vec<QuadInfo>,
}

impl ChunkMesh {
    /// Creates an empty mesh.
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            quads: Vec::new(),
        }
    }

    /// Pushes a single merged quad into the mesh.
    ///
    /// `layer`, `u`, `v` are in chunk-local voxel coordinates.
    /// `w` and `h` are the quad dimensions along the u and v axes.
    #[allow(clippy::too_many_arguments)]
    pub fn push_quad(
        &mut self,
        direction: FaceDirection,
        layer: usize,
        u: usize,
        v: usize,
        w: usize,
        h: usize,
        voxel_type: VoxelTypeId,
    ) {
        let (layer_axis, u_axis, v_axis) = direction.sweep_axes();
        let normal = direction.normal();

        // Determine layer offset: positive faces sit on the far side of the voxel.
        let layer_pos = match direction {
            FaceDirection::PosX | FaceDirection::PosY | FaceDirection::PosZ => layer as f32 + 1.0,
            FaceDirection::NegX | FaceDirection::NegY | FaceDirection::NegZ => layer as f32,
        };

        // Build 4 corners: (u, v), (u+w, v), (u+w, v+h), (u, v+h)
        let corners = [
            (u as f32, v as f32),
            (u as f32 + w as f32, v as f32),
            (u as f32 + w as f32, v as f32 + h as f32),
            (u as f32, v as f32 + h as f32),
        ];
        let uvs = [
            [0.0, 0.0],
            [w as f32, 0.0],
            [w as f32, h as f32],
            [0.0, h as f32],
        ];

        let base = self.vertices.len() as u32;

        for (i, &(cu, cv)) in corners.iter().enumerate() {
            let mut pos = [0.0_f32; 3];
            pos[layer_axis] = layer_pos;
            pos[u_axis] = cu;
            pos[v_axis] = cv;

            self.vertices.push(MeshVertex {
                position: pos,
                normal,
                uv: uvs[i],
                voxel_type,
            });
        }

        // Two triangles with correct winding for front-face rendering.
        // For positive-direction faces, CCW winding when viewed from outside.
        // For negative-direction faces, reverse winding.
        match direction {
            FaceDirection::PosX | FaceDirection::PosY | FaceDirection::PosZ => {
                self.indices.extend_from_slice(&[
                    base,
                    base + 1,
                    base + 2,
                    base,
                    base + 2,
                    base + 3,
                ]);
            }
            FaceDirection::NegX | FaceDirection::NegY | FaceDirection::NegZ => {
                self.indices.extend_from_slice(&[
                    base,
                    base + 2,
                    base + 1,
                    base,
                    base + 3,
                    base + 2,
                ]);
            }
        }

        self.quads.push(QuadInfo { direction });
    }

    /// Counts the number of quads emitted for a specific face direction.
    pub fn count_quads_for_direction(&self, direction: FaceDirection) -> usize {
        self.quads
            .iter()
            .filter(|q| q.direction == direction)
            .count()
    }

    /// Returns the total number of quads in the mesh.
    pub fn quad_count(&self) -> usize {
        self.quads.len()
    }
}

impl Default for ChunkMesh {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_mesh() {
        let mesh = ChunkMesh::new();
        assert_eq!(mesh.vertices.len(), 0);
        assert_eq!(mesh.indices.len(), 0);
        assert_eq!(mesh.quad_count(), 0);
    }

    #[test]
    fn test_push_single_quad() {
        let mut mesh = ChunkMesh::new();
        mesh.push_quad(FaceDirection::PosY, 0, 0, 0, 1, 1, VoxelTypeId(1));
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.indices.len(), 6);
        assert_eq!(mesh.quad_count(), 1);
    }

    #[test]
    fn test_count_quads_by_direction() {
        let mut mesh = ChunkMesh::new();
        mesh.push_quad(FaceDirection::PosY, 0, 0, 0, 1, 1, VoxelTypeId(1));
        mesh.push_quad(FaceDirection::PosY, 0, 1, 0, 1, 1, VoxelTypeId(1));
        mesh.push_quad(FaceDirection::NegY, 0, 0, 0, 1, 1, VoxelTypeId(1));
        assert_eq!(mesh.count_quads_for_direction(FaceDirection::PosY), 2);
        assert_eq!(mesh.count_quads_for_direction(FaceDirection::NegY), 1);
        assert_eq!(mesh.count_quads_for_direction(FaceDirection::PosX), 0);
    }
}
