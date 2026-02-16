//! High-level chunk wrapper with bounds-checked voxel access, dirty flags, and versioning.
//!
//! [`Chunk`] wraps [`ChunkData`] and provides a safe, coordinate-based API
//! using `u8` coordinates in the range `[0, 32)`. Out-of-bounds access is
//! handled gracefully without panics.

use crate::chunk::{CHUNK_SIZE, ChunkData};
use crate::registry::VoxelTypeId;

/// Dirty-flag bit: chunk mesh needs rebuilding.
pub const MESH_DIRTY: u8 = 0b0000_0001;
/// Dirty-flag bit: chunk needs saving to disk.
pub const SAVE_DIRTY: u8 = 0b0000_0010;
/// Dirty-flag bit: chunk needs network sync.
pub const NETWORK_DIRTY: u8 = 0b0000_0100;

/// All dirty flags combined.
const ALL_DIRTY: u8 = MESH_DIRTY | SAVE_DIRTY | NETWORK_DIRTY;

/// A voxel chunk with bounds-checked access, dirty tracking, and versioning.
///
/// Coordinates are `u8` values in `[0, 32)`. Out-of-bounds reads return Air,
/// and out-of-bounds writes are silently ignored (with a warning log).
#[derive(Clone, Debug)]
pub struct Chunk {
    /// The underlying palette-compressed storage.
    data: ChunkData,
    /// Bitfield of dirty flags.
    dirty: u8,
    /// Monotonically increasing version counter, incremented on each mutation.
    version: u64,
}

impl Chunk {
    /// Creates a new chunk filled with Air (`VoxelTypeId(0)`).
    pub fn new() -> Self {
        Self {
            data: ChunkData::new_air(),
            dirty: 0,
            version: 0,
        }
    }

    /// Creates a new chunk filled with the given voxel type.
    pub fn new_filled(voxel: VoxelTypeId) -> Self {
        Self {
            data: ChunkData::new(voxel),
            dirty: 0,
            version: 0,
        }
    }

    /// Returns the voxel type at `(x, y, z)`.
    ///
    /// Returns `VoxelTypeId(0)` (Air) if any coordinate is out of bounds (`>= 32`).
    pub fn get(&self, x: u8, y: u8, z: u8) -> VoxelTypeId {
        if !Self::in_bounds(x, y, z) {
            tracing::warn!("Chunk::get out of bounds: ({}, {}, {})", x, y, z);
            return VoxelTypeId(0);
        }
        self.data.get(x as usize, y as usize, z as usize)
    }

    /// Sets the voxel type at `(x, y, z)`.
    ///
    /// No-op with a warning log if any coordinate is out of bounds (`>= 32`).
    pub fn set(&mut self, x: u8, y: u8, z: u8, voxel: VoxelTypeId) {
        if !Self::in_bounds(x, y, z) {
            tracing::warn!("Chunk::set out of bounds: ({}, {}, {})", x, y, z);
            return;
        }
        self.data.set(x as usize, y as usize, z as usize, voxel);
        self.dirty |= ALL_DIRTY;
        self.version += 1;
    }

    /// Fills every voxel in the chunk with the given type.
    ///
    /// Optimized to reset palette and storage in O(1).
    pub fn fill(&mut self, voxel: VoxelTypeId) {
        self.data.fill(voxel);
        self.dirty |= ALL_DIRTY;
        self.version += 1;
    }

    /// Returns the current dirty flags.
    pub fn dirty_flags(&self) -> u8 {
        self.dirty
    }

    /// Returns `true` if the specified dirty flag (or combination) is set.
    pub fn is_dirty(&self, flag: u8) -> bool {
        self.dirty & flag == flag
    }

    /// Mark specific dirty flags.
    pub fn mark_dirty(&mut self, flags: u8) {
        self.dirty |= flags;
    }

    /// Clears the specified dirty flag bits.
    pub fn clear_dirty(&mut self, flags: u8) {
        self.dirty &= !flags;
    }

    /// Returns the current version counter.
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Returns a reference to the underlying [`ChunkData`].
    pub fn data(&self) -> &ChunkData {
        &self.data
    }

    /// Returns a mutable reference to the underlying [`ChunkData`].
    ///
    /// Callers must manage dirty flags and version manually when using this.
    pub fn data_mut(&mut self) -> &mut ChunkData {
        &mut self.data
    }

    /// Returns the palette length of the underlying storage.
    pub fn palette_len(&self) -> usize {
        self.data.palette_len()
    }

    /// Checks whether `(x, y, z)` are all within `[0, 32)`.
    fn in_bounds(x: u8, y: u8, z: u8) -> bool {
        (x as usize) < CHUNK_SIZE && (y as usize) < CHUNK_SIZE && (z as usize) < CHUNK_SIZE
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_empty_chunk_returns_air() {
        let chunk = Chunk::new();
        assert_eq!(chunk.get(0, 0, 0), VoxelTypeId(0));
        assert_eq!(chunk.get(15, 15, 15), VoxelTypeId(0));
        assert_eq!(chunk.get(31, 31, 31), VoxelTypeId(0));
    }

    #[test]
    fn test_set_then_get_roundtrip() {
        let mut chunk = Chunk::new();
        chunk.set(5, 10, 20, VoxelTypeId(7));
        assert_eq!(chunk.get(5, 10, 20), VoxelTypeId(7));
        // Surrounding voxels remain air.
        assert_eq!(chunk.get(4, 10, 20), VoxelTypeId(0));
        assert_eq!(chunk.get(6, 10, 20), VoxelTypeId(0));
        assert_eq!(chunk.get(5, 9, 20), VoxelTypeId(0));
        assert_eq!(chunk.get(5, 11, 20), VoxelTypeId(0));
    }

    #[test]
    fn test_set_out_of_bounds_no_panic() {
        let mut chunk = Chunk::new();
        chunk.set(32, 0, 0, VoxelTypeId(1));
        chunk.set(0, 255, 0, VoxelTypeId(1));
        chunk.set(0, 0, 40, VoxelTypeId(1));
        // Chunk data should be unchanged (still all air).
        assert_eq!(chunk.get(0, 0, 0), VoxelTypeId(0));
        assert_eq!(chunk.version(), 0);
    }

    #[test]
    fn test_get_out_of_bounds_returns_air() {
        let chunk = Chunk::new_filled(VoxelTypeId(5));
        assert_eq!(chunk.get(32, 0, 0), VoxelTypeId(0));
        assert_eq!(chunk.get(0, 255, 0), VoxelTypeId(0));
        assert_eq!(chunk.get(0, 0, 40), VoxelTypeId(0));
    }

    #[test]
    fn test_set_same_voxel_twice() {
        let mut chunk = Chunk::new();
        let type_a = VoxelTypeId(3);
        let type_b = VoxelTypeId(9);

        chunk.set(3, 3, 3, type_a);
        assert_eq!(chunk.get(3, 3, 3), type_a);

        chunk.set(3, 3, 3, type_b);
        assert_eq!(chunk.get(3, 3, 3), type_b);

        chunk.set(3, 3, 3, type_a);
        assert_eq!(chunk.get(3, 3, 3), type_a);
    }

    #[test]
    fn test_fill_entire_chunk() {
        let mut chunk = Chunk::new();
        chunk.fill(VoxelTypeId(5));

        // Spot-check several positions.
        for z in (0..32).step_by(7) {
            for y in (0..32).step_by(7) {
                for x in (0..32).step_by(7) {
                    assert_eq!(chunk.get(x, y, z), VoxelTypeId(5));
                }
            }
        }
        assert_eq!(chunk.palette_len(), 1);

        // Fill back to air.
        chunk.fill(VoxelTypeId(0));
        for z in (0..32).step_by(7) {
            for y in (0..32).step_by(7) {
                for x in (0..32).step_by(7) {
                    assert_eq!(chunk.get(x, y, z), VoxelTypeId(0));
                }
            }
        }
        assert_eq!(chunk.palette_len(), 1);
    }

    #[test]
    fn test_dirty_flags_and_version() {
        let mut chunk = Chunk::new();
        assert_eq!(chunk.dirty_flags(), 0);
        assert_eq!(chunk.version(), 0);

        chunk.set(0, 0, 0, VoxelTypeId(1));
        assert_ne!(chunk.dirty_flags(), 0);
        assert_eq!(chunk.dirty_flags() & MESH_DIRTY, MESH_DIRTY);
        assert_eq!(chunk.dirty_flags() & SAVE_DIRTY, SAVE_DIRTY);
        assert_eq!(chunk.dirty_flags() & NETWORK_DIRTY, NETWORK_DIRTY);
        assert_eq!(chunk.version(), 1);

        chunk.clear_dirty(MESH_DIRTY);
        assert_eq!(chunk.dirty_flags() & MESH_DIRTY, 0);
        assert_ne!(chunk.dirty_flags(), 0); // other flags still set

        chunk.set(1, 0, 0, VoxelTypeId(2));
        assert_eq!(chunk.version(), 2);
    }

    #[test]
    fn test_new_chunk_is_not_dirty() {
        let chunk = Chunk::new();
        assert!(!chunk.is_dirty(MESH_DIRTY));
        assert!(!chunk.is_dirty(SAVE_DIRTY));
        assert!(!chunk.is_dirty(NETWORK_DIRTY));
    }

    #[test]
    fn test_set_voxel_marks_all_flags() {
        let mut chunk = Chunk::new();
        chunk.set(0, 0, 0, VoxelTypeId(1));
        assert!(chunk.is_dirty(MESH_DIRTY));
        assert!(chunk.is_dirty(SAVE_DIRTY));
        assert!(chunk.is_dirty(NETWORK_DIRTY));
    }

    #[test]
    fn test_clear_one_flag_preserves_others() {
        let mut chunk = Chunk::new();
        chunk.set(0, 0, 0, VoxelTypeId(1));
        chunk.clear_dirty(MESH_DIRTY);
        assert!(!chunk.is_dirty(MESH_DIRTY));
        assert!(chunk.is_dirty(SAVE_DIRTY));
        assert!(chunk.is_dirty(NETWORK_DIRTY));
    }

    #[test]
    fn test_fill_sets_dirty_and_version() {
        let mut chunk = Chunk::new();
        chunk.fill(VoxelTypeId(5));
        assert_ne!(chunk.dirty_flags(), 0);
        assert_eq!(chunk.version(), 1);
    }
}
