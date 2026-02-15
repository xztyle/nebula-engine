# RLE Compression

## Problem

Palette compression reduces per-voxel storage from 16 bits to 2-8 bits, but the bit-packed index array still stores one index per voxel (32,768 entries). In practice, terrain chunks have large contiguous regions of the same material — solid stone below the surface, air above, continuous dirt layers. These runs of identical values compress extremely well with Run-Length Encoding (RLE). Without RLE, a chunk that is half air and half stone still stores 32,768 palette indices even though the data could be described as "16,384 x air, 16,384 x stone" — just two (count, value) pairs. RLE is critical for reducing serialized chunk size for disk storage and especially for network transfer, where bandwidth is the primary bottleneck.

## Solution

Implement RLE compression as a serialization-time transformation in `nebula-voxel`. RLE is applied to the linearized palette index array after palette compression but before writing to the binary format. At runtime, chunks use the random-access palette-compressed format (story 02); RLE is only used for serialization and deserialization.

### Encoding Format

RLE operates on the palette index values (not raw `VoxelTypeId`). Each run is encoded as:

```rust
struct RleRun {
    /// Number of consecutive identical values (1..=65535).
    count: u16,
    /// The palette index value.
    value: u16,
}
```

The value field width matches the chunk's current bit width (2, 4, 8, or 16 bits), but for simplicity in the serialization format, values are stored as `u16` with the upper bits zeroed. The count is always `u16`, allowing runs up to 65,535 (more than the 32,768 total voxels — a single run can cover the entire chunk).

### Serialized Layout

The RLE data appears in place of the raw bit-packed index data in the chunk binary format (story 05). A flag in the format header indicates whether RLE is used:

```
Offset          Size    Field
------          ----    -----
0               4       Magic bytes ("NVCK")
4               1       Format version (2 when RLE is present)
5               1       Compression flags (bit 0: RLE enabled)
6               2       Palette length (u16)
8               N*2     Palette entries
8+N*2           4       RLE run count (u32)
12+N*2          R*4     RLE runs (R x (count: u16, value: u16))
```

### Compression Algorithm

```rust
pub fn rle_encode(indices: &[u16]) -> Vec<RleRun> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < indices.len() {
        let value = indices[i];
        let mut count: u16 = 1;
        while i + count as usize < indices.len()
            && indices[i + count as usize] == value
            && count < u16::MAX
        {
            count += 1;
        }
        runs.push(RleRun { count, value });
        i += count as usize;
    }
    runs
}

pub fn rle_decode(runs: &[RleRun], expected_len: usize) -> Result<Vec<u16>, RleError> {
    let mut result = Vec::with_capacity(expected_len);
    for run in runs {
        for _ in 0..run.count {
            result.push(run.value);
        }
    }
    if result.len() != expected_len {
        return Err(RleError::LengthMismatch {
            expected: expected_len,
            actual: result.len(),
        });
    }
    Ok(result)
}
```

### Compression Effectiveness

| Chunk Type | Raw Index Size | RLE Runs | RLE Size | Ratio |
|---|---|---|---|---|
| Uniform (all air) | 0 bytes (special) | 1 run | 4 bytes | N/A |
| Half air / half stone | 8,192 bytes (2-bit) | 2 runs | 8 bytes | 1024:1 |
| Layered terrain (5 layers) | 1,024 bytes (2-bit) | ~5 runs | 20 bytes | 51:1 |
| Noisy surface | 1,024 bytes (2-bit) | ~500 runs | 2,000 bytes | 0.5:1 (worse) |
| Checkerboard (worst case) | 8,192 bytes (2-bit) | 32,768 runs | 131,072 bytes | 0.06:1 (much worse) |

RLE can increase size for highly varied data. The serializer should compare RLE size against raw size and use whichever is smaller, indicated by the compression flag.

### Adaptive Selection

```rust
pub fn serialize_indices(indices: &[u16], bit_width: u8) -> (bool, Vec<u8>) {
    let rle_runs = rle_encode(indices);
    let rle_size = rle_runs.len() * 4; // 4 bytes per run
    let raw_size = (indices.len() * bit_width as usize + 7) / 8;

    if rle_size < raw_size {
        (true, encode_rle_to_bytes(&rle_runs))
    } else {
        (false, encode_raw_to_bytes(indices, bit_width))
    }
}
```

## Outcome

An `rle_encode()` and `rle_decode()` function pair in `nebula-voxel` that compresses palette index arrays for serialization. Typical terrain chunks achieve 4:1 or better compression. The serializer adaptively chooses RLE or raw format based on which is smaller. The chunk binary format is extended with a compression flag.

## Demo Integration

**Demo crate:** `nebula-demo`

The console logs compression ratios for each chunk: `Chunk (0,0): RLE 256B, palette 192B, raw 32KB`. Highly repetitive chunks compress dramatically.

## Crates & Dependencies

- **`thiserror`** `2.0` — Error type for `RleError`
- No additional external dependencies; RLE is implemented in pure Rust

## Unit Tests

- **`test_uniform_chunk_single_run`** — Create an array of 32,768 identical values, RLE-encode it, and assert the result is exactly 1 run with `count == 32768`.
- **`test_alternating_voxels_no_compression`** — Create an array alternating between two values (`[0, 1, 0, 1, ...]`), RLE-encode it, and assert the result has 32,768 runs (each of length 1). Verify the adaptive serializer chooses raw format over RLE.
- **`test_rle_roundtrip`** — Generate a realistic terrain-like index array (runs of varying lengths), encode with RLE, decode, and assert the decoded array matches the original exactly.
- **`test_compression_ratio_terrain`** — Create a terrain-like chunk (bottom half stone, top half air, thin grass/dirt layers), RLE-encode the indices, and assert the compression ratio (raw_size / rle_size) is greater than 4.0.
- **`test_empty_chunk_minimal_size`** — RLE-encode a uniform array and assert the encoded size is exactly 4 bytes (one run: 2 bytes count + 2 bytes value).
