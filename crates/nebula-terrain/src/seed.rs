//! Deterministic seeded generation utilities.
//!
//! Provides per-chunk RNG derivation from a world seed and chunk address,
//! deterministic math functions via `libm`, and a fixed-point fBm accumulator
//! for cross-platform bit-exact terrain generation.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use nebula_voxel::{CHUNK_SIZE, ChunkAddress, ChunkData};
use noise::NoiseFn;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use crate::GenerationTask;

// ---------------------------------------------------------------------------
// Seed derivation
// ---------------------------------------------------------------------------

/// Derive a u64 seed for a chunk from the world seed and chunk address.
///
/// Uses SipHash (via std's `DefaultHasher`) to combine the world seed with
/// the chunk address into a well-distributed u64.
pub fn derive_chunk_seed(world_seed: u64, address: &ChunkAddress) -> u64 {
    let mut hasher = DefaultHasher::new();
    world_seed.hash(&mut hasher);
    address.face.hash(&mut hasher);
    address.x.hash(&mut hasher);
    address.y.hash(&mut hasher);
    address.z.hash(&mut hasher);
    hasher.finish()
}

/// Derive a deterministic RNG for a specific chunk.
///
/// The returned RNG will produce an identical sequence of random numbers
/// for the same `(world_seed, address)` pair, regardless of thread or platform.
pub fn chunk_rng(world_seed: u64, address: &ChunkAddress) -> ChaCha8Rng {
    let chunk_seed = derive_chunk_seed(world_seed, address);
    ChaCha8Rng::seed_from_u64(chunk_seed)
}

// ---------------------------------------------------------------------------
// Deterministic math (libm)
// ---------------------------------------------------------------------------

/// Deterministic sine using libm (not platform libc).
#[inline]
pub fn det_sin(x: f64) -> f64 {
    libm::sin(x)
}

/// Deterministic cosine using libm.
#[inline]
pub fn det_cos(x: f64) -> f64 {
    libm::cos(x)
}

/// Deterministic atan2 using libm.
#[inline]
pub fn det_atan2(y: f64, x: f64) -> f64 {
    libm::atan2(y, x)
}

/// Deterministic sqrt using libm.
#[inline]
pub fn det_sqrt(x: f64) -> f64 {
    libm::sqrt(x)
}

// ---------------------------------------------------------------------------
// Fixed-point arithmetic
// ---------------------------------------------------------------------------

/// 64-bit fixed-point number with 32 integer bits and 32 fractional bits.
///
/// Used for bit-exact accumulation of noise values across platforms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FixedPoint64(i64);

impl FixedPoint64 {
    /// The zero value.
    pub const ZERO: Self = Self(0);

    const FRAC_BITS: u32 = 32;

    /// Convert from `f64` to fixed-point (saturating on overflow).
    #[inline]
    pub fn from_f64(v: f64) -> Self {
        let scaled = v * (1_i64 << Self::FRAC_BITS) as f64;
        Self(scaled as i64)
    }

    /// Convert from fixed-point back to `f64`.
    #[inline]
    pub fn to_f64(self) -> f64 {
        self.0 as f64 / (1_i64 << Self::FRAC_BITS) as f64
    }
}

impl std::ops::Add for FixedPoint64 {
    type Output = Self;
    #[inline]
    fn add(self, rhs: Self) -> Self {
        Self(self.0.wrapping_add(rhs.0))
    }
}

impl std::ops::Mul for FixedPoint64 {
    type Output = Self;
    #[inline]
    fn mul(self, rhs: Self) -> Self {
        // Use 128-bit intermediate to avoid overflow.
        let wide = (self.0 as i128) * (rhs.0 as i128);
        Self((wide >> Self::FRAC_BITS) as i64)
    }
}

/// Accumulate noise octaves using fixed-point arithmetic.
///
/// This guarantees bit-exact results across all platforms for the accumulation
/// step. The noise sampling itself uses `f64` (the `noise` crate is
/// deterministic for the same input), but the summation uses fixed-point.
pub fn fbm_fixed_point(
    noise_fn: &impl NoiseFn<f64, 3>,
    point: glam::DVec3,
    octaves: u32,
    lacunarity: f64,
    persistence: f64,
    base_frequency: f64,
    amplitude: f64,
) -> FixedPoint64 {
    let mut total = FixedPoint64::ZERO;
    let mut freq = FixedPoint64::from_f64(base_frequency);
    let mut amp = FixedPoint64::from_f64(amplitude);
    let lac = FixedPoint64::from_f64(lacunarity);
    let pers = FixedPoint64::from_f64(persistence);

    for _ in 0..octaves {
        let f = freq.to_f64();
        let noise_val = noise_fn.get([point.x * f, point.y * f, point.z * f]);
        let noise_fixed = FixedPoint64::from_f64(noise_val);

        total = total + noise_fixed * amp;
        freq = freq * lac;
        amp = amp * pers;
    }

    total
}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

/// Generate a chunk and return its content hash for determinism verification.
///
/// Hashes every voxel in the chunk (32³ voxels) to produce a u64 digest.
pub fn generate_and_hash(task: &GenerationTask) -> u64 {
    let chunk = crate::generate_chunk_sync(task);
    hash_chunk_data(&chunk)
}

/// Hash the contents of a `ChunkData` for determinism comparison.
pub fn hash_chunk_data(chunk: &ChunkData) -> u64 {
    let mut hasher = DefaultHasher::new();
    for x in 0..CHUNK_SIZE {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                chunk.get(x, y, z).0.hash(&mut hasher);
            }
        }
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nebula_cubesphere::PlanetDef;
    use nebula_math::WorldPosition;
    use rand::RngCore;

    fn dummy_planet() -> PlanetDef {
        PlanetDef::earth_like("TestPlanet", WorldPosition::default(), 42)
    }

    #[test]
    fn test_derive_chunk_seed_deterministic() {
        let addr = ChunkAddress::new(42, 13, 7, 1);
        let seed_a = derive_chunk_seed(999, &addr);
        let seed_b = derive_chunk_seed(999, &addr);
        assert_eq!(seed_a, seed_b, "Same inputs must produce same derived seed");
    }

    #[test]
    fn test_derive_chunk_seed_different_addresses() {
        let addr_a = ChunkAddress::new(0, 0, 0, 0);
        let addr_b = ChunkAddress::new(0, 0, 1, 0);
        let seed_a = derive_chunk_seed(42, &addr_a);
        let seed_b = derive_chunk_seed(42, &addr_b);
        assert_ne!(
            seed_a, seed_b,
            "Adjacent chunk addresses should produce different seeds"
        );
    }

    #[test]
    fn test_derive_chunk_seed_different_world_seeds() {
        let addr = ChunkAddress::new(5, 5, 5, 0);
        let seed_a = derive_chunk_seed(0, &addr);
        let seed_b = derive_chunk_seed(1, &addr);
        assert_ne!(
            seed_a, seed_b,
            "Different world seeds should produce different chunk seeds"
        );
    }

    #[test]
    fn test_chacha8_rng_deterministic() {
        let addr = ChunkAddress::new(10, 20, 30, 5);
        let mut rng_a = chunk_rng(42, &addr);
        let mut rng_b = chunk_rng(42, &addr);

        for _ in 0..1000 {
            assert_eq!(
                rng_a.next_u64(),
                rng_b.next_u64(),
                "ChaCha8Rng sequences must match for same seed"
            );
        }
    }

    #[test]
    fn test_deterministic_math_functions() {
        let x = 1.234_567_890_123_4;
        assert_eq!(det_sin(x), det_sin(x), "det_sin must be deterministic");
        assert_eq!(det_cos(x), det_cos(x), "det_cos must be deterministic");
        assert_eq!(det_sqrt(x), det_sqrt(x), "det_sqrt must be deterministic");
        assert_eq!(
            det_atan2(x, 0.5),
            det_atan2(x, 0.5),
            "det_atan2 must be deterministic"
        );
    }

    #[test]
    fn test_fixed_point_round_trip() {
        let values = [0.0, 1.0, -1.0, 0.5, 123.456, -99.99];
        for &v in &values {
            let fp = FixedPoint64::from_f64(v);
            let back = fp.to_f64();
            assert!(
                (back - v).abs() < 1e-6,
                "Round-trip failed for {v}: got {back}"
            );
        }
    }

    #[test]
    fn test_fixed_point_arithmetic() {
        let a = FixedPoint64::from_f64(2.5);
        let b = FixedPoint64::from_f64(3.0);
        let sum = (a + b).to_f64();
        assert!(
            (sum - 5.5).abs() < 1e-6,
            "Addition: expected 5.5, got {sum}"
        );

        let prod = (a * b).to_f64();
        assert!(
            (prod - 7.5).abs() < 1e-6,
            "Multiplication: expected 7.5, got {prod}"
        );
    }

    #[test]
    fn test_generate_same_chunk_twice_identical() {
        let task = GenerationTask {
            address: ChunkAddress::new(10, 5, 20, 0),
            seed: 12345,
            planet: dummy_planet(),
            priority: 0,
        };

        let hash_a = generate_and_hash(&task);
        let hash_b = generate_and_hash(&task);

        assert_eq!(
            hash_a, hash_b,
            "Generating the same chunk twice must produce identical data"
        );
    }

    #[test]
    fn test_generate_on_different_threads_identical() {
        let task = GenerationTask {
            address: ChunkAddress::new(3, 7, 1, 4),
            seed: 67890,
            planet: dummy_planet(),
            priority: 0,
        };

        let task_clone = task.clone();

        let handle_a = std::thread::spawn(move || generate_and_hash(&task));
        let handle_b = std::thread::spawn(move || generate_and_hash(&task_clone));

        let hash_a = handle_a.join().unwrap();
        let hash_b = handle_b.join().unwrap();

        assert_eq!(
            hash_a, hash_b,
            "Same chunk generated on different threads must produce identical hash"
        );
    }

    #[test]
    fn test_different_chunk_addresses_different_data() {
        let task_a = GenerationTask {
            address: ChunkAddress::new(0, 0, 0, 0),
            seed: 42,
            planet: dummy_planet(),
            priority: 0,
        };
        let task_b = GenerationTask {
            address: ChunkAddress::new(10, 0, 10, 0),
            seed: 42,
            planet: dummy_planet(),
            priority: 0,
        };

        let hash_a = generate_and_hash(&task_a);
        let hash_b = generate_and_hash(&task_b);

        assert_ne!(
            hash_a, hash_b,
            "Different chunk addresses should produce different chunk data"
        );
    }

    #[test]
    fn test_seed_0_vs_seed_1_different_worlds() {
        // Use y=0 so the chunk straddles the surface (where seed matters).
        let addr = ChunkAddress::new(0, 0, 0, 0);

        let task_a = GenerationTask {
            address: addr,
            seed: 0,
            planet: dummy_planet(),
            priority: 0,
        };
        let task_b = GenerationTask {
            address: addr,
            seed: 9999,
            planet: dummy_planet(),
            priority: 0,
        };

        let hash_a = generate_and_hash(&task_a);
        let hash_b = generate_and_hash(&task_b);

        assert_ne!(
            hash_a, hash_b,
            "Seed 0 and seed 1 should produce different worlds"
        );
    }

    #[test]
    fn test_fbm_fixed_point_deterministic() {
        let noise = noise::Simplex::new(42);
        let point = glam::DVec3::new(1.5, 2.5, 3.5);

        let a = fbm_fixed_point(&noise, point, 4, 2.0, 0.5, 0.02, 16.0);
        let b = fbm_fixed_point(&noise, point, 4, 2.0, 0.5, 0.02, 16.0);

        assert_eq!(a, b, "Fixed-point fBm must be bit-exact");
    }
}
