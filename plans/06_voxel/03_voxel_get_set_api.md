# Voxel Get/Set API

## Problem

Every system in the engine that interacts with voxels — terrain generation, player block placement, meshing, lighting, physics — needs to read and write individual voxel cells within a chunk. Without a clean, well-defined API, each system would need to understand the internal palette-compressed storage format, perform its own bounds checking, and handle palette management manually. This would lead to duplicated logic, subtle indexing bugs, and tight coupling between game systems and the storage implementation. The API must be safe (no panics on bad input from gameplay code), efficient (no unnecessary allocations on the hot path), and ergonomic (simple coordinate-based access).

## Solution

Expose two primary methods on the `Chunk` struct (which wraps `ChunkData` along with metadata like position and dirty flags):

### API Surface

```rust
impl Chunk {
    /// Read the voxel type at the given local coordinates.
    /// Returns VoxelTypeId(0) (Air) if coordinates are out of bounds.
    pub fn get(&self, x: u8, y: u8, z: u8) -> VoxelTypeId;

    /// Write a voxel type at the given local coordinates.
    /// No-op with a warning log if coordinates are out of bounds.
    pub fn set(&mut self, x: u8, y: u8, z: u8, voxel: VoxelTypeId);
}
```

### Coordinate Convention

- Coordinates `x`, `y`, `z` are `u8` values in the range `[0, 32)` (0 through 31 inclusive).
- Using `u8` instead of `usize` makes the valid range explicit at the type level and prevents negative indices entirely.
- Values 32 and above are out of bounds.

### Bounds Handling

- **`get()` out of bounds**: Returns `VoxelTypeId(0)` (Air). This is the safe default because sampling outside a chunk should behave as if the space is empty, which is correct for meshing (no face generated) and physics (no collision). A `debug_assert!` fires in debug builds to catch accidental out-of-bounds access during development, and a `tracing::warn!` is emitted in release builds.
- **`set()` out of bounds**: Silently ignored with a `tracing::warn!` log. This prevents panics when gameplay code computes coordinates that slightly overshoot chunk boundaries (e.g., explosion radius crossing a chunk edge). The caller is expected to translate world coordinates to the correct chunk and local position, but defensive behavior here avoids crashes.

### Internal Mechanics

The `set()` method performs the following steps:

1. Compute the linear index: `idx = x as usize + (y as usize) * 32 + (z as usize) * 1024`.
2. Check if the new voxel type is already in the palette. If not, add it.
3. If adding to the palette exceeds the current bit width capacity, upgrade the storage (see palette compression story).
4. Write the palette index into the bit-packed array at the computed linear index.
5. Set dirty flags (`MESH_DIRTY | SAVE_DIRTY | NETWORK_DIRTY`).
6. Increment the chunk version counter.

The `get()` method:

1. Compute the linear index.
2. Read the palette index from the bit-packed array.
3. Map through the palette to return the `VoxelTypeId`.

### Bulk Operations

In addition to single-voxel access, provide a `fill()` method for setting an entire chunk to one type (common during terrain generation):

```rust
impl Chunk {
    /// Set every voxel in the chunk to the given type.
    pub fn fill(&mut self, voxel: VoxelTypeId);
}
```

This is optimized to reset the palette to a single entry and clear the bit-packed storage, avoiding 32,768 individual `set()` calls.

## Outcome

A `Chunk` struct with `get()`, `set()`, and `fill()` methods that provide safe, bounds-checked, coordinate-based voxel access. All palette management and bit-packing details are hidden behind this API. Out-of-bounds access is handled gracefully without panics.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo fills a chunk procedurally — stone below y=16, dirt at y=16, grass at y=17, air above — and logs the result. Voxel data is ready for meshing.

## Crates & Dependencies

- **`tracing`** `0.1` — Structured logging for out-of-bounds warnings
- No additional dependencies beyond what `ChunkData` (palette compression story) already requires

## Unit Tests

- **`test_get_empty_chunk_returns_air`** — Create a new chunk (all air) and assert `chunk.get(0, 0, 0) == VoxelTypeId(0)`, `chunk.get(15, 15, 15) == VoxelTypeId(0)`, and `chunk.get(31, 31, 31) == VoxelTypeId(0)`.
- **`test_set_then_get_roundtrip`** — Set position `(5, 10, 20)` to `VoxelTypeId(7)`, then assert `chunk.get(5, 10, 20) == VoxelTypeId(7)`. Verify surrounding voxels remain Air.
- **`test_set_out_of_bounds_no_panic`** — Call `chunk.set(32, 0, 0, VoxelTypeId(1))`, `chunk.set(0, 255, 0, VoxelTypeId(1))`, and `chunk.set(0, 0, 40, VoxelTypeId(1))`. Assert no panic occurs and the chunk data is unchanged (still all air).
- **`test_set_same_voxel_twice`** — Set position `(3, 3, 3)` to type A, then set it again to type B. Assert `chunk.get(3, 3, 3) == B`. Set it again to type A and assert it reads back correctly.
- **`test_fill_entire_chunk`** — Call `chunk.fill(VoxelTypeId(5))` and verify all 32x32x32 positions return `VoxelTypeId(5)`. Then call `chunk.fill(VoxelTypeId(0))` and verify all positions return Air. Assert the palette has exactly 1 entry after each fill.
