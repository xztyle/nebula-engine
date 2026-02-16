//! Run-Length Encoding (RLE) for palette index arrays.
//!
//! RLE compresses runs of identical palette indices into `(count, value)` pairs.
//! Used at serialization time to reduce chunk size for disk and network transfer.

/// A single RLE run: `count` consecutive occurrences of `value`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RleRun {
    /// Number of consecutive identical values (1..=65535).
    pub count: u16,
    /// The palette index value.
    pub value: u16,
}

/// Errors that can occur during RLE decoding.
#[derive(Debug, thiserror::Error)]
pub enum RleError {
    /// Decoded length does not match expected length.
    #[error("RLE length mismatch: expected {expected}, got {actual}")]
    LengthMismatch {
        /// Expected number of elements.
        expected: usize,
        /// Actual number of decoded elements.
        actual: usize,
    },
}

/// Encodes a slice of palette indices into RLE runs.
///
/// Runs are capped at `u16::MAX` length. An empty input produces an empty output.
pub fn rle_encode(indices: &[u16]) -> Vec<RleRun> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < indices.len() {
        let value = indices[i];
        let mut count: u16 = 1;
        loop {
            let idx = i + (count as usize);
            if idx >= indices.len() || indices[idx] != value || count == u16::MAX {
                break;
            }
            count += 1;
        }
        runs.push(RleRun { count, value });
        i += count as usize;
    }
    runs
}

/// Decodes RLE runs back into a flat index array.
///
/// Returns an error if the total decoded length does not match `expected_len`.
pub fn rle_decode(runs: &[RleRun], expected_len: usize) -> Result<Vec<u16>, RleError> {
    let mut result = Vec::with_capacity(expected_len);
    for run in runs {
        result.extend(std::iter::repeat_n(run.value, run.count as usize));
    }
    if result.len() != expected_len {
        return Err(RleError::LengthMismatch {
            expected: expected_len,
            actual: result.len(),
        });
    }
    Ok(result)
}

/// Encodes RLE runs to bytes: each run is `count: u16 LE` + `value: u16 LE`.
pub fn rle_to_bytes(runs: &[RleRun]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(runs.len() * 4);
    for run in runs {
        buf.extend_from_slice(&run.count.to_le_bytes());
        buf.extend_from_slice(&run.value.to_le_bytes());
    }
    buf
}

/// Decodes RLE runs from bytes. Each run is 4 bytes: `count: u16 LE` + `value: u16 LE`.
///
/// `data` length must be a multiple of 4, and contain exactly `run_count` runs.
pub fn rle_from_bytes(data: &[u8], run_count: usize) -> Vec<RleRun> {
    let mut runs = Vec::with_capacity(run_count);
    for chunk in data[..run_count * 4].chunks_exact(4) {
        runs.push(RleRun {
            count: u16::from_le_bytes([chunk[0], chunk[1]]),
            value: u16::from_le_bytes([chunk[2], chunk[3]]),
        });
    }
    runs
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_uniform_chunk_single_run() {
        let indices = vec![0u16; 32_768];
        let runs = rle_encode(&indices);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].count, 32_768);
        assert_eq!(runs[0].value, 0);
    }

    #[test]
    fn test_alternating_voxels_no_compression() {
        let indices: Vec<u16> = (0..32_768).map(|i| (i % 2) as u16).collect();
        let runs = rle_encode(&indices);
        assert_eq!(runs.len(), 32_768);

        // Adaptive: RLE size (32768*4=131072) > raw 2-bit size (8192), should pick raw
        let rle_size = runs.len() * 4;
        let raw_size = (32_768_usize * 2).div_ceil(8); // 2-bit width
        assert!(
            rle_size > raw_size,
            "RLE should be worse for alternating data"
        );
    }

    #[test]
    fn test_rle_roundtrip() {
        // Terrain-like: runs of varying lengths
        let mut indices = Vec::with_capacity(32_768);
        // 16384 air, 1024 grass, 1024 dirt, 14336 stone
        indices.extend(std::iter::repeat_n(0u16, 16_384));
        indices.extend(std::iter::repeat_n(1u16, 1_024));
        indices.extend(std::iter::repeat_n(2u16, 1_024));
        indices.extend(std::iter::repeat_n(3u16, 14_336));

        let runs = rle_encode(&indices);
        let decoded = rle_decode(&runs, 32_768).expect("decode failed");
        assert_eq!(decoded, indices);
    }

    #[test]
    fn test_compression_ratio_terrain() {
        // Bottom half stone (idx 1), top half air (idx 0), thin grass/dirt layers
        let mut indices = Vec::with_capacity(32_768);
        // stone: 14336, dirt: 1024, grass: 1024, air: 16384
        indices.extend(std::iter::repeat_n(1u16, 14_336));
        indices.extend(std::iter::repeat_n(2u16, 1_024));
        indices.extend(std::iter::repeat_n(3u16, 1_024));
        indices.extend(std::iter::repeat_n(0u16, 16_384));

        let runs = rle_encode(&indices);
        let rle_size = runs.len() * 4;
        let raw_size = (32_768_usize * 2).div_ceil(8); // 2-bit = 8192 bytes
        let ratio = raw_size as f64 / rle_size as f64;
        assert!(
            ratio > 4.0,
            "compression ratio {ratio:.1} should be > 4.0 (rle={rle_size}, raw={raw_size})"
        );
    }

    #[test]
    fn test_empty_chunk_minimal_size() {
        let indices = vec![0u16; 32_768];
        let runs = rle_encode(&indices);
        let bytes = rle_to_bytes(&runs);
        assert_eq!(bytes.len(), 4, "single run should be 4 bytes");
    }

    #[test]
    fn test_rle_bytes_roundtrip() {
        let runs = vec![
            RleRun {
                count: 100,
                value: 0,
            },
            RleRun {
                count: 200,
                value: 3,
            },
        ];
        let bytes = rle_to_bytes(&runs);
        let decoded = rle_from_bytes(&bytes, 2);
        assert_eq!(decoded, runs);
    }

    #[test]
    fn test_decode_length_mismatch() {
        let runs = vec![RleRun {
            count: 10,
            value: 0,
        }];
        let result = rle_decode(&runs, 20);
        assert!(matches!(
            result,
            Err(RleError::LengthMismatch {
                expected: 20,
                actual: 10
            })
        ));
    }
}
