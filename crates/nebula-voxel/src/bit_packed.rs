//! Bit-packed array for storing fixed-width integer values in a compact `Vec<u64>`.
//!
//! Each element occupies exactly `bits` bits (0, 2, 4, 8, or 16). Elements are
//! packed tightly across `u64` word boundaries.

use serde::{Deserialize, Serialize};

/// A compact array where each element is stored using a fixed number of bits.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BitPackedArray {
    /// Raw storage. Elements are packed into 64-bit words.
    data: Vec<u64>,
    /// Bits per element (0, 2, 4, 8, or 16).
    bits: u8,
    /// Total number of logical elements.
    len: usize,
}

impl BitPackedArray {
    /// Creates a new array with `len` elements, all initialized to zero.
    ///
    /// `bits` must be one of 0, 2, 4, 8, or 16.
    pub fn new(bits: u8, len: usize) -> Self {
        debug_assert!(
            matches!(bits, 0 | 2 | 4 | 8 | 16),
            "bits must be 0, 2, 4, 8, or 16"
        );
        let word_count = if bits == 0 {
            0
        } else {
            let total_bits = len as u64 * u64::from(bits);
            total_bits.div_ceil(64) as usize
        };
        Self {
            data: vec![0u64; word_count],
            bits,
            len,
        }
    }

    /// Returns the value at the given index.
    ///
    /// # Panics
    ///
    /// Panics if `index >= len` in debug builds.
    pub fn get(&self, index: usize) -> u16 {
        debug_assert!(index < self.len, "index out of bounds");
        if self.bits == 0 {
            return 0;
        }
        let bit_index = index as u64 * u64::from(self.bits);
        let word = (bit_index / 64) as usize;
        let offset = (bit_index % 64) as u32;
        let mask = (1u64 << self.bits) - 1;
        ((self.data[word] >> offset) & mask) as u16
    }

    /// Sets the value at the given index.
    ///
    /// # Panics
    ///
    /// Panics if `index >= len` in debug builds, or if `value` exceeds
    /// the maximum for the current bit width.
    pub fn set(&mut self, index: usize, value: u16) {
        debug_assert!(index < self.len, "index out of bounds");
        if self.bits == 0 {
            return;
        }
        debug_assert!(
            self.bits >= 16 || value < (1u16 << self.bits),
            "value {value} exceeds {}-bit capacity",
            self.bits
        );
        let bit_index = index as u64 * u64::from(self.bits);
        let word = (bit_index / 64) as usize;
        let offset = (bit_index % 64) as u32;
        let mask = (1u64 << self.bits) - 1;
        self.data[word] &= !(mask << offset);
        self.data[word] |= u64::from(value) << offset;
    }

    /// Returns the number of bits per element.
    pub fn bits(&self) -> u8 {
        self.bits
    }

    /// Returns the number of logical elements.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` if the array has no elements.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the size of the backing storage in bytes (not counting struct overhead).
    pub fn storage_bytes(&self) -> usize {
        self.data.len() * 8
    }

    /// Returns a reference to the raw `u64` storage words.
    pub fn raw_data(&self) -> &[u64] {
        &self.data
    }

    /// Constructs a `BitPackedArray` from raw parts.
    ///
    /// # Safety (logical)
    ///
    /// The caller must ensure `data` has the correct number of words for
    /// `bits * len` total bits, and all stored values fit within `bits` bits.
    pub fn from_raw(bits: u8, len: usize, data: Vec<u64>) -> Self {
        Self { data, bits, len }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zero_bit_array() {
        let arr = BitPackedArray::new(0, 100);
        assert_eq!(arr.get(0), 0);
        assert_eq!(arr.get(99), 0);
        assert_eq!(arr.storage_bytes(), 0);
    }

    #[test]
    fn test_two_bit_roundtrip() {
        let mut arr = BitPackedArray::new(2, 64);
        for i in 0..64 {
            arr.set(i, (i % 4) as u16);
        }
        for i in 0..64 {
            assert_eq!(arr.get(i), (i % 4) as u16);
        }
    }

    #[test]
    fn test_four_bit_roundtrip() {
        let mut arr = BitPackedArray::new(4, 32);
        for i in 0..32 {
            arr.set(i, (i % 16) as u16);
        }
        for i in 0..32 {
            assert_eq!(arr.get(i), (i % 16) as u16);
        }
    }

    #[test]
    fn test_eight_bit_roundtrip() {
        let mut arr = BitPackedArray::new(8, 256);
        for i in 0..256 {
            arr.set(i, i as u16);
        }
        for i in 0..256 {
            assert_eq!(arr.get(i), i as u16);
        }
    }

    #[test]
    fn test_sixteen_bit_roundtrip() {
        let mut arr = BitPackedArray::new(16, 100);
        for i in 0..100 {
            arr.set(i, i as u16 * 100);
        }
        for i in 0..100 {
            assert_eq!(arr.get(i), i as u16 * 100);
        }
    }

    #[test]
    fn test_storage_sizes() {
        // 32768 voxels at 2 bits = 8192 bytes
        let arr = BitPackedArray::new(2, 32768);
        assert_eq!(arr.storage_bytes(), 8192);

        // 32768 voxels at 4 bits = 16384 bytes
        let arr = BitPackedArray::new(4, 32768);
        assert_eq!(arr.storage_bytes(), 16384);

        // 32768 voxels at 8 bits = 32768 bytes
        let arr = BitPackedArray::new(8, 32768);
        assert_eq!(arr.storage_bytes(), 32768);
    }
}
