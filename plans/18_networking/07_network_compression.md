# Network Compression

## Problem

The Nebula Engine streams voxel chunk data from the server to clients. A single chunk on a cubesphere-voxel planet can contain thousands of voxel entries, and the serialized `ChunkData` message can easily reach tens or hundreds of kilobytes. Sending this data uncompressed wastes bandwidth and increases latency, especially for players on slower connections. Terrain data compresses extremely well because voxels are spatially correlated — large runs of the same material (air, stone, dirt) compress to nearly nothing. However, small messages like `Ping`, `PlayerPosition`, and `PlayerAction` should not pay the overhead of compression (both CPU time and the fixed metadata cost). The solution is conditional compression: only compress messages above a configurable size threshold, and include a flag in the frame header so the receiver knows whether to decompress.

## Solution

### Compression header

The length-prefixed framing layer (story 03) delivers raw payload bytes. This story adds a 1-byte compression header as the first byte of the payload, before the protocol version byte (story 04):

```
+-------------------+-------------------+-------------------+-------------------+
| frame length (4B) | compression (1B)  | version (1B)      | postcard body     |
| u32 LE            | 0x00 or 0x01      | protocol version  | message bytes     |
+-------------------+-------------------+-------------------+-------------------+
```

- `0x00`: payload is uncompressed. The remaining bytes are the versioned message.
- `0x01`: payload is LZ4-compressed. The remaining bytes must be LZ4-decompressed to yield the versioned message.

### Compression threshold

Messages smaller than the threshold are sent uncompressed. The default threshold is 256 bytes (measured after serialization, before compression). This is configurable via `CompressionConfig`.

```rust
pub struct CompressionConfig {
    /// Minimum serialized size (bytes) before compression is applied. Default: 256.
    pub threshold: usize,
    /// Whether compression is enabled at all. Default: true.
    pub enabled: bool,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            threshold: 256,
            enabled: true,
        }
    }
}
```

### Compression and decompression

LZ4 is chosen for its speed — it is the fastest general-purpose compressor, decompressing at multiple GB/s, which is critical for a game engine that must process many chunks per second. The `lz4_flex` crate is a pure-Rust LZ4 implementation with no C dependencies, ensuring easy cross-platform compilation on Linux, Windows, and macOS.

```rust
use lz4_flex::{compress_prepend_size, decompress_size_prepended};

const COMPRESSION_FLAG_NONE: u8 = 0x00;
const COMPRESSION_FLAG_LZ4: u8 = 0x01;

/// Wrap a serialized message payload with optional compression.
///
/// Input: the versioned message bytes (version byte + postcard body).
/// Output: compression flag byte + (possibly compressed) data, ready for framing.
pub fn compress_payload(data: &[u8], config: &CompressionConfig) -> Vec<u8> {
    if !config.enabled || data.len() < config.threshold {
        // No compression — prepend the "uncompressed" flag.
        let mut out = Vec::with_capacity(1 + data.len());
        out.push(COMPRESSION_FLAG_NONE);
        out.extend_from_slice(data);
        out
    } else {
        // Compress with LZ4.
        let compressed = compress_prepend_size(data);
        let mut out = Vec::with_capacity(1 + compressed.len());
        out.push(COMPRESSION_FLAG_LZ4);
        out.extend_from_slice(&compressed);
        out
    }
}

/// Unwrap a received payload, decompressing if necessary.
///
/// Input: compression flag byte + (possibly compressed) data.
/// Output: the versioned message bytes.
pub fn decompress_payload(data: &[u8]) -> Result<Vec<u8>, CompressionError> {
    if data.is_empty() {
        return Err(CompressionError::EmptyPayload);
    }

    match data[0] {
        COMPRESSION_FLAG_NONE => Ok(data[1..].to_vec()),
        COMPRESSION_FLAG_LZ4 => {
            let decompressed = decompress_size_prepended(&data[1..])
                .map_err(|e| CompressionError::DecompressFailed(e.to_string()))?;
            Ok(decompressed)
        }
        flag => Err(CompressionError::UnknownFlag(flag)),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CompressionError {
    #[error("empty payload — no compression flag")]
    EmptyPayload,
    #[error("LZ4 decompression failed: {0}")]
    DecompressFailed(String),
    #[error("unknown compression flag: 0x{0:02X}")]
    UnknownFlag(u8),
}
```

### Expected compression ratios

Typical terrain chunk data compresses very well with LZ4:

| Content | Uncompressed | Compressed | Ratio |
|---------|-------------|------------|-------|
| Uniform air chunk | ~32 KB | ~200 B | 99% |
| Mixed terrain (stone/dirt/air) | ~32 KB | ~6 KB | 80% |
| Dense varied chunk | ~32 KB | ~16 KB | 50% |

### Integration

The send path becomes: serialize (story 04) -> compress (this story) -> frame (story 03) -> TCP write.
The receive path becomes: TCP read -> deframe (story 03) -> decompress (this story) -> deserialize (story 04).

## Outcome

A `compression.rs` module in `crates/nebula_net/src/` exporting `compress_payload`, `decompress_payload`, `CompressionConfig`, `CompressionError`, and the compression flag constants. Large messages (especially chunk data) are LZ4-compressed before transmission, reducing bandwidth by 50-99% for typical terrain. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

Large messages (chunk data) are compressed with LZ4 before transmission. The console logs compression ratios: `Compressed 32KB -> 4KB (87%)`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `lz4_flex` | `0.11` | Pure-Rust LZ4 compression and decompression |
| `thiserror` | `2.0` | Derive `Error` for `CompressionError` |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> CompressionConfig {
        CompressionConfig::default()
    }

    #[test]
    fn test_small_message_is_not_compressed() {
        let config = default_config();
        let data = b"tiny"; // Well below 256-byte threshold
        let payload = compress_payload(data, &config);

        assert_eq!(payload[0], COMPRESSION_FLAG_NONE);
        assert_eq!(&payload[1..], data);
    }

    #[test]
    fn test_large_message_is_compressed() {
        let config = default_config();
        // Create data above the threshold
        let data = vec![42u8; 1024];
        let payload = compress_payload(&data, &config);

        assert_eq!(payload[0], COMPRESSION_FLAG_LZ4);
        // Compressed output should be smaller than the original for repetitive data
        assert!(
            payload.len() < data.len(),
            "Compressed size {} should be less than original {}",
            payload.len(),
            data.len()
        );
    }

    #[test]
    fn test_compressed_message_decompresses_correctly() {
        let config = default_config();
        let data = vec![7u8; 2048]; // Repetitive, above threshold

        let payload = compress_payload(&data, &config);
        let decompressed = decompress_payload(&payload).unwrap();

        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_uncompressed_message_decompresses_correctly() {
        let config = default_config();
        let data = b"short message";

        let payload = compress_payload(data, &config);
        let decompressed = decompress_payload(&payload).unwrap();

        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_compression_reduces_size_for_chunk_data() {
        let config = default_config();
        // Simulate a chunk with runs of the same voxel type (highly compressible)
        let mut chunk = Vec::with_capacity(32_768);
        for _ in 0..16_384 {
            chunk.push(0x00); // Air
        }
        for _ in 0..8_192 {
            chunk.push(0x01); // Stone
        }
        for _ in 0..8_192 {
            chunk.push(0x02); // Dirt
        }

        let payload = compress_payload(&chunk, &config);
        let compressed_size = payload.len() - 1; // Subtract flag byte
        let ratio = 1.0 - (compressed_size as f64 / chunk.len() as f64);

        assert!(
            ratio > 0.5,
            "Expected at least 50% compression, got {:.1}%",
            ratio * 100.0
        );
    }

    #[test]
    fn test_compression_flag_is_set_correctly() {
        let config = CompressionConfig {
            threshold: 10,
            enabled: true,
        };

        let small = b"tiny";
        let large = b"this is a longer message that exceeds the threshold";

        let small_payload = compress_payload(small, &config);
        assert_eq!(small_payload[0], COMPRESSION_FLAG_NONE);

        let large_payload = compress_payload(large, &config);
        assert_eq!(large_payload[0], COMPRESSION_FLAG_LZ4);
    }

    #[test]
    fn test_compression_disabled() {
        let config = CompressionConfig {
            threshold: 256,
            enabled: false,
        };
        let data = vec![0u8; 1024]; // Above threshold but compression disabled
        let payload = compress_payload(&data, &config);

        assert_eq!(payload[0], COMPRESSION_FLAG_NONE);
        assert_eq!(&payload[1..], &data[..]);
    }

    #[test]
    fn test_empty_payload_error() {
        let result = decompress_payload(&[]);
        assert!(matches!(result, Err(CompressionError::EmptyPayload)));
    }

    #[test]
    fn test_unknown_flag_error() {
        let result = decompress_payload(&[0xFF, 0x01, 0x02]);
        assert!(matches!(result, Err(CompressionError::UnknownFlag(0xFF))));
    }

    #[test]
    fn test_roundtrip_preserves_data_integrity() {
        let config = default_config();
        // Test with various sizes around the threshold
        for size in [0, 1, 100, 255, 256, 257, 1000, 10_000] {
            let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
            let payload = compress_payload(&data, &config);
            let recovered = decompress_payload(&payload).unwrap();
            assert_eq!(recovered, data, "Roundtrip failed for size {size}");
        }
    }
}
```
