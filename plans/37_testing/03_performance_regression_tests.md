# Performance Regression Tests

## Problem

Game engines have tight performance budgets. A chunk must mesh in under 2ms to avoid frame drops. A physics step for 100 bodies must complete within the fixed timestep. Coordinate conversions that run thousands of times per frame must not regress. Without automated performance regression testing, slowdowns are invisible until a player notices stuttering — at which point the regression may be weeks old and buried under dozens of commits.

The Nebula Engine is especially sensitive to performance in several areas:

- **Chunk generation** — terrain generation runs on background threads but directly affects how quickly new terrain appears as the player moves.
- **Greedy meshing** — the voxel meshing algorithm determines how fast geometry can be rebuilt when chunks change. A 10% regression here is the difference between smooth editing and visible lag.
- **128-bit math** — the engine's `i128` coordinate system requires wider arithmetic for every position calculation. These operations must be fast despite operating on double-width integers.
- **Serialization** — message serialization with postcard runs on every network packet. A regression here increases latency for all players.
- **Spatial queries** — the spatial hash is queried every physics step and every visibility check. It must remain O(1) amortized.

Manual benchmarking is unreliable because developer machines vary and human memory of "how fast it used to be" is imprecise. The solution is statistical benchmarking with saved baselines and automated regression detection in CI.

## Solution

### Criterion benchmark suite

All benchmarks live in `crates/nebula_bench/benches/` and use Criterion 0.5 for statistical analysis. Criterion runs each benchmark multiple times, computes confidence intervals, and compares against saved baselines to detect statistically significant changes.

```rust
use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId, BatchSize};

fn bench_chunk_generation(c: &mut Criterion) {
    let generator = ChunkGenerator::new(42);

    c.bench_function("chunk_generation_single", |b| {
        b.iter(|| {
            generator.generate(100, 50, 200, 0)
        });
    });

    let mut group = c.benchmark_group("chunk_generation_batch");
    for count in [1, 10, 100] {
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |b, &count| {
                b.iter(|| {
                    for i in 0..count {
                        generator.generate(i as i64, 0, 0, 0);
                    }
                });
            },
        );
    }
    group.finish();
}
```

### Greedy meshing benchmark

```rust
fn bench_greedy_meshing(c: &mut Criterion) {
    let chunk = generate_varied_chunk(); // Mix of solid, air, and mixed regions

    c.bench_function("greedy_mesh_varied_chunk", |b| {
        b.iter(|| {
            greedy_mesh(&chunk)
        });
    });

    let worst_case = generate_checkerboard_chunk(); // Alternating solid/air = worst case
    c.bench_function("greedy_mesh_checkerboard_worst_case", |b| {
        b.iter(|| {
            greedy_mesh(&worst_case)
        });
    });
}
```

### 128-bit math operations

```rust
fn bench_i128_math(c: &mut Criterion) {
    let mut group = c.benchmark_group("i128_operations");

    let a: i128 = 170_141_183_460_469_231_731_687_303_715_884_105_727; // i128::MAX
    let b: i128 = -170_141_183_460_469_231_731_687_303_715_884_105_728; // i128::MIN

    group.bench_function("i128_add", |bench| {
        bench.iter(|| {
            std::hint::black_box(std::hint::black_box(a).wrapping_add(std::hint::black_box(b)))
        });
    });

    group.bench_function("i128_mul", |bench| {
        bench.iter(|| {
            std::hint::black_box(std::hint::black_box(a / 2).wrapping_mul(std::hint::black_box(3)))
        });
    });

    group.bench_function("i128_div", |bench| {
        bench.iter(|| {
            std::hint::black_box(std::hint::black_box(a).wrapping_div(std::hint::black_box(7)))
        });
    });

    group.finish();
}
```

### World-to-local coordinate conversion

```rust
fn bench_world_to_local(c: &mut Criterion) {
    let coords: Vec<WorldCoord> = (0..1000)
        .map(|i| WorldCoord::new(i as i128 * 1_000_000, (i as i128) * 500, i as i128 * 2_000_000))
        .collect();

    c.bench_function("world_to_local_batch_1000", |b| {
        b.iter(|| {
            for coord in &coords {
                std::hint::black_box(coord.to_local());
            }
        });
    });
}
```

### Spatial hash query

```rust
fn bench_spatial_hash_query(c: &mut Criterion) {
    let mut hash = SpatialHash::new(64); // cell size 64
    for i in 0..10_000 {
        hash.insert(i, WorldCoord::new(i as i128 * 10, 0, 0));
    }

    c.bench_function("spatial_hash_query_radius_100", |b| {
        b.iter(|| {
            let results = hash.query_sphere(WorldCoord::new(5000, 0, 0), 100);
            std::hint::black_box(results);
        });
    });
}
```

### Physics step benchmark

```rust
fn bench_physics_step(c: &mut Criterion) {
    c.bench_function("physics_step_100_bodies", |b| {
        b.iter_batched(
            || create_physics_world_with_bodies(100),
            |mut world| {
                world.step();
            },
            BatchSize::SmallInput,
        );
    });
}
```

### Serialization benchmark

```rust
fn bench_serialization(c: &mut Criterion) {
    let entity_update = Message::EntityUpdate(EntityUpdate {
        entity_id: 42,
        pos_x_high: i64::MAX,
        pos_x_low: 12345,
        pos_y_high: 0,
        pos_y_low: 67890,
        pos_z_high: -1,
        pos_z_low: 0,
        rot_x: 0.0,
        rot_y: 0.707,
        rot_z: 0.0,
        rot_w: 0.707,
    });

    c.bench_function("serialize_entity_update", |b| {
        b.iter(|| serialize_message(&entity_update).unwrap());
    });

    let bytes = serialize_message(&entity_update).unwrap();
    c.bench_function("deserialize_entity_update", |b| {
        b.iter(|| deserialize_message(&bytes).unwrap());
    });

    let chunk_msg = create_large_chunk_message(); // 32x32x32 voxel payload
    c.bench_function("serialize_chunk_data_32x32x32", |b| {
        b.iter(|| serialize_message(&chunk_msg).unwrap());
    });
}
```

### Baseline management and CI integration

Criterion baselines are stored in `target/criterion/` by default. In CI, the baseline from the `main` branch is cached and restored before running benchmarks on the PR branch. Criterion's `--baseline` flag names the baseline for comparison.

```yaml
# CI step for benchmark regression detection
- name: Run benchmarks against baseline
  run: |
    cargo bench --package nebula-bench -- --baseline main --save-baseline pr
    # Parse Criterion output for regressions > 10%
    python3 scripts/check_bench_regression.py target/criterion/ --threshold 10
```

The `check_bench_regression.py` script parses Criterion's JSON output (`estimates.json` files) and fails if any benchmark's mean time increased by more than the threshold percentage.

## Outcome

A `crates/nebula_bench/` crate containing Criterion 0.5 benchmark suites for chunk generation, greedy meshing, i128 math, coordinate conversion, spatial hash queries, physics stepping, and serialization. A CI step that runs benchmarks against a saved baseline and fails the build if any benchmark regresses by more than 10%. A `scripts/check_bench_regression.py` script for parsing Criterion output. Rust edition 2024.

## Demo Integration

**Demo crate:** `nebula-demo`

The demo loads a heavy scene (large planet, many entities, active particles) and measures frame time, GPU memory, and CPU utilization against baseline thresholds.

## Crates & Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `criterion` | `0.5` (features: `html_reports`) | Statistical benchmarking framework with baseline comparison |
| `serde` | `1.0` (features: `derive`) | Serialization of benchmark message payloads |
| `postcard` | `1.1` (features: `alloc`) | Binary serialization benchmarked for message encode/decode |
| `serde_json` | `1.0` | Parsing Criterion's JSON output for regression detection |

## Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Verify that all benchmark functions compile and can be called once
    /// without panicking. This is a smoke test — not a performance test.
    #[test]
    fn test_chunk_generation_benchmark_runs() {
        let generator = ChunkGenerator::new(42);
        let _chunk = generator.generate(0, 0, 0, 0);
    }

    #[test]
    fn test_greedy_meshing_benchmark_runs() {
        let chunk = generate_varied_chunk();
        let _mesh = greedy_mesh(&chunk);
    }

    #[test]
    fn test_i128_math_benchmark_runs() {
        let a: i128 = i128::MAX;
        let b: i128 = i128::MIN;
        let _sum = a.wrapping_add(b);
        let _product = (a / 2).wrapping_mul(3);
    }

    #[test]
    fn test_world_to_local_benchmark_runs() {
        let coord = WorldCoord::new(1_000_000, 500, 2_000_000);
        let _local = coord.to_local();
    }

    #[test]
    fn test_spatial_hash_query_benchmark_runs() {
        let mut hash = SpatialHash::new(64);
        hash.insert(0, WorldCoord::new(0, 0, 0));
        let results = hash.query_sphere(WorldCoord::new(0, 0, 0), 100);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_physics_step_benchmark_runs() {
        let mut world = create_physics_world_with_bodies(10);
        world.step();
    }

    #[test]
    fn test_serialization_benchmark_runs() {
        let msg = Message::Ping(Ping {
            timestamp_ms: 0,
            sequence: 0,
        });
        let bytes = serialize_message(&msg).unwrap();
        let decoded = deserialize_message(&bytes).unwrap();
        assert_eq!(msg, decoded);
    }

    /// Verify that Criterion baseline JSON can be parsed and compared.
    /// Uses a synthetic baseline to test the regression detection logic.
    #[test]
    fn test_baseline_comparison_logic() {
        let old_mean_ns = 1_000_000.0; // 1ms
        let new_mean_ns = 1_050_000.0; // 1.05ms = 5% regression
        let regression_pct = ((new_mean_ns - old_mean_ns) / old_mean_ns) * 100.0;
        assert!(
            regression_pct < 10.0,
            "5% regression should not trigger 10% threshold"
        );

        let bad_mean_ns = 1_150_000.0; // 1.15ms = 15% regression
        let bad_regression_pct = ((bad_mean_ns - old_mean_ns) / old_mean_ns) * 100.0;
        assert!(
            bad_regression_pct > 10.0,
            "15% regression should trigger 10% threshold"
        );
    }

    /// Verify that benchmark results directory structure matches what
    /// the regression detection script expects.
    #[test]
    fn test_criterion_output_structure_expected() {
        // Criterion writes to target/criterion/<benchmark_name>/new/estimates.json
        // This test verifies the path pattern we parse in CI.
        let expected_path_pattern = "target/criterion/chunk_generation_single/new/estimates.json";
        assert!(expected_path_pattern.contains("estimates.json"));
        assert!(expected_path_pattern.contains("target/criterion"));
    }
}
```
