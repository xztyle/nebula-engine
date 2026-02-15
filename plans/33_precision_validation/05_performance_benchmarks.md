# Performance Benchmarks

## Problem

The Nebula Engine's precision architecture trades native hardware speed (64-bit floats, single-instruction vector math) for correctness (128-bit integers, explicit coordinate space transitions). This trade-off must be quantified. Without benchmarks, performance regressions go undetected, optimization efforts lack data, and architectural decisions (e.g., "should distance_squared use i128 or f64?") are based on guesswork. A `criterion` benchmark suite provides statistical rigor: it runs each benchmark hundreds of times, measures variance, detects regressions, and generates HTML reports that visualize performance over time. Story 02_math/10 established per-operation microbenchmarks. This story adds system-level benchmarks for the precision-critical operations that run every frame: batch conversions, spatial queries, and coordinate space transitions at realistic scales.

## Solution

Create a benchmark suite in the workspace root (file: `benches/precision_benchmarks.rs`) using `criterion` 0.5. Each benchmark operates on batches of realistic data to measure throughput rather than single-operation latency.

### Cargo.toml additions (workspace root or relevant crate)

```toml
[dev-dependencies]
criterion = { version = "0.5", features = ["html_reports"] }

[[bench]]
name = "precision_benchmarks"
harness = false
```

### Benchmark implementations

```rust
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use nebula_math::*;
use nebula_coords::*;

// =========================================================================
// Data generators
// =========================================================================

/// Generate N WorldPositions scattered around an origin within a given radius.
fn generate_positions(origin: WorldPosition, radius_mm: i128, count: usize) -> Vec<WorldPosition> {
    (0..count)
        .map(|i| {
            let offset = (i as i128 * 7919) % radius_mm; // Deterministic pseudo-scatter
            WorldPosition::new(
                origin.x + offset,
                origin.y + (offset * 3 / 7),
                origin.z + (offset * 5 / 11),
            )
        })
        .collect()
}

/// Generate N pairs of WorldPositions for distance calculations.
fn generate_pairs(count: usize) -> Vec<(WorldPosition, WorldPosition)> {
    (0..count)
        .map(|i| {
            let a = WorldPosition::new(
                i as i128 * 1_000,
                i as i128 * 2_000,
                i as i128 * 3_000,
            );
            let b = WorldPosition::new(
                i as i128 * 1_000 + 500_000,
                i as i128 * 2_000 + 300_000,
                i as i128 * 3_000 + 100_000,
            );
            (a, b)
        })
        .collect()
}

/// Generate N Aabb128 pairs for intersection testing.
fn generate_aabb_pairs(count: usize) -> Vec<(Aabb128, Aabb128)> {
    (0..count)
        .map(|i| {
            let base = i as i128 * 10_000;
            let a = Aabb128::new(
                WorldPosition::new(base, base, base),
                WorldPosition::new(base + 5_000, base + 5_000, base + 5_000),
            );
            let b = Aabb128::new(
                WorldPosition::new(base + 3_000, base + 3_000, base + 3_000),
                WorldPosition::new(base + 8_000, base + 8_000, base + 8_000),
            );
            (a, b)
        })
        .collect()
}

// =========================================================================
// Benchmarks
// =========================================================================

fn bench_i128_vector_add_10k(c: &mut Criterion) {
    let vectors: Vec<(Vec3I128, Vec3I128)> = (0..10_000)
        .map(|i| {
            (
                Vec3I128::new(i * 100, i * 200, i * 300),
                Vec3I128::new(i * 50, i * 150, i * 250),
            )
        })
        .collect();

    c.bench_function("i128_vector_add_batch_10k", |b| {
        b.iter(|| {
            for (a, v) in &vectors {
                black_box(*a + *v);
            }
        })
    });
}

fn bench_world_to_local_10k(c: &mut Criterion) {
    let origin = WorldPosition::new(
        1_000_000_000_000_000,
        2_000_000_000_000_000,
        3_000_000_000_000_000,
    );
    let positions = generate_positions(origin, 5_000_000, 10_000); // Within 5 km
    let mut out = Vec::with_capacity(10_000);

    c.bench_function("world_to_local_batch_10k", |b| {
        b.iter(|| {
            to_local_batch(black_box(&positions), black_box(origin), &mut out);
            black_box(&out);
        })
    });
}

fn bench_distance_squared_10k(c: &mut Criterion) {
    let pairs = generate_pairs(10_000);

    c.bench_function("distance_squared_batch_10k", |b| {
        b.iter(|| {
            for (a, p) in &pairs {
                black_box(distance_squared(*a, *p));
            }
        })
    });
}

fn bench_aabb_intersection_10k(c: &mut Criterion) {
    let pairs = generate_aabb_pairs(10_000);

    c.bench_function("aabb_intersection_batch_10k", |b| {
        b.iter(|| {
            for (a, o) in &pairs {
                black_box(a.intersects(o));
            }
        })
    });
}

fn bench_spatial_hash_lookup_1k(c: &mut Criterion) {
    // Build a spatial hash with 10,000 entities, then perform 1,000 lookups
    let mut spatial_hash = SpatialHash::new();
    for i in 0..10_000u64 {
        let pos = WorldPosition::new(
            (i as i128) * 1_000,
            (i as i128) * 500,
            (i as i128) * 250,
        );
        spatial_hash.insert(EntityId(i), pos);
    }

    let query_positions: Vec<WorldPosition> = (0..1_000)
        .map(|i| WorldPosition::new(i * 1_000, i * 500, i * 250))
        .collect();

    c.bench_function("spatial_hash_lookup_1k", |b| {
        b.iter(|| {
            for pos in &query_positions {
                black_box(spatial_hash.query_radius(pos, 5_000));
            }
        })
    });
}

fn bench_sector_conversion_10k(c: &mut Criterion) {
    let positions: Vec<WorldPosition> = (0..10_000)
        .map(|i| {
            WorldPosition::new(
                i as i128 * 1_000_000_000,
                -(i as i128) * 500_000_000,
                i as i128 * 250_000_000,
            )
        })
        .collect();

    c.bench_function("sector_conversion_batch_10k", |b| {
        b.iter(|| {
            for pos in &positions {
                black_box(SectorCoord::from_world(pos));
            }
        })
    });
}

// =========================================================================
// Comparison: i128 vs f64
// =========================================================================

fn bench_i128_vs_f64_distance(c: &mut Criterion) {
    let mut group = c.benchmark_group("distance_i128_vs_f64");

    let pairs_i128 = generate_pairs(10_000);
    let pairs_f64: Vec<(glam::DVec3, glam::DVec3)> = pairs_i128
        .iter()
        .map(|(a, b)| {
            (
                glam::DVec3::new(a.x as f64, a.y as f64, a.z as f64),
                glam::DVec3::new(b.x as f64, b.y as f64, b.z as f64),
            )
        })
        .collect();

    group.bench_function("i128_distance_squared_10k", |b| {
        b.iter(|| {
            for (a, p) in &pairs_i128 {
                black_box(distance_squared(*a, *p));
            }
        })
    });

    group.bench_function("f64_distance_squared_10k", |b| {
        b.iter(|| {
            for (a, p) in &pairs_f64 {
                black_box((*a - *p).length_squared());
            }
        })
    });

    group.finish();
}

// =========================================================================
// Criterion boilerplate
// =========================================================================

criterion_group!(
    precision_benches,
    bench_i128_vector_add_10k,
    bench_world_to_local_10k,
    bench_distance_squared_10k,
    bench_aabb_intersection_10k,
    bench_spatial_hash_lookup_1k,
    bench_sector_conversion_10k,
    bench_i128_vs_f64_distance,
);
criterion_main!(precision_benches);
```

### Expected performance targets

| Benchmark | Expected time | Per-operation | Budget at 60 FPS |
|-----------|--------------|---------------|-------------------|
| i128 vector add (10K) | < 50 us | ~5 ns | 0.3% of frame |
| World-to-local (10K) | < 100 us | ~10 ns | 0.6% of frame |
| distance_squared (10K) | < 150 us | ~15 ns | 0.9% of frame |
| AABB intersection (10K) | < 80 us | ~8 ns | 0.5% of frame |
| Spatial hash lookup (1K) | < 500 us | ~500 ns | 3.0% of frame |
| Sector conversion (10K) | < 30 us | ~3 ns | 0.2% of frame |

Total budget for all precision-critical operations: < 5% of a 16.6 ms frame.

### CI integration

Add a CI step that runs `cargo bench` and stores the results. Use criterion's baseline comparison to detect regressions > 10%:

```bash
cargo bench --bench precision_benchmarks -- --save-baseline current
cargo bench --bench precision_benchmarks -- --baseline current --load-baseline previous
```

## Outcome

After this story is complete:

- A `criterion` 0.5 benchmark suite compiles and runs with `cargo bench --bench precision_benchmarks`
- All six precision-critical operations are benchmarked at realistic batch sizes
- An i128 vs f64 comparison benchmark quantifies the overhead of integer precision
- HTML reports are generated in `target/criterion/` for visual analysis
- Baseline performance data is established for regression detection
- Per-operation timing data validates that the i128 approach fits within real-time frame budgets
- CI can run benchmarks and flag regressions

## Demo Integration

**Demo crate:** `nebula-demo`

i128 math operations are benchmarked on the current hardware. Results display in the console: `IVec3_128 add: 2.1ns, distance: 8.3ns, world-to-local: 12ns`.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `criterion` | `0.5` | Statistical benchmarking framework with HTML reports |
| `glam` | `0.32` | f64 vector types for comparison benchmarks (`DVec3`) |

Rust edition 2024. Both crates are `dev-dependencies` only and do not affect the runtime binary.

## Unit Tests

Benchmarks are not unit tests, but the following smoke tests ensure the benchmark infrastructure compiles and the measured operations produce correct results:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_benchmarks_compile_and_run() {
        // Verify that each benchmarked operation can be called without panic
        let origin = WorldPosition::new(1_000_000, 2_000_000, 3_000_000);
        let positions = generate_positions(origin, 5_000_000, 100);
        let mut out = Vec::new();
        to_local_batch(&positions, origin, &mut out);
        assert_eq!(out.len(), 100);

        let pairs = generate_pairs(100);
        for (a, b) in &pairs {
            let _ = distance_squared(*a, *b);
        }

        let aabb_pairs = generate_aabb_pairs(100);
        for (a, b) in &aabb_pairs {
            let _ = a.intersects(b);
        }

        let pos = WorldPosition::new(999_999_999_999, -42, 0);
        let _ = SectorCoord::from_world(&pos);
    }

    #[test]
    fn test_benchmark_results_include_timing_data() {
        // This is a meta-test: run `cargo bench` and verify that
        // target/criterion/ directory is populated with reports.
        // In practice, this is validated by CI rather than a unit test.
        // Here we just verify the operations produce non-trivial results.
        let a = WorldPosition::new(3_000, 4_000, 0);
        let b = WorldPosition::new(0, 0, 0);
        let dist = distance_squared(a, b);
        assert_eq!(dist, 25_000_000); // 3000^2 + 4000^2 = 25_000_000
    }

    #[test]
    fn test_comparison_f64_produces_similar_results() {
        // Verify that i128 and f64 distance calculations agree for small values
        let a = WorldPosition::new(3_000, 4_000, 0);
        let b = WorldPosition::new(0, 0, 0);
        let i128_dist = distance_squared(a, b) as f64;

        let a_f64 = glam::DVec3::new(3_000.0, 4_000.0, 0.0);
        let b_f64 = glam::DVec3::new(0.0, 0.0, 0.0);
        let f64_dist = (a_f64 - b_f64).length_squared();

        assert!(
            (i128_dist - f64_dist).abs() < 1.0,
            "i128 and f64 distance must agree for small values"
        );
    }

    #[test]
    fn test_sector_conversion_benchmark_data_is_valid() {
        let pos = WorldPosition::new(5_000_000_000, -3_000_000_000, 1_000_000_000);
        let sector = SectorCoord::from_world(&pos);
        let recovered = sector.to_world();
        assert_eq!(recovered, pos, "Sector benchmark data must produce valid roundtrips");
    }
}
```
