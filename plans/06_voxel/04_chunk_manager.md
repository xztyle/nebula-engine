# Chunk Manager

## Problem

The engine must manage thousands of chunks simultaneously — loading them when the player approaches, unloading them when the player moves away, and providing fast access to any currently loaded chunk by its spatial address. Without a central owner for chunk lifetimes, chunks would leak memory, be loaded redundantly, or be accessed after unloading. Systems like meshing, physics, and rendering all need to query "give me the chunk at this address" and get back either a reference or a clear indication that the chunk is not loaded. The chunk manager is the single authority for which chunks exist in memory at any given time.

## Solution

Introduce a `ChunkManager` struct in `nebula-voxel` that owns all loaded chunks in a `HashMap` keyed by `ChunkAddress`.

### ChunkAddress

```rust
/// Identifies a chunk's position in the world.
/// Uses i128 coordinates from nebula-math, divided by chunk size (32).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkAddress {
    pub x: i64,
    pub y: i64,
    pub z: i64,
    /// Which face of the cubesphere this chunk belongs to (0-5),
    /// or a special value for non-planetary chunks.
    pub face: u8,
}
```

The `i64` range per axis gives 9.2 x 10^18 chunks per axis. At 32 voxels per chunk, this covers 2.9 x 10^20 voxels per axis — more than sufficient even with 128-bit world coordinates, since chunks at planetary distances use LOD, not full voxel resolution.

### ChunkManager Implementation

```rust
pub struct ChunkManager {
    chunks: HashMap<ChunkAddress, Chunk>,
}

impl ChunkManager {
    pub fn new() -> Self;

    /// Insert a chunk at the given address.
    /// If a chunk already exists at this address, it is replaced (idempotent reload).
    pub fn load_chunk(&mut self, addr: ChunkAddress, chunk: Chunk);

    /// Remove and drop the chunk at the given address.
    /// Returns the removed chunk, or None if no chunk was loaded there.
    pub fn unload_chunk(&mut self, addr: ChunkAddress) -> Option<Chunk>;

    /// Immutable access to a loaded chunk.
    pub fn get_chunk(&self, addr: &ChunkAddress) -> Option<&Chunk>;

    /// Mutable access to a loaded chunk (for voxel modification).
    pub fn get_chunk_mut(&mut self, addr: &ChunkAddress) -> Option<&mut Chunk>;

    /// Number of currently loaded chunks.
    pub fn loaded_count(&self) -> usize;

    /// Iterate over all loaded chunk addresses.
    pub fn loaded_addresses(&self) -> impl Iterator<Item = &ChunkAddress>;

    /// Iterate over all loaded chunks (address, chunk) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&ChunkAddress, &Chunk)>;

    /// Mutable iteration (for batch operations like dirty flag clearing).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&ChunkAddress, &mut Chunk)>;
}
```

### Design Decisions

- **Synchronous API**: This story covers the in-memory chunk map only. Asynchronous I/O for loading chunk data from disk or generating terrain is handled by separate systems that produce `Chunk` values and feed them into `load_chunk()`. The `ChunkManager` itself is not async.
- **HashMap over BTreeMap**: Chunk access patterns are random (lookup by address), not sequential. `HashMap` gives O(1) amortized access. The hash for `ChunkAddress` is computed via `rustc_hash::FxHashMap` for speed, since chunk addresses are small fixed-size structs.
- **Owned chunks**: The `ChunkManager` owns chunks by value (not `Arc`). Copy-on-write semantics (story 09) are handled within the `Chunk` struct itself via `Arc<ChunkData>`. The manager does not need to know about CoW.
- **Idempotent load**: Calling `load_chunk()` with an address that already has a chunk replaces the old chunk. This simplifies the terrain generation pipeline — if a chunk is regenerated (e.g., due to a world edit), the new version simply overwrites the old one.

### ECS Integration

The `ChunkManager` is stored as a Bevy ECS resource:

```rust
app.insert_resource(ChunkManager::new());
```

Systems access it via `Res<ChunkManager>` (read) or `ResMut<ChunkManager>` (write). This automatically enforces Bevy's borrowing rules — only one system can mutate the chunk manager at a time.

## Outcome

A `ChunkManager` struct that provides O(1) chunk lookup by address, correct ownership semantics, and a clean API for load/unload lifecycle management. All chunk systems (meshing, physics, terrain generation, networking) interact with chunks exclusively through this manager.

## Demo Integration

**Demo crate:** `nebula-demo`

A 5x5 grid of chunks is loaded for one cube face. The title shows `Chunks loaded: 25`. As the camera moves, chunks at the trailing edge unload and new ones load.

## Crates & Dependencies

- **`rustc-hash`** `2.1` — `FxHashMap` for fast hashing of `ChunkAddress` (small fixed-size keys benefit from a simpler hash function than SipHash)
- **`bevy_ecs`** `0.15` — ECS resource integration (workspace dependency)

## Unit Tests

- **`test_load_then_get_returns_some`** — Create a `ChunkManager`, load a chunk at address `(0, 0, 0, face: 0)`, then assert `get_chunk(&addr).is_some()` and the returned chunk's data matches what was loaded.
- **`test_unload_then_get_returns_none`** — Load a chunk, then unload it. Assert `get_chunk(&addr).is_none()`. Assert the returned `Option<Chunk>` from `unload_chunk()` is `Some`.
- **`test_loaded_count_tracks_correctly`** — Start with an empty manager (`loaded_count() == 0`). Load 3 chunks at distinct addresses, assert `loaded_count() == 3`. Unload 1, assert `loaded_count() == 2`. Unload a non-existent address, assert `loaded_count() == 2` (unchanged).
- **`test_double_load_is_idempotent`** — Load a chunk at address A, then load a different chunk at the same address A. Assert `loaded_count() == 1`. Assert `get_chunk(&A)` returns the second chunk, not the first.
