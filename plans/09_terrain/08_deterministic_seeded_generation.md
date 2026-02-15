# Deterministic Seeded Generation

## Problem

Multiplayer games, world sharing, and reproducible bug reports all require that terrain generation is 100% deterministic: given the same world seed and chunk address, the engine must produce byte-identical chunk data regardless of which thread performed the generation, the order in which chunks were generated, or the platform (Windows, Linux, macOS, x86, ARM). Floating-point operations are a notorious source of non-determinism -- different compilers and instruction sets can produce slightly different results for transcendental functions (sin, cos, sqrt), and reordering of operations can change rounding behavior. The engine needs a seeding strategy that derives a unique, reproducible RNG state for each chunk from the world seed and chunk address, and must guard against floating-point variance in critical paths.

## Solution

Implement a `DeterministicSeedProvider` in the `nebula-terrain` crate that derives per-chunk RNG seeds from the world seed and chunk address using a cryptographic hash, and provide guidelines and utilities for avoiding floating-point non-determinism in terrain generation code.

### Seed Derivation

Each chunk gets a unique seed derived from the world seed and its `ChunkAddress`. The derivation uses a hash function (not a simple XOR) to ensure that nearby chunk addresses produce uncorrelated seeds:

```rust
use rand_chacha::ChaCha8Rng;
use rand::SeedableRng;
use std::hash::{Hash, Hasher};

/// Derive a deterministic RNG for a specific chunk.
///
/// The returned RNG will produce an identical sequence of random numbers
/// for the same `(world_seed, address)` pair, regardless of thread or platform.
pub fn chunk_rng(world_seed: u64, address: &ChunkAddress) -> ChaCha8Rng {
    let chunk_seed = derive_chunk_seed(world_seed, address);
    ChaCha8Rng::seed_from_u64(chunk_seed)
}

/// Derive a u64 seed for a chunk from the world seed and chunk address.
///
/// Uses SipHash (via std's DefaultHasher) to combine the world seed with
/// the chunk address into a well-distributed u64. SipHash is fast and
/// produces excellent distribution even for sequential inputs.
pub fn derive_chunk_seed(world_seed: u64, address: &ChunkAddress) -> u64 {
    use std::collections::hash_map::DefaultHasher;

    let mut hasher = DefaultHasher::new();
    world_seed.hash(&mut hasher);
    address.face.hash(&mut hasher);
    address.x.hash(&mut hasher);
    address.y.hash(&mut hasher);
    address.z.hash(&mut hasher);
    hasher.finish()
}
```

### ChaCha8Rng

The engine uses `ChaCha8Rng` from `rand_chacha` rather than weaker PRNGs because:

1. **Deterministic across platforms**: ChaCha8 is defined algorithmically (not implementation-dependent like `StdRng`). The same seed produces the same sequence on x86, ARM, and WASM.
2. **Cryptographic quality**: No observable patterns even with sequential seeds (unlike LCGs or xorshift).
3. **Fast enough**: ChaCha8 (8 rounds) is significantly faster than ChaCha20 while still having excellent statistical properties for game use.

### Floating-Point Determinism Strategy

Floating-point non-determinism comes from three sources:

1. **Instruction reordering by the compiler**: Mitigated by using `#[inline(never)]` on critical noise functions and avoiding `--fast-math` / `-ffast-math` compiler flags.
2. **Different instruction sets (SSE vs AVX vs NEON)**: Mitigated by using the `noise` crate's software implementations rather than hardware-accelerated intrinsics.
3. **Transcendental function implementations**: `sin`, `cos`, and `atan` differ between libm implementations. Mitigated by using `libm` crate's pure-Rust implementations for any transcendental functions in terrain generation.

```rust
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
```

### Fixed-Point Intermediate Calculations

For the most critical determinism requirements (e.g., multiplayer chunk verification), intermediate noise accumulation can optionally use fixed-point arithmetic to eliminate floating-point rounding entirely:

```rust
use crate::math::FixedPoint64;

/// Accumulate noise octaves using fixed-point arithmetic.
/// This guarantees bit-exact results across all platforms.
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
        // Noise sampling itself uses f64 (the noise crate is deterministic
        // for the same input), but accumulation uses fixed-point.
        let f = freq.to_f64();
        let noise_val = noise_fn.get([
            point.x * f,
            point.y * f,
            point.z * f,
        ]);
        let noise_fixed = FixedPoint64::from_f64(noise_val);

        total = total + noise_fixed * amp;
        freq = freq * lac;
        amp = amp * pers;
    }

    total
}
```

### Verification Utility

A utility function generates a chunk and returns its hash, enabling quick comparison across threads and platforms:

```rust
use std::collections::hash_map::DefaultHasher;

/// Generate a chunk and return its content hash for determinism verification.
pub fn generate_and_hash(task: &GenerationTask) -> u64 {
    let chunk = generate_chunk_sync(task);
    let mut hasher = DefaultHasher::new();
    chunk.hash(&mut hasher);
    hasher.finish()
}
```

## Outcome

A `derive_chunk_seed` function, `chunk_rng` helper, deterministic math utilities (`det_sin`, `det_cos`, etc.), and optional fixed-point fBm accumulator in `nebula-terrain`. All terrain generation code uses these primitives to ensure byte-identical output for the same seed and chunk address. Running `cargo test -p nebula-terrain` passes all determinism tests.

## Demo Integration

**Demo crate:** `nebula-demo`

Restarting the demo with the same seed produces identical terrain. The console logs `Seed: 0xDEADBEEF, deterministic: true`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `rand` | 0.9 | `SeedableRng` trait and `Rng` trait for random number generation |
| `rand_chacha` | 0.9 | `ChaCha8Rng` for platform-deterministic PRNG |
| `libm` | 0.2 | Pure-Rust implementations of transcendental math functions for cross-platform determinism |

The crate uses Rust edition 2024.

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ChunkAddress;
    use crate::cubesphere::CubeFace;

    #[test]
    fn test_generate_same_chunk_twice_identical() {
        let task = GenerationTask {
            address: ChunkAddress::new(CubeFace::PosX, 10, 5, 20),
            seed: 12345,
            planet: PlanetDef::default(),
            priority: 0,
        };

        let chunk_a = generate_chunk_sync(&task);
        let chunk_b = generate_chunk_sync(&task);

        assert_eq!(
            chunk_a, chunk_b,
            "Generating the same chunk twice must produce identical data"
        );
    }

    #[test]
    fn test_generate_on_different_threads_identical() {
        let task = GenerationTask {
            address: ChunkAddress::new(CubeFace::NegY, 3, 7, 1),
            seed: 67890,
            planet: PlanetDef::default(),
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
            address: ChunkAddress::new(CubeFace::PosX, 0, 0, 0),
            seed: 42,
            planet: PlanetDef::default(),
            priority: 0,
        };
        let task_b = GenerationTask {
            address: ChunkAddress::new(CubeFace::PosX, 10, 0, 10),
            seed: 42,
            planet: PlanetDef::default(),
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
        let addr = ChunkAddress::new(CubeFace::PosZ, 5, 5, 5);

        let task_a = GenerationTask {
            address: addr,
            seed: 0,
            planet: PlanetDef::default(),
            priority: 0,
        };
        let task_b = GenerationTask {
            address: addr,
            seed: 1,
            planet: PlanetDef::default(),
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
    fn test_derive_chunk_seed_deterministic() {
        let addr = ChunkAddress::new(CubeFace::PosY, 42, 13, 7);
        let seed_a = derive_chunk_seed(999, &addr);
        let seed_b = derive_chunk_seed(999, &addr);
        assert_eq!(seed_a, seed_b, "Same inputs must produce same derived seed");
    }

    #[test]
    fn test_derive_chunk_seed_different_addresses() {
        let addr_a = ChunkAddress::new(CubeFace::PosX, 0, 0, 0);
        let addr_b = ChunkAddress::new(CubeFace::PosX, 0, 0, 1);
        let seed_a = derive_chunk_seed(42, &addr_a);
        let seed_b = derive_chunk_seed(42, &addr_b);
        assert_ne!(
            seed_a, seed_b,
            "Adjacent chunk addresses should produce different seeds"
        );
    }

    #[test]
    fn test_derive_chunk_seed_different_world_seeds() {
        let addr = ChunkAddress::new(CubeFace::PosX, 5, 5, 5);
        let seed_a = derive_chunk_seed(0, &addr);
        let seed_b = derive_chunk_seed(1, &addr);
        assert_ne!(
            seed_a, seed_b,
            "Different world seeds should produce different chunk seeds"
        );
    }

    #[test]
    fn test_chacha8_rng_deterministic() {
        let addr = ChunkAddress::new(CubeFace::NegZ, 10, 20, 30);
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
        // Verify libm functions return consistent results.
        let x = 1.2345678901234;
        let sin_a = det_sin(x);
        let sin_b = det_sin(x);
        assert_eq!(sin_a, sin_b, "det_sin must be perfectly deterministic");

        let cos_a = det_cos(x);
        let cos_b = det_cos(x);
        assert_eq!(cos_a, cos_b, "det_cos must be perfectly deterministic");

        let sqrt_a = det_sqrt(x);
        let sqrt_b = det_sqrt(x);
        assert_eq!(sqrt_a, sqrt_b, "det_sqrt must be perfectly deterministic");
    }
}
```
