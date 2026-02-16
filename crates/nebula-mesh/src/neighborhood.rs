//! Cross-chunk neighbor access for face visibility at chunk boundaries.

use nebula_voxel::{CHUNK_SIZE, ChunkData, VoxelTypeId};

/// Provides access to voxels in the six face-adjacent chunks.
///
/// When a voxel sits at the boundary of its chunk, its neighbor lives in
/// an adjacent chunk. `ChunkNeighborhood` stores optional references to
/// those neighbors. If a neighbor chunk is not loaded, lookups return air
/// (`VoxelTypeId(0)`) — the conservative default that keeps boundary faces
/// visible rather than creating invisible walls.
pub struct ChunkNeighborhood {
    /// Adjacent chunk data indexed by direction:
    /// 0 = +X, 1 = −X, 2 = +Y, 3 = −Y, 4 = +Z, 5 = −Z.
    neighbors: [Option<ChunkData>; 6],
}

impl ChunkNeighborhood {
    /// Creates a neighborhood where all six sides are unloaded (return air).
    pub fn all_air() -> Self {
        Self {
            neighbors: [None, None, None, None, None, None],
        }
    }

    /// Creates a neighborhood with a single −X neighbor; all others return air.
    pub fn with_neg_x(chunk: ChunkData) -> Self {
        Self {
            neighbors: [None, Some(chunk), None, None, None, None],
        }
    }

    /// Sets the neighbor chunk for the given direction index (0–5).
    pub fn set(&mut self, direction_index: usize, chunk: ChunkData) {
        if direction_index < 6 {
            self.neighbors[direction_index] = Some(chunk);
        }
    }

    /// Looks up a voxel at coordinates that have fallen outside `[0, CHUNK_SIZE)`.
    ///
    /// The coordinates `(nx, ny, nz)` are the raw neighbor position (possibly
    /// negative or `>= CHUNK_SIZE`). Returns `VoxelTypeId(0)` (air) if the
    /// relevant neighbor chunk is not loaded.
    pub fn get(&self, nx: i32, ny: i32, nz: i32) -> VoxelTypeId {
        let size = CHUNK_SIZE as i32;

        let (dir_index, lx, ly, lz) = if nx < 0 {
            (1, (nx + size) as usize, ny as usize, nz as usize) // −X
        } else if nx >= size {
            (0, (nx - size) as usize, ny as usize, nz as usize) // +X
        } else if ny < 0 {
            (3, nx as usize, (ny + size) as usize, nz as usize) // −Y
        } else if ny >= size {
            (2, nx as usize, (ny - size) as usize, nz as usize) // +Y
        } else if nz < 0 {
            (5, nx as usize, ny as usize, (nz + size) as usize) // −Z
        } else if nz >= size {
            (4, nx as usize, ny as usize, (nz - size) as usize) // +Z
        } else {
            // Coordinates are actually in-bounds — shouldn't be called, but handle gracefully.
            return VoxelTypeId(0);
        };

        match &self.neighbors[dir_index] {
            Some(chunk) => chunk.get(lx, ly, lz),
            None => VoxelTypeId(0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_air_returns_air() {
        let n = ChunkNeighborhood::all_air();
        assert_eq!(n.get(-1, 0, 0), VoxelTypeId(0));
        assert_eq!(n.get(32, 0, 0), VoxelTypeId(0));
        assert_eq!(n.get(0, -1, 0), VoxelTypeId(0));
    }

    #[test]
    fn test_with_neg_x_returns_neighbor_voxel() {
        let mut chunk = ChunkData::new_air();
        chunk.set(31, 10, 10, VoxelTypeId(5));
        let n = ChunkNeighborhood::with_neg_x(chunk);
        // nx = -1 maps to local x = 31 in the −X neighbor
        assert_eq!(n.get(-1, 10, 10), VoxelTypeId(5));
        assert_eq!(n.get(-1, 0, 0), VoxelTypeId(0));
    }

    #[test]
    fn test_set_pos_y_neighbor() {
        let mut n = ChunkNeighborhood::all_air();
        let mut chunk = ChunkData::new_air();
        chunk.set(5, 0, 5, VoxelTypeId(9));
        n.set(2, chunk); // +Y direction index
        // ny = 32 maps to local y = 0 in the +Y neighbor
        assert_eq!(n.get(5, 32, 5), VoxelTypeId(9));
    }
}
