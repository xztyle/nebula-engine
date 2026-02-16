//! Mesh cache invalidation: tracks chunk data versions and determines which
//! chunks need remeshing after voxel edits.

use nebula_voxel::ChunkAddress;

use crate::FaceDirection;

/// Metadata for a chunk's mesh cache state.
#[derive(Clone, Debug)]
pub struct ChunkMeshState {
    /// Version of the voxel data when the current mesh was generated.
    /// If this doesn't match the chunk's current data version, the mesh is stale.
    pub meshed_version: u64,
    /// Whether a remesh task is already in-flight for this chunk.
    pub remesh_pending: bool,
}

impl ChunkMeshState {
    /// Creates a new mesh state with no mesh generated yet.
    pub fn new() -> Self {
        Self {
            meshed_version: 0,
            remesh_pending: false,
        }
    }

    /// Returns `true` if the mesh is stale (version mismatch with current data).
    pub fn is_stale(&self, current_data_version: u64) -> bool {
        self.meshed_version != current_data_version
    }

    /// Returns `true` if this chunk needs a remesh task submitted.
    pub fn needs_remesh(&self, current_data_version: u64) -> bool {
        self.is_stale(current_data_version) && !self.remesh_pending
    }
}

impl Default for ChunkMeshState {
    fn default() -> Self {
        Self::new()
    }
}

/// Determines which chunks need remeshing after a voxel edit.
pub struct MeshInvalidator;

impl MeshInvalidator {
    /// Returns the set of chunk addresses that should be invalidated after
    /// a voxel is edited at `local_pos` within `edited_chunk`.
    ///
    /// The edited chunk itself is always included. Adjacent chunks are included
    /// when the edit is on a chunk boundary, because their meshes depend on
    /// the neighbor voxel for face culling and ambient occlusion.
    pub fn invalidate(
        edited_chunk: ChunkAddress,
        local_pos: (usize, usize, usize),
        chunk_size: usize,
    ) -> Vec<ChunkAddress> {
        let mut dirty = vec![edited_chunk];
        let (x, y, z) = local_pos;

        if x == 0 {
            dirty.push(neighbor_addr(edited_chunk, FaceDirection::NegX));
        }
        if x == chunk_size - 1 {
            dirty.push(neighbor_addr(edited_chunk, FaceDirection::PosX));
        }
        if y == 0 {
            dirty.push(neighbor_addr(edited_chunk, FaceDirection::NegY));
        }
        if y == chunk_size - 1 {
            dirty.push(neighbor_addr(edited_chunk, FaceDirection::PosY));
        }
        if z == 0 {
            dirty.push(neighbor_addr(edited_chunk, FaceDirection::NegZ));
        }
        if z == chunk_size - 1 {
            dirty.push(neighbor_addr(edited_chunk, FaceDirection::PosZ));
        }

        dirty
    }
}

/// Returns the chunk address of the neighbor in the given face direction.
fn neighbor_addr(addr: ChunkAddress, dir: FaceDirection) -> ChunkAddress {
    let (dx, dy, dz): (i64, i64, i64) = match dir {
        FaceDirection::PosX => (1, 0, 0),
        FaceDirection::NegX => (-1, 0, 0),
        FaceDirection::PosY => (0, 1, 0),
        FaceDirection::NegY => (0, -1, 0),
        FaceDirection::PosZ => (0, 0, 1),
        FaceDirection::NegZ => (0, 0, -1),
    };
    addr.offset(dx, dy, dz)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn origin() -> ChunkAddress {
        ChunkAddress::new(0, 0, 0, 0)
    }

    /// Changing a voxel should invalidate the chunk's own mesh.
    #[test]
    fn test_voxel_change_invalidates_own_mesh() {
        let pos = origin();
        let dirty = MeshInvalidator::invalidate(pos, (16, 16, 16), 32);
        assert!(dirty.contains(&pos), "Edited chunk must be in dirty set");
    }

    /// Changing a voxel at a chunk boundary should invalidate the neighbor.
    #[test]
    fn test_boundary_voxel_change_invalidates_neighbor_mesh() {
        let pos = origin();

        // Edit at x=0: should invalidate the -X neighbor
        let dirty = MeshInvalidator::invalidate(pos, (0, 16, 16), 32);
        let neg_x = pos.offset(-1, 0, 0);
        assert!(
            dirty.contains(&neg_x),
            "-X neighbor should be invalidated for edit at x=0"
        );

        // Edit at x=31: should invalidate the +X neighbor
        let dirty = MeshInvalidator::invalidate(pos, (31, 16, 16), 32);
        let pos_x = pos.offset(1, 0, 0);
        assert!(
            dirty.contains(&pos_x),
            "+X neighbor should be invalidated for edit at x=31"
        );

        // Edit at y=0: should invalidate the -Y neighbor
        let dirty = MeshInvalidator::invalidate(pos, (16, 0, 16), 32);
        let neg_y = pos.offset(0, -1, 0);
        assert!(
            dirty.contains(&neg_y),
            "-Y neighbor should be invalidated for edit at y=0"
        );
    }

    /// An interior edit should NOT invalidate any neighbor.
    #[test]
    fn test_interior_change_does_not_invalidate_neighbors() {
        let pos = origin();
        let dirty = MeshInvalidator::invalidate(pos, (16, 16, 16), 32);
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0], pos);
    }

    /// A version mismatch should be detected as stale.
    #[test]
    fn test_version_mismatch_triggers_remesh() {
        let mut state = ChunkMeshState {
            meshed_version: 1,
            remesh_pending: false,
        };

        assert!(state.is_stale(2));
        assert!(state.needs_remesh(2));

        state.meshed_version = 2;
        assert!(!state.is_stale(2));
        assert!(!state.needs_remesh(2));
    }

    /// A clean chunk should NOT be marked for remesh.
    #[test]
    fn test_clean_chunk_not_remeshed() {
        let state = ChunkMeshState {
            meshed_version: 5,
            remesh_pending: false,
        };
        assert!(!state.is_stale(5));
        assert!(!state.needs_remesh(5));
    }

    /// A corner edit (0,0,0) should invalidate 3 face neighbors + self.
    #[test]
    fn test_corner_edit_invalidates_three_face_neighbors() {
        let pos = origin();
        let dirty = MeshInvalidator::invalidate(pos, (0, 0, 0), 32);

        assert!(
            dirty.len() >= 4,
            "Corner edit should invalidate at least 4 chunks, got {}",
            dirty.len()
        );
        assert!(dirty.contains(&pos));
        assert!(dirty.contains(&pos.offset(-1, 0, 0)));
        assert!(dirty.contains(&pos.offset(0, -1, 0)));
        assert!(dirty.contains(&pos.offset(0, 0, -1)));
    }

    /// A chunk with remesh_pending should not need another remesh.
    #[test]
    fn test_pending_remesh_suppresses_resubmit() {
        let state = ChunkMeshState {
            meshed_version: 1,
            remesh_pending: true,
        };
        assert!(state.is_stale(2));
        assert!(!state.needs_remesh(2));
    }
}
