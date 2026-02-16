//! Copy-on-Write wrapper around [`ChunkData`].
//!
//! Multiple [`CowChunk`] instances can share the same underlying [`ChunkData`]
//! via [`Arc`]. On first mutation, the data is cloned to ensure exclusivity.
//! This saves significant memory when many chunks contain identical data
//! (e.g. all-air chunks in space or uniform underground chunks).

use std::sync::{Arc, LazyLock};

use crate::chunk::ChunkData;
use crate::registry::VoxelTypeId;

/// Shared all-air chunk data singleton. Every default empty chunk points to
/// this single allocation.
static AIR_CHUNK: LazyLock<Arc<ChunkData>> =
    LazyLock::new(|| Arc::new(ChunkData::new(VoxelTypeId(0))));

/// Copy-on-Write wrapper around chunk voxel data.
///
/// Multiple `CowChunk` instances can share the same underlying [`ChunkData`]
/// via [`Arc`]. Reads are zero-cost (pointer dereference). Writes clone the
/// data only when shared (`Arc` strong count > 1).
#[derive(Clone, Debug)]
pub struct CowChunk {
    data: Arc<ChunkData>,
}

impl CowChunk {
    /// Creates a new `CowChunk` wrapping the given data.
    pub fn new(data: ChunkData) -> Self {
        Self {
            data: Arc::new(data),
        }
    }

    /// Creates a default all-air chunk that shares storage with all other
    /// default air chunks. Extremely cheap — just an `Arc` clone.
    pub fn new_air() -> Self {
        Self {
            data: Arc::clone(&AIR_CHUNK),
        }
    }

    /// Creates a shared clone that points to the same underlying data.
    ///
    /// This is cheaper than cloning the data — it only increments the
    /// reference count.
    pub fn clone_shared(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
        }
    }

    /// Immutable access to the underlying [`ChunkData`]. Always cheap.
    pub fn get(&self) -> &ChunkData {
        &self.data
    }

    /// Mutable access to the underlying [`ChunkData`].
    ///
    /// Clones the data if shared (`Arc` strong count > 1). After this call,
    /// `self` is guaranteed to have exclusive ownership.
    pub fn get_mut(&mut self) -> &mut ChunkData {
        Arc::make_mut(&mut self.data)
    }

    /// Returns `true` if this `CowChunk` shares data with any other instance.
    pub fn is_shared(&self) -> bool {
        Arc::strong_count(&self.data) > 1
    }

    /// Number of `CowChunk` instances (plus statics) sharing this data.
    pub fn ref_count(&self) -> usize {
        Arc::strong_count(&self.data)
    }

    /// Returns `true` if two `CowChunk` instances point to the same allocation.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }
}

impl Default for CowChunk {
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
    fn test_two_default_chunks_share_arc() {
        let a = CowChunk::new_air();
        let b = CowChunk::new_air();
        assert!(a.ptr_eq(&b), "two air chunks should share the same Arc");
        // Both share with the static, so at least 3 (static + a + b).
        assert!(a.ref_count() >= 3);
    }

    #[test]
    fn test_write_to_one_does_not_affect_other() {
        let mut a = CowChunk::new_air();
        let b = a.clone_shared();

        // Mutate a — should trigger CoW clone.
        a.get_mut().set(0, 0, 0, VoxelTypeId(42));

        assert_eq!(a.get().get(0, 0, 0), VoxelTypeId(42));
        assert_eq!(
            b.get().get(0, 0, 0),
            VoxelTypeId(0),
            "b should be unchanged"
        );
        assert!(!a.ptr_eq(&b), "after mutation they should not share");
    }

    #[test]
    fn test_arc_strong_count_drops_after_clone() {
        // Use a non-global chunk to avoid interference from parallel tests
        // sharing the static AIR_CHUNK.
        let a = CowChunk::new(ChunkData::new(VoxelTypeId(99)));
        let b = a.clone_shared();
        let mut c = a.clone_shared();

        assert_eq!(a.ref_count(), 3); // a + b + c
        // Mutate c — triggers CoW, c gets its own allocation.
        c.get_mut().set(1, 1, 1, VoxelTypeId(7));

        assert_eq!(a.ref_count(), 2); // a + b
        assert_eq!(c.ref_count(), 1, "mutated chunk should be exclusive");
        // b still shares with a.
        assert!(a.ptr_eq(&b));
    }

    #[test]
    fn test_all_air_chunks_share_storage() {
        let chunks: Vec<CowChunk> = (0..1000).map(|_| CowChunk::new_air()).collect();
        // All should point to the same allocation.
        for chunk in &chunks[1..] {
            assert!(chunks[0].ptr_eq(chunk));
        }
    }

    #[test]
    fn test_memory_savings_measurable() {
        // 100 shared air chunks: all point to same data.
        let shared: Vec<CowChunk> = (0..100).map(|_| CowChunk::new_air()).collect();
        // 100 independent chunks: each has its own allocation.
        let independent: Vec<CowChunk> = (0..100)
            .map(|_| CowChunk::new(ChunkData::new_air()))
            .collect();

        // All shared point to same pointer.
        for c in &shared[1..] {
            assert!(shared[0].ptr_eq(c));
        }
        // Independent chunks do NOT share.
        for c in &independent[1..] {
            assert!(!independent[0].ptr_eq(c));
        }
    }
}
