//! Conditional LZ4 compression for network payloads.
//!
//! Large messages (e.g. chunk data) are compressed with LZ4 before transmission,
//! while small messages skip compression to avoid overhead.

use lz4_flex::{compress_prepend_size, decompress_size_prepended};

/// Compression flag: payload is uncompressed.
pub const COMPRESSION_FLAG_NONE: u8 = 0x00;

/// Compression flag: payload is LZ4-compressed.
pub const COMPRESSION_FLAG_LZ4: u8 = 0x01;

/// Controls when payloads are compressed.
#[derive(Debug, Clone)]
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

/// Wrap a serialized message payload with optional compression.
///
/// Input: the versioned message bytes (version byte + postcard body).
/// Output: compression flag byte + (possibly compressed) data, ready for framing.
pub fn compress_payload(data: &[u8], config: &CompressionConfig) -> Vec<u8> {
    if !config.enabled || data.len() < config.threshold {
        let mut out = Vec::with_capacity(1 + data.len());
        out.push(COMPRESSION_FLAG_NONE);
        out.extend_from_slice(data);
        out
    } else {
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

/// Errors that can occur during payload decompression.
#[derive(Debug, thiserror::Error)]
pub enum CompressionError {
    /// The payload was empty — no compression flag present.
    #[error("empty payload — no compression flag")]
    EmptyPayload,
    /// LZ4 decompression failed.
    #[error("LZ4 decompression failed: {0}")]
    DecompressFailed(String),
    /// An unknown compression flag byte was encountered.
    #[error("unknown compression flag: 0x{0:02X}")]
    UnknownFlag(u8),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> CompressionConfig {
        CompressionConfig::default()
    }

    #[test]
    fn test_small_message_is_not_compressed() {
        let config = default_config();
        let data = b"tiny";
        let payload = compress_payload(data, &config);

        assert_eq!(payload[0], COMPRESSION_FLAG_NONE);
        assert_eq!(&payload[1..], data);
    }

    #[test]
    fn test_large_message_is_compressed() {
        let config = default_config();
        let data = vec![42u8; 1024];
        let payload = compress_payload(&data, &config);

        assert_eq!(payload[0], COMPRESSION_FLAG_LZ4);
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
        let data = vec![7u8; 2048];

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
        let mut chunk = Vec::with_capacity(32_768);
        chunk.extend(std::iter::repeat_n(0x00u8, 16_384));
        chunk.extend(std::iter::repeat_n(0x01u8, 8_192));
        chunk.extend(std::iter::repeat_n(0x02u8, 8_192));

        let payload = compress_payload(&chunk, &config);
        let compressed_size = payload.len() - 1;
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
        let data = vec![0u8; 1024];
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
        for size in [0, 1, 100, 255, 256, 257, 1000, 10_000] {
            let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
            let payload = compress_payload(&data, &config);
            let recovered = decompress_payload(&payload).unwrap();
            assert_eq!(recovered, data, "Roundtrip failed for size {size}");
        }
    }
}
