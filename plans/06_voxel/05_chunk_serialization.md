# Chunk Serialization

## Problem

Chunks must be saved to disk when unloaded and restored when the player returns. They must also be transmitted over the network for multiplayer replication. Both use cases demand a compact, versioned binary format that can be serialized and deserialized quickly. The format must handle palette-compressed data natively (serializing the palette and bit-packed indices, not expanding to raw u16 arrays) and include enough metadata for forward compatibility (version byte) and integrity checking (magic bytes). A typical surface chunk with 4-8 voxel types should serialize to under 4 KB to keep disk I/O and network bandwidth manageable.

## Solution

Define a binary chunk format in `nebula-voxel` and implement `serialize()` and `deserialize()` methods on `ChunkData`.

### Binary Format

```
Offset  Size    Field
------  ----    -----
0       4       Magic bytes: [0x4E, 0x56, 0x43, 0x4B] ("NVCK" — Nebula Voxel Chunk)
4       1       Format version (u8, currently 1)
5       2       Palette length (u16, little-endian)
7       N*2     Palette entries (N x u16 VoxelTypeId, little-endian)
7+N*2   1       Bit width (u8: 0, 2, 4, 8, or 16)
8+N*2   M       Bit-packed voxel index data (length M depends on bit width)
```

Where:
- `N` = palette length
- `M` = `ceil(32768 * bit_width / 8)` bytes (0 bytes when bit_width is 0)

### Size Estimates

| Scenario | Palette | Bit Width | Index Data | Total |
|---|---|---|---|---|
| All air (uniform) | 1 entry (2 bytes) | 0 | 0 bytes | 8 bytes |
| Surface chunk (4 types) | 4 entries (8 bytes) | 2 | 1,024 bytes | ~1,040 bytes |
| Varied chunk (10 types) | 10 entries (20 bytes) | 4 | 16,384 bytes | ~16,412 bytes |
| Dense chunk (200 types) | 200 entries (400 bytes) | 8 | 32,768 bytes | ~33,176 bytes |

### Implementation

```rust
impl ChunkData {
    /// Serialize to a byte vector in the NVCK binary format.
    pub fn serialize(&self) -> Vec<u8>;

    /// Deserialize from a byte slice. Returns an error if the data is
    /// corrupted, has an unrecognized version, or is truncated.
    pub fn deserialize(data: &[u8]) -> Result<Self, ChunkSerError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ChunkSerError {
    #[error("invalid magic bytes")]
    InvalidMagic,
    #[error("unsupported format version: {0}")]
    UnsupportedVersion(u8),
    #[error("data truncated: expected {expected} bytes, got {actual}")]
    Truncated { expected: usize, actual: usize },
    #[error("invalid bit width: {0}")]
    InvalidBitWidth(u8),
    #[error("palette entry out of range")]
    InvalidPaletteEntry,
}
```

### Design Decisions

- **Custom format over `postcard`**: While `postcard` is convenient, a hand-rolled format gives precise control over byte layout, makes the format language-agnostic (important for potential future tooling in other languages), and avoids `postcard`'s overhead for small structs. However, `postcard` can still be used as an alternative serializer behind a feature flag.
- **Little-endian**: All multi-byte integers are stored in little-endian byte order, matching the dominant platform (x86/ARM) and avoiding byte-swap overhead in the common case.
- **No built-in compression**: The raw binary format does not apply general-purpose compression (zstd, lz4). This is handled at a higher layer — the disk storage system or network transport can wrap chunks in compression as needed. The palette compression and optional RLE (story 08) already provide significant size reduction.
- **Version byte**: Allows future format evolution. Deserializers check the version and can either handle multiple versions or reject unknown ones with a clear error.

### Integration

Serialization is used by:
- **Disk storage**: The chunk saving system serializes dirty chunks and writes them to region files.
- **Network**: The multiplayer system serializes chunks for transmission to clients.
- **Undo/redo**: The editor can snapshot chunk state by serializing before modifications.

## Outcome

A `serialize()` / `deserialize()` API on `ChunkData` that produces a compact, versioned binary format. Typical surface chunks serialize to approximately 1-2 KB. The format is self-describing (magic bytes, version) and validates input during deserialization.

## Demo Integration

**Demo crate:** `nebula-demo`

Chunks are serialized to postcard bytes and deserialized back. The console logs `Serialized 25 chunks: 6.4KB total, 256B avg`. Round-trip integrity is validated.

## Crates & Dependencies

- **`thiserror`** `2.0` — Error type derivation for `ChunkSerError`
- **`byteorder`** `1.5` — Explicit little-endian reading/writing of multi-byte integers (or use `u16::to_le_bytes()` / `u16::from_le_bytes()` from std)

## Unit Tests

- **`test_serialize_deserialize_roundtrip`** — Create a chunk with a mix of voxel types, serialize it, deserialize the bytes, and assert every voxel matches the original. Test with 1, 4, 20, and 300 palette entries to exercise all bit width tiers.
- **`test_empty_chunk_serializes_small`** — Serialize an all-air chunk and assert the output size is less than 16 bytes (magic + version + palette(1) + bit_width + no index data = 8 bytes).
- **`test_full_chunk_serializes_correctly`** — Fill a chunk with 32,768 distinct patterns (e.g., `VoxelTypeId(x + y * 32)` clamped to available types), serialize, deserialize, and verify the roundtrip.
- **`test_version_byte_present`** — Serialize any chunk and assert `output[4] == 1` (current format version).
- **`test_corrupted_data_returns_error`** — Attempt to deserialize `&[0xFF, 0xFF]` (invalid magic), `&[0x4E, 0x56, 0x43, 0x4B, 99]` (unsupported version), and a truncated valid header. Assert each returns the appropriate `ChunkSerError` variant.
