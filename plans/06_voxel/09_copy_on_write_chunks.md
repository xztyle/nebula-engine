# Copy-on-Write Chunks

## Problem

In a typical world, the vast majority of chunks are either completely empty (all air, in space or above terrain) or completely filled with a single material (deep underground solid stone). Thousands of these identical chunks exist simultaneously, each consuming its own allocation for the palette and metadata. With 10,000 all-air chunks, this wastes memory on 10,000 identical copies of the same trivial data structure. The engine needs a way to share storage between chunks that have identical data, allocating unique storage only when a chunk is actually modified. This is especially important during initial world loading, where large volumes of space are generated as uniform chunks before the player interacts with them.

## Solution

Implement Copy-on-Write (CoW) semantics for chunk data using `Arc<ChunkData>` in the `nebula-voxel` crate.

### CowChunk Wrapper

```rust
use std::sync::Arc;

/// Copy-on-Write wrapper around chunk voxel data.
/// Multiple CowChunk instances can share the same underlying ChunkData
/// via Arc. On first mutation, the data is cloned to ensure exclusivity.
pub struct CowChunk {
    data: Arc<ChunkData>,
}

impl CowChunk {
    /// Create a new CowChunk with the given data.
    pub fn new(data: ChunkData) -> Self {
        Self { data: Arc::new(data) }
    }

    /// Create a CowChunk that shares storage with another.
    pub fn clone_shared(&self) -> Self {
        Self { data: Arc::clone(&self.data) }
    }

    /// Immutable access — always cheap (no clone).
    pub fn get(&self) -> &ChunkData {
        &self.data
    }

    /// Mutable access — clones the data if shared (Arc strong count > 1).
    /// After this call, `self` is guaranteed to have exclusive ownership.
    pub fn get_mut(&mut self) -> &mut ChunkData {
        Arc::make_mut(&mut self.data)
    }

    /// Check whether this CowChunk shares data with any other instance.
    pub fn is_shared(&self) -> bool {
        Arc::strong_count(&self.data) > 1
    }

    /// Number of CowChunk instances sharing this data.
    pub fn ref_count(&self) -> usize {
        Arc::strong_count(&self.data)
    }
}
```

### Shared Default Chunks

Common chunk configurations are pre-allocated as shared singletons:

```rust
lazy_static! {
    /// Shared all-air chunk data. Every newly created empty chunk
    /// points to this single allocation.
    static ref AIR_CHUNK: Arc<ChunkData> = Arc::new(ChunkData::new_uniform(VoxelTypeId(0)));
}

impl CowChunk {
    /// Create a default all-air chunk that shares storage with all other
    /// default chunks. Extremely cheap — just an Arc clone.
    pub fn new_air() -> Self {
        Self { data: Arc::clone(&AIR_CHUNK) }
    }
}
```

### Integration with Chunk

The `Chunk` struct uses `CowChunk` instead of owning `ChunkData` directly:

```rust
pub struct Chunk {
    pub(crate) data: CowChunk,
    pub(crate) address: ChunkAddress,
    pub(crate) dirty: DirtyFlags,
    pub(crate) version: u64,
}

impl Chunk {
    pub fn get(&self, x: u8, y: u8, z: u8) -> VoxelTypeId {
        self.data.get().get_voxel(x, y, z)
    }

    pub fn set(&mut self, x: u8, y: u8, z: u8, voxel: VoxelTypeId) {
        // CoW: clones data only if shared
        self.data.get_mut().set_voxel(x, y, z, voxel);
        self.mark_dirty(DirtyFlags::ALL);
        self.version += 1;
    }
}
```

### Memory Savings

With 10,000 all-air chunks sharing one `ChunkData`:
- **Without CoW**: 10,000 x ~64 bytes (minimal ChunkData with 1-entry palette) = 640 KB
- **With CoW**: 1 x ~64 bytes + 10,000 x 8 bytes (Arc pointer) = 80 KB
- **Savings**: ~560 KB for air alone, more for other common uniform types

The real savings are larger when considering that non-uniform but identical chunks (e.g., chunks generated from the same terrain seed at symmetric positions) can also share storage, though this requires explicit deduplication.

### Thread Safety

`Arc<ChunkData>` is `Send + Sync`, so `CowChunk` can be safely shared across threads. The `Arc::make_mut()` call handles the clone-on-write atomically — if the strong count is 1, it returns a mutable reference without cloning. This is ideal for the common case where a chunk has already been uniquely owned (after the first modification).

## Outcome

A `CowChunk` wrapper in `nebula-voxel` that allows multiple chunks to share identical data via `Arc`. Reads are zero-cost (just a pointer dereference). Writes clone the data only when shared. All-air chunks created via `CowChunk::new_air()` share a single static allocation, saving significant memory in worlds with large empty volumes.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo clones a chunk and modifies the clone. The console confirms the original is unchanged: `Original CRC: 0xABCD, Clone CRC: 0x1234`.

## Crates & Dependencies

- **`lazy_static`** `1.5` or **`once_cell`** `1.20` — Singleton allocation for shared default chunk data (or use `std::sync::LazyLock` on Rust edition 2024)
- No additional external dependencies; `Arc` is from `std::sync`

## Unit Tests

- **`test_two_default_chunks_share_arc`** — Create two chunks via `CowChunk::new_air()`. Assert `Arc::ptr_eq()` on their internal data (same allocation). Assert `ref_count() == 2` (plus 1 for the static = 3, or 2 if not counting the static, depending on implementation).
- **`test_write_to_one_does_not_affect_other`** — Create two shared chunks. Write a voxel to chunk A. Assert chunk B still reads Air at that position. Assert chunk A reads the new voxel type.
- **`test_arc_strong_count_drops_after_clone`** — Create a shared chunk with ref count 3. Call `get_mut()` on one instance (triggering a clone). Assert the original Arc's strong count decreased by 1, and the mutated instance's strong count is 1 (exclusive).
- **`test_all_air_chunks_share_storage`** — Create 1,000 `CowChunk::new_air()` instances. Assert they all share the same underlying pointer. Assert total memory consumed is significantly less than 1,000 separate allocations.
- **`test_memory_savings_measurable`** — Create 100 shared air chunks and 100 independently allocated air chunks. Compare the memory footprint (via `Arc::strong_count` and data pointer equality) and assert the shared variant uses fewer unique allocations.
