# Sector Coordinates

## Problem

Operating directly on 128-bit absolute coordinates for every spatial operation is both expensive and unnecessary. Most gameplay, networking, and chunk management decisions are local: a player only interacts with entities and terrain within a few kilometers. Sending raw `i128` triples over the network for every entity update wastes bandwidth and leaks the full universe position to clients who only need local awareness. Chunk loading decisions require knowing which region of space the player occupies, but comparing 128-bit values for spatial partitioning is cumbersome. The engine needs a way to decompose the 128-bit universe into manageable, uniformly-sized regions -- sectors -- so that subsystems can reason about spatial locality without touching full-precision coordinates at every step.

## Solution

### Sector Geometry

Divide the universe into a regular grid of cubic sectors. Each sector is **2^32 mm per side** (exactly 4,294,967,296 mm = ~4,294.967 km). This size is chosen because it aligns with the 32-bit local offset: the lower 32 bits of an `i128` coordinate give the position within the sector, and the upper bits give the sector index. No division or modulo is needed -- only bit shifts and masks.

### Data Structures

```rust
/// The index of a sector in the universe grid.
/// Each component is the upper 96 bits of the corresponding i128 axis.
/// Stored as three i128 values with the lower 32 bits always zero
/// (or equivalently, as a custom i96 triple).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SectorIndex {
    pub x: i128,
    pub y: i128,
    pub z: i128,
}

/// The local offset within a sector, in millimeters.
/// Range: [0, 2^32 - 1] for each axis when the full i128 coordinate is non-negative.
/// For negative coordinates, the offset is adjusted so it is always non-negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectorOffset {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// A position decomposed into sector index + local offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectorCoord {
    pub sector: SectorIndex,
    pub offset: SectorOffset,
}
```

### Conversion: WorldPosition to SectorCoord

The sector index is obtained by arithmetic right-shifting each axis by 32 bits. The local offset is the lower 32 bits, masked off. For negative coordinates, Rust's arithmetic right shift preserves the sign, and the bitwise AND with `0xFFFF_FFFF` yields a non-negative remainder in the range `[0, 2^32)`:

```rust
const SECTOR_BITS: u32 = 32;
const SECTOR_MASK: i128 = 0xFFFF_FFFF;

impl SectorCoord {
    /// Decompose an absolute world position into sector index + local offset.
    pub fn from_world(pos: &WorldPosition) -> Self {
        SectorCoord {
            sector: SectorIndex {
                // Arithmetic right shift: for -1 >> 32 this gives -1,
                // meaning the sector at index -1 (one sector in the negative direction).
                x: pos.x >> SECTOR_BITS,
                y: pos.y >> SECTOR_BITS,
                z: pos.z >> SECTOR_BITS,
            },
            offset: SectorOffset {
                // Mask off the lower 32 bits. For negative coordinates,
                // this produces a positive offset within the sector.
                // E.g., i128 value -1: sector = -1, offset = 0xFFFFFFFF = 4294967295.
                x: (pos.x & SECTOR_MASK) as i32,
                y: (pos.y & SECTOR_MASK) as i32,
                z: (pos.z & SECTOR_MASK) as i32,
            },
        }
    }

    /// Reconstruct the absolute world position from sector + offset.
    pub fn to_world(&self) -> WorldPosition {
        WorldPosition {
            x: (self.sector.x << SECTOR_BITS) | (self.offset.x as i128 & SECTOR_MASK),
            y: (self.sector.y << SECTOR_BITS) | (self.offset.y as i128 & SECTOR_MASK),
            z: (self.sector.z << SECTOR_BITS) | (self.offset.z as i128 & SECTOR_MASK),
        }
    }
}
```

### Handling Negative Coordinates

For negative world coordinates, the arithmetic right shift floors toward negative infinity (Rust's behavior for signed integers), and the mask always produces a non-negative offset. Example:

- World position: `x = -1`
- `x >> 32 = -1` (sector index -1)
- `x & 0xFFFF_FFFF = 0xFFFF_FFFF = 4294967295` (offset within sector -1)
- Reconstruction: `(-1 << 32) | 4294967295 = -4294967296 + 4294967295 = -1` (correct)

This means sector (-1, 0, 0) contains world x-coordinates from -4,294,967,296 to -1.

### SectorKey for Hashing

A `SectorKey` newtype wraps `SectorIndex` and derives `Hash` + `Eq` so it can be used as a key in `HashMap` and `HashSet`:

```rust
/// Lightweight key type for sector-based hash maps.
/// The hash combines all three axis indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SectorKey(pub SectorIndex);

impl From<&SectorCoord> for SectorKey {
    fn from(coord: &SectorCoord) -> Self {
        SectorKey(coord.sector)
    }
}
```

### Use Cases

- **Networking**: The server only replicates entities in sectors within the client's interest radius. Sector indices are compact to compare and transmit.
- **Chunk management**: The chunk loader maintains a set of loaded sectors around the player. When the player crosses a sector boundary, new sectors are enqueued for loading and distant ones are unloaded.
- **Spatial hashing**: Entity lookup tables are keyed by `SectorKey`, providing O(1) access to all entities within a sector.
- **Persistence**: Each sector can be serialized independently to a file named by its index, enabling streaming save/load.

## Outcome

The `nebula-coords` crate exports `SectorIndex`, `SectorOffset`, `SectorCoord`, and `SectorKey`. Calling `SectorCoord::from_world(&world_pos)` decomposes any `WorldPosition` into sector + offset with pure bit operations (no division, no branching). Calling `.to_world()` reconstructs the original position exactly. The types are `Copy`, `Hash`, and `Eq`, ready for use as hash map keys and network serialization. Running `cargo test -p nebula-coords` passes all sector coordinate tests.

## Demo Integration

**Demo crate:** `nebula-demo`

When the moving position crosses a sector boundary, the console logs the transition: `Entered sector (5, 0, 7)`. Sector addresses roll over like an odometer.

## Crates & Dependencies

- **`nebula-math`** (workspace) — Provides `IVec3_128` / `WorldPosition` type
- No external dependencies; sector decomposition uses only bitwise operations on primitive `i128` values

## Unit Tests

- **`test_origin_maps_to_sector_zero`** — Create `WorldPosition { x: 0, y: 0, z: 0 }`. Convert to `SectorCoord`. Assert sector index is `(0, 0, 0)` and offset is `(0, 0, 0)`.

- **`test_position_at_sector_boundary`** — Create `WorldPosition { x: (1_i128 << 32), y: 0, z: 0 }` (exactly at the start of sector 1). Convert to `SectorCoord`. Assert sector index x is `1` and offset x is `0`. Then test `x = (1_i128 << 32) - 1` and assert sector index x is `0` and offset x is `4294967295` (last position in sector 0).

- **`test_roundtrip_world_to_sector_to_world`** — For a set of test positions including `(0, 0, 0)`, `(1_000_000, -5_000_000_000, 42)`, `(i128::MAX, i128::MIN, 0)`, and several random values: convert to `SectorCoord` and back to `WorldPosition`. Assert the reconstructed position equals the original exactly.

- **`test_negative_coordinates`** — Create `WorldPosition { x: -1, y: -4294967296, z: -4294967297 }`. Convert to `SectorCoord`. Assert:
  - `x`: sector = -1, offset = 4294967295
  - `y`: sector = -1, offset = 0
  - `z`: sector = -2, offset = 4294967295

- **`test_sector_index_for_large_coordinates`** — Create `WorldPosition { x: 100_i128 << 32, y: -(50_i128 << 32), z: 1_i128 << 96 }`. Assert sector indices are `100`, `-50`, and `1 << 64` respectively, with all offsets `0`.

- **`test_sector_key_hash_equality`** — Create two `SectorCoord` values with the same sector index but different offsets. Convert both to `SectorKey`. Assert the keys are equal and produce the same hash. Then create a `SectorCoord` with a different sector index and assert its `SectorKey` is not equal to the first.

- **`test_sector_key_usable_as_hashmap_key`** — Insert three entries into a `HashMap<SectorKey, String>` with distinct sector keys. Retrieve each by key and assert the correct value is returned. Query a key not in the map and assert `None`.
