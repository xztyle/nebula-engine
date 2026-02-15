# Chunk Object Pool

## Problem

The Nebula Engine loads and unloads chunks continuously as the player moves across a cubesphere planet. Each chunk contains a palette-compressed voxel array (story 06_voxel/02), dirty flags, metadata, and potentially associated mesh data. Without pooling, every chunk load allocates a fresh `Chunk` on the heap -- allocating the palette vector, the index arrays, and internal bookkeeping -- only to deallocate all of it moments later when the chunk is unloaded. With a typical view distance loading 4,000 to 10,000 chunks and player movement causing hundreds of chunk transitions per second, this creates a steady stream of allocation and deallocation calls that fragments the heap and puts unnecessary pressure on the global allocator.

An object pool solves this by maintaining a reserve of pre-initialized `Chunk` objects. When the chunk manager needs a new chunk, it acquires one from the pool instead of allocating from the heap. When a chunk is unloaded, it is returned to the pool -- its data is cleared, but its heap allocations (vectors, palette storage) are retained. The next chunk that is acquired can reuse those allocations without going back to the allocator.

## Solution

Implement a `ChunkPool` in the `nebula_memory` crate (or `nebula_voxel`, depending on final crate boundaries) that pre-allocates a configurable number of `Chunk` objects at startup and manages their lifecycle.

### ChunkPool

```rust
/// A pool of reusable `Chunk` objects.
///
/// Chunks acquired from the pool are guaranteed to be in a clean initial state
/// (all voxels set to air, no dirty flags, no metadata). When released back to
/// the pool, chunks are reset but their internal allocations are preserved.
pub struct ChunkPool {
    /// Available chunks ready for reuse.
    free_list: Vec<Chunk>,
    /// Total number of chunks ever created by this pool (including those in use).
    total_created: usize,
    /// High watermark: maximum number of chunks simultaneously in use.
    high_watermark: usize,
    /// Current number of chunks in use (created - returned).
    in_use: usize,
}

impl ChunkPool {
    /// Create a new pool with `initial_capacity` pre-allocated chunks.
    ///
    /// Each chunk is initialized with default (empty) data.
    pub fn new(initial_capacity: usize) -> Self {
        let mut free_list = Vec::with_capacity(initial_capacity);
        for _ in 0..initial_capacity {
            free_list.push(Chunk::new_empty());
        }
        Self {
            free_list,
            total_created: initial_capacity,
            high_watermark: 0,
            in_use: 0,
        }
    }

    /// Acquire a chunk from the pool.
    ///
    /// If the pool has free chunks, one is removed from the free list, reset
    /// to a clean state, and returned. If the pool is empty, a new chunk is
    /// allocated from the heap (the pool grows dynamically).
    ///
    /// The returned chunk is guaranteed to be in the same state as `Chunk::new_empty()`.
    pub fn acquire(&mut self) -> Chunk {
        let mut chunk = if let Some(chunk) = self.free_list.pop() {
            chunk
        } else {
            // Pool is empty -- grow by allocating a new chunk.
            self.total_created += 1;
            Chunk::new_empty()
        };

        // Ensure the chunk is in a clean initial state.
        // This clears voxel data and metadata but preserves the internal
        // Vec allocations (capacity is retained, length is set to 0 or
        // palette is reset to a single "air" entry).
        chunk.reset();

        self.in_use += 1;
        if self.in_use > self.high_watermark {
            self.high_watermark = self.in_use;
        }

        chunk
    }

    /// Release a chunk back to the pool for future reuse.
    ///
    /// The chunk's data is not cleared here -- it will be cleared on the next
    /// `acquire()` call. This avoids doing unnecessary work if the chunk is
    /// never reused (e.g., during shutdown).
    pub fn release(&mut self, chunk: Chunk) {
        debug_assert!(self.in_use > 0, "released more chunks than acquired");
        self.in_use -= 1;
        self.free_list.push(chunk);
    }

    /// Number of chunks currently available in the pool (not in use).
    pub fn available(&self) -> usize {
        self.free_list.len()
    }

    /// Number of chunks currently in use (acquired but not released).
    pub fn in_use(&self) -> usize {
        self.in_use
    }

    /// Total number of chunks ever created by this pool.
    pub fn total_created(&self) -> usize {
        self.total_created
    }

    /// Maximum number of chunks that were simultaneously in use.
    pub fn high_watermark(&self) -> usize {
        self.high_watermark
    }

    /// Pre-allocate additional chunks to the pool.
    /// Useful if you know a spike in chunk demand is coming (e.g., teleportation).
    pub fn reserve(&mut self, additional: usize) {
        for _ in 0..additional {
            self.free_list.push(Chunk::new_empty());
            self.total_created += 1;
        }
    }

    /// Shrink the free list to at most `max_free` chunks, dropping the rest.
    /// Useful for releasing memory after a high-demand period.
    pub fn shrink_to(&mut self, max_free: usize) {
        self.free_list.truncate(max_free);
        self.free_list.shrink_to_fit();
    }
}
```

### Chunk::reset()

The `reset()` method on `Chunk` clears the voxel data without freeing the underlying allocation:

```rust
impl Chunk {
    /// Reset the chunk to its initial empty state.
    ///
    /// This clears voxel data, dirty flags, and metadata, but preserves
    /// the internal vector capacity so that the next use avoids allocation.
    pub fn reset(&mut self) {
        // Reset the palette to a single "air" entry.
        // This keeps the palette Vec's capacity intact.
        self.palette.clear();
        self.palette.push(VoxelType::AIR);

        // Reset all voxel indices to 0 (pointing to the "air" palette entry).
        // This reuses the index array without reallocating.
        self.indices.fill(0);

        // Clear metadata
        self.dirty = false;
        self.version = 0;
        self.face = CubeFace::PosX; // will be overwritten on load
        self.address = ChunkAddress::default();
    }
}
```

### Integration with ChunkManager

The `ChunkManager` (story 06_voxel/04) is modified to use the pool:

```rust
pub struct ChunkManager {
    chunks: FxHashMap<ChunkAddress, Chunk>,
    pool: ChunkPool,
}

impl ChunkManager {
    pub fn new(pool_initial_capacity: usize) -> Self {
        Self {
            chunks: FxHashMap::default(),
            pool: ChunkPool::new(pool_initial_capacity),
        }
    }

    /// Acquire an empty chunk from the pool for terrain generation.
    pub fn acquire_chunk(&mut self) -> Chunk {
        self.pool.acquire()
    }

    /// Unload a chunk, returning it to the pool instead of dropping it.
    pub fn unload_chunk(&mut self, addr: ChunkAddress) -> bool {
        if let Some(chunk) = self.chunks.remove(&addr) {
            self.pool.release(chunk);
            true
        } else {
            false
        }
    }
}
```

### Design Decisions

- **Lazy reset**: The chunk is reset on `acquire()`, not on `release()`. This avoids wasting work clearing a chunk that may never be reused (e.g., at shutdown when all chunks are released). It also means the pool does not need to guarantee the state of its free list entries.
- **Dynamic growth**: If the pool is empty, `acquire()` allocates a new chunk rather than returning an error. The pool never blocks. This ensures correctness even under unexpected demand, while still providing the performance benefit of reuse under normal conditions.
- **High watermark**: Tracking the peak number of simultaneously active chunks helps tune the initial pool size. If the watermark is consistently 5,000, pre-allocating 5,000 chunks at startup avoids all growth allocations during gameplay.
- **No `Arc` or `Mutex`**: The pool is owned by the `ChunkManager`, which is a Bevy ECS resource. Bevy's scheduling ensures only one system mutates the resource at a time, so no synchronization is needed within the pool itself.

## Outcome

The `nebula_memory` crate (or `nebula_voxel`) exports `ChunkPool` with `acquire()`, `release()`, `available()`, `in_use()`, and `high_watermark()`. The chunk manager uses the pool for all chunk lifecycle operations, eliminating per-chunk heap allocation during normal gameplay. Running `cargo test -p nebula_memory` passes all pool tests. The crate uses Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Chunk data objects are recycled from a pre-warmed pool. Loading a new chunk grabs from the pool instead of allocating. The console logs `Pool: 128 available, 256 in use`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `nebula_voxel` | workspace | `Chunk`, `VoxelType`, `ChunkAddress` types |
| `rustc-hash` | `2.1` | `FxHashMap` for chunk address lookup (workspace dependency) |

No external pool crates are used. The pool is a simple `Vec`-backed free list.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Acquiring a chunk from a pre-allocated pool should return a valid,
    /// empty chunk.
    #[test]
    fn test_acquire_returns_valid_chunk() {
        let mut pool = ChunkPool::new(10);

        let chunk = pool.acquire();

        // The chunk should be in the empty initial state.
        assert_eq!(chunk.palette.len(), 1, "palette should have only air");
        assert_eq!(chunk.palette[0], VoxelType::AIR);
        assert!(!chunk.dirty, "chunk should not be dirty after acquire");
        assert_eq!(pool.in_use(), 1);
        assert_eq!(pool.available(), 9);
    }

    /// Releasing a chunk should return it to the pool's free list.
    #[test]
    fn test_release_returns_chunk_to_pool() {
        let mut pool = ChunkPool::new(5);
        assert_eq!(pool.available(), 5);

        let chunk = pool.acquire();
        assert_eq!(pool.available(), 4);

        pool.release(chunk);
        assert_eq!(pool.available(), 5);
        assert_eq!(pool.in_use(), 0);
    }

    /// Acquiring after releasing should reuse the same chunk object,
    /// avoiding a new heap allocation. The chunk's internal Vec capacity
    /// should be preserved.
    #[test]
    fn test_acquire_after_release_reuses_chunk() {
        let mut pool = ChunkPool::new(1);

        let mut chunk = pool.acquire();
        // Write some data to force the palette Vec to grow.
        chunk.set_voxel(0, 0, 0, VoxelType::STONE);
        chunk.set_voxel(1, 0, 0, VoxelType::DIRT);
        let palette_capacity_before = chunk.palette.capacity();
        pool.release(chunk);

        let reused = pool.acquire();
        // The chunk should be reset (empty) but the palette Vec's capacity
        // should be >= what it was before, proving the allocation was reused.
        assert_eq!(reused.palette.len(), 1, "palette should be reset to air only");
        assert!(
            reused.palette.capacity() >= palette_capacity_before,
            "palette capacity should be preserved (was {palette_capacity_before}, got {})",
            reused.palette.capacity()
        );
        assert_eq!(pool.total_created(), 1, "no new chunk should be created");
    }

    /// If the pool is empty, acquire should allocate a new chunk from the heap.
    #[test]
    fn test_pool_grows_if_empty() {
        let mut pool = ChunkPool::new(0); // start with zero pre-allocated
        assert_eq!(pool.available(), 0);
        assert_eq!(pool.total_created(), 0);

        let chunk = pool.acquire();
        assert_eq!(pool.total_created(), 1, "should create one new chunk");
        assert_eq!(pool.in_use(), 1);

        // The acquired chunk should still be valid and empty.
        assert_eq!(chunk.palette.len(), 1);
        assert_eq!(chunk.palette[0], VoxelType::AIR);
    }

    /// A chunk acquired from the pool should have its data fully reset,
    /// even if the previous user wrote voxel data to it.
    #[test]
    fn test_chunk_data_is_reset_on_acquire() {
        let mut pool = ChunkPool::new(2);

        let mut chunk = pool.acquire();
        // Dirty the chunk with non-air voxels.
        for x in 0..4 {
            for y in 0..4 {
                for z in 0..4 {
                    chunk.set_voxel(x, y, z, VoxelType::STONE);
                }
            }
        }
        chunk.dirty = true;
        pool.release(chunk);

        let clean = pool.acquire();
        // All voxels should be air after reset.
        for x in 0..4 {
            for y in 0..4 {
                for z in 0..4 {
                    assert_eq!(
                        clean.get_voxel(x, y, z),
                        VoxelType::AIR,
                        "voxel at ({x},{y},{z}) should be air after reset"
                    );
                }
            }
        }
        assert!(!clean.dirty, "dirty flag should be cleared on acquire");
    }

    /// The high watermark should track peak simultaneous usage.
    #[test]
    fn test_high_watermark_tracks_peak() {
        let mut pool = ChunkPool::new(10);

        let a = pool.acquire();
        let b = pool.acquire();
        let c = pool.acquire();
        assert_eq!(pool.high_watermark(), 3);

        pool.release(a);
        pool.release(b);
        assert_eq!(pool.high_watermark(), 3, "watermark should not decrease");

        let _d = pool.acquire();
        assert_eq!(pool.high_watermark(), 3, "2 in use, watermark stays at 3");

        let _e = pool.acquire();
        let _f = pool.acquire();
        let _g = pool.acquire();
        assert_eq!(pool.high_watermark(), 5, "watermark should update to 5");
    }
}
```
