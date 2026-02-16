//! Binary serialization and deserialization for [`ChunkData`].
//!
//! The NVCK (Nebula Voxel Chunk) format is a compact, versioned binary format
//! designed for disk storage, network transmission, and undo/redo snapshots.
//!
//! ## Binary Layout
//!
//! | Offset | Size | Field |
//! |--------|------|-------|
//! | 0 | 4 | Magic bytes `[0x4E, 0x56, 0x43, 0x4B]` ("NVCK") |
//! | 4 | 1 | Format version (`u8`, currently 1) |
//! | 5 | 2 | Palette length (`u16`, little-endian) |
//! | 7 | N×2 | Palette entries (N × `u16` `VoxelTypeId`, little-endian) |
//! | 7+N×2 | 1 | Bit width (`u8`: 0, 2, 4, 8, or 16) |
//! | 8+N×2 | M | Bit-packed voxel index data |
//!
//! Where M = `ceil(32768 × bit_width / 8)` bytes (0 when bit_width is 0).

use crate::bit_packed::BitPackedArray;
use crate::chunk::{CHUNK_VOLUME, ChunkData};
use crate::registry::VoxelTypeId;

/// Magic bytes identifying the NVCK format.
const MAGIC: [u8; 4] = [0x4E, 0x56, 0x43, 0x4B];

/// Current format version.
const FORMAT_VERSION: u8 = 1;

/// Errors that can occur during chunk deserialization.
#[derive(Debug, thiserror::Error)]
pub enum ChunkSerError {
    /// The data does not start with the expected magic bytes.
    #[error("invalid magic bytes")]
    InvalidMagic,
    /// The format version is not supported by this build.
    #[error("unsupported format version: {0}")]
    UnsupportedVersion(u8),
    /// The data is shorter than expected.
    #[error("data truncated: expected {expected} bytes, got {actual}")]
    Truncated {
        /// Minimum expected byte count.
        expected: usize,
        /// Actual byte count received.
        actual: usize,
    },
    /// The bit width byte is not one of the valid values (0, 2, 4, 8, 16).
    #[error("invalid bit width: {0}")]
    InvalidBitWidth(u8),
    /// A palette entry references an out-of-range voxel type.
    #[error("palette entry out of range")]
    InvalidPaletteEntry,
}

impl ChunkData {
    /// Serializes this chunk to a byte vector in the NVCK binary format.
    pub fn serialize(&self) -> Vec<u8> {
        let palette = self.palette();
        let bit_width = self.bit_width();
        let index_bytes = index_data_len(bit_width);
        let total = 4 + 1 + 2 + palette.len() * 2 + 1 + index_bytes;

        let mut buf = Vec::with_capacity(total);

        // Magic + version
        buf.extend_from_slice(&MAGIC);
        buf.push(FORMAT_VERSION);

        // Palette
        buf.extend_from_slice(&(palette.len() as u16).to_le_bytes());
        for entry in palette {
            buf.extend_from_slice(&entry.0.to_le_bytes());
        }

        // Bit width
        buf.push(bit_width);

        // Index data: convert u64 words to little-endian bytes
        if bit_width > 0 {
            let raw = self.storage().raw_data();
            for word in raw {
                buf.extend_from_slice(&word.to_le_bytes());
            }
            // Trim to exact index_bytes (last word may be partially used)
            buf.truncate(total);
        }

        buf
    }

    /// Deserializes a chunk from a byte slice in the NVCK binary format.
    ///
    /// Returns an error if the data is corrupted, has an unrecognized version,
    /// or is truncated.
    pub fn deserialize(data: &[u8]) -> Result<Self, ChunkSerError> {
        // Check magic bytes first (need at least 4 bytes)
        if data.len() < 4 {
            return Err(ChunkSerError::InvalidMagic);
        }
        if data[0..4] != MAGIC {
            return Err(ChunkSerError::InvalidMagic);
        }

        // Check version (need at least 5 bytes)
        if data.len() < 5 {
            return Err(ChunkSerError::Truncated {
                expected: 5,
                actual: data.len(),
            });
        }
        let version = data[4];
        if version != FORMAT_VERSION {
            return Err(ChunkSerError::UnsupportedVersion(version));
        }

        // Need at least magic(4) + version(1) + palette_len(2) + bit_width(1) = 8
        if data.len() < 8 {
            return Err(ChunkSerError::Truncated {
                expected: 8,
                actual: data.len(),
            });
        }

        // Palette length
        let palette_len = u16::from_le_bytes([data[5], data[6]]) as usize;

        // Check we have enough data for palette + bit_width byte
        let palette_end = 7 + palette_len * 2;
        let header_end = palette_end + 1;
        if data.len() < header_end {
            return Err(ChunkSerError::Truncated {
                expected: header_end,
                actual: data.len(),
            });
        }

        // Read palette
        let mut palette = Vec::with_capacity(palette_len);
        for i in 0..palette_len {
            let offset = 7 + i * 2;
            let id = u16::from_le_bytes([data[offset], data[offset + 1]]);
            palette.push(VoxelTypeId(id));
        }

        // Bit width
        let bit_width = data[palette_end];
        if !matches!(bit_width, 0 | 2 | 4 | 8 | 16) {
            return Err(ChunkSerError::InvalidBitWidth(bit_width));
        }

        // Validate palette size vs bit width
        if palette_len == 0 {
            return Err(ChunkSerError::InvalidPaletteEntry);
        }

        // Index data
        let index_bytes = index_data_len(bit_width);
        let total_expected = header_end + index_bytes;
        if data.len() < total_expected {
            return Err(ChunkSerError::Truncated {
                expected: total_expected,
                actual: data.len(),
            });
        }

        let storage = if bit_width == 0 {
            BitPackedArray::new(0, CHUNK_VOLUME)
        } else {
            let index_data = &data[header_end..header_end + index_bytes];
            // Convert bytes to u64 words (little-endian)
            let word_count = index_bytes.div_ceil(8);
            let mut words = Vec::with_capacity(word_count);
            for chunk in index_data.chunks(8) {
                let mut word_bytes = [0u8; 8];
                word_bytes[..chunk.len()].copy_from_slice(chunk);
                words.push(u64::from_le_bytes(word_bytes));
            }
            BitPackedArray::from_raw(bit_width, CHUNK_VOLUME, words)
        };

        Ok(ChunkData::from_raw_parts(palette, storage, bit_width))
    }
}

/// Returns the number of bytes needed for the bit-packed index data.
fn index_data_len(bit_width: u8) -> usize {
    if bit_width == 0 {
        return 0;
    }
    let total_bits = CHUNK_VOLUME * bit_width as usize;
    total_bits.div_ceil(8)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        // Test with various palette sizes: 1 (uniform), 4, 20, 300
        for &num_types in &[1usize, 4, 20, 300] {
            let mut chunk = ChunkData::new_air();
            for i in 1..num_types {
                let idx = i;
                chunk.set(
                    idx % 32,
                    (idx / 32) % 32,
                    (idx / 1024) % 32,
                    VoxelTypeId(i as u16),
                );
            }

            let bytes = chunk.serialize();
            let restored = ChunkData::deserialize(&bytes)
                .unwrap_or_else(|e| panic!("deserialize failed for {num_types} types: {e}"));

            // Verify every voxel matches
            for z in 0..32 {
                for y in 0..32 {
                    for x in 0..32 {
                        assert_eq!(
                            chunk.get(x, y, z),
                            restored.get(x, y, z),
                            "mismatch at ({x},{y},{z}) with {num_types} types"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_empty_chunk_serializes_small() {
        let chunk = ChunkData::new_air();
        let bytes = chunk.serialize();
        assert!(
            bytes.len() < 16,
            "uniform chunk serialized to {} bytes, expected < 16",
            bytes.len()
        );
    }

    #[test]
    fn test_full_chunk_serializes_correctly() {
        let mut chunk = ChunkData::new_air();
        // Use distinct patterns clamped to available types
        for z in 0..32usize {
            for y in 0..32usize {
                for x in 0..32usize {
                    let id = ((x + y * 32) % 256) as u16;
                    chunk.set(x, y, z, VoxelTypeId(id));
                }
            }
        }

        let bytes = chunk.serialize();
        let restored = ChunkData::deserialize(&bytes).expect("deserialize failed");

        for z in 0..32 {
            for y in 0..32 {
                for x in 0..32 {
                    assert_eq!(
                        chunk.get(x, y, z),
                        restored.get(x, y, z),
                        "mismatch at ({x},{y},{z})"
                    );
                }
            }
        }
    }

    #[test]
    fn test_version_byte_present() {
        let chunk = ChunkData::new_air();
        let bytes = chunk.serialize();
        assert_eq!(bytes[4], 1, "format version should be 1");
    }

    #[test]
    fn test_corrupted_data_returns_error() {
        // Invalid magic
        let result = ChunkData::deserialize(&[0xFF, 0xFF]);
        assert!(
            matches!(result, Err(ChunkSerError::InvalidMagic)),
            "expected InvalidMagic, got {result:?}"
        );

        // Unsupported version
        let result = ChunkData::deserialize(&[0x4E, 0x56, 0x43, 0x4B, 99, 0, 0, 0]);
        assert!(
            matches!(result, Err(ChunkSerError::UnsupportedVersion(99))),
            "expected UnsupportedVersion(99), got {result:?}"
        );

        // Truncated: valid header but missing palette data
        let result = ChunkData::deserialize(&[0x4E, 0x56, 0x43, 0x4B, 1, 5, 0]);
        assert!(
            matches!(result, Err(ChunkSerError::Truncated { .. })),
            "expected Truncated, got {result:?}"
        );

        // Invalid bit width
        let result = ChunkData::deserialize(&[0x4E, 0x56, 0x43, 0x4B, 1, 1, 0, 0, 0, 3]);
        assert!(
            matches!(result, Err(ChunkSerError::InvalidBitWidth(3))),
            "expected InvalidBitWidth(3), got {result:?}"
        );
    }
}
