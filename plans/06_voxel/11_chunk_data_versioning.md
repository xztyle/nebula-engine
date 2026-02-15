# Chunk Data Versioning

## Problem

Multiple systems cache derived data from chunks — the meshing system caches the generated mesh, the physics system caches the collision shape, and the networking system tracks which version of a chunk each client has received. When a chunk is modified, these caches become stale. Dirty flags (story 06) indicate *that* a chunk has changed, but they do not answer the question "is my cached data still current?" A system that caches a mesh from a chunk needs to know whether the chunk has been modified *since the mesh was built*, not just whether the chunk is dirty right now (the dirty flag may have been cleared by another system). A monotonically increasing version number solves this: the system records the version at the time it builds the cache, and later compares that stored version against the chunk's current version to determine staleness.

## Solution

Add a `version: u64` field to the `Chunk` struct that increments on every modification.

### Version Field

```rust
pub struct Chunk {
    pub(crate) data: CowChunk,
    pub(crate) address: ChunkAddress,
    pub(crate) dirty: DirtyFlags,
    pub(crate) version: u64,
}

impl Chunk {
    /// Get the current version number.
    pub fn version(&self) -> u64 {
        self.version
    }
}
```

### Increment Semantics

- A newly created chunk starts at `version = 0`.
- Every call to `Chunk::set()` increments `version` by 1 (even if the set is a no-op due to same-type, although story 10 suppresses events for same-type — version increment is tied to actual data changes only).
- `Chunk::fill()` increments `version` by 1 (not by 32,768 — it is a single logical modification).
- Bulk operations increment `version` once per operation, not once per voxel. This keeps version numbers manageable and semantically meaningful ("how many modification operations have occurred").

### Cache Invalidation Pattern

Systems use the version for cache invalidation:

```rust
pub struct CachedMesh {
    /// The chunk version at the time this mesh was built.
    pub built_at_version: u64,
    /// The mesh data.
    pub mesh: Mesh,
}

impl CachedMesh {
    /// Check if this cached mesh is still valid for the given chunk.
    pub fn is_valid_for(&self, chunk: &Chunk) -> bool {
        self.built_at_version == chunk.version()
    }
}
```

This is more reliable than dirty flags for caching because:
- Dirty flags are binary (dirty/clean) and shared across systems.
- Version numbers are monotonic and can be compared independently by each cache.
- A system that was offline for several frames can still determine staleness by comparing versions, whereas dirty flags only tell you about the *current* state.

### Network Sync

For multiplayer, the server tracks each client's last-received chunk version:

```rust
pub struct ClientChunkState {
    /// Map of chunk address -> last version sent to this client.
    versions: HashMap<ChunkAddress, u64>,
}

impl ClientChunkState {
    /// Determine if this client needs an update for the given chunk.
    pub fn needs_update(&self, addr: &ChunkAddress, current_version: u64) -> bool {
        match self.versions.get(addr) {
            Some(&client_version) => client_version < current_version,
            None => true, // Client has never received this chunk
        }
    }
}
```

### Serialization

The version number is included in the serialized chunk format (appended after the voxel data):

```
... existing chunk format ...
+8 bytes    version (u64, little-endian)
```

This allows the save system to detect whether a loaded chunk has been modified since it was last saved, and allows the network system to send delta updates.

### Overflow Considerations

A `u64` version counter can increment once per frame at 240 FPS for 2.4 x 10^9 years before overflowing. Even with multiple modifications per frame, overflow is not a practical concern. No wrapping logic is needed.

## Outcome

A `version: u64` field on `Chunk` that monotonically increments on every modification. Systems use stored version numbers to determine cache staleness without relying on shared dirty flags. The version survives serialization for network sync and disk persistence.

## Demo Integration

**Demo crate:** `nebula-demo`

Chunks carry a version counter. Each modification increments it. The title shows `Chunk (0,0) v47` climbing as voxels are modified each frame.

## Crates & Dependencies

- No additional external dependencies beyond what `Chunk` already uses
- **`serde`** `1.0` with `derive` feature — Version field is included in chunk serialization

## Unit Tests

- **`test_new_chunk_version_is_zero`** — Create a new `Chunk` and assert `chunk.version() == 0`.
- **`test_each_set_increments_version`** — Create a chunk with version 0. Call `set()` three times (to different positions). Assert `chunk.version() == 3`. Call `set()` once more and assert `chunk.version() == 4`.
- **`test_version_survives_serialization_roundtrip`** — Create a chunk, modify it 5 times (version = 5), serialize it, deserialize it, and assert the deserialized chunk has `version() == 5`.
- **`test_two_chunks_same_data_different_versions`** — Create chunk A (version 0, all air) and chunk B (set one voxel then set it back to air, version 2). Both contain identical voxel data (all air), but `A.version() != B.version()`. Assert version comparison distinguishes them even though their voxel content is identical.
