//! Central owner for all loaded chunks, keyed by [`ChunkAddress`].
//!
//! The [`ChunkManager`] provides O(1) chunk lookup, insert, and removal
//! using an [`FxHashMap`](rustc_hash::FxHashMap) for fast hashing of
//! small fixed-size keys.

use rustc_hash::FxHashMap;

use crate::chunk_api::Chunk;

/// Identifies a chunk's position in the world.
///
/// Uses `i64` coordinates representing chunk-grid positions (world
/// millimetre coordinates divided by chunk size). The `face` field
/// indicates which cube-sphere face the chunk belongs to (0–5), or a
/// special value for non-planetary chunks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChunkAddress {
    /// Chunk-grid X coordinate.
    pub x: i64,
    /// Chunk-grid Y coordinate.
    pub y: i64,
    /// Chunk-grid Z coordinate.
    pub z: i64,
    /// Cube-sphere face index (0–5) or special value for free-space chunks.
    pub face: u8,
}

impl ChunkAddress {
    /// Creates a new chunk address.
    pub fn new(x: i64, y: i64, z: i64, face: u8) -> Self {
        Self { x, y, z, face }
    }

    /// Returns the address of the neighboring chunk offset by `(dx, dy, dz)`.
    ///
    /// Typically called with unit offsets (e.g. `(1,0,0)` for +X neighbor).
    /// The `face` field is preserved (same cube-sphere face).
    pub fn offset(self, dx: i64, dy: i64, dz: i64) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            z: self.z + dz,
            face: self.face,
        }
    }
}

/// Owns all currently-loaded chunks and provides fast access by [`ChunkAddress`].
///
/// This is the single authority for which chunks exist in memory.
/// Systems like meshing, physics, and rendering query chunks through
/// this manager exclusively.
pub struct ChunkManager {
    chunks: FxHashMap<ChunkAddress, Chunk>,
}

impl ChunkManager {
    /// Creates an empty chunk manager with no loaded chunks.
    pub fn new() -> Self {
        Self {
            chunks: FxHashMap::default(),
        }
    }

    /// Inserts a chunk at the given address.
    ///
    /// If a chunk already exists at this address it is replaced
    /// (idempotent reload).
    pub fn load_chunk(&mut self, addr: ChunkAddress, chunk: Chunk) {
        self.chunks.insert(addr, chunk);
    }

    /// Removes and returns the chunk at the given address.
    ///
    /// Returns `None` if no chunk was loaded there.
    pub fn unload_chunk(&mut self, addr: ChunkAddress) -> Option<Chunk> {
        self.chunks.remove(&addr)
    }

    /// Immutable access to a loaded chunk.
    pub fn get_chunk(&self, addr: &ChunkAddress) -> Option<&Chunk> {
        self.chunks.get(addr)
    }

    /// Mutable access to a loaded chunk (for voxel modification).
    pub fn get_chunk_mut(&mut self, addr: &ChunkAddress) -> Option<&mut Chunk> {
        self.chunks.get_mut(addr)
    }

    /// Number of currently loaded chunks.
    pub fn loaded_count(&self) -> usize {
        self.chunks.len()
    }

    /// Iterates over all loaded chunk addresses.
    pub fn loaded_addresses(&self) -> impl Iterator<Item = &ChunkAddress> {
        self.chunks.keys()
    }

    /// Iterates over all loaded `(address, chunk)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&ChunkAddress, &Chunk)> {
        self.chunks.iter()
    }

    /// Mutable iteration over all loaded `(address, chunk)` pairs.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&ChunkAddress, &mut Chunk)> {
        self.chunks.iter_mut()
    }

    /// Iterates over addresses of chunks that have the given dirty flag set.
    pub fn iter_dirty(&self, flag: u8) -> impl Iterator<Item = &ChunkAddress> {
        self.chunks
            .iter()
            .filter(move |(_, chunk)| chunk.is_dirty(flag))
            .map(|(addr, _)| addr)
    }

    /// Iterates over mutable references to chunks with the given dirty flag.
    ///
    /// Useful for clearing flags after processing.
    pub fn iter_dirty_mut(
        &mut self,
        flag: u8,
    ) -> impl Iterator<Item = (&ChunkAddress, &mut Chunk)> {
        self.chunks
            .iter_mut()
            .filter(move |(_, chunk)| chunk.is_dirty(flag))
    }
}

impl Default for ChunkManager {
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
    use crate::registry::VoxelTypeId;

    fn addr(x: i64, y: i64, z: i64) -> ChunkAddress {
        ChunkAddress::new(x, y, z, 0)
    }

    #[test]
    fn test_load_then_get_returns_some() {
        let mut mgr = ChunkManager::new();
        let a = addr(0, 0, 0);
        let mut chunk = Chunk::new();
        chunk.set(1, 2, 3, VoxelTypeId(7));

        mgr.load_chunk(a, chunk);

        let got = mgr.get_chunk(&a);
        assert!(got.is_some());
        assert_eq!(got.expect("just checked").get(1, 2, 3), VoxelTypeId(7));
    }

    #[test]
    fn test_unload_then_get_returns_none() {
        let mut mgr = ChunkManager::new();
        let a = addr(1, 2, 3);

        mgr.load_chunk(a, Chunk::new());
        let removed = mgr.unload_chunk(a);
        assert!(removed.is_some());
        assert!(mgr.get_chunk(&a).is_none());
    }

    #[test]
    fn test_loaded_count_tracks_correctly() {
        let mut mgr = ChunkManager::new();
        assert_eq!(mgr.loaded_count(), 0);

        mgr.load_chunk(addr(0, 0, 0), Chunk::new());
        mgr.load_chunk(addr(1, 0, 0), Chunk::new());
        mgr.load_chunk(addr(0, 1, 0), Chunk::new());
        assert_eq!(mgr.loaded_count(), 3);

        mgr.unload_chunk(addr(1, 0, 0));
        assert_eq!(mgr.loaded_count(), 2);

        // Unloading non-existent address doesn't change count.
        mgr.unload_chunk(addr(99, 99, 99));
        assert_eq!(mgr.loaded_count(), 2);
    }

    #[test]
    fn test_iter_dirty_returns_only_dirty_chunks() {
        use crate::chunk_api::MESH_DIRTY;

        let mut mgr = ChunkManager::new();
        let a1 = addr(0, 0, 0);
        let a2 = addr(1, 0, 0);
        let a3 = addr(2, 0, 0);

        let mut c1 = Chunk::new();
        c1.set(0, 0, 0, VoxelTypeId(1)); // dirty
        let c2 = Chunk::new(); // clean
        let mut c3 = Chunk::new();
        c3.set(1, 1, 1, VoxelTypeId(2)); // dirty

        mgr.load_chunk(a1, c1);
        mgr.load_chunk(a2, c2);
        mgr.load_chunk(a3, c3);

        let dirty: Vec<_> = mgr.iter_dirty(MESH_DIRTY).copied().collect();
        assert_eq!(dirty.len(), 2);
        assert!(dirty.contains(&a1));
        assert!(dirty.contains(&a3));
        assert!(!dirty.contains(&a2));
    }

    #[test]
    fn test_double_load_is_idempotent() {
        let mut mgr = ChunkManager::new();
        let a = addr(5, 5, 5);

        let chunk1 = Chunk::new_filled(VoxelTypeId(1));
        mgr.load_chunk(a, chunk1);

        let chunk2 = Chunk::new_filled(VoxelTypeId(2));
        mgr.load_chunk(a, chunk2);

        assert_eq!(mgr.loaded_count(), 1);
        let got = mgr.get_chunk(&a).expect("should exist");
        assert_eq!(got.get(0, 0, 0), VoxelTypeId(2));
    }
}
