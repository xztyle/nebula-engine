# Dirty Tracking

## Problem

When a voxel changes, multiple downstream systems need to react: the meshing system must rebuild the chunk's mesh, the save system must write the chunk to disk, and the networking system must replicate the change to other clients. However, these systems run at different cadences — meshing runs every frame, saving runs periodically or on unload, and networking runs on its own tick rate. Without explicit tracking of which chunks have pending changes for which systems, the engine would either wastefully reprocess unchanged chunks every frame or miss changes entirely. Each system needs its own independent "dirty" flag so that clearing the mesh-dirty flag after remeshing does not suppress the save-dirty flag that the save system has not yet acted on.

## Solution

Implement a `DirtyFlags` bitfield on each `Chunk` that independently tracks modification state for each consuming system.

### Flag Definitions

```rust
use bitflags::bitflags;

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct DirtyFlags: u8 {
        /// Chunk mesh needs to be rebuilt.
        const MESH_DIRTY    = 0b0000_0001;
        /// Chunk needs to be saved to disk.
        const SAVE_DIRTY    = 0b0000_0010;
        /// Chunk needs to be replicated over the network.
        const NETWORK_DIRTY = 0b0000_0100;
        /// All flags combined (convenience for set operations).
        const ALL           = Self::MESH_DIRTY.bits()
                            | Self::SAVE_DIRTY.bits()
                            | Self::NETWORK_DIRTY.bits();
    }
}
```

### Chunk Integration

```rust
pub struct Chunk {
    pub(crate) data: CowChunk,       // voxel data (CoW wrapper)
    pub(crate) address: ChunkAddress,
    pub(crate) dirty: DirtyFlags,
    pub(crate) version: u64,
    // ...
}

impl Chunk {
    /// Mark specific dirty flags.
    pub fn mark_dirty(&mut self, flags: DirtyFlags) {
        self.dirty |= flags;
    }

    /// Check if a specific flag (or combination) is set.
    pub fn is_dirty(&self, flag: DirtyFlags) -> bool {
        self.dirty.contains(flag)
    }

    /// Clear specific dirty flags (called by the system that processed the change).
    pub fn clear_dirty(&mut self, flags: DirtyFlags) {
        self.dirty.remove(flags);
    }
}
```

### Automatic Marking

The `Chunk::set()` method (from story 03) automatically calls `self.mark_dirty(DirtyFlags::ALL)` whenever a voxel is modified. This ensures no system misses a change. `Chunk::fill()` does the same.

### Iteration by Dirty Flag

The `ChunkManager` provides a method to iterate over chunks that have a specific dirty flag set:

```rust
impl ChunkManager {
    /// Iterate over addresses of chunks that have the given dirty flag set.
    pub fn iter_dirty(&self, flag: DirtyFlags) -> impl Iterator<Item = &ChunkAddress> {
        self.chunks.iter()
            .filter(move |(_, chunk)| chunk.is_dirty(flag))
            .map(|(addr, _)| addr)
    }

    /// Iterate over mutable references to chunks with the given dirty flag.
    /// Useful for clearing flags after processing.
    pub fn iter_dirty_mut(&mut self, flag: DirtyFlags)
        -> impl Iterator<Item = (&ChunkAddress, &mut Chunk)>
    {
        self.chunks.iter_mut()
            .filter(move |(_, chunk)| chunk.is_dirty(flag))
    }
}
```

### System Usage Pattern

Each system follows this pattern:

```rust
fn meshing_system(mut chunk_manager: ResMut<ChunkManager>) {
    for (addr, chunk) in chunk_manager.iter_dirty_mut(DirtyFlags::MESH_DIRTY) {
        rebuild_mesh(addr, chunk);
        chunk.clear_dirty(DirtyFlags::MESH_DIRTY);
    }
}
```

This ensures each system processes changes independently and only clears its own flag.

## Outcome

A `DirtyFlags` bitfield integrated into the `Chunk` struct, with automatic marking on voxel modification and per-system clearing. The `ChunkManager` provides filtered iteration over dirty chunks. No system misses changes, and no system redundantly processes unchanged chunks.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo modifies one voxel per frame in a loaded chunk and the title shows `Dirty chunks: 1/25`. Only the modified chunk is flagged for re-meshing.

## Crates & Dependencies

- **`bitflags`** `2.9` — Type-safe bitfield macro for `DirtyFlags`

## Unit Tests

- **`test_new_chunk_is_not_dirty`** — Create a new `Chunk` and assert `chunk.is_dirty(DirtyFlags::MESH_DIRTY) == false`, `chunk.is_dirty(DirtyFlags::SAVE_DIRTY) == false`, and `chunk.is_dirty(DirtyFlags::NETWORK_DIRTY) == false`.
- **`test_set_voxel_marks_all_flags`** — Call `chunk.set(0, 0, 0, VoxelTypeId(1))` and assert `chunk.is_dirty(DirtyFlags::MESH_DIRTY)`, `chunk.is_dirty(DirtyFlags::SAVE_DIRTY)`, and `chunk.is_dirty(DirtyFlags::NETWORK_DIRTY)` are all `true`.
- **`test_clear_one_flag_preserves_others`** — Mark all flags dirty via a voxel set. Call `chunk.clear_dirty(DirtyFlags::MESH_DIRTY)`. Assert `is_dirty(MESH_DIRTY) == false` but `is_dirty(SAVE_DIRTY) == true` and `is_dirty(NETWORK_DIRTY) == true`.
- **`test_iter_dirty_returns_only_dirty_chunks`** — Load 3 chunks into a `ChunkManager`. Modify voxels in chunks 1 and 3 but not chunk 2. Call `iter_dirty(DirtyFlags::MESH_DIRTY)` and assert it yields exactly the addresses of chunks 1 and 3.
