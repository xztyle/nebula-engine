//! Palette-compressed chunk storage for 32×32×32 voxel volumes.
//!
//! Each chunk maintains a palette of distinct [`VoxelTypeId`] values and a
//! bit-packed index array. The bit width automatically scales with the number
//! of palette entries, keeping memory usage minimal for homogeneous chunks.

use serde::{Deserialize, Serialize};

use crate::bit_packed::BitPackedArray;
use crate::registry::VoxelTypeId;

/// Side length of a chunk in voxels.
pub const CHUNK_SIZE: usize = 32;

/// Total number of voxels in a chunk (32³).
pub const CHUNK_VOLUME: usize = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

/// Palette-compressed voxel storage for a 32×32×32 chunk.
///
/// Voxels are stored as indices into a local palette, using the minimum number
/// of bits required. A uniform (single-type) chunk uses zero bytes of index
/// storage.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkData {
    /// Palette mapping local indices to global [`VoxelTypeId`] values.
    palette: Vec<VoxelTypeId>,
    /// Bit-packed voxel indices into the palette.
    storage: BitPackedArray,
    /// Current bits per index (0, 2, 4, 8, or 16).
    bit_width: u8,
}

impl ChunkData {
    /// Creates a new chunk filled entirely with the given voxel type.
    ///
    /// The resulting chunk is uniform (bit width 0, zero index storage).
    pub fn new(fill: VoxelTypeId) -> Self {
        Self {
            palette: vec![fill],
            storage: BitPackedArray::new(0, CHUNK_VOLUME),
            bit_width: 0,
        }
    }

    /// Creates a new chunk filled with Air (`VoxelTypeId(0)`).
    pub fn new_air() -> Self {
        Self::new(VoxelTypeId(0))
    }

    /// Returns the voxel type at position `(x, y, z)`.
    ///
    /// Each coordinate must be in `0..32`.
    pub fn get(&self, x: usize, y: usize, z: usize) -> VoxelTypeId {
        let index = Self::linear_index(x, y, z);
        if self.bit_width == 0 {
            return self.palette[0];
        }
        let palette_index = self.storage.get(index) as usize;
        self.palette[palette_index]
    }

    /// Sets the voxel type at position `(x, y, z)`.
    ///
    /// If the new type is not in the palette, it is added. If the palette grows
    /// past the current bit-width capacity, the storage is upgraded.
    pub fn set(&mut self, x: usize, y: usize, z: usize, voxel: VoxelTypeId) {
        let palette_idx = self.palette_index_or_insert(voxel);
        let linear = Self::linear_index(x, y, z);
        if self.bit_width == 0 {
            // Uniform chunk: if setting to the same type, nothing to do.
            if palette_idx == 0 {
                return;
            }
            // Otherwise upgrade was already done by palette_index_or_insert.
        }
        self.storage.set(linear, palette_idx as u16);
    }

    /// Returns the number of entries in the palette.
    pub fn palette_len(&self) -> usize {
        self.palette.len()
    }

    /// Returns the current bit width per voxel index.
    pub fn bit_width(&self) -> u8 {
        self.bit_width
    }

    /// Resets the chunk to a uniform fill of the given voxel type.
    ///
    /// This is optimized to reset the palette to a single entry and clear
    /// the bit-packed storage, avoiding 32,768 individual `set()` calls.
    pub fn fill(&mut self, voxel: VoxelTypeId) {
        self.palette = vec![voxel];
        self.storage = BitPackedArray::new(0, CHUNK_VOLUME);
        self.bit_width = 0;
    }

    /// Returns approximate memory used by voxel index storage (bytes).
    pub fn storage_bytes(&self) -> usize {
        self.storage.storage_bytes()
    }

    /// Returns a reference to the palette.
    pub fn palette(&self) -> &[VoxelTypeId] {
        &self.palette
    }

    /// Returns a reference to the underlying bit-packed storage.
    pub fn storage(&self) -> &BitPackedArray {
        &self.storage
    }

    /// Constructs `ChunkData` from raw parts (used by deserialization).
    ///
    /// The caller must ensure the palette, storage, and bit width are consistent.
    pub(crate) fn from_raw_parts(
        palette: Vec<VoxelTypeId>,
        storage: BitPackedArray,
        bit_width: u8,
    ) -> Self {
        Self {
            palette,
            storage,
            bit_width,
        }
    }

    /// Compacts the palette by removing unused entries and potentially
    /// downgrading the bit width.
    ///
    /// This scans all voxels to determine which palette entries are still in
    /// use. Call sparingly (e.g. before serialization), not on every `set()`.
    pub fn compact(&mut self) {
        if self.bit_width == 0 {
            return;
        }

        // Count which palette indices are used.
        let mut used = vec![false; self.palette.len()];
        for i in 0..CHUNK_VOLUME {
            used[self.storage.get(i) as usize] = true;
        }

        let used_count = used.iter().filter(|&&u| u).count();

        // If only one type remains, collapse to uniform.
        if used_count <= 1 {
            let single = used
                .iter()
                .position(|&u| u)
                .map(|i| self.palette[i])
                .unwrap_or(self.palette[0]);
            self.palette = vec![single];
            self.storage = BitPackedArray::new(0, CHUNK_VOLUME);
            self.bit_width = 0;
            return;
        }

        // Build a mapping from old palette index to new index.
        let mut old_to_new = vec![0u16; self.palette.len()];
        let mut new_palette = Vec::with_capacity(used_count);
        for (old_idx, &is_used) in used.iter().enumerate() {
            if is_used {
                old_to_new[old_idx] = new_palette.len() as u16;
                new_palette.push(self.palette[old_idx]);
            }
        }

        let new_bits = Self::bits_for_palette_size(new_palette.len());

        // Rebuild storage with new indices and potentially narrower bit width.
        let mut new_storage = BitPackedArray::new(new_bits, CHUNK_VOLUME);
        for i in 0..CHUNK_VOLUME {
            let old_idx = self.storage.get(i) as usize;
            new_storage.set(i, old_to_new[old_idx]);
        }

        self.palette = new_palette;
        self.storage = new_storage;
        self.bit_width = new_bits;
    }

    /// Converts `(x, y, z)` to a linear index (x varies fastest).
    fn linear_index(x: usize, y: usize, z: usize) -> usize {
        debug_assert!(x < CHUNK_SIZE && y < CHUNK_SIZE && z < CHUNK_SIZE);
        x + y * CHUNK_SIZE + z * CHUNK_SIZE * CHUNK_SIZE
    }

    /// Returns the required bit width for a palette of the given size.
    fn bits_for_palette_size(size: usize) -> u8 {
        match size {
            0 | 1 => 0,
            2..=4 => 2,
            5..=16 => 4,
            17..=256 => 8,
            _ => 16,
        }
    }

    /// Finds or inserts a voxel type in the palette, upgrading storage if needed.
    ///
    /// Returns the palette index for the given voxel type.
    fn palette_index_or_insert(&mut self, voxel: VoxelTypeId) -> usize {
        // Check if already present.
        if let Some(idx) = self.palette.iter().position(|&v| v == voxel) {
            return idx;
        }

        // Need to add a new entry.
        let new_size = self.palette.len() + 1;
        let new_bits = Self::bits_for_palette_size(new_size);

        if new_bits != self.bit_width {
            self.upgrade_storage(new_bits);
        }

        let idx = self.palette.len();
        self.palette.push(voxel);
        idx
    }

    /// Rebuilds the storage array at a wider bit width, preserving existing data.
    fn upgrade_storage(&mut self, new_bits: u8) {
        let mut new_storage = BitPackedArray::new(new_bits, CHUNK_VOLUME);
        if self.bit_width > 0 {
            for i in 0..CHUNK_VOLUME {
                new_storage.set(i, self.storage.get(i));
            }
        }
        // bit_width == 0 means uniform: all indices are 0, and new_storage is already zeroed.
        self.storage = new_storage;
        self.bit_width = new_bits;
    }
}

impl Default for ChunkData {
    fn default() -> Self {
        Self::new_air()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_chunk_single_palette_entry() {
        let chunk = ChunkData::new_air();
        assert_eq!(chunk.palette_len(), 1);
        assert_eq!(chunk.palette[0], VoxelTypeId(0));
        assert_eq!(chunk.bit_width(), 0);
        assert_eq!(chunk.storage_bytes(), 0);
    }

    #[test]
    fn test_single_type_change_grows_palette() {
        let mut chunk = ChunkData::new_air();
        chunk.set(0, 0, 0, VoxelTypeId(1));
        assert_eq!(chunk.palette_len(), 2);
        assert_eq!(chunk.bit_width(), 2);
        assert_eq!(chunk.get(0, 0, 0), VoxelTypeId(1));
        // Other voxels remain air.
        assert_eq!(chunk.get(1, 0, 0), VoxelTypeId(0));
    }

    #[test]
    fn test_palette_compresses_back() {
        let mut chunk = ChunkData::new_air();
        chunk.set(5, 5, 5, VoxelTypeId(42));
        assert_eq!(chunk.palette_len(), 2);
        assert_eq!(chunk.bit_width(), 2);

        // Set it back to air.
        chunk.set(5, 5, 5, VoxelTypeId(0));
        chunk.compact();
        assert_eq!(chunk.palette_len(), 1);
        assert_eq!(chunk.bit_width(), 0);
    }

    #[test]
    fn test_all_voxels_accessible() {
        let mut chunk = ChunkData::new_air();
        // Use 3 types so we stay at 2-bit width.
        let types = [VoxelTypeId(0), VoxelTypeId(1), VoxelTypeId(2)];
        chunk.set(0, 0, 0, VoxelTypeId(1)); // prime palette
        chunk.set(0, 0, 0, VoxelTypeId(0)); // restore
        // Ensure type 2 is in palette.
        chunk.set(0, 0, 0, VoxelTypeId(2));
        chunk.set(0, 0, 0, VoxelTypeId(0));

        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let t = types[(x + y + z) % 3];
                    chunk.set(x, y, z, t);
                }
            }
        }
        for z in 0..CHUNK_SIZE {
            for y in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let expected = types[(x + y + z) % 3];
                    assert_eq!(chunk.get(x, y, z), expected, "mismatch at ({x}, {y}, {z})");
                }
            }
        }
    }

    #[test]
    fn test_bit_width_upgrades_at_thresholds() {
        let mut chunk = ChunkData::new_air();
        assert_eq!(chunk.bit_width(), 0); // 1 type

        // Add types 1..=3 → 2-4 types → bit_width 2
        for i in 1..=3u16 {
            chunk.set(i as usize, 0, 0, VoxelTypeId(i));
        }
        assert_eq!(chunk.bit_width(), 2);
        assert_eq!(chunk.palette_len(), 4);

        // Add type 4 → 5 types → bit_width 4
        chunk.set(4, 0, 0, VoxelTypeId(4));
        assert_eq!(chunk.bit_width(), 4);
        assert_eq!(chunk.palette_len(), 5);

        // Add types up to 16 → still bit_width 4
        for i in 5..=15u16 {
            chunk.set(i as usize, 0, 0, VoxelTypeId(i));
        }
        assert_eq!(chunk.bit_width(), 4);
        assert_eq!(chunk.palette_len(), 16);

        // Add type 16 → 17 types → bit_width 8
        chunk.set(16, 0, 0, VoxelTypeId(16));
        assert_eq!(chunk.bit_width(), 8);
        assert_eq!(chunk.palette_len(), 17);

        // Add types up to 256 → still bit_width 8
        for i in 17..=255u16 {
            chunk.set(
                i as usize % CHUNK_SIZE,
                i as usize / CHUNK_SIZE,
                0,
                VoxelTypeId(i),
            );
        }
        assert_eq!(chunk.bit_width(), 8);
        assert_eq!(chunk.palette_len(), 256);

        // Add type 256 → 257 types → bit_width 16
        chunk.set(0, 8, 0, VoxelTypeId(256));
        assert_eq!(chunk.bit_width(), 16);
        assert_eq!(chunk.palette_len(), 257);
    }

    #[test]
    fn test_uniform_chunk_get_returns_fill_type() {
        let chunk = ChunkData::new(VoxelTypeId(7));
        for z in 0..2 {
            for y in 0..2 {
                for x in 0..2 {
                    assert_eq!(chunk.get(x, y, z), VoxelTypeId(7));
                }
            }
        }
    }

    #[test]
    fn test_set_same_type_on_uniform_is_noop() {
        let mut chunk = ChunkData::new(VoxelTypeId(3));
        chunk.set(0, 0, 0, VoxelTypeId(3));
        assert_eq!(chunk.bit_width(), 0);
        assert_eq!(chunk.palette_len(), 1);
    }
}
