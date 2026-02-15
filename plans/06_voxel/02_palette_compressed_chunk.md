# Palette-Compressed Chunk

## Problem

A naive voxel chunk of 32x32x32 stores one `u16` per voxel, consuming 32,768 x 2 = 65,536 bytes (64 KB) per chunk. With thousands of chunks loaded around the player, this adds up to hundreds of megabytes of raw voxel data. However, most chunks in practice use only a handful of distinct voxel types — a surface chunk might contain just air, grass, dirt, and stone (4 types). Storing a full 16-bit ID per voxel wastes enormous amounts of memory when 2 bits would suffice. The engine needs a compression scheme that operates in-place (not just at serialization time) to keep runtime memory footprint manageable, while still allowing fast random-access reads and writes.

## Solution

Implement palette-compressed chunk storage in the `nebula-voxel` crate. Instead of storing raw `VoxelTypeId` values, each chunk maintains a **palette** (a small array of the distinct `VoxelTypeId` values actually present in the chunk) and a **bit-packed index array** where each voxel cell stores an index into the palette using the minimum number of bits required.

### Bit Width Tiers

| Palette Size | Bits per Index | Storage for 32,768 Voxels |
|---|---|---|
| 1 (uniform) | 0 (special case) | 0 bytes (just the palette) |
| 2-4 | 2 | 8,192 bits = 1,024 bytes |
| 5-16 | 4 | 16,384 bytes = 2,048 bytes (truncated: actually 131,072 bits = 16,384 bytes -- correction: 32,768 x 4 = 131,072 bits = 16,384 bytes) |
| 17-256 | 8 | 32,768 bytes |
| 257-65536 | 16 (raw) | 65,536 bytes |

Correction on the table (cleaned up):

| Palette Size | Bits per Index | Storage for 32,768 Voxels |
|---|---|---|
| 1 (uniform) | 0 (special case) | 0 bytes |
| 2 - 4 | 2 | 8,192 bits = 1,024 bytes (~1 KB) |
| 5 - 16 | 4 | 131,072 bits = 16,384 bytes (~16 KB) |
| 17 - 256 | 8 | 32,768 bytes (~32 KB) |
| 257+ | 16 (uncompressed) | 65,536 bytes (~64 KB) |

*Note: The 4-bit tier is 32,768 x 4 / 8 = 16,384 bytes, not 4 KB. This is still a 4x improvement over the 16-bit raw case. For the corrected 4-bit storage: 32768 * 4 = 131072 bits = 16384 bytes = 16 KB.*

### Data Structures

```rust
pub struct ChunkData {
    /// Palette mapping local indices to global VoxelTypeId values.
    palette: Vec<VoxelTypeId>,
    /// Bit-packed voxel indices. Length depends on bit width.
    storage: BitPackedArray,
    /// Current bits per index (0, 2, 4, 8, or 16).
    bit_width: u8,
}

pub struct BitPackedArray {
    /// Raw storage. u64 elements to allow efficient bit manipulation.
    data: Vec<u64>,
    /// Bits per element.
    bits: u8,
    /// Total number of elements.
    len: usize,
}
```

### Palette Management

- **Upgrade**: When a `set()` call introduces a new voxel type that would exceed the current bit width's capacity (e.g., a 5th type when at 2-bit width), the storage is upgraded to the next tier. All existing indices are repacked into the wider format.
- **Downgrade**: After a `set()` that overwrites the last instance of a palette entry, a compaction pass can remove unused palette entries and potentially downgrade to a narrower bit width. Compaction is deferred — it runs only when explicitly requested or when the chunk is about to be serialized, to avoid per-voxel overhead.

### Indexing

Voxel position `(x, y, z)` maps to a linear index via `x + y * 32 + z * 32 * 32` (x varies fastest, then y, then z). This is a Y-up coordinate system consistent with the rest of the engine.

### Uniform Chunk Optimization

When the palette has exactly one entry, the bit-packed array is empty (zero bytes). Every `get()` returns the single palette entry. This is the common case for all-air chunks deep underground or in empty space, and costs only the palette overhead (~4 bytes).

## Outcome

A `ChunkData` struct in `nebula-voxel` that stores 32x32x32 voxels with automatic palette compression. An all-air chunk consumes approximately 4 bytes of voxel storage. A typical surface chunk with 4-8 types uses 1-2 KB instead of 64 KB. The struct supports O(1) random-access `get()` and amortized O(1) `set()` with occasional palette resizing.

## Demo Integration

**Demo crate:** `nebula-demo`

A chunk is filled with a pattern and the console logs its memory usage: `Chunk: 256 bytes (palette: 2 entries)` versus the uncompressed `32KB`.

## Crates & Dependencies

- **`bitvec`** `1.0` — Bit-level addressing and manipulation for the packed index array (alternatively, hand-roll bit packing for tighter control and fewer dependencies)
- **`serde`** `1.0` with `derive` feature — Serialization support for chunk data

## Unit Tests

- **`test_empty_chunk_single_palette_entry`** — Create a new `ChunkData` and assert `palette.len() == 1` and `palette[0] == VoxelTypeId(0)` (Air). Assert `bit_width == 0`.
- **`test_single_type_change_grows_palette`** — Create a new chunk, set one voxel to a non-air type, assert `palette.len() == 2` and `bit_width == 2`.
- **`test_palette_compresses_back`** — Set a voxel to type A, then set it back to Air, run compaction, and assert the palette shrinks back to 1 entry and `bit_width` returns to 0.
- **`test_all_voxels_accessible`** — Iterate all 32x32x32 = 32,768 positions, set each to a known type, then read each back and assert correctness. This validates the linear index calculation and bit-packing across all positions.
- **`test_bit_width_upgrades_at_thresholds`** — Progressively add distinct types: assert `bit_width == 0` at 1 type, `bit_width == 2` at 2-4 types, `bit_width == 4` at 5-16 types, `bit_width == 8` at 17-256 types, and `bit_width == 16` at 257+ types.
